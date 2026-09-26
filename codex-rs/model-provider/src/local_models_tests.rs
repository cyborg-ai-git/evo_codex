use super::*;
use codex_http_client::OutboundProxyPolicy;
use pretty_assertions::assert_eq;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

#[test]
fn discovers_custom_names_without_duplicates_or_control_characters() {
    let models = decode_models(br#"{"data":[{"id":"deepseek-r1:14b"},{"id":"custom/uncensored:latest"},{"id":"deepseek-r1:14b"},{"id":""},{"id":"bad\nname"}]}"#).unwrap();
    assert_eq!(
        models
            .into_iter()
            .map(|model| model.slug)
            .collect::<Vec<_>>(),
        vec!["custom/uncensored:latest", "deepseek-r1:14b"]
    );
    assert!(decode_models(br#"{"models":[]}"#).is_err());
}

#[tokio::test]
async fn local_discovery_uses_models_endpoint_and_keeps_offline_catalog() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data":[{"id":"deepseek-r1:14b"}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = codex_model_provider_info::create_oss_provider_with_base_url(
        &format!("{}/v1", server.uri()),
        codex_model_provider_info::WireApi::Responses,
    );
    let manager = LocalModelsManager::new(provider);
    let factory = HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault);
    let online = manager
        .raw_model_catalog(RefreshStrategy::Online, factory.clone())
        .await;
    let offline = manager
        .raw_model_catalog(RefreshStrategy::Offline, factory)
        .await;
    assert_eq!(online, offline);
    assert_eq!(offline.models.len(), 1);
    let config = ModelsManagerConfig {
        model_context_window: Some(8192),
        ..Default::default()
    };
    let info = manager.get_model_info("deepseek-r1:14b", &config).await;
    assert_eq!(info.context_window, Some(8192));
    assert!(!info.supports_reasoning_summary_parameter);
}

#[tokio::test]
async fn rejects_http_errors_and_oversized_local_catalogs() {
    for (status, body) in [
        (401, "unauthorized".to_string()),
        (200, "x".repeat(1024 * 1024 + 1)),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;
        let provider = codex_model_provider_info::create_oss_provider_with_base_url(
            &server.uri(),
            codex_model_provider_info::WireApi::Responses,
        );
        assert!(
            discover_local_models(
                &provider,
                HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault)
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn mlx_discovery_uses_server_context_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data":[{"id":"test-mlx"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"configured_context_limit":8192})),
        )
        .mount(&server)
        .await;
    let mut provider =
        codex_model_provider_info::built_in_model_providers(/*openai_base_url*/ None)
            .remove("mlx")
            .unwrap();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    let (models, context) = discover_served_models(
        &provider,
        HttpClientFactory::new(OutboundProxyPolicy::ReqwestDefault),
    )
    .await
    .unwrap();
    assert_eq!((models[0].slug.as_str(), context), ("test-mlx", Some(8192)));
}
