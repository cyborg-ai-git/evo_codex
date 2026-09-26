//! Discover local OpenAI-compatible models without a hand-written catalog.

use codex_http_client::ClientRouteClass;
use codex_http_client::HttpClientFactory;
use codex_http_client::RouteAwareClientPool;
use codex_login::AuthManager;
use codex_model_provider_info::ModelProviderInfo;
use codex_models_manager::ModelsManagerConfig;
use codex_models_manager::manager::ModelsManager;
use codex_models_manager::manager::ModelsManagerFuture;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelsResponse;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::sync::TryLockError;

#[derive(Debug)]
pub(crate) struct LocalModelsManager {
    provider: ModelProviderInfo,
    models: RwLock<Vec<ModelInfo>>,
}

impl LocalModelsManager {
    pub(crate) fn new(provider: ModelProviderInfo) -> Self {
        Self {
            provider,
            models: RwLock::new(Vec::new()),
        }
    }
}

/// Query a server's model IDs with a bounded download and deadline.
/// Names are not filtered: fine-tuned and uncensored variants remain selectable.
pub async fn discover_local_models(
    provider: &ModelProviderInfo,
    factory: HttpClientFactory,
) -> Result<Vec<ModelInfo>, String> {
    let is_mlx = provider.name == codex_model_provider_info::MLX_PROVIDER_NAME;
    let local_root = is_mlx
        .then(|| {
            provider
                .base_url
                .as_deref()
                .and_then(crate::mlx_models::loopback_address)
        })
        .flatten()
        .and_then(|_| crate::mlx_models::models_root());
    let mut disk_models = if let Some(root) = local_root {
        tokio::task::spawn_blocking(move || {
            let mut models = crate::mlx_models::discover_checkpoints(&root);
            crate::mlx_catalog::add_downloads(&root, &mut models);
            models
        })
        .await
        .map_err(|_| "Local checkpoint discovery failed")?
    } else {
        Vec::new()
    };
    let (served, context_limit) = match discover_served_models(provider, factory).await {
        Ok(models) => models,
        Err(error) if !disk_models.is_empty() => {
            tracing::debug!(%error, "Showing downloaded MLX checkpoints while the server is unavailable");
            return Ok(disk_models);
        }
        Err(error) => return Err(error),
    };
    for mut model in served {
        if is_mlx {
            model.description = Some("Available from the MLX server".into());
        }
        if let Some(index) = disk_models.iter().position(|disk| disk.slug == model.slug) {
            model.display_name = disk_models[index].display_name.clone();
            disk_models[index] = model;
        } else {
            disk_models.push(model);
        }
    }
    if let Some(limit) = context_limit {
        for model in &mut disk_models {
            model.context_window = Some(limit);
        }
    }
    disk_models.sort_by(|left, right| left.slug.cmp(&right.slug));
    Ok(disk_models)
}

