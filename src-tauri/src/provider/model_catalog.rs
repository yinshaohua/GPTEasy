use serde::Serialize;

use super::reasoning_capability::{
    CapabilityState, CodexMetadataSnapshot, ReasoningCapability, merge_exact,
};

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

/// Compatibility wrapper for callers that do not have a trusted Codex snapshot.
#[cfg(test)]
pub(crate) fn render(
    discovered_models: &[String],
    default_model: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    render_with_snapshot(discovered_models, default_model, None)
}

/// Render the catalog using only exact model metadata from a version-bound snapshot.
///
/// A missing, stale, failed, or non-native snapshot deliberately keeps the old explicit
/// unknown-model representation. No model/vendor name is used as a capability hint.
pub(crate) fn render_with_snapshot(
    discovered_models: &[String],
    default_model: &str,
    snapshot: Option<&CodexMetadataSnapshot>,
) -> Result<Vec<u8>, serde_json::Error> {
    render_for_environment(discovered_models, default_model, snapshot, "native")
}

pub(crate) fn render_for_environment(
    discovered_models: &[String],
    default_model: &str,
    snapshot: Option<&CodexMetadataSnapshot>,
    environment: &str,
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
    // 快照必须完整覆盖本次目录的全部模型；部分覆盖时统一降级，避免用户看到
    // 同一份目录中一部分模型有档位、另一部分模型却被误认为未知。
    let trusted_snapshot =
        snapshot.filter(|snapshot| snapshot.covers_models_in_environment(&ids, environment));
    let entries = ids
        .into_iter()
        .map(|model| {
            let capability = trusted_snapshot
                .and_then(|snapshot| snapshot.capability_for(&model))
                .map(|codex| merge_exact(Some(&codex), None, None));
            entry(model, capability.as_ref())
        })
        .collect();
    serde_json::to_vec_pretty(&ModelCatalog { models: entries })
}

