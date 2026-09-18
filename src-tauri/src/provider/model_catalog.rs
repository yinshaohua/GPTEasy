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
    pub input_modalities: [&'static str; 1],
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

pub(crate) fn render(models: &[String], default_model: &str) -> Result<Vec<u8>, serde_json::Error> {
    let mut ids = models
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
    let entries = ids
        .into_iter()
        .map(|slug| {
            let is_default = slug == default_model;
            entry(slug, is_default)
        })
        .collect();
    serde_json::to_vec_pretty(&ModelCatalog { models: entries })
}

fn entry(slug: String, is_default: bool) -> ModelCatalogEntry {
    let (levels, default) = reasoning_profile(&slug);
    let display_name = slug.replace(['-', '_'], " ");
    ModelCatalogEntry {
        description: if levels.is_empty() {
            "能力未识别的供应商模型".to_owned()
        } else {
            "供应商已验证模型".to_owned()
        },
        display_name,
        default_reasoning_level: (is_default && !levels.is_empty()).then_some(default),
        supported_reasoning_levels: levels,
        slug,
        shell_type: "default",
        visibility: "list",
        supported_in_api: true,
        priority: 1,
        support_verbosity: false,
        default_verbosity: None,
        context_window: 128_000,
        max_context_window: 128_000,
        input_modalities: ["text"],
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

fn reasoning_profile(model: &str) -> (Vec<ReasoningLevel>, String) {
    let lower = model.to_ascii_lowercase();
    let family = if lower.contains("deepseek") || lower.contains("reasoner") {
        Some("deepseek")
    } else if lower.contains("gpt")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        Some("gpt")
    } else if lower.contains("claude") || lower.contains("anthropic") {
        Some("anthropic")
    } else if ["qwen", "gemini", "glm", "kimi", "mistral", "llama"]
        .iter()
        .any(|family| lower.contains(family))
    {
        Some("other")
    } else {
        None
    };
    let levels = family
        .map(|_| {
            ["low", "medium", "high"]
                .into_iter()
                .map(|effort| ReasoningLevel {
                    effort: effort.to_owned(),
                    description: effort.to_owned(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let default = match family {
        Some("gpt") | Some("deepseek") => "high".to_owned(),
        Some(_) if levels.len() >= 3 => "medium".to_owned(),
        Some(_) => levels
            .last()
            .map(|level| level.effort.clone())
            .unwrap_or_default(),
        None => String::new(),
    };
    (levels, default)
}

pub(crate) fn default_reasoning_effort(model: &str) -> Option<String> {
    let (_, default) = reasoning_profile(model);
    (!default.is_empty()).then_some(default)
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
        assert!(
            model["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(model["default_reasoning_level"].is_null());
    }

    #[test]
    fn deepseek_defaults_to_high_when_supported() {
        assert_eq!(
            default_reasoning_effort("DeepSeek-R1"),
            Some("high".to_owned())
        );
    }
}
