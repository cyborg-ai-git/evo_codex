//! Streaming, resumable downloads with pinned revisions and SHA-256 verification.
use codex_http_client::ClientRouteClass;
use codex_http_client::HttpClientFactory;
use codex_http_client::RouteAwareClientPool;
use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::io;
use std::path::Component;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
pub(crate) struct Manifest {
    repo: String,
    revision: String,
    files: Vec<String>,
    pub(crate) sizes: BTreeMap<String, u64>,
    sha256: BTreeMap<String, String>,
    #[serde(default)]
    sources: BTreeMap<String, String>,
}

fn safe_path(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

async fn verified(path: &Path, size: u64, hash: &str) -> io::Result<bool> {
    let Ok(metadata) = tokio::fs::symlink_metadata(path).await else {
        return Ok(false);
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::other(
            "Model files must be regular files, not symlinks",
        ));
    }
    if metadata.len() != size {
        return Ok(false);
    }
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()) == hash)
}

pub(crate) async fn download(
    directory: &Path,
    manifest: &Manifest,
    factory: HttpClientFactory,
    progress: impl Fn(String) + Send + Sync,
) -> Result<(), String> {
    download_inner(
        directory,
        manifest,
        factory,
        "https://huggingface.co",
        progress,
    )
    .await
    .map_err(|error| error.to_string())
}

async fn download_inner(
    directory: &Path,
    manifest: &Manifest,
    factory: HttpClientFactory,
    origin: &str,
    progress: impl Fn(String) + Send + Sync,
) -> io::Result<()> {
    for name in &manifest.files {
        let source = manifest.sources.get(name).unwrap_or(name);
        if !safe_path(name)
            || Path::new(name).components().count() != 1
            || !safe_path(source)
            || !manifest.sizes.contains_key(name)
            || !manifest.sha256.contains_key(name)
        {
            return Err(io::Error::other("Invalid pinned model manifest"));
        }
    }
    tokio::fs::create_dir_all(directory).await?;
    let lock_path = directory.join(".evo-download.lock");
    if std::fs::symlink_metadata(&lock_path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(io::Error::other("Download lock cannot be a symlink"));
    }
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock().map_err(|_| {
        io::Error::other("This model is already being downloaded by another process")
    })?;
    let client = RouteAwareClientPool::with_connect_timeout(
        factory,
        ClientRouteClass::Other,
        Duration::from_secs(15),
    );
    let total = manifest.sizes.values().sum::<u64>();
    let mut completed = 0;
    let mut tick = Instant::now();
    for name in &manifest.files {
        progress(format!(
            "Verifying {name} · {:.1}%",
            completed as f64 * 100.0 / total as f64
        ));
        let path = directory.join(name);
        let size = manifest.sizes[name];
        if verified(&path, size, &manifest.sha256[name]).await? {
            completed += size;
            continue;
        }
        if path.exists() {
            return Err(io::Error::other(format!(
                "Existing file {name} failed verification. Move it aside before retrying; it has not been overwritten."
            )));
        }
        tokio::fs::write(directory.join(".evo-incomplete"), &manifest.revision).await?;
        let part = directory.join(format!("{name}.part"));
        if std::fs::symlink_metadata(&part)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err(io::Error::other("Partial download must be a regular file"));
        }
        let mut offset = tokio::fs::metadata(&part)
            .await
            .map_or(0, |metadata| metadata.len());
        if offset > size {
            tokio::fs::remove_file(&part).await?;
            offset = 0;
        }
        if offset < size {
            let source = manifest.sources.get(name).unwrap_or(name);
            let url = format!(
                "{origin}/{}/resolve/{}/{source}",
                manifest.repo, manifest.revision
            );
            let mut request = client.get(url);
            if offset > 0 {
                request = request.header(http::header::RANGE, format!("bytes={offset}-"));
            }
            let mut response = tokio::time::timeout(Duration::from_secs(60), request.send())
                .await
                .map_err(io::Error::other)?
                .map_err(io::Error::other)?;
            if response.status() == http::StatusCode::OK {
                offset = 0;
            } else if response.status() == http::StatusCode::PARTIAL_CONTENT {
                let expected = format!("bytes {offset}-{}/{size}", size - 1);
                if response
                    .headers()
                    .get(http::header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    != Some(expected.as_str())
                {
                    return Err(io::Error::other(
                        "Unexpected Content-Range; partial file preserved",
                    ));
                }
            } else {
                return Err(io::Error::other(format!(
                    "Model download returned HTTP {}",
                    response.status()
                )));
            }
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(offset == 0)
                .append(offset > 0)
                .open(&part)
                .await?;
            loop {
                let chunk = tokio::time::timeout(Duration::from_secs(60), response.chunk())
                    .await
                    .map_err(io::Error::other)?
                    .map_err(io::Error::other)?;
                let Some(chunk) = chunk else { break };
                if offset + chunk.len() as u64 > size {
                    return Err(io::Error::other("Download exceeds pinned size"));
                }
                file.write_all(&chunk).await?;
                offset += chunk.len() as u64;
                if tick.elapsed() >= Duration::from_millis(200) {
                    progress(format!(
                        "Downloading {name} · {:.1}% · {:.2}/{:.2} GiB",
                        (completed + offset) as f64 * 100.0 / total as f64,
                        (completed + offset) as f64 / 1_073_741_824.0,
                        total as f64 / 1_073_741_824.0
                    ));
                    tick = Instant::now();
                }
            }
            file.flush().await?;
            file.sync_all().await?;
        }
        progress(format!("Verifying SHA-256: {name}"));
        if !verified(&part, size, &manifest.sha256[name]).await? {
            tokio::fs::remove_file(&part).await?;
            return Err(io::Error::other(format!(
                "Integrity check failed for {name}; retry the download"
            )));
        }
        tokio::fs::rename(&part, &path).await?;
        completed += size;
    }
    let marker = directory.join(".evo-incomplete");
    if marker.exists() {
        tokio::fs::remove_file(marker).await?;
    }
    progress("Model verified and ready".into());
    Ok(())
}

#[cfg(test)]
#[path = "model_download_tests.rs"]
mod tests;