fn entry(slug: String, capability: Option<&ReasoningCapability>) -> ModelCatalogEntry {
    let display_name = slug.replace(['-', '_'], " ");
    let (description, default_reasoning_level, supported_reasoning_levels) =
        capability_fields(capability);
    ModelCatalogEntry {
        description,
        display_name,
        default_reasoning_level,
        supported_reasoning_levels,
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

fn capability_fields(
    capability: Option<&ReasoningCapability>,
) -> (String, Option<String>, Vec<ReasoningLevel>) {
    let Some(capability) = capability else {
        return ("供应商已发现模型，能力未识别".to_owned(), None, Vec::new());
    };
    match capability.state {
        CapabilityState::KnownNonEmpty => (
            "Codex 已验证精确模型推理元数据".to_owned(),
            capability.default_level.clone(),
            capability
                .levels
                .iter()
                .map(|effort| ReasoningLevel {
                    effort: effort.clone(),
                    description: format!("Codex 声明的 {effort} 推理档位"),
                })
                .collect(),
        ),
        CapabilityState::KnownEmpty => (
            "Codex 已验证精确模型没有推理档位".to_owned(),
            None,
            Vec::new(),
        ),
        CapabilityState::NotFound => (
            "供应商已发现模型，Codex 未识别精确模型能力".to_owned(),
            None,
            Vec::new(),
        ),
        CapabilityState::ProbeFailed => (
            "供应商已发现模型，Codex 元数据探测失败".to_owned(),
            None,
            Vec::new(),
        ),
        CapabilityState::Conflict => ("推理能力来源冲突，未生成档位".to_owned(), None, Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::reasoning_capability::{
        CapabilitySource, CodexModelMetadata, SNAPSHOT_SCHEMA_VERSION, SnapshotStatus,
    };

    fn snapshot(models: Vec<CodexModelMetadata>) -> CodexMetadataSnapshot {
        CodexMetadataSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            target_environment: "native".to_owned(),
            codex_version: Some("0.160.1".to_owned()),
            executable_fingerprint: Some("sha256:test".to_owned()),
            observed_at_epoch_seconds: 1,
            status: SnapshotStatus::Complete,
            models,
        }
    }

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
    fn exact_codex_metadata_populates_only_the_exact_model() {
        let snapshot = snapshot(vec![
            CodexModelMetadata {
                model_id: "gpt-6.1-sol".to_owned(),
                state: CapabilityState::KnownNonEmpty,
                default_reasoning_effort: Some("medium".to_owned()),
                supported_reasoning_efforts: vec![
                    "low".to_owned(),
                    "medium".to_owned(),
                    "xhigh".to_owned(),
                ],
                source: CapabilitySource::CodexBuiltin,
                source_id: "codex.model/list".to_owned(),
            },
            CodexModelMetadata {
                model_id: "gpt-6.1-sol-preview".to_owned(),
                state: CapabilityState::NotFound,
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
                source: CapabilitySource::CodexBuiltin,
                source_id: "codex.model/list".to_owned(),
            },
        ]);
        let bytes = render_with_snapshot(
            &["gpt-6.1-sol".to_owned(), "gpt-6.1-sol-preview".to_owned()],
            "gpt-6.1-sol",
            Some(&snapshot),
        )
        .expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        let models = value["models"].as_array().expect("models");
        let known = models
            .iter()
            .find(|model| model["slug"] == "gpt-6.1-sol")
            .expect("known model");
        assert_eq!(known["default_reasoning_level"], "medium");
        assert_eq!(
            known["supported_reasoning_levels"],
            serde_json::json!([
                {"effort":"low","description":"Codex 声明的 low 推理档位"},
                {"effort":"medium","description":"Codex 声明的 medium 推理档位"},
                {"effort":"xhigh","description":"Codex 声明的 xhigh 推理档位"}
            ])
        );
        let preview = models
            .iter()
            .find(|model| model["slug"] == "gpt-6.1-sol-preview")
            .expect("provider-only model");
        assert!(
            preview["supported_reasoning_levels"]
                .as_array()
                .expect("levels")
                .is_empty()
        );
        assert!(preview["default_reasoning_level"].is_null());
    }

    #[test]
    fn partial_snapshot_downgrades_every_model_instead_of_mixing_trust() {
        let snapshot = snapshot(vec![CodexModelMetadata {
            model_id: "gpt-6.1-sol".to_owned(),
            state: CapabilityState::KnownNonEmpty,
            default_reasoning_effort: Some("high".to_owned()),
            supported_reasoning_efforts: vec!["high".to_owned()],
            source: CapabilitySource::CodexBuiltin,
            source_id: "codex.model/list".to_owned(),
        }]);
        let bytes = render_with_snapshot(
            &["gpt-6.1-sol".to_owned(), "provider-only".to_owned()],
            "gpt-6.1-sol",
            Some(&snapshot),
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
            assert_eq!(model["description"], "供应商已发现模型，能力未识别");
        }
    }

    #[test]
    fn known_empty_and_not_found_remain_distinguishable() {
        let snapshot = snapshot(vec![
            CodexModelMetadata {
                model_id: "known-empty".to_owned(),
                state: CapabilityState::KnownEmpty,
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
                source: CapabilitySource::CodexBuiltin,
                source_id: "codex.model/list".to_owned(),
            },
            CodexModelMetadata {
                model_id: "not-found".to_owned(),
                state: CapabilityState::NotFound,
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
                source: CapabilitySource::CodexBuiltin,
                source_id: "codex.model/list".to_owned(),
            },
        ]);
        let bytes = render_with_snapshot(
            &["known-empty".to_owned(), "not-found".to_owned()],
            "known-empty",
            Some(&snapshot),
        )
        .expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        let models = value["models"].as_array().expect("models");
        assert_eq!(models[0]["description"], "Codex 已验证精确模型没有推理档位");
        assert_eq!(
            models[1]["description"],
            "供应商已发现模型，Codex 未识别精确模型能力"
        );
    }

    #[test]
    fn stale_or_non_native_snapshots_are_unknown() {
        let mut snapshot = snapshot(vec![CodexModelMetadata {
            model_id: "gpt-6.1-sol".to_owned(),
            state: CapabilityState::KnownNonEmpty,
            default_reasoning_effort: Some("high".to_owned()),
            supported_reasoning_efforts: vec!["high".to_owned()],
            source: CapabilitySource::CodexBuiltin,
            source_id: "codex.model/list".to_owned(),
        }]);
        snapshot.target_environment = "wsl".to_owned();
        let bytes =
            render_with_snapshot(&["gpt-6.1-sol".to_owned()], "gpt-6.1-sol", Some(&snapshot))
                .expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        assert!(
            value["models"][0]["supported_reasoning_levels"]
                .as_array()
                .expect("levels")
                .is_empty()
        );
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
        let bytes = render_with_snapshot(
            &[
                "DeepSeek-R1".to_owned(),
                "gpt-5".to_owned(),
                "Claude-4".to_owned(),
            ],
            "DeepSeek-R1",
            None,
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

    #[test]
    fn dayway_ds_compatibility_snapshot_renders_exact_deepseek_models() {
        let snapshot = snapshot(
            [
                ("deepseek-v4-flash", true),
                ("deepseek-v4-pro", true),
                ("deepseek-v4-pro-preview", false),
            ]
            .into_iter()
            .map(|(model_id, compatible)| CodexModelMetadata {
                model_id: model_id.to_owned(),
                state: if compatible {
                    CapabilityState::KnownNonEmpty
                } else {
                    CapabilityState::NotFound
                },
                default_reasoning_effort: compatible.then_some("high".to_owned()),
                supported_reasoning_efforts: if compatible {
                    vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()]
                } else {
                    Vec::new()
                },
                source: if compatible {
                    CapabilitySource::VendorCompatibility
                } else {
                    CapabilitySource::CodexBuiltin
                },
                source_id: if compatible {
                    "dayway-ds-deepseek-effort-v1".to_owned()
                } else {
                    "codex.model/list".to_owned()
                },
            })
            .collect(),
        );
        let bytes = render_with_snapshot(
            &[
                "deepseek-v4-flash".to_owned(),
                "deepseek-v4-pro".to_owned(),
                "deepseek-v4-pro-preview".to_owned(),
            ],
            "deepseek-v4-pro",
            Some(&snapshot),
        )
        .expect("json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("catalog");
        for model_id in ["deepseek-v4-flash", "deepseek-v4-pro"] {
            let model = value["models"]
                .as_array()
                .expect("models")
                .iter()
                .find(|model| model["slug"] == model_id)
                .expect("compatible model");
            assert_eq!(model["default_reasoning_level"], "high");
            assert_eq!(
                model["supported_reasoning_levels"],
                serde_json::json!([
                    {"effort": "low", "description": "Codex 声明的 low 推理档位"},
                    {"effort": "medium", "description": "Codex 声明的 medium 推理档位"},
                    {"effort": "high", "description": "Codex 声明的 high 推理档位"}
                ])
            );
        }
        let preview = value["models"]
            .as_array()
            .expect("models")
            .iter()
            .find(|model| model["slug"] == "deepseek-v4-pro-preview")
            .expect("preview model");
        assert!(
            preview["supported_reasoning_levels"]
                .as_array()
                .expect("levels")
                .is_empty()
        );
        assert!(preview["default_reasoning_level"].is_null());
    }
}
