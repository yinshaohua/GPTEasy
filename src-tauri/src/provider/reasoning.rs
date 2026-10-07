use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use super::reasoning_capability::{
    CapabilitySource, CapabilityState, CodexMetadataSnapshot, ReasoningCapability,
};

pub(crate) const UNMAPPED_PROVIDER_RULE: &str = "unmapped-provider-v1";
pub(crate) const OPENAI_HIGH_RULE: &str = "openai-official-high-v1";
pub(crate) const DEEPSEEK_HIGH_RULE: &str = "deepseek-official-high-equivalent-v1";
pub(crate) const DAYWAY_DS_DEEPSEEK_RULE: &str = "dayway-ds-deepseek-effort-v1";
pub(crate) const CODEX_MODEL_INFO_SCHEMA: &str = "codex_model_info_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReasoningSelection {
    pub effort: Option<String>,
    pub rule_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReasoningAuditContext {
    pub operation_id: String,
    pub target_environment: String,
    pub provider_id: String,
    pub base_url: String,
    pub default_model: String,
    pub selection: ReasoningSelection,
    pub capability: Option<ReasoningCapability>,
    pub codex_version: Option<String>,
    codex_model_match: &'static str,
}

impl ReasoningAuditContext {
    #[cfg(test)]
    pub(crate) fn new(provider_id: String, base_url: String, default_model: String) -> Self {
        Self::for_environment("native", provider_id, base_url, default_model, None)
    }

    pub(crate) fn for_environment(
        target_environment: &str,
        provider_id: String,
        base_url: String,
        default_model: String,
        capability_snapshot: Option<&CodexMetadataSnapshot>,
    ) -> Self {
        Self::for_provider_environment(
            target_environment,
            provider_id,
            None,
            base_url,
            default_model,
            capability_snapshot,
        )
    }

    pub(crate) fn for_provider_environment(
        target_environment: &str,
        provider_id: String,
        provider_name: Option<&str>,
        base_url: String,
        default_model: String,
        capability_snapshot: Option<&CodexMetadataSnapshot>,
    ) -> Self {
        let selection = provider_name
            .map(|name| for_provider(name, &base_url))
            .unwrap_or_else(|| for_base_url(&base_url));
        let capability =
            capability_snapshot.and_then(|snapshot| snapshot.capability_for(&default_model));
        let codex_model_match = match capability_snapshot {
            None => "unknown",
            Some(_) => match capability.as_ref().map(|value| value.state) {
                Some(CapabilityState::NotFound) | None => "not_found",
                Some(CapabilityState::ProbeFailed) => "unknown",
                Some(_) => "recognized",
            },
        };
        Self {
            operation_id: Uuid::new_v4().to_string(),
            target_environment: target_environment.to_owned(),
            provider_id,
            base_url,
            default_model,
            selection,
            capability,
            codex_version: capability_snapshot.and_then(|snapshot| snapshot.codex_version.clone()),
            codex_model_match,
        }
    }

    pub(crate) fn unknown(target_environment: &str) -> Self {
        Self::for_environment(
            target_environment,
            String::new(),
            String::new(),
            String::new(),
            None,
        )
    }
}

impl ReasoningSelection {
    pub(crate) fn unmapped() -> Self {
        Self {
            effort: None,
            rule_id: UNMAPPED_PROVIDER_RULE.to_owned(),
        }
    }
}

impl Default for ReasoningSelection {
    fn default() -> Self {
        Self::unmapped()
    }
}

pub(crate) fn for_base_url(base_url: &str) -> ReasoningSelection {
    let Ok(url) = Url::parse(base_url) else {
        return ReasoningSelection::unmapped();
    };
    let Some(host) = url.host_str() else {
        return ReasoningSelection::unmapped();
    };
    let host = host.to_ascii_lowercase();
    match host.as_str() {
        "api.openai.com" => ReasoningSelection {
            effort: Some("high".to_owned()),
            rule_id: OPENAI_HIGH_RULE.to_owned(),
        },
        "api.deepseek.com" => ReasoningSelection {
            effort: Some("high".to_owned()),
            rule_id: DEEPSEEK_HIGH_RULE.to_owned(),
        },
        _ => ReasoningSelection::unmapped(),
    }
}

