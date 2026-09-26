//! Model-family presentation at constrained sizes and with configured list bindings.

use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn model_picker_compact_hint_and_actions_follow_configured_bindings() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.5")).await;
    let mut keymap = crate::keymap::RuntimeKeymap::defaults();
    keymap.list.accept = vec![key_hint::plain(KeyCode::F(3))];
    keymap.list.cancel = vec![key_hint::plain(KeyCode::F(2))];
    chat.bottom_pane.set_keymap_bindings(&keymap);
    chat.open_all_models_popup();
    let popup = render_bottom_popup(&chat, /*width*/ 40);
    assert_eq!(popup.lines().last().unwrap().trim(), "f3 select · f2 back");
    chat.handle_key_event(KeyEvent::from(KeyCode::F(3)));
    assert_matches!(rx.try_recv(), Ok(AppEvent::OpenReasoningPopup { .. }));
    chat.handle_key_event(KeyEvent::from(KeyCode::F(2)));
    assert!(chat.no_modal_or_popup_active());
}
#[tokio::test]
async fn native_provider_picker_and_deepseek_efforts() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.5")).await;
    chat.thread_id = Some(ThreadId::new());
    chat.open_provider_picker();
    assert_chatwidget_snapshot!(
        "native_provider_picker",
        render_bottom_popup(&chat, /*width*/ 100)
    );
    chat.handle_key_event(KeyEvent::from(KeyCode::Esc));
    let request_id = chat.begin_provider_models();
    let models = codex_models_manager::deepseek::catalog()
        .models
        .into_iter()
        .map(Into::into)
        .collect();
    chat.show_provider_models(request_id, "deepseek".into(), Ok(models));
    assert_chatwidget_snapshot!(
        "native_deepseek_picker",
        render_bottom_popup(&chat, /*width*/ 100)
    );
    chat.handle_key_event(KeyEvent::from(KeyCode::Enter));
    assert_matches!(rx.try_recv(), Ok(AppEvent::SelectProviderModel { provider, model, effort: Some(ReasoningEffortConfig::Low), .. }) if provider == "deepseek" && model == "deepseek-flash");
}

#[tokio::test]
async fn native_provider_picker_ignores_dismissed_and_obsolete_replies() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.5")).await;
    let old_request = chat.begin_provider_models();
    chat.handle_key_event(KeyEvent::from(KeyCode::Esc));
    let request = chat.begin_provider_models();
    chat.show_provider_models(old_request, "deepseek".into(), Err("stale failure".into()));
    assert_eq!(chat.model_popup_request_id, Some(request));
    chat.handle_key_event(KeyEvent::from(KeyCode::Esc));
    chat.show_provider_models(
        request,
        "deepseek".into(),
        Ok(codex_models_manager::deepseek::catalog()
            .models
            .into_iter()
            .map(Into::into)
            .collect()),
    );
    assert!(chat.no_modal_or_popup_active());
}

#[tokio::test]
async fn native_mlx_download_progress_is_cancellable() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.5")).await;
    let id = uuid::Uuid::new_v4();
    chat.start_local_download(id);
    chat.update_local_download(
        id,
        "Downloading model.safetensors · 42.0% · 0.68/1.63 GiB".into(),
    );
    assert_chatwidget_snapshot!(
        "native_mlx_download",
        render_bottom_popup(&chat, /*width*/ 100)
    );
    chat.handle_key_event(KeyEvent::from(KeyCode::Esc));
    assert_matches!(rx.try_recv(), Ok(AppEvent::CancelLocalDownload(cancelled)) if cancelled == id);
    assert!(chat.no_modal_or_popup_active());
}

#[tokio::test]
async fn native_mlx_picker_shows_installed_and_downloadable_models() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.5")).await;
    let request_id = chat.begin_provider_models();
    let mut installed = codex_models_manager::deepseek::local_model_info("/models/Qwen3.5-2B-4bit");
    installed.display_name = "Qwen3.5-2B-4bit".into();
    installed.description = Some("On disk · loads on the first request".into());
    let mut download = codex_models_manager::deepseek::local_model_info("/models/Qwen3.5-4B-4bit");
    download.display_name = "Qwen3.5-4B-4bit".into();
    download.description =
        Some("Download 2.9 GiB · at least 6 GiB RAM · Apple Silicon / MLX".into());
    chat.show_provider_models(
        request_id,
        "mlx".into(),
        Ok(vec![installed.into(), download.into()]),
    );
    assert_chatwidget_snapshot!(
        "native_mlx_picker",
        render_bottom_popup(&chat, /*width*/ 110)
    );
}

#[tokio::test]
async fn native_mlx_download_completion_releases_the_settings_input_guard() {
    let (mut chat, mut rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.5")).await;
    chat.start_local_download(uuid::Uuid::new_v4());
    chat.defer_input_until_settings_applied();
    chat.finish_local_download();
    assert!(chat.no_modal_or_popup_active());
    assert_matches!(rx.try_recv(), Ok(AppEvent::SettingsSelectionClosed));
}
