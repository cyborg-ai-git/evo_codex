//! Lifetime-managed startup of an already installed external MLX server.

use crate::mlx_models::DEFAULT_CONTEXT;
use crate::mlx_models::models_root;
use codex_model_provider_info::ModelProviderInfo;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Debug, Default)]
pub(crate) struct MlxRuntime {
    child: Mutex<Option<Child>>,
}

#[cfg(test)]
#[path = "mlx_runtime_tests.rs"]
mod tests;

async fn is_listening(address: SocketAddr) -> bool {
    tokio::task::spawn_blocking(move || {
        std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok()
    })
    .await
    .unwrap_or(false)
}

impl MlxRuntime {
    // Serialize startup and readiness checks so concurrent requests share one owned server.
    #[allow(clippy::await_holding_invalid_type)]
    pub(crate) async fn ensure_ready(&self, provider: &ModelProviderInfo) -> io::Result<()> {
        let Some(address) = provider
            .base_url
            .as_deref()
            .and_then(crate::mlx_models::loopback_address)
        else {
            return Ok(());
        };
        let mut child = self.child.lock().await;
        if is_listening(address).await {
            return Ok(());
        }
        if let Some(process) = child.as_mut() {
            if let Some(status) = process.try_wait()? {
                *child = None;
                return Err(io::Error::other(format!(
                    "MLX server exited with {status}; inspect evo-codex-mlx.log in the model directory"
                )));
            }
        } else {
            if std::env::var("CODEX_MLX_AUTO_START").as_deref() == Ok("0") {
                return Err(io::Error::other(
                    "MLX server is stopped and automatic startup is disabled",
                ));
            }
            let root = models_root().ok_or_else(|| {
                io::Error::other("Set CODEX_MLX_MODEL_DIR to your local model directory")
            })?;
            let python = std::env::var_os("CODEX_MLX_PYTHON")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join(".venv/bin/python"));
            if !python.is_file() {
                return Err(io::Error::other(
                    "MLX runtime not found. Set CODEX_MLX_PYTHON to an existing MLX-VLM Python executable, or start your MLX server separately.",
                ));
            }
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join("evo-codex-mlx.log"))?;
            let mut command = Command::new(python);
            command
                .args([
                    "-m",
                    "mlx_vlm.server",
                    "--host",
                    &address.ip().to_string(),
                    "--port",
                    &address.port().to_string(),
                    "--max-kv-size",
                    &DEFAULT_CONTEXT.to_string(),
                    "--max-tokens",
                    "2048",
                    "--max-num-seqs",
                    "1",
                    "--prefill-step-size",
                    "256",
                ])
                .env("HF_HUB_OFFLINE", "1")
                .env("TOKENIZERS_PARALLELISM", "false")
                .stdin(Stdio::null())
                .stdout(log.try_clone()?)
                .stderr(log)
                .kill_on_drop(true);
            if let Some(key) = provider
                .api_key()
                .map_err(io::Error::other)?
                .or_else(|| provider.experimental_bearer_token.as_deref().cloned())
            {
                command.env("MLX_VLM_SERVER_API_KEY", key);
            }
            *child = Some(command.spawn()?);
        }
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let process = child.as_mut().ok_or_else(|| io::Error::other("MLX process is no longer available"))?;
                if let Some(status) = process.try_wait()? {
                    return Err(io::Error::other(format!("MLX startup failed with {status}; inspect evo-codex-mlx.log in the model directory")));
                }
                if is_listening(address).await {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }).await.unwrap_or_else(|_| Err(io::Error::other("MLX startup timed out; inspect evo-codex-mlx.log in the model directory")));
        if result.is_err()
            && let Some(mut process) = child.take()
        {
            let _ = process.kill().await;
        }
        result
    }
}