pub(crate) fn for_provider(provider_name: &str, base_url: &str) -> ReasoningSelection {
    if provider_name.eq_ignore_ascii_case("DayWay-DS") {
        return ReasoningSelection {
            effort: Some("high".to_owned()),
            rule_id: DAYWAY_DS_DEEPSEEK_RULE.to_owned(),
        };
    }
    for_base_url(base_url)
}

pub(crate) fn is_valid_effort(effort: &str) -> bool {
    matches!(effort, "low" | "medium" | "high" | "xhigh")
}

pub(crate) fn audit_details(
    context: Option<&ReasoningAuditContext>,
    stage: &str,
    status: &str,
    pending_restart: Option<bool>,
) -> String {
    let operation_id = context
        .map(|context| context.operation_id.as_str())
        .unwrap_or("unknown");
    let target_environment = context
        .map(|context| context.target_environment.as_str())
        .unwrap_or("unknown");
    let provider_ref = context
        .filter(|context| !context.provider_id.is_empty())
        .map(|context| short_digest(&context.provider_id))
        .unwrap_or_else(|| "unknown".to_owned());
    let provider_host_ref = context
        .and_then(|context| provider_host(&context.base_url))
        .map(|host| short_digest(&host))
        .unwrap_or_else(|| "unknown".to_owned());
    let model_ref = context
        .filter(|context| !context.default_model.is_empty())
        .map(|context| short_digest(&context.default_model))
        .unwrap_or_else(|| "unknown".to_owned());
    let rule_id = context
        .map(|context| context.selection.rule_id.as_str())
        .unwrap_or("unknown");
    let target_effort = context
        .and_then(|context| context.selection.effort.as_deref())
        .unwrap_or("none");
    let mapping = context
        .map(|context| {
            if context.selection.effort.is_some() {
                "resolved"
            } else {
                "missing"
            }
        })
        .unwrap_or("unknown");
    let capability_state = context
        .and_then(|context| {
            context
                .capability
                .as_ref()
                .map(|capability| capability.state)
        })
        .map(capability_state_name)
        .or_else(|| {
            context
                .filter(|context| context.codex_model_match == "not_found")
                .map(|_| "not_found")
        })
        .unwrap_or("unknown");
    let capability_source = context
        .and_then(|context| {
            context
                .capability
                .as_ref()
                .map(|capability| capability.source)
        })
        .map(capability_source_name)
        .unwrap_or("unknown");
    let source_id_ref = context
        .and_then(|context| {
            context
                .capability
                .as_ref()
                .and_then(|capability| capability.source_id.as_deref())
        })
        .filter(|source_id| !source_id.is_empty())
        .map(short_digest)
        .unwrap_or_else(|| "unknown".to_owned());
    let codex_version_ref = context
        .and_then(|context| context.codex_version.as_deref())
        .filter(|version| !version.is_empty())
        .map(short_digest)
        .unwrap_or_else(|| "unknown".to_owned());
    let capability_message_id = context
        .and_then(|context| context.capability.as_ref())
        .map(|capability| capability.message_id.as_str())
        .or_else(|| {
            context.and_then(|context| {
                (context.codex_model_match == "not_found")
                    .then_some("reasoning_metadata.codex_not_found")
            })
        })
        .unwrap_or("reasoning_metadata.unknown");
    let metadata_stage = if status == "started" && stage == "reasoning_metadata.inspect" {
        "reasoning_metadata.inspect"
    } else {
        context
            .map(metadata_stage)
            .unwrap_or("reasoning_metadata.unknown")
    };
    let restart = match pending_restart {
        Some(true) => "pending",
        Some(false) => "not_required",
        None => "unknown",
    };
    let codex_model_match = codex_model_match(context);
    let vendor_fallback = context
        .and_then(|context| context.capability.as_ref())
        .is_some_and(|capability| capability.source == CapabilitySource::VendorCompatibility);
    format!(
        "operation_id={operation_id}; target_environment={target_environment}; provider_ref={provider_ref}; provider_host_ref={provider_host_ref}; model_ref={model_ref}; capability_state={capability_state}; capability_source={capability_source}; source_id_ref={source_id_ref}; codex_version_ref={codex_version_ref}; capability_message_id={capability_message_id}; codex_model_match={codex_model_match}; vendor_fallback={}; metadata_stage={metadata_stage}; rule_id={rule_id}; target_effort={target_effort}; mapping={mapping}; catalog_schema={CODEX_MODEL_INFO_SCHEMA}; stage={stage}; status={status}; codex_restart={restart}",
        if vendor_fallback {
            "enabled"
        } else {
            "disabled"
        }
    )
}

