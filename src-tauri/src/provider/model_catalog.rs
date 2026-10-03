use serde::Serialize;

/// Client compatibility policy, not verified supplier reasoning capabilities.
pub(crate) const REASONING_SELECTOR_POLICY: &str = "common-reasoning-selector-v1";
pub(crate) const DEFAULT_REASONING_EFFORT: &str = "high";
const REASONING_CHOICES: [(&str, &str); 4] = [
    ("low", "较少思考，优先响应速度"),
    ("medium", "平衡思考深度与响应速度"),
    ("high", "深入思考，适合复杂任务"),
    ("xhigh", "更多思考，可能增加耗时与用量"),
];

/// The on-disk catalog format consumed by Codex's `model_catalog_json` option.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ModelCatalog {
    pub models: Vec<ModelCatalogEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ModelCatalogEntry {
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub default_reasoning_level: Option<String>,
    pub supported_reasoning_levels: Vec<ReasoningLevel>,
    pub shell_type: &'static str,
    pub visibility: &'static str,
    pub supported_in_api: bool,
    pub priority: i32,
    pub support_verbosity: bool,
    pub default_verbosity: Option<&'static str>,
    pub context_window: u64,
    pub max_context_window: u64,
    pub input_modalities: [&'static str; 2],
    pub supports_image_detail_original: bool,
    pub supports_parallel_tool_calls: bool,
    pub truncation_policy: TruncationPolicy,
    pub experimental_supported_tools: Vec<String>,
    pub base_instructions: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TruncationPolicy {
    pub mode: &'static str,
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ReasoningLevel {
    pub effort: String,
    pub description: String,
}

#[derive(Debug)]
pub(crate) enum ModelCatalogError {
    InvalidModelSet,
    Serialization,
}

#[derive(Debug)]
pub(crate) enum ModelCatalogSnapshotError {
    InvalidBinding,
    InvalidFormat,
    InvalidModelSet,
}

pub(crate) fn normalize_discovered_models(
    discovered_models: &[String],
    default_model: &str,
) -> Result<Vec<String>, ModelCatalogError> {
    let default_model = default_model.trim();
    if default_model.is_empty() {
        return Err(ModelCatalogError::InvalidModelSet);
    }

    let mut ids = discovered_models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err(ModelCatalogError::InvalidModelSet);
    }
    if !ids.iter().any(|model| model == default_model) {
        ids.push(default_model.to_owned());
    }
    ids.sort_by(|left, right| {
        left.to_ascii_lowercase()
            .cmp(&right.to_ascii_lowercase())
            .then_with(|| left.cmp(right))
    });
    ids.dedup();
    Ok(ids)
}

pub(crate) fn validate_snapshot(
    provider_id: &str,
    verification_fingerprint: &str,
    default_model: &str,
    snapshot_provider_id: &str,
    snapshot_fingerprint: &str,
    models_json: &str,
) -> Result<Vec<String>, ModelCatalogSnapshotError> {
    if snapshot_provider_id != provider_id || snapshot_fingerprint != verification_fingerprint {
        return Err(ModelCatalogSnapshotError::InvalidBinding);
    }
    let models = serde_json::from_str::<Vec<String>>(models_json)
        .map_err(|_| ModelCatalogSnapshotError::InvalidFormat)?;
    normalize_discovered_models(&models, default_model)
        .map_err(|_| ModelCatalogSnapshotError::InvalidModelSet)
}

pub(crate) fn render(
    discovered_models: &[String],
    default_model: &str,
) -> Result<Vec<u8>, ModelCatalogError> {
    let ids = normalize_discovered_models(discovered_models, default_model)?;
    let entries = ids.into_iter().map(entry).collect();
    serde_json::to_vec_pretty(&ModelCatalog { models: entries })
        .map_err(|_| ModelCatalogError::Serialization)
}

fn entry(slug: String) -> ModelCatalogEntry {
    let display_name = slug.replace(['-', '_'], " ");
    ModelCatalogEntry {
        description: "供应商已发现模型，能力未识别".to_owned(),
        display_name,
        // Keep both fields: Codex requires the list, and a null default becomes
        // `none` in its model picker. Supplier acceptance is decided upstream.
        default_reasoning_level: Some(DEFAULT_REASONING_EFFORT.to_owned()),
        supported_reasoning_levels: REASONING_CHOICES
            .iter()
            .map(|(effort, description)| ReasoningLevel {
                effort: (*effort).to_owned(),
                description: (*description).to_owned(),
            })
            .collect(),
        slug,
        shell_type: "default",
        visibility: "list",
        supported_in_api: true,
        priority: 1,
        support_verbosity: false,
        default_verbosity: None,
        context_window: 128_000,
        max_context_window: 128_000,
        input_modalities: ["text", "image"],
        supports_image_detail_original: false,
        supports_parallel_tool_calls: false,
        truncation_policy: TruncationPolicy {
            mode: "tokens",
            limit: 10_000,
        },
        experimental_supported_tools: Vec::new(),
        base_instructions: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovered_models_offer_common_reasoning_choices_without_per_model_configuration() {
        let bytes = render(
            &[
                "gpt-new-model".to_owned(),
                "deepseek-new-model".to_owned(),
                "vendor-model".to_owned(),
            ],
            "vendor-model",
        )
        .expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        for model in value["models"].as_array().expect("models") {
            let efforts = model["supported_reasoning_levels"]
                .as_array()
                .expect("levels")
                .iter()
                .map(|level| level["effort"].as_str().expect("effort"))
                .collect::<Vec<_>>();
            assert_eq!(
                efforts,
                ["low", "medium", "high", "xhigh"],
                "{} has no selectable reasoning",
                model["slug"]
            );
        }
    }

    #[test]
    fn unknown_models_keep_their_unverified_description_and_use_the_client_default() {
        let bytes = render(&["vendor-model".to_owned()], "vendor-model").expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        let model = &value["models"][0];
        assert_eq!(model["slug"], "vendor-model");
        assert_eq!(model["description"], "供应商已发现模型，能力未识别");
        assert_eq!(model["default_reasoning_level"], "high");
        assert_eq!(
            model["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn every_model_optimistically_allows_text_and_image_without_original_detail() {
        let bytes = render(
            &[
                "text-only-model".to_owned(),
                "known-gpt-model".to_owned(),
                "fallback-model".to_owned(),
            ],
            "fallback-model",
        )
        .expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        let models = value["models"].as_array().expect("models");

        assert_eq!(models.len(), 3);
        for model in models {
            assert_eq!(
                model["input_modalities"],
                serde_json::json!(["text", "image"])
            );
            assert_eq!(model["supports_image_detail_original"], false);
        }
    }

    #[test]
    fn catalog_generation_rejects_empty_sets_and_adds_a_missing_default() {
        assert!(matches!(
            render(&[], "default-model"),
            Err(ModelCatalogError::InvalidModelSet)
        ));
        let models = normalize_discovered_models(&["other-model".to_owned()], "default-model")
            .expect("default model is included");
        assert_eq!(models, ["default-model", "other-model"]);
    }

    #[test]
    fn normalization_trims_sorts_and_deduplicates_without_inserting_models() {
        let models = normalize_discovered_models(
            &[
                " model-b ".to_owned(),
                "model-a".to_owned(),
                "model-b".to_owned(),
                "  ".to_owned(),
            ],
            "model-a",
        )
        .expect("valid discovered set");
        assert_eq!(models, ["model-a", "model-b"]);
    }

    #[test]
    fn model_names_do_not_change_the_common_selector_policy() {
        let bytes = render(
            &[
                "DeepSeek-R1".to_owned(),
                "gpt-5".to_owned(),
                "Claude-4".to_owned(),
            ],
            "DeepSeek-R1",
        )
        .expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        let models = value["models"].as_array().expect("models");
        for model in models {
            assert_eq!(model["default_reasoning_level"], "high");
            assert_eq!(
                model["supported_reasoning_levels"],
                models[0]["supported_reasoning_levels"]
            );
        }
    }
}

#[cfg(test)]
#[path = "model_catalog_contract_tests.rs"]
mod contract_tests;
