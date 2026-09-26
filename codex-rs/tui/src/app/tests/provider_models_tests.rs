use super::*;
use crate::app::session_lifecycle::ThreadAttachPresentation;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn native_provider_switch_preserves_previous_task_and_updates_same_provider_model()
-> Result<()> {
    let (mut app, mut events, _ops) = make_test_app_with_channels().await;
    let openai_models = Arc::new(app.model_catalog.try_list_models()?);
    let models: Arc<Vec<codex_protocol::openai_models::ModelPreset>> = Arc::new(
        codex_models_manager::deepseek::catalog()
            .models
            .into_iter()
            .map(Into::into)
            .collect(),
    );
    std::fs::write(
        app.config.codex_home.join("config.toml"),
        "[model_providers.deepseek]\nname = 'DeepSeek'\nexperimental_bearer_token = 'test-key'\n",
    )?;
    let provider = app.config.model_providers.get_mut("deepseek").unwrap();
    provider.env_key = None;
    provider.experimental_bearer_token = Some("test-key".into());
    let mut server = start_config_write_test_app_server(&app).await?;
    let original = server.start_thread(&app.config).await?;
    let original_id = original.session.thread_id;
    let mut tui = crate::tui::test_support::make_test_tui()?;
    app.replace_chat_widget_with_app_server_thread(
        &mut tui,
        original,
        ThreadAttachPresentation::Fresh,
        /*initial_user_message*/ None,
    )
    .await?;
    app.select_provider_model(
        &mut tui,
        &mut server,
        "deepseek".into(),
        "deepseek-flash".into(),
        Some(ReasoningEffortConfig::High),
        models.clone(),
    )
    .await?;
    assert_eq!(app.config.model_provider_id, "deepseek");
    assert_eq!(app.model_catalog.models, *models);
    assert_eq!(app.chat_widget.model_catalog().models, *models);
    assert!(!app.chat_widget.requires_openai_auth);
    let request_id = uuid::Uuid::new_v4();
    server.fetch_models(
        request_id,
        Some("deepseek".into()),
        app.app_event_tx.clone(),
    );
    let refreshed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(AppEvent::ModelsLoaded {
                request_id: received,
                result,
            }) = events.recv().await
                && received == request_id
            {
                break result.unwrap();
            }
        }
    })
    .await?;
    assert_eq!(
        refreshed
            .iter()
            .map(|model| &model.model)
            .collect::<Vec<_>>(),
        models.iter().map(|model| &model.model).collect::<Vec<_>>()
    );
    let deepseek_id = app.active_thread_id.unwrap();
    assert_ne!(deepseek_id, original_id);
    assert_eq!(
        server
            .thread_read(original_id, /*include_turns*/ false)
            .await?
            .model_provider,
        "openai"
    );
    assert_eq!(
        server
            .thread_read(deepseek_id, /*include_turns*/ false)
            .await?
            .model_provider,
        "deepseek"
    );
    app.select_provider_model(
        &mut tui,
        &mut server,
        "deepseek".into(),
        "deepseek-v4-pro".into(),
        Some(ReasoningEffortConfig::Low),
        models,
    )
    .await?;
    assert_eq!(app.active_thread_id, Some(deepseek_id));
    assert_eq!(app.chat_widget.current_model(), "deepseek-v4-pro");
    app.select_provider_model(
        &mut tui,
        &mut server,
        "openai".into(),
        "gpt-5.5".into(),
        Some(ReasoningEffortConfig::Medium),
        openai_models.clone(),
    )
    .await?;
    assert_eq!(app.model_catalog.models, *openai_models);
    assert_eq!(app.chat_widget.model_catalog().models, *openai_models);
    assert!(app.chat_widget.requires_openai_auth);
    Ok(())
}
