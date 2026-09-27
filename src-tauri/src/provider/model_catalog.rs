use serde::Serialize;

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

pub(crate) fn render(
    discovered_models: &[String],
    default_model: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut ids = discovered_models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !ids.iter().any(|model| model == default_model) {
        ids.push(default_model.to_owned());
    }
    ids.sort_by_key(|model| model.to_ascii_lowercase());
    ids.dedup();
    let entries = ids.into_iter().map(entry).collect();
    serde_json::to_vec_pretty(&ModelCatalog { models: entries })
}

fn entry(slug: String) -> ModelCatalogEntry {
    let display_name = slug.replace(['-', '_'], " ");
    ModelCatalogEntry {
        description: "供应商已发现模型，能力未识别".to_owned(),
        display_name,
        default_reasoning_level: None,
        supported_reasoning_levels: Vec::new(),
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
    fn unknown_models_are_listed_without_capability_claims() {
        let bytes = render(&["vendor-model".to_owned()], "vendor-model").expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        let model = &value["models"][0];
        assert_eq!(model["slug"], "vendor-model");
        assert_eq!(model["description"], "供应商已发现模型，能力未识别");
        assert!(
            model["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(model["default_reasoning_level"].is_null());
    }

    #[test]
    fn every_model_optimistically_allows_text_and_image_without_original_detail() {
        let bytes = render(
            &["text-only-model".to_owned(), "known-gpt-model".to_owned()],
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
    fn model_names_never_create_reasoning_capabilities() {
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
        for model in value["models"].as_array().expect("models") {
            assert!(model["default_reasoning_level"].is_null());
            assert!(
                model["supported_reasoning_levels"]
                    .as_array()
                    .expect("levels")
                    .is_empty()
            );
        }
    }
}
