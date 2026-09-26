//! Native DeepSeek metadata. No user-managed model catalog is required.

use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::config_types::Verbosity;
use codex_protocol::openai_models::ApplyPatchToolType;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use codex_protocol::openai_models::ToolMode;
use codex_protocol::openai_models::TruncationPolicyConfig;

/// Models supported by DeepSeek's native Responses API.
pub fn catalog() -> ModelsResponse {
    ModelsResponse {
        models: ["deepseek-flash", "deepseek-v4-pro"]
            .into_iter()
            .map(model_info)
            .collect(),
    }
}

/// Conservative metadata for locally served models, including custom fine-tunes.
pub fn local_model_info(slug: &str) -> ModelInfo {
    let mut model = crate::model_info::minimal_model_info(slug);
    model.visibility = ModelVisibility::List;
    model.input_modalities = vec![InputModality::Text];
    model.context_window = Some(32_768);
    model.max_context_window = None;
    model.supports_reasoning_summary_parameter = false;
    model.default_reasoning_summary = ReasoningSummary::None;
    model.tool_mode = Some(ToolMode::Direct);
    model.node_repl_disabled = true;
    model.used_fallback_model_metadata = false;
    model
}

fn model_info(slug: &str) -> ModelInfo {
    let mut model = local_model_info(slug);
    model.display_name = match slug {
        "deepseek-flash" => "DeepSeek Flash",
        _ => "DeepSeek V4 Pro",
    }
    .into();
    model.description = Some("DeepSeek API · native Responses transport".into());
    model.context_window = Some(1_048_576);
    model.max_context_window = Some(1_048_576);
    model.multi_agent_version = Some(codex_protocol::protocol::MultiAgentVersion::V2);
    model.default_reasoning_level = Some(ReasoningEffort::High);
    model.supported_reasoning_levels = [
        ReasoningEffort::Low,
        ReasoningEffort::High,
        ReasoningEffort::Max,
    ]
    .into_iter()
    .map(|effort| ReasoningEffortPreset {
        description: format!("{effort} reasoning effort"),
        effort,
    })
    .collect();
    model.support_verbosity = true;
    model.default_verbosity = Some(Verbosity::Low);
    model.apply_patch_tool_type = Some(ApplyPatchToolType::Freeform);
    model.truncation_policy = TruncationPolicyConfig::tokens(/*limit*/ 10_000);
    model.include_skills_usage_instructions = true;
    model.include_plugin_usage_instructions = true;
    if slug == "deepseek-flash" {
        model.input_modalities.push(InputModality::Image);
        model.supports_image_detail_original = true;
    }
    model
}
