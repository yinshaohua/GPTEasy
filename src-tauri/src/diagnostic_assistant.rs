use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::diagnostic_report::DiagnosticReport;
use crate::environment::{AuthenticationMode, EnvironmentSnapshot, EnvironmentState};
use crate::provider::ProviderSummary;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticManagementContext {
    environment_inspection: DiagnosticEnvironmentInspection,
    provider_catalog: DiagnosticProviderCatalog,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticEnvironmentInspection {
    status: &'static str,
    state: Option<EnvironmentState>,
    mode: Option<AuthenticationMode>,
    message_id: Option<String>,
    revision: Option<String>,
    actual_current_provider_id: Option<String>,
    actual_current_provider_name: Option<String>,
    takeover_available: Option<bool>,
    pending_restart: Option<bool>,
    error_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticProviderCatalog {
    status: &'static str,
    entries: Vec<DiagnosticProviderCatalogEntry>,
    error_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticProviderCatalogEntry {
    id: String,
    name: String,
    recorded_current: bool,
    verified_at_epoch_seconds: u64,
}

impl DiagnosticManagementContext {
    pub(crate) fn inspect(
        environment: Result<EnvironmentSnapshot, &'static str>,
        providers: Result<Vec<ProviderSummary>, &'static str>,
    ) -> Self {
        let environment_inspection = match environment {
            Ok(snapshot) => DiagnosticEnvironmentInspection {
                status: "available",
                state: Some(snapshot.state),
                mode: snapshot.mode,
                message_id: Some(snapshot.message_id.to_owned()),
                revision: Some(snapshot.revision),
                actual_current_provider_id: snapshot
                    .current_provider
                    .as_ref()
                    .map(|provider| provider.id.clone()),
                actual_current_provider_name: snapshot
                    .current_provider
                    .map(|provider| provider.name),
                takeover_available: Some(snapshot.takeover_available),
                pending_restart: Some(snapshot.pending_restart),
                error_message_id: None,
            },
            Err(message_id) => DiagnosticEnvironmentInspection {
                status: "unavailable",
                state: None,
                mode: None,
                message_id: None,
                revision: None,
                actual_current_provider_id: None,
                actual_current_provider_name: None,
                takeover_available: None,
                pending_restart: None,
                error_message_id: Some(message_id.to_owned()),
            },
        };
        let provider_catalog = match providers {
            Ok(providers) => DiagnosticProviderCatalog {
                status: "available",
                entries: providers
                    .into_iter()
                    .map(|provider| DiagnosticProviderCatalogEntry {
                        id: provider.id,
                        name: provider.name,
                        recorded_current: provider.is_current,
                        verified_at_epoch_seconds: provider.verified_at_epoch_seconds,
                    })
                    .collect(),
                error_message_id: None,
            },
            Err(message_id) => DiagnosticProviderCatalog {
                status: "unavailable",
                entries: Vec::new(),
                error_message_id: Some(message_id.to_owned()),
            },
        };
        Self {
            environment_inspection,
            provider_catalog,
        }
    }

    pub(crate) fn redacted_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_owned())
    }

    pub(crate) fn redacted_markdown(&self) -> String {
        let environment = &self.environment_inspection;
        let catalog = &self.provider_catalog;
        let entries = if catalog.entries.is_empty() {
            "无".to_owned()
        } else {
            catalog
                .entries
                .iter()
                .map(|entry| {
                    format!(
                        "{} (`{}`，目录记录当前={})",
                        markdown_context_text(&entry.name),
                        markdown_context_text(&entry.id),
                        entry.recorded_current
                    )
                })
                .collect::<Vec<_>>()
                .join("、")
        };
        format!(
            concat!(
                "## GPTEasy 管理上下文\n\n",
                "| 项目 | 状态 |\n| --- | --- |\n",
                "| 环境检查 | {} |\n",
                "| 环境状态 | {:?} |\n",
                "| 认证模式 | {:?} |\n",
                "| 环境消息 ID | {} |\n",
                "| 环境检查错误 | {} |\n",
                "| 环境版本指纹 | {} |\n",
                "| 实际当前供应商 | {} |\n",
                "| 可确认接管 | {:?} |\n",
                "| 待重启 | {:?} |\n",
                "| 供应商目录检查 | {} |\n",
                "| 供应商目录错误 | {} |\n",
                "| 已验证供应商目录 | {} |\n\n",
            ),
            environment.status,
            environment.state,
            environment.mode,
            environment.message_id.as_deref().unwrap_or("无法确认"),
            environment.error_message_id.as_deref().unwrap_or("无"),
            environment.revision.as_deref().unwrap_or("无法确认"),
            environment
                .actual_current_provider_name
                .as_deref()
                .unwrap_or("无"),
            environment.takeover_available,
            environment.pending_restart,
            catalog.status,
            catalog.error_message_id.as_deref().unwrap_or("无"),
            entries,
        )
    }
}

fn markdown_context_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\r', '\n'], " ")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRepairPlanItem {
    pub id: String,
    pub finding_code: String,
    pub title: String,
    pub description: String,
    pub action: &'static str,
    pub preview_id: Option<String>,
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticAssistantResult {
    pub provider_id: String,
    pub provider_name: String,
    pub explanation: String,
    pub repair_plan: Vec<DiagnosticRepairPlanItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticConversationMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticChatResult {
    pub provider_id: String,
    pub provider_name: String,
    pub reply: String,
    pub repair_plan: Vec<DiagnosticRepairPlanItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticAssistantFailure {
    pub message_id: &'static str,
}

#[derive(Debug, Serialize)]
struct ResponsesRequest<'a> {
    model: &'a str,
    input: String,
}

#[derive(Debug, Deserialize)]
struct ResponsesResponse {
    output_text: Option<String>,
    output: Option<Vec<OutputItem>>,
}

#[derive(Debug, Deserialize)]
struct OutputItem {
    content: Option<Vec<OutputContent>>,
}

#[derive(Debug, Deserialize)]
struct OutputContent {
    text: Option<String>,
}

pub(crate) async fn analyze(
    provider_id: String,
    provider_name: String,
    base_url: String,
    api_key: String,
    model: String,
    report: &DiagnosticReport,
    management: &DiagnosticManagementContext,
) -> Result<DiagnosticAssistantResult, DiagnosticAssistantFailure> {
    let prompt = build_prompt(report, management);
    let endpoint = format!("{}/responses", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| failed())?;
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&ResponsesRequest {
            model: &model,
            input: prompt,
        })
        .send()
        .await
        .map_err(|_| failed())?;
    if !response.status().is_success() {
        return Err(if response.status().as_u16() == 401 {
            DiagnosticAssistantFailure {
                message_id: "diagnostics.assistant_authentication_failed",
            }
        } else {
            failed()
        });
    }
    let payload = response
        .json::<ResponsesResponse>()
        .await
        .map_err(|_| failed())?;
    let text = payload
        .output_text
        .or_else(|| {
            payload.output.and_then(|items| {
                Some(
                    items
                        .into_iter()
                        .flat_map(|item| item.content.unwrap_or_default())
                        .filter_map(|content| content.text)
                        .collect::<Vec<_>>()
                        .join(""),
                )
            })
        })
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(failed)?;
    let parsed = parse_assistant_json(&text);
    let repair_plan = build_repair_plan(report, parsed.repair_plan);
    Ok(DiagnosticAssistantResult {
        provider_id,
        provider_name,
        explanation: parsed.explanation,
        repair_plan,
    })
}

pub(crate) async fn chat(
    provider_id: String,
    provider_name: String,
    base_url: String,
    api_key: String,
    model: String,
    report: &DiagnosticReport,
    management: &DiagnosticManagementContext,
    message: String,
    history: &[DiagnosticConversationMessage],
) -> Result<DiagnosticChatResult, DiagnosticAssistantFailure> {
    let prompt = build_chat_prompt(report, management, &message, history);
    let endpoint = format!("{}/responses", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| failed())?;
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&ResponsesRequest {
            model: &model,
            input: prompt,
        })
        .send()
        .await
        .map_err(|_| failed())?;
    if !response.status().is_success() {
        return Err(if response.status().as_u16() == 401 {
            DiagnosticAssistantFailure {
                message_id: "diagnostics.assistant_authentication_failed",
            }
        } else {
            failed()
        });
    }
    let payload = response
        .json::<ResponsesResponse>()
        .await
        .map_err(|_| failed())?;
    let text = payload
        .output_text
        .or_else(|| {
            payload.output.and_then(|items| {
                Some(
                    items
                        .into_iter()
                        .flat_map(|item| item.content.unwrap_or_default())
                        .filter_map(|content| content.text)
                        .collect::<Vec<_>>()
                        .join(""),
                )
            })
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(failed)?;
    let parsed = parse_chat_json(&text);
    Ok(DiagnosticChatResult {
        provider_id,
        provider_name,
        reply: parsed.reply,
        repair_plan: build_repair_plan(report, parsed.repair_plan),
    })
}

fn build_prompt(report: &DiagnosticReport, management: &DiagnosticManagementContext) -> String {
    let redacted_report = report.redacted_json();
    let management_context = management.redacted_json();
    format!(
        concat!(
            "你是 GPTEasy 的诊断助手。只分析下面的结构化脱敏诊断，不要猜测或要求任何密钥、token、完整配置或请求正文。使用简体中文回答。\n\n",
            "请返回 JSON：{{\"explanation\": string, \"repairPlan\": [{{\"findingCode\": string, \"title\": string, \"description\": string}}]}}。",
            "修复计划只能描述用户确认后由 GPTEasy 确定性流程执行的动作，不得声称已经修改文件。",
            "必须区分 Codex 配置中的 declaredProviders、GPTEasy 已验证供应商目录以及环境实际当前供应商；三者不相等时要明确指出。",
            "回答应包含已观察事实、判断与证据、建议步骤、仍无法确认的信息，并保留相关 messageId 便于导出后排错。",
            "不要把登录状态未认证等同于供应商目录为空，也不要把 declaredProviders 为空描述为 GPTEasy 尚未保存供应商。",
            "当前返回协议只有 repair_custom_provider 能形成可确认 repairPlan；apply_verified_provider、switch_openai_login 和 restore_last_environment_config 只能作为界面操作建议，不得声称本次对话已经生成可执行原子计划。",
            "\n\nCodex 配置与本机诊断：{}\n\nGPTEasy 管理上下文：{}"
        ),
        redacted_report, management_context,
    )
}

fn build_chat_prompt(
    report: &DiagnosticReport,
    management: &DiagnosticManagementContext,
    message: &str,
    history: &[DiagnosticConversationMessage],
) -> String {
    let runbook = include_str!("../../docs/diagnostics/ASSISTANT-RUNBOOK.md");
    let redacted_report = report.redacted_json();
    let management_context = management.redacted_json();
    let transcript = history
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|item| format!("{}: {}", item.role, redact_user_text(&item.content)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "你是 GPTEasy 诊断助手。使用简体中文。只能依据运行手册、脱敏诊断、GPTEasy 管理上下文和对话回答。不要索取密钥、token、完整配置或私密日志。只能提出运行手册中已注册的动作，不能声称已经修改。请返回 JSON：{{\"reply\": string, \"repairPlan\": [{{\"findingCode\": string, \"title\": string, \"description\": string}}]}}。回复必须详细包含：1. 已观察事实；2. 判断与逐项证据；3. 可执行的下一步；4. 仍无法确认的信息。必须保留相关 messageId。Codex 配置中的 declaredProviders 只是 config.toml 的声明，不是 GPTEasy 已验证供应商目录；登录状态也不代表目录是否为空。若三套状态不一致，必须明确列出，不得合并推断。当前返回协议只有 repair_custom_provider 能形成可确认 repairPlan；apply_verified_provider、switch_openai_login 和 restore_last_environment_config 只能作为界面操作建议，不得声称本次对话已经生成可执行原子计划。\n\n运行手册：{runbook}\n\nCodex 配置与本机诊断：{redacted_report}\n\nGPTEasy 管理上下文：{management_context}\n\n历史对话：{transcript}\n\n用户新问题：{}",
        redact_user_text(message)
    )
}

pub(crate) fn redact_user_text(value: &str) -> String {
    let mut output = value.to_owned();
    for marker in ["sk-", "api_key=", "OPENAI_API_KEY="] {
        while let Some(start) = output.find(marker) {
            let end = output[start..]
                .find(|character: char| {
                    character.is_whitespace() || character == '"' || character == '\''
                })
                .map(|offset| start + offset)
                .unwrap_or(output.len());
            output.replace_range(start..end, "[已脱敏]");
        }
    }
    output
}

fn build_repair_plan(
    report: &DiagnosticReport,
    items: Vec<ParsedPlanItem>,
) -> Vec<DiagnosticRepairPlanItem> {
    items
        .into_iter()
        .filter_map(|item| {
            let finding = report.findings.iter().find(|finding| {
                finding.code == item.finding_code
                    && finding.code == "model_provider_missing_definition"
                    && finding.repairable
            })?;
            let preview_id = report.repair_preview.as_ref()?.preview_id.clone();
            Some(DiagnosticRepairPlanItem {
                id: "repair-custom-provider".to_owned(),
                finding_code: finding.code.to_owned(),
                title: item.title,
                description: item.description,
                action: "repair_custom_provider",
                preview_id: Some(preview_id),
                requires_confirmation: true,
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParsedAssistant {
    explanation: String,
    #[serde(default)]
    repair_plan: Vec<ParsedPlanItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParsedChat {
    reply: String,
    #[serde(default)]
    repair_plan: Vec<ParsedPlanItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParsedPlanItem {
    finding_code: String,
    title: String,
    description: String,
}

fn parse_chat_json(text: &str) -> ParsedChat {
    let candidate = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let value = serde_json::from_str::<Value>(candidate).ok().or_else(|| {
        let start = candidate.find('{')?;
        let end = candidate.rfind('}')?;
        serde_json::from_str::<Value>(&candidate[start..=end]).ok()
    });
    value
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| ParsedChat {
            reply: text.trim().to_owned(),
            repair_plan: Vec::new(),
        })
}

fn parse_assistant_json(text: &str) -> ParsedAssistant {
    let candidate = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let value = serde_json::from_str::<Value>(candidate).ok().or_else(|| {
        let start = candidate.find('{')?;
        let end = candidate.rfind('}')?;
        serde_json::from_str::<Value>(&candidate[start..=end]).ok()
    });
    value
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| ParsedAssistant {
            explanation: text.trim().to_owned(),
            repair_plan: Vec::new(),
        })
}

fn failed() -> DiagnosticAssistantFailure {
    DiagnosticAssistantFailure {
        message_id: "diagnostics.assistant_failed",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{DiagnosticManagementContext, build_prompt, parse_assistant_json};
    use crate::codex::LoginStatus;
    use crate::consumer::ConsumerStatus;
    use crate::diagnostic_report::{DiagnosticApplication, DiagnosticObservations};
    use crate::provider::ProviderSummary;

    #[test]
    fn parses_json_embedded_in_markdown() {
        let parsed =
            parse_assistant_json("```json\n{\"explanation\":\"需要修复\",\"repairPlan\":[]}\n```");
        assert_eq!(parsed.explanation, "需要修复");
    }

    #[test]
    fn keeps_plain_text_when_model_does_not_return_json() {
        let parsed = parse_assistant_json("请先检查 Codex 配置。");
        assert_eq!(parsed.explanation, "请先检查 Codex 配置。");
        assert!(parsed.repair_plan.is_empty());
    }

    #[test]
    fn redacts_credentials_from_conversation_exports() {
        let redacted = super::redact_user_text(
            "sk-private-value api_key=second-secret OPENAI_API_KEY=third-secret",
        );

        assert_eq!(redacted, "[已脱敏] [已脱敏] [已脱敏]");
    }

    #[test]
    fn prompt_distinguishes_redacted_codex_state_from_the_provider_catalog() {
        let codex_home = tempdir().expect("temp codex home");
        fs::write(
            codex_home.path().join("config.toml"),
            concat!(
                "model_provider = \"custom\"\n",
                "[model_providers.custom]\n",
                "base_url = \"https://provider.example/v1\"\n",
                "api_key = \"never-send-this-secret\"\n",
                "request_body = \"never-send-this-body\"\n",
            ),
        )
        .expect("write config");
        fs::write(
            codex_home.path().join("auth.json"),
            r#"{"token":"never-send-this-token"}"#,
        )
        .expect("write auth");
        let report = DiagnosticApplication::new(codex_home.path(), None).inspect_with(
            &DiagnosticObservations {
                login_status: LoginStatus::LoggedIn,
                desktop_status: ConsumerStatus::Stopped,
                cli_status: ConsumerStatus::Stopped,
                codex_cli_version: None,
            },
            &[],
        );

        let management = DiagnosticManagementContext::inspect(
            Err("environment.state_unavailable"),
            Ok(vec![ProviderSummary {
                id: "provider-1".to_owned(),
                name: "Saved Provider".to_owned(),
                base_url: "https://private-catalog.example/v1".to_owned(),
                default_model: "private-catalog-model".to_owned(),
                verified_at_epoch_seconds: 1_786_140_000,
                is_current: true,
                recommendation_id: None,
                has_recommendation_update: false,
                recommendation_template_base_url: None,
            }]),
        );
        let prompt = build_prompt(&report, &management);

        assert!(prompt.contains("\"schemaVersion\""));
        assert!(prompt.contains("\"declaredProviders\""));
        assert!(prompt.contains("\"providerCatalog\""));
        assert!(prompt.contains("Saved Provider"));
        assert!(prompt.contains("environment.state_unavailable"));
        assert!(prompt.contains("不要把登录状态未认证等同于供应商目录为空"));
        assert!(!prompt.contains("never-send-this-secret"));
        assert!(!prompt.contains("never-send-this-token"));
        assert!(!prompt.contains("never-send-this-body"));
        assert!(!prompt.contains("request_body"));
        assert!(!prompt.contains("repairPreview"));
        assert!(!prompt.contains("private-catalog.example"));
        assert!(!prompt.contains("private-catalog-model"));
    }
}
