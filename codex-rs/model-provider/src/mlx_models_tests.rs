use super::*;
use pretty_assertions::assert_eq;

#[test]
fn discovers_complete_checkpoints_and_rejects_partial_or_escaping_shards() {
    let root = tempfile::tempdir().unwrap();
    let model = root.path().join("custom-uncensored");
    fs::create_dir(&model).unwrap();
    for (name, body) in [
        ("config.json", r#"{"quantization":{"bits":4}}"#),
        ("tokenizer_config.json", "{}"),
        ("tokenizer.json", "{}"),
        ("model.safetensors", "weights"),
    ] {
        fs::write(model.join(name), body).unwrap();
    }
    assert_eq!(
        discover_checkpoints(root.path())
            .into_iter()
            .map(|m| m.display_name)
            .collect::<Vec<_>>(),
        vec!["custom-uncensored"]
    );
    fs::write(model.join(".evo-incomplete"), "pending").unwrap();
    assert!(discover_checkpoints(root.path()).is_empty());
    fs::remove_file(model.join(".evo-incomplete")).unwrap();
    fs::write(
        model.join("model.safetensors.index.json"),
        r#"{"weight_map":{"a":"../model.safetensors"}}"#,
    )
    .unwrap();
    assert!(discover_checkpoints(root.path()).is_empty());
}

#[test]
fn offline_catalog_includes_downloads_without_duplicate_installed_models() {
    let root = tempfile::tempdir().unwrap();
    let slug = root
        .path()
        .join("Qwen3.5-2B-4bit")
        .to_string_lossy()
        .into_owned();
    let mut models = vec![codex_models_manager::deepseek::local_model_info(&slug)];
    crate::mlx_catalog::add_downloads(root.path(), &mut models);
    assert_eq!(models.iter().filter(|model| model.slug == slug).count(), 1);
    assert!(models.iter().any(|model| {
        model
            .description
            .as_deref()
            .is_some_and(|description| description.starts_with("Download"))
    }));
}

#[test]
fn startup_is_limited_to_plain_loopback_endpoints() {
    assert!(loopback_address("http://127.0.0.1:8080/v1").is_some());
    for url in [
        "https://127.0.0.1/v1",
        "http://example.com/v1",
        "http://user@localhost:8080/v1",
    ] {
        assert!(loopback_address(url).is_none());
    }
}
