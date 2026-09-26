use super::*;
use codex_http_client::OutboundProxyPolicy;
use pretty_assertions::assert_eq;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[tokio::test]
async fn resumes_partial_download_and_verifies_before_rename() {
    let server = MockServer::start().await;
    let directory = tempfile::tempdir().unwrap();
    let bytes = b"model weights";
    let manifest: Manifest = serde_json::from_value(serde_json::json!({
        "repo": "owner/model", "revision": "pinned", "files": ["weights"],
        "sizes": {"weights": bytes.len()}, "sha256": {"weights": format!("{:x}", Sha256::digest(bytes))}
    })).unwrap();
    tokio::fs::write(directory.path().join("weights.part"), &bytes[..5])
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/owner/model/resolve/pinned/weights"))
        .and(header("range", "bytes=5-"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("Content-Range", "bytes 5-12/13")
                .set_body_bytes(bytes[5..].to_vec()),
        )
        .expect(1)
        .mount(&server)
        .await;
    download_inner(
        directory.path(),
        &manifest,
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        &server.uri(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(
        tokio::fs::read(directory.path().join("weights"))
            .await
            .unwrap(),
        bytes
    );
    assert!(!directory.path().join("weights.part").exists());
    assert!(!directory.path().join(".evo-incomplete").exists());
    download_inner(
        directory.path(),
        &manifest,
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
        &server.uri(),
        |_| {},
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn rejects_bad_hash_without_publishing_checkpoint() {
    let server = MockServer::start().await;
    let directory = tempfile::tempdir().unwrap();
    let manifest = Manifest {
        repo: "owner/model".into(),
        revision: "pinned".into(),
        files: vec!["weights".into()],
        sizes: BTreeMap::from([("weights".into(), 3)]),
        sha256: BTreeMap::from([("weights".into(), "bad-hash".into())]),
        sources: BTreeMap::from([("weights".into(), "weights".into())]),
    };
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("bad"))
        .mount(&server)
        .await;
    assert!(
        download_inner(
            directory.path(),
            &manifest,
            HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
            &server.uri(),
            |_| {}
        )
        .await
        .is_err()
    );
    assert!(!directory.path().join("weights").exists());
    assert!(directory.path().join(".evo-incomplete").exists());
}

#[tokio::test]
async fn server_ignoring_range_restarts_partial_file_and_bad_ranges_are_rejected() {
    for status in [200, 206] {
        let server = MockServer::start().await;
        let directory = tempfile::tempdir().unwrap();
        let bytes = b"new weights";
        let manifest = Manifest {
            repo: "owner/model".into(),
            revision: "pinned".into(),
            files: vec!["weights".into()],
            sizes: BTreeMap::from([("weights".into(), bytes.len() as u64)]),
            sha256: BTreeMap::from([("weights".into(), format!("{:x}", Sha256::digest(bytes)))]),
            sources: BTreeMap::from([("weights".into(), "weights".into())]),
        };
        tokio::fs::write(directory.path().join("weights.part"), b"old")
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(header("range", "bytes=3-"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("Content-Range", "bytes 0-10/11")
                    .set_body_bytes(bytes.to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = download_inner(
            directory.path(),
            &manifest,
            HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
            &server.uri(),
            |_| {},
        )
        .await;
        if status == 200 {
            result.unwrap();
            assert_eq!(
                tokio::fs::read(directory.path().join("weights"))
                    .await
                    .unwrap(),
                bytes
            );
        } else {
            assert!(result.is_err());
            assert_eq!(
                tokio::fs::read(directory.path().join("weights.part"))
                    .await
                    .unwrap(),
                b"old"
            );
            assert!(!directory.path().join("weights").exists());
        }
    }
}

#[tokio::test]
async fn rejects_concurrent_downloads_and_never_overwrites_existing_corrupt_files() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = Manifest {
        repo: "owner/model".into(),
        revision: "pinned".into(),
        files: vec!["weights".into()],
        sizes: BTreeMap::from([("weights".into(), 3)]),
        sha256: BTreeMap::from([("weights".into(), "bad-hash".into())]),
        sources: BTreeMap::from([("weights".into(), "weights".into())]),
    };
    let lock = std::fs::File::create(directory.path().join(".evo-download.lock")).unwrap();
    lock.try_lock().unwrap();
    let factory = HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault);
    let result = download_inner(
        directory.path(),
        &manifest,
        factory.clone(),
        "http://127.0.0.1:1",
        |_| {},
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("already being downloaded")
    );
    drop(lock);
    tokio::fs::write(directory.path().join("weights"), b"old")
        .await
        .unwrap();
    assert!(
        download_inner(
            directory.path(),
            &manifest,
            factory,
            "http://127.0.0.1:1",
            |_| {}
        )
        .await
        .is_err()
    );
    assert_eq!(
        tokio::fs::read(directory.path().join("weights"))
            .await
            .unwrap(),
        b"old"
    );
}

#[tokio::test]
#[ignore = "Downloads five small pinned config files from Hugging Face"]
async fn live_hugging_face_download_verifies_pinned_config() {
    for json in [
        include_str!("../manifests/qwen35-2.json"),
        include_str!("../manifests/qwen35-4.json"),
        include_str!("../manifests/qwen27-4.json"),
        include_str!("../manifests/qwen27-6.json"),
        include_str!("../manifests/qwen27-8.json"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut manifest: Manifest = serde_json::from_str(json).unwrap();
        manifest.files.retain(|name| name == "config.json");
        manifest.sizes.retain(|name, _| name == "config.json");
        manifest.sha256.retain(|name, _| name == "config.json");
        manifest.sources.retain(|name, _| name == "config.json");
        download(
            directory.path(),
            &manifest,
            HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::metadata(directory.path().join("config.json"))
                .await
                .unwrap()
                .len(),
            manifest.sizes["config.json"]
        );
    }
}