async fn discover_served_models(
    provider: &ModelProviderInfo,
    factory: HttpClientFactory,
) -> Result<(Vec<ModelInfo>, Option<i64>), String> {
    let base_url = provider
        .base_url
        .as_deref()
        .ok_or("Local provider has no base URL")?;
    let client = RouteAwareClientPool::with_connect_timeout(
        factory,
        ClientRouteClass::Other,
        Duration::from_secs(3),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        let auth = crate::create_model_provider(provider.clone(), /*auth_manager*/ None)
            .api_auth().await.map_err(|error| error.to_string())?;
        let mut headers = provider.to_api_provider(/*auth_mode*/ None).map_err(|error| error.to_string())?.headers;
        headers.extend(auth.resolve_auth_headers().await.map_err(|_| "Local model authentication failed")?);
        let mut response = client
            .get(format!("{}/models", base_url.trim_end_matches('/')))
            .headers(headers.clone())
            .send()
            .await
            .map_err(|_| format!("Cannot connect to the {} model server. Start the server, check its endpoint, then retry /model.", provider.name))?;
        if !response.status().is_success() {
            return Err(format!(
                "Local model discovery returned HTTP {}",
                response.status()
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Cannot read local model catalog")?
        {
            if body.len() + chunk.len() > 1024 * 1024 {
                return Err("Local model catalog exceeds 1 MiB".into());
            }
            body.extend_from_slice(&chunk);
        }
        let models = decode_models(&body)?;
        let context = if provider.name == codex_model_provider_info::MLX_PROVIDER_NAME {
            tokio::time::timeout(Duration::from_secs(1), async {
                let url = url::Url::parse(base_url).ok()?.join("/health").ok()?;
                let mut response = client.get(url).headers(headers).send().await.ok()?;
                if !response.status().is_success() { return None; }
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.ok()? {
                    if body.len() + chunk.len() > 64 * 1024 { return None; }
                    body.extend_from_slice(&chunk);
                }
                serde_json::from_slice::<serde_json::Value>(&body).ok()?
                    .get("configured_context_limit")?.as_i64()
                    .filter(|limit| *limit > 0)
                    .map(|limit| limit.min(crate::mlx_models::DEFAULT_CONTEXT))
            }).await.ok().flatten()
        } else { None };
        Ok((models, context))
    })
    .await
    .map_err(|_| "Local model discovery timed out".to_string())?
}

fn decode_models(body: &[u8]) -> Result<Vec<ModelInfo>, String> {
    #[derive(Deserialize)]
    struct Catalog {
        data: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }
    let catalog: Catalog =
        serde_json::from_slice(body).map_err(|_| "Invalid local /models response")?;
    let mut ids: Vec<_> = catalog
        .data
        .into_iter()
        .map(|model| model.id)
        .filter(|id| !id.trim().is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids
        .into_iter()
        .map(|id| codex_models_manager::deepseek::local_model_info(&id))
        .collect())
}

impl ModelsManager for LocalModelsManager {
    fn raw_model_catalog(
        &self,
        strategy: RefreshStrategy,
        factory: HttpClientFactory,
    ) -> ModelsManagerFuture<'_, ModelsResponse> {
        Box::pin(async move {
            if strategy == RefreshStrategy::Online
                || (strategy == RefreshStrategy::OnlineIfUncached
                    && self.models.read().await.is_empty())
            {
                match discover_local_models(&self.provider, factory).await {
                    Ok(models) => *self.models.write().await = models,
                    Err(error) => tracing::warn!(%error, "Local model discovery failed"),
                }
            }
            ModelsResponse {
                models: self.models.read().await.clone(),
            }
        })
    }

    fn get_remote_models(&self) -> ModelsManagerFuture<'_, Vec<ModelInfo>> {
        Box::pin(async { self.models.read().await.clone() })
    }

    fn try_get_remote_models(&self) -> Result<Vec<ModelInfo>, TryLockError> {
        Ok(self.models.try_read()?.clone())
    }

    fn auth_manager(&self) -> Option<&AuthManager> {
        None
    }

    fn refresh_if_new_etag(
        &self,
        _etag: String,
        _factory: HttpClientFactory,
    ) -> ModelsManagerFuture<'_, ()> {
        Box::pin(async {})
    }

    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask> {
        codex_models_manager::collaboration_mode_presets::builtin_collaboration_mode_presets()
    }

    fn get_model_info<'a>(
        &'a self,
        model: &'a str,
        config: &'a ModelsManagerConfig,
    ) -> ModelsManagerFuture<'a, ModelInfo> {
        Box::pin(async move {
            let info = self
                .models
                .read()
                .await
                .iter()
                .find(|info| info.slug == model)
                .cloned()
                .unwrap_or_else(|| codex_models_manager::deepseek::local_model_info(model));
            codex_models_manager::model_info::with_config_overrides(info, config)
        })
    }
}

#[cfg(test)]
#[path = "local_models_tests.rs"]
mod tests;