fn capability_state_name(state: CapabilityState) -> &'static str {
    match state {
        CapabilityState::KnownNonEmpty => "known_nonempty",
        CapabilityState::KnownEmpty => "known_empty",
        CapabilityState::NotFound => "not_found",
        CapabilityState::ProbeFailed => "probe_failed",
        CapabilityState::Conflict => "conflict",
    }
}

fn capability_source_name(source: CapabilitySource) -> &'static str {
    match source {
        CapabilitySource::CodexBuiltin => "codex_builtin",
        CapabilitySource::ProviderDeclared => "provider_declared",
        CapabilitySource::VendorCompatibility => "vendor_compatibility",
        CapabilitySource::Unknown => "unknown",
        CapabilitySource::Conflict => "conflict",
    }
}

fn codex_model_match(context: Option<&ReasoningAuditContext>) -> &'static str {
    context
        .map(|context| context.codex_model_match)
        .unwrap_or("unknown")
}

fn metadata_stage(context: &ReasoningAuditContext) -> &'static str {
    let Some(capability) = context.capability.as_ref() else {
        return "reasoning_metadata.unknown";
    };
    if capability.state == CapabilityState::Conflict
        || capability.source == CapabilitySource::Conflict
    {
        return "reasoning_metadata.conflict";
    }
    match capability.source {
        CapabilitySource::CodexBuiltin if capability.state != CapabilityState::ProbeFailed => {
            "reasoning_metadata.codex_probe"
        }
        CapabilitySource::ProviderDeclared => "reasoning_metadata.provider_declaration",
        CapabilitySource::VendorCompatibility => "reasoning_metadata.vendor_fallback",
        _ => "reasoning_metadata.unknown",
    }
}

fn provider_host(base_url: &str) -> Option<String> {
    Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
}

