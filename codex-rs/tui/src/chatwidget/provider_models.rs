//! Native provider selection from /model; cross-provider selections start a fresh task.

use super::*;

const PROVIDER_VIEW: &str = "provider-models";

impl ChatWidget {
    pub(crate) fn open_provider_picker(&mut self) {
        if !self.is_session_configured()
            || !matches!(
                self.config.model_provider_id.as_str(),
                "openai" | "deepseek" | "ollama" | "lmstudio" | "mlx"
            )
        {
            self.open_model_popup();
            return;
        }
        self.model_popup_request_id = None;
        let items = [
            ("openai", "OpenAI", "ChatGPT and OpenAI API models"),
            (
                "deepseek",
                "DeepSeek API",
                "DeepSeek Flash and V4 Pro · DEEPSEEK_API_KEY",
            ),
            (
                "mlx",
                "Local · MLX / Qwen",
                "Discover downloaded MLX models and the local server",
            ),
            (
                "ollama",
                "Local · Ollama",
                "Discover installed models, including custom fine-tunes",
            ),
            (
                "lmstudio",
                "Local · LM Studio",
                "Discover models served by LM Studio",
            ),
        ]
        .into_iter()
        .map(|(provider, name, description)| SelectionItem {
            name: name.into(),
            description: Some(description.into()),
            is_current: self.config.model_provider_id == provider,
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenProviderModels(provider.into()))
            })],
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Select Model Provider".into()),
            subtitle: Some(
                "Changing provider starts a new task; previous tasks remain resumable.".into(),
            ),
            items,
            ..SelectionViewParams::picker()
        });
    }

    pub(crate) fn begin_provider_models(&mut self) -> uuid::Uuid {
        let request_id = uuid::Uuid::new_v4();
        self.model_popup_request_id = Some(request_id);
        self.bottom_pane.show_selection_view(SelectionViewParams {
            view_id: Some(PROVIDER_VIEW),
            title: Some("Loading models…".into()),
            items: vec![SelectionItem {
                name: "Waiting for provider…".into(),
                ..Default::default()
            }],
            ..SelectionViewParams::picker()
        });
        request_id
    }

    pub(crate) fn show_provider_models(
        &mut self,
        request_id: uuid::Uuid,
        provider: String,
        result: Result<Vec<ModelPreset>, String>,
    ) {
        if self.model_popup_request_id != Some(request_id)
            || self
                .bottom_pane
                .selected_index_for_present_view(PROVIDER_VIEW)
                .is_none()
        {
            return;
        }
        self.model_popup_request_id = None;
        let models = match result {
            Ok(models) if !models.is_empty() => models,
            Ok(_) => {
                self.bottom_pane.dismiss_view_by_id(PROVIDER_VIEW);
                self.add_error_message(
                    "No models found. Load a model in your local server and try /model again."
                        .into(),
                );
                return;
            }
            Err(error) => {
                self.bottom_pane.dismiss_view_by_id(PROVIDER_VIEW);
                self.add_error_message(error);
                return;
            }
        };
        if provider == self.config.model_provider_id {
            Arc::make_mut(&mut self.model_catalog).models = models.clone();
        }
        let mut items = Vec::new();
        if provider == "openai" && self.config.model_provider_id == provider {
            self.bottom_pane.dismiss_view_by_id(PROVIDER_VIEW);
            self.open_model_popup_with_presets(models);
            return;
        }
        let catalog = Arc::new(models.clone());
        for model in models.into_iter().filter(|model| model.show_in_picker) {
            let efforts: Vec<_> = if model.supported_reasoning_efforts.is_empty() {
                vec![Some(ReasoningEffortConfig::None)]
            } else {
                model
                    .supported_reasoning_efforts
                    .iter()
                    .map(|preset| Some(preset.effort.clone()))
                    .collect()
            };
            for effort in efforts {
                let provider = provider.clone();
                let models = catalog.clone();
                let slug = model.model.clone();
                let name = effort.as_ref().map_or_else(
                    || model.display_name.clone(),
                    |effort| format!("{} · {effort}", model.display_name),
                );
                items.push(SelectionItem {
                    name,
                    description: (provider == "mlx").then(|| model.description.clone()),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::SelectProviderModel {
                            provider: provider.clone(),
                            model: slug.clone(),
                            effort: effort.clone(),
                            models: models.clone(),
                        })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                });
            }
        }
        self.bottom_pane.replace_selection_view_if_present(
            PROVIDER_VIEW,
            SelectionViewParams {
                view_id: Some(PROVIDER_VIEW),
                title: Some(format!("Select {provider} model")),
                subtitle: Some(
                    "Select a model and effort. A provider change starts a new task.".into(),
                ),
                items,
                ..SelectionViewParams::picker()
            },
        );
    }
}

const DOWNLOAD_VIEW: &str = "local-model-download";

fn download_view(id: uuid::Uuid, message: String) -> SelectionViewParams {
    SelectionViewParams {
        view_id: Some(DOWNLOAD_VIEW),
        title: Some("Preparing local model".into()),
        subtitle: Some(message),
        items: vec![SelectionItem {
            name: "Cancel (partial files are kept for resume)".into(),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::CancelLocalDownload(id))
            })],
            dismiss_on_select: true,
            ..Default::default()
        }],
        on_cancel: Some(Box::new(move |tx| {
            tx.send(AppEvent::CancelLocalDownload(id))
        })),
        ..SelectionViewParams::picker()
    }
}

impl ChatWidget {
    pub(crate) fn start_local_download(&mut self, id: uuid::Uuid) {
        self.bottom_pane
            .show_selection_view(download_view(id, "Checking model files…".into()));
    }
    pub(crate) fn update_local_download(&mut self, id: uuid::Uuid, message: String) {
        self.bottom_pane
            .replace_selection_view_if_present(DOWNLOAD_VIEW, download_view(id, message));
    }
    pub(crate) fn finish_local_download(&mut self) {
        self.bottom_pane.dismiss_view_by_id(DOWNLOAD_VIEW);
        self.on_modal_or_popup_closed();
    }
}
