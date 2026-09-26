//! Native DeepSeek uses the standard agent loop and keeps provider credentials isolated.

use codex_model_provider_info::DEEPSEEK_PROVIDER_ID;
use codex_model_provider_info::built_in_model_providers;
use core_test_support::responses;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deepseek_native_catalog_and_responses_transport() -> anyhow::Result<()> {
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("response-tool"),
                responses::ev_function_call(
                    "call-deepseek",
                    "exec_command",
                    r#"{"cmd":"echo deepseek-native","max_output_tokens":100}"#,
                ),
                responses::ev_completed("response-tool"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("response-deepseek"),
                responses::ev_assistant_message("message-deepseek", "Ready."),
                responses::ev_completed("response-deepseek"),
            ]),
        ],
    )
    .await;
    let base_url = server.uri();
    let mut builder = test_codex()
        .with_model("deepseek-flash")
        .with_config(move |config| {
            let mut provider = built_in_model_providers(/*openai_base_url*/ None)
                .remove(DEEPSEEK_PROVIDER_ID)
                .unwrap();
            provider.base_url = Some(base_url);
            provider.env_key = None;
            provider.experimental_bearer_token = Some("deepseek-test-key".into());
            config.model_provider_id = DEEPSEEK_PROVIDER_ID.into();
            config.model_provider = provider;
            config.model_catalog = None;
        });
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_turn("Say ready.").await?;
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let request = &requests[0];
    let body = request.body_json();
    assert_eq!(body["model"], "deepseek-flash");
    assert_eq!(
        request.header("authorization").as_deref(),
        Some("Bearer deepseek-test-key")
    );
    assert_eq!(request.header("chatgpt-account-id"), None);
    assert!(
        body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| !matches!(
                tool["type"].as_str(),
                Some("web_search" | "image_generation" | "namespace")
            ))
    );
    assert!(!test.home.path().join("models.json").exists());
    assert!(
        requests[1]
            .function_call_output("call-deepseek")
            .to_string()
            .contains("deepseek-native")
    );
    Ok(())
}