fn short_digest(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let short = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{short}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_only_official_hosts() {
        assert_eq!(
            for_base_url("https://api.openai.com/v1"),
            ReasoningSelection {
                effort: Some("high".to_owned()),
                rule_id: OPENAI_HIGH_RULE.to_owned(),
            }
        );
        assert_eq!(
            for_base_url("https://API.DEEPSEEK.COM/v1"),
            ReasoningSelection {
                effort: Some("high".to_owned()),
                rule_id: DEEPSEEK_HIGH_RULE.to_owned(),
            }
        );
        assert_eq!(
            for_base_url("https://provider.example/v1"),
            ReasoningSelection::unmapped()
        );
    }

    #[test]
    fn mapping_does_not_depend_on_model_id() {
        assert_eq!(
            for_base_url("https://api.openai.com/v1").effort,
            Some("high".to_owned())
        );
        assert_eq!(
            for_base_url("https://api.deepseek.com/v1").effort,
            Some("high".to_owned())
        );
    }

    #[test]
    fn maps_dayway_ds_by_exact_provider_profile() {
        assert_eq!(
            for_provider("DayWay-DS", "https://dayway.site/v1"),
            ReasoningSelection {
                effort: Some("high".to_owned()),
                rule_id: DAYWAY_DS_DEEPSEEK_RULE.to_owned(),
            }
        );
        assert_eq!(
            for_provider("dayway-ds", "https://provider.example/v1").rule_id,
            DAYWAY_DS_DEEPSEEK_RULE
        );
        assert_eq!(
            for_provider("DayWay", "https://dayway.site/v1"),
            ReasoningSelection::unmapped()
        );
    }

    #[test]
    fn audit_details_are_safe_and_distinguish_mapping_and_restart_state() {
        let context = ReasoningAuditContext::new(
            "provider-id-with-secret".to_owned(),
            "https://api.openai.com/v1".to_owned(),
            "model/with-secret-looking-name".to_owned(),
        );
        let details = audit_details(
            Some(&context),
            "reasoning_metadata.commit",
            "applied",
            Some(true),
        );
        assert!(details.contains("operation_id="));
        assert!(details.contains("target_environment=native"));
        assert!(details.contains("provider_host_ref=sha256:"));
        assert!(details.contains("rule_id=openai-official-high-v1"));
        assert!(details.contains("target_effort=high"));
        assert!(details.contains("capability_state=unknown"));
        assert!(details.contains("capability_source=unknown"));
        assert!(details.contains("capability_message_id=reasoning_metadata.unknown"));
        assert!(details.contains("metadata_stage=reasoning_metadata.unknown"));
        assert!(details.contains("stage=reasoning_metadata.commit"));
        assert!(details.contains("catalog_schema=codex_model_info_v1"));
        assert!(details.contains("codex_restart=pending"));
        assert!(details.contains("vendor_fallback=disabled"));
        assert!(!details.contains("provider-id-with-secret"));
        assert!(!details.contains("model/with-secret-looking-name"));
        assert!(!details.contains("api.openai.com"));
        assert!(!details.contains("https://"));
    }

    #[test]
    fn audit_details_trace_exact_capability_without_exposing_source_or_version() {
        let snapshot = CodexMetadataSnapshot {
            schema_version: 1,
            target_environment: "native".to_owned(),
            codex_version: Some("codex-cli 0.160.1".to_owned()),
            executable_fingerprint: Some("sha256:executable".to_owned()),
            observed_at_epoch_seconds: 1,
            status: super::super::reasoning_capability::SnapshotStatus::Complete,
            models: vec![super::super::reasoning_capability::CodexModelMetadata {
                model_id: "gpt-6.1-sol".to_owned(),
                state: CapabilityState::KnownNonEmpty,
                default_reasoning_effort: Some("medium".to_owned()),
                supported_reasoning_efforts: vec!["low".to_owned(), "medium".to_owned()],
                source: CapabilitySource::CodexBuiltin,
                source_id: "codex.model/list".to_owned(),
            }],
        };
        let context = ReasoningAuditContext::for_environment(
            "linux_export",
            "provider-sensitive".to_owned(),
            "https://sensitive.example/v1".to_owned(),
            "gpt-6.1-sol".to_owned(),
            Some(&snapshot),
        );
        let details = audit_details(Some(&context), "reasoning_metadata.commit", "applied", None);
        assert!(details.contains("target_environment=linux_export"));
        assert!(details.contains("capability_state=known_nonempty"));
        assert!(details.contains("capability_source=codex_builtin"));
        assert!(details.contains("metadata_stage=reasoning_metadata.codex_probe"));
        assert!(details.contains("capability_message_id=reasoning_metadata.codex_known_nonempty"));
        assert!(details.contains("codex_model_match=recognized"));
        assert!(details.contains("source_id_ref=sha256:"));
        assert!(details.contains("codex_version_ref=sha256:"));
        assert!(!details.contains("provider-sensitive"));
        assert!(!details.contains("sensitive.example"));
        assert!(!details.contains("gpt-6.1-sol"));
        assert!(!details.contains("codex.model/list"));
        assert!(!details.contains("codex-cli 0.160.1"));
    }

    #[test]
    fn audit_details_distinguish_not_found_and_conflict() {
        let not_found = ReasoningAuditContext::for_environment(
            "native",
            "provider".to_owned(),
            "https://provider.example/v1".to_owned(),
            "missing-model".to_owned(),
            Some(&CodexMetadataSnapshot {
                schema_version: 1,
                target_environment: "native".to_owned(),
                codex_version: Some("codex-cli 0.160.1".to_owned()),
                executable_fingerprint: Some("sha256:executable".to_owned()),
                observed_at_epoch_seconds: 1,
                status: super::super::reasoning_capability::SnapshotStatus::Complete,
                models: vec![super::super::reasoning_capability::CodexModelMetadata {
                    model_id: "other-model".to_owned(),
                    state: CapabilityState::KnownNonEmpty,
                    default_reasoning_effort: Some("high".to_owned()),
                    supported_reasoning_efforts: vec!["high".to_owned()],
                    source: CapabilitySource::CodexBuiltin,
                    source_id: "codex.model/list".to_owned(),
                }],
            }),
        );
        let not_found_details = audit_details(
            Some(&not_found),
            "reasoning_metadata.unknown",
            "failed",
            None,
        );
        assert!(not_found_details.contains("capability_state=not_found"));
        assert!(not_found_details.contains("codex_model_match=not_found"));
        assert!(
            not_found_details.contains("capability_message_id=reasoning_metadata.codex_not_found")
        );
        assert!(not_found_details.contains("metadata_stage=reasoning_metadata.unknown"));

        let conflict_capability = ReasoningCapability {
            state: CapabilityState::Conflict,
            levels: Vec::new(),
            default_level: None,
            source: CapabilitySource::Conflict,
            source_id: Some("codex+provider".to_owned()),
            codex_version: Some("codex-cli 0.160.1".to_owned()),
            observed_at_epoch_seconds: 1,
            message_id: "reasoning_metadata.conflict".to_owned(),
        };
        let mut conflict = ReasoningAuditContext::new(
            "provider".to_owned(),
            "https://provider.example/v1".to_owned(),
            "model".to_owned(),
        );
        conflict.capability = Some(conflict_capability);
        conflict.codex_model_match = "recognized";
        let conflict_details =
            audit_details(Some(&conflict), "reasoning_metadata.render", "failed", None);
        assert!(conflict_details.contains("capability_state=conflict"));
        assert!(conflict_details.contains("capability_source=conflict"));
        assert!(conflict_details.contains("metadata_stage=reasoning_metadata.conflict"));
    }

    #[test]
    fn audit_details_records_vendor_fallback_without_exposing_provider_data() {
        let mut context = ReasoningAuditContext::for_provider_environment(
            "native",
            "provider-id".to_owned(),
            Some("DayWay-DS"),
            "https://dayway.site/v1".to_owned(),
            "deepseek-v4-pro".to_owned(),
            None,
        );
        context.capability = Some(ReasoningCapability {
            state: CapabilityState::KnownNonEmpty,
            levels: vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()],
            default_level: Some("high".to_owned()),
            source: CapabilitySource::VendorCompatibility,
            source_id: Some("dayway-ds-deepseek-effort-v1".to_owned()),
            codex_version: None,
            observed_at_epoch_seconds: 1,
            message_id: "reasoning_metadata.vendor_fallback".to_owned(),
        });
        let details = audit_details(Some(&context), "reasoning_metadata.render", "applied", None);
        assert!(details.contains("rule_id=dayway-ds-deepseek-effort-v1"));
        assert!(details.contains("target_effort=high"));
        assert!(details.contains("vendor_fallback=enabled"));
        assert!(details.contains("capability_source=vendor_compatibility"));
        assert!(!details.contains("DayWay-DS"));
        assert!(!details.contains("deepseek-v4-pro"));
    }
}
