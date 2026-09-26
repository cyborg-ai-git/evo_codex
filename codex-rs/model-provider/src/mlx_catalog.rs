//! Pinned, built-in model downloads. Users never need to maintain a JSON catalog.
use crate::mlx_models::models_root;
use crate::model_download::Manifest;
use codex_http_client::HttpClientFactory;
use codex_protocol::openai_models::ModelInfo;
use std::path::Path;

const CATALOG: [(&str, u64, &str); 5] = [
    (
        "Qwen3.5-2B-4bit",
        4,
        include_str!("../manifests/qwen35-2.json"),
    ),
    (
        "Qwen3.5-4B-4bit",
        6,
        include_str!("../manifests/qwen35-4.json"),
    ),
    (
        "Qwen3.8-27B-Uncensored-MLX-4bit",
        20,
        include_str!("../manifests/qwen27-4.json"),
    ),
    (
        "Qwen3.8-27B-Uncensored-MLX-6bit",
        32,
        include_str!("../manifests/qwen27-6.json"),
    ),
    (
        "Qwen3.8-27B-Uncensored-MLX-8bit",
        40,
        include_str!("../manifests/qwen27-8.json"),
    ),
];

pub(crate) fn add_downloads(root: &Path, models: &mut Vec<ModelInfo>) {
    for (name, ram, json) in CATALOG {
        let path = root.join(name);
        if models.iter().any(|model| Path::new(&model.slug) == path) {
            continue;
        }
        let Ok(manifest) = serde_json::from_str::<Manifest>(json) else {
            continue;
        };
        let mut model = codex_models_manager::deepseek::local_model_info(&path.to_string_lossy());
        model.display_name = name.into();
        model.context_window = Some(crate::mlx_models::DEFAULT_CONTEXT);
        let gib = manifest.sizes.values().sum::<u64>() as f64 / 1_073_741_824.0;
        model.description = Some(format!(
            "Download {gib:.1} GiB · at least {ram} GiB RAM · Apple Silicon / MLX"
        ));
        models.push(model);
    }
}

/// Whether selection refers to a checkpoint in the built-in local download catalog.
pub fn is_downloadable_mlx_model(model: &str) -> bool {
    models_root().is_some_and(|root| {
        CATALOG
            .iter()
            .any(|(name, _, _)| root.join(name) == Path::new(model))
    })
}

/// Download only the explicitly selected model, preserving resumable partial files on cancellation.
pub async fn prepare_mlx_model(
    model: &str,
    factory: HttpClientFactory,
    progress: impl Fn(String) + Send + Sync,
) -> Result<(), String> {
    let root = models_root().ok_or("Local model directory is not configured")?;
    let (_, ram, json) = CATALOG
        .iter()
        .find(|(name, _, _)| root.join(name) == Path::new(model))
        .ok_or("Model is not in the built-in download catalog")?;
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err("These MLX checkpoints require Apple Silicon. Use Ollama or LM Studio on other systems.".into());
    }
    let memory = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "hw.memsize"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Reading system memory timed out")?
    .map_err(|error| error.to_string())?;
    let bytes = String::from_utf8_lossy(&memory.stdout)
        .trim()
        .parse::<u64>()
        .map_err(|_| "Cannot determine installed RAM")?;
    if bytes < ram * 1_073_741_824 {
        return Err(format!(
            "This model requires at least {ram} GiB RAM; choose a smaller model."
        ));
    }
    let manifest = serde_json::from_str::<Manifest>(json).map_err(|error| error.to_string())?;
    let mut remaining = 0u64;
    for (name, size) in &manifest.sizes {
        if !Path::new(model).join(name).is_file() {
            let partial = std::fs::metadata(Path::new(model).join(format!("{name}.part")))
                .map_or(0, |metadata| metadata.len().min(*size));
            remaining += size - partial;
        }
    }
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|error| error.to_string())?;
    let disk = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::process::Command::new("/bin/df")
            .arg("-Pk")
            .arg(&root)
            .env("LC_ALL", "C")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Reading available disk space timed out")?
    .map_err(|error| error.to_string())?;
    let free = String::from_utf8_lossy(&disk.stdout)
        .lines()
        .last()
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or("Cannot determine available disk space")?
        * 1024;
    if free < remaining.saturating_add(512 * 1024 * 1024) {
        return Err("Insufficient disk space for this model and a 512 MiB reserve.".into());
    }
    crate::model_download::download(Path::new(model), &manifest, factory, progress).await
}
