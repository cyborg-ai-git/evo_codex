//! Discovery of complete local MLX checkpoints without a user-maintained catalog.

use codex_protocol::openai_models::ModelInfo;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use url::Url;

pub(crate) const DEFAULT_CONTEXT: i64 = 32_768;

pub(crate) fn models_root() -> Option<PathBuf> {
    std::env::var_os("CODEX_MLX_MODEL_DIR")
        .or_else(|| std::env::var_os("QWEN_HOME"))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Documents/Model"))
        })
        .and_then(|path| {
            let absolute = if path.is_absolute() {
                path
            } else {
                std::env::current_dir().ok()?.join(path)
            };
            Some(absolute.canonicalize().unwrap_or(absolute))
        })
}

pub(crate) fn loopback_address(base: &str) -> Option<std::net::SocketAddr> {
    let url = Url::parse(base).ok()?;
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let ip = match url.host()? {
        url::Host::Domain("localhost") => std::net::Ipv4Addr::LOCALHOST.into(),
        url::Host::Ipv4(ip) if ip.is_loopback() => ip.into(),
        url::Host::Ipv6(ip) if ip.is_loopback() => ip.into(),
        _ => return None,
    };
    Some(std::net::SocketAddr::new(ip, url.port_or_known_default()?))
}

fn read_json(path: &Path) -> Option<Value> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > 1024 * 1024 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn checkpoint(path: &Path) -> Option<ModelInfo> {
    if path.join(".evo-incomplete").exists() {
        return None;
    }
    let config = read_json(&path.join("config.json"))?;
    // MLX quantization metadata distinguishes these checkpoints from arbitrary HF weights.
    if config.get("quantization").is_none() && config.get("quantization_config").is_none() {
        return None;
    }
    if !path.join("tokenizer_config.json").is_file() || !path.join("tokenizer.json").is_file() {
        return None;
    }
    let index = path.join("model.safetensors.index.json");
    let weights: BTreeSet<String> = if index.exists() {
        read_json(&index)?
            .get("weight_map")?
            .as_object()?
            .values()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<_>>()?
    } else {
        BTreeSet::from(["model.safetensors".into()])
    };
    if weights.is_empty()
        || weights.iter().any(|name| {
            let mut components = Path::new(name).components();
            !matches!(
                (components.next(), components.next()),
                (Some(Component::Normal(_)), None)
            ) || !name.ends_with(".safetensors")
                || !fs::metadata(path.join(name))
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
        })
    {
        return None;
    }
    let path = path.canonicalize().ok()?;
    let mut model = codex_models_manager::deepseek::local_model_info(path.to_str()?);
    model.display_name = path.file_name()?.to_str()?.to_owned();
    model.description =
        Some("On disk · loads on the first request using the installed MLX runtime".into());
    model.context_window = Some(DEFAULT_CONTEXT);
    Some(model)
}

pub(crate) fn discover_checkpoints(root: &Path) -> Vec<ModelInfo> {
    if let Some(model) = checkpoint(root) {
        return vec![model];
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut models: Vec<_> = entries
        .take(256)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| checkpoint(&entry.path()))
        .collect();
    models.sort_by(|left, right| left.slug.cmp(&right.slug));
    models.dedup_by(|left, right| left.slug == right.slug);
    models
}

#[cfg(test)]
#[path = "mlx_models_tests.rs"]
mod tests;
