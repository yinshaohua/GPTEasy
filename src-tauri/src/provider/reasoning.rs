use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

pub(crate) const UNMAPPED_PROVIDER_RULE: &str = "unmapped-provider-v1";
pub(crate) const OPENAI_HIGH_RULE: &str = "openai-official-high-v1";
pub(crate) const DEEPSEEK_HIGH_RULE: &str = "deepseek-official-high-equivalent-v1";
pub(crate) const CODEX_MODEL_INFO_SCHEMA: &str = "codex_model_info_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReasoningSelection {
    pub effort: Option<String>,
    pub rule_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReasoningAuditContext {
    pub provider_id: String,
    pub base_url: String,
    pub default_model: String,
    pub selection: ReasoningSelection,
}

impl ReasoningAuditContext {
    pub(crate) fn new(provider_id: String, base_url: String, default_model: String) -> Self {
        let selection = for_base_url(&base_url);
        Self {
            provider_id,
            base_url,
            default_model,
            selection,
        }
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

pub(crate) fn is_valid_effort(effort: &str) -> bool {
    matches!(effort, "low" | "medium" | "high" | "xhigh")
}

pub(crate) fn audit_details(
    context: Option<&ReasoningAuditContext>,
    stage: &str,
    status: &str,
    pending_restart: Option<bool>,
) -> String {
    let (provider_ref, provider_host, model_ref, rule_id, target_effort, mapping) = context
        .map(|context| {
            (
                short_digest(&context.provider_id),
                provider_host(&context.base_url),
                short_digest(&context.default_model),
                context.selection.rule_id.as_str(),
                context.selection.effort.as_deref().unwrap_or("none"),
                if context.selection.effort.is_some() {
                    "resolved"
                } else {
                    "missing"
                },
            )
        })
        .unwrap_or((
            "unknown".to_owned(),
            "unknown".to_owned(),
            "unknown".to_owned(),
            "unknown",
            "none",
            "unknown",
        ));
    let restart = match pending_restart {
        Some(true) => "pending",
        Some(false) => "not_required",
        None => "unknown",
    };
    format!(
        "provider_ref={provider_ref}; provider_host={provider_host}; model_ref={model_ref}; rule_id={rule_id}; target_effort={target_effort}; mapping={mapping}; catalog_schema={CODEX_MODEL_INFO_SCHEMA}; stage={stage}; status={status}; codex_restart={restart}"
    )
}

fn provider_host(base_url: &str) -> String {
    Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "invalid".to_owned())
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
    fn audit_details_are_safe_and_distinguish_mapping_and_restart_state() {
        let context = ReasoningAuditContext::new(
            "provider-id".to_owned(),
            "https://api.openai.com/v1".to_owned(),
            "model/with-secret-looking-name".to_owned(),
        );
        let details = audit_details(Some(&context), "config_write", "applied", Some(true));
        assert!(details.contains("provider_host=api.openai.com"));
        assert!(details.contains("rule_id=openai-official-high-v1"));
        assert!(details.contains("target_effort=high"));
        assert!(details.contains("catalog_schema=codex_model_info_v1"));
        assert!(details.contains("stage=config_write"));
        assert!(details.contains("codex_restart=pending"));
        assert!(!details.contains("provider-id"));
        assert!(!details.contains("model/with-secret-looking-name"));
        assert!(!details.contains("https://"));
    }
}
