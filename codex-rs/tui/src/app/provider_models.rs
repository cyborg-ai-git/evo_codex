//! Provider discovery and transactional task startup for the native model picker.

use super::*;
use crate::app::session_lifecycle::ThreadAttachPresentation;
use crate::app_server_session::ThreadParamsMode;

impl App {
    pub(super) fn fetch_provider_models(
        &mut self,
        app_server: &AppServerSession,
        provider: String,
    ) {
        if app_server.thread_params_mode() == ThreadParamsMode::Remote {
            if provider == self.chat_widget.config_ref().model_provider_id {
                self.chat_widget.open_model_popup();
            } else {
                self.chat_widget.add_error_message(
                    "Native provider switching requires the local app server.".into(),
                );
            }
            return;
        }
        let request_id = self.chat_widget.begin_provider_models();
        let tx = self.app_event_tx.clone();
        let request_handle = app_server.request_handle();
        tokio::spawn(async move {
            let result = request_handle
                .request_typed::<codex_app_server_protocol::ModelListResponse>(
                    codex_app_server_protocol::ClientRequest::ModelList {
                        request_id: codex_app_server_protocol::RequestId::String(format!(
                            "provider-models-{request_id}"
                        )),
                        params: codex_app_server_protocol::ModelListParams {
                            model_provider: Some(provider.clone()),
                            include_hidden: Some(true),
                            ..Default::default()
                        },
                    },
                )
                .await
                .map(|response| {
                    response
                        .data
                        .into_iter()
                        .map(crate::app_server_session::model_preset_from_api_model)
                        .collect()
                })
                .map_err(|error| error.to_string());
            tx.send(AppEvent::ProviderModelsLoaded {
                request_id,
                provider,
                result,
            });
        });
    }

    pub(super) async fn select_provider_model(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        provider: String,
        model: String,
        effort: Option<codex_protocol::openai_models::ReasoningEffort>,
        models: Arc<Vec<codex_protocol::openai_models::ModelPreset>>,
    ) -> Result<()> {
        if app_server.thread_params_mode() == ThreadParamsMode::Remote {
            self.chat_widget.add_error_message(
                "Native provider switching requires the local app server.".into(),
            );
            return Ok(());
        }
        if self.reject_pending_permission_root_switch() {
            return Ok(());
        }
        if self.chat_widget.is_user_turn_pending_or_running() {
            self.chat_widget.add_error_message(
                "Wait for the active turn to finish before switching models.".into(),
            );
            return Ok(());
        }
        if provider != self.chat_widget.config_ref().model_provider_id
            && self.chat_widget.has_queued_follow_up_messages()
        {
            self.chat_widget.add_error_message(
                "Send or remove queued messages before changing providers.".into(),
            );
            return Ok(());
        }
        if provider == self.chat_widget.config_ref().model_provider_id {
            let Some(mut params) = self.active_thread_model_setting_update_params(model.clone())
            else {
                return Ok(());
            };
            params.effort = effort.clone();
            if let Some(mode) = params.collaboration_mode.as_mut() {
                mode.settings.model = model.clone();
                mode.settings.reasoning_effort = effort.clone();
            }
            if !self.send_thread_settings_update(app_server, params).await {
                return Ok(());
            }
            self.chat_widget.set_model(&model);
            self.on_update_reasoning_effort(effort);
            return Ok(());
        }
        let mut config = self.chat_widget.config_ref().clone();
        let Some(info) = config.model_providers.get(&provider).cloned() else {
            return Ok(());
        };
        // Resolve credentials before creating a task. Never borrow OpenAI credentials for DeepSeek.
        if provider == "deepseek" {
            let runtime = codex_model_provider::create_model_provider(
                info.clone(),
                /*auth_manager*/ None,
            );
            if let Err(error) = runtime.api_auth().await {
                self.chat_widget.add_error_message(error.to_string());
                return Ok(());
            }
        }
        config.model_provider_id = provider;
        config.model_provider = info;
        config.model = Some(model);
        config.model_reasoning_effort = effort;
        config.service_tier = None;
        config.model_catalog = None;
        let previous_models = self.model_catalog.try_list_models()?;
        app_server.set_available_models(models.as_ref().clone());
        let started = match app_server
            .start_thread_with_session_start_source(
                &self.local_settings,
                &config,
                /*session_start_source*/ None,
                /*remote_cwd_override*/ None,
                /*selected_profile*/ None,
            )
            .await
        {
            Ok(started) => started,
            Err(error) => {
                app_server.set_available_models(previous_models);
                self.chat_widget
                    .add_error_message(format!("Could not switch provider: {error}"));
                return Ok(());
            }
        };
        self.detach_current_thread_for_navigation(app_server, Some(started.session.thread_id))
            .await;
        Arc::make_mut(&mut self.model_catalog).models = models.as_ref().clone();
        self.chat_widget.requires_openai_auth = config.model_provider.requires_openai_auth;
        self.config = config;
        self.replace_chat_widget_with_app_server_thread(
            tui,
            started,
            ThreadAttachPresentation::Fresh,
            /*initial_user_message*/ None,
        )
        .await
    }
}

pub(super) struct LocalDownload {
    pub(super) id: uuid::Uuid,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) effort: Option<codex_protocol::openai_models::ReasoningEffort>,
    pub(super) models: Arc<Vec<codex_protocol::openai_models::ModelPreset>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for LocalDownload {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl App {
    pub(super) fn begin_local_download(
        &mut self,
        app_server: &AppServerSession,
        provider: String,
        model: String,
        effort: Option<codex_protocol::openai_models::ReasoningEffort>,
        models: Arc<Vec<codex_protocol::openai_models::ModelPreset>>,
    ) {
        if app_server.thread_params_mode() == ThreadParamsMode::Remote {
            self.chat_widget
                .add_error_message("Local downloads require the local app server.".into());
            return;
        }
        if self.chat_widget.is_user_turn_pending_or_running() {
            self.chat_widget.add_error_message(
                "Wait for the active turn to finish before downloading a model.".into(),
            );
            return;
        }
        let id = uuid::Uuid::new_v4();
        let tx = self.app_event_tx.clone();
        let factory = self.config.http_client_factory();
        let selected = model.clone();
        self.chat_widget.start_local_download(id);
        let task = tokio::spawn(async move {
            let result = codex_model_provider::prepare_mlx_model(&selected, factory, |message| {
                tx.send(AppEvent::LocalDownloadProgress { id, message });
            })
            .await;
            tx.send(AppEvent::LocalDownloadFinished { id, result });
        });
        self.local_download = Some(LocalDownload {
            id,
            provider,
            model,
            effort,
            models,
            task,
        });
    }
}
