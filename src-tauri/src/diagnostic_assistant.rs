use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::diagnostic_report::DiagnosticReport;

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

pub async fn analyze(
    provider_id: String,
    provider_name: String,
    base_url: String,
    api_key: String,
    model: String,
    report: &DiagnosticReport,
) -> Result<DiagnosticAssistantResult, DiagnosticAssistantFailure> {
    let prompt = build_prompt(report);
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
    let repair_plan = parsed
        .repair_plan
        .into_iter()
        .find_map(|item| {
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
        .into_iter()
        .collect();
    Ok(DiagnosticAssistantResult {
        provider_id,
        provider_name,
        explanation: parsed.explanation,
        repair_plan,
    })
}

fn build_prompt(report: &DiagnosticReport) -> String {
    let redacted_report = report.redacted_json();
    format!(
        concat!(
            "你是 GPTEasy 的诊断助手。只分析下面的结构化脱敏诊断，不要猜测或要求任何密钥、token、完整配置或请求正文。使用简体中文回答。\n\n",
            "请返回 JSON：{{\"explanation\": string, \"repairPlan\": [{{\"findingCode\": string, \"title\": string, \"description\": string}}]}}。",
            "修复计划只能描述用户确认后由 GPTEasy 确定性流程执行的动作，不得声称已经修改文件。\n\n诊断：{}"
        ),
        redacted_report
    )
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
struct ParsedPlanItem {
    finding_code: String,
    title: String,
    description: String,
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

    use super::{build_prompt, parse_assistant_json};
    use crate::codex::LoginStatus;
    use crate::consumer::ConsumerStatus;
    use crate::diagnostic_report::{DiagnosticApplication, DiagnosticObservations};

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
    fn prompt_contains_only_the_structured_redacted_report() {
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

        let prompt = build_prompt(&report);

        assert!(prompt.contains("\"schemaVersion\""));
        assert!(prompt.contains("\"declaredProviders\""));
        assert!(!prompt.contains("never-send-this-secret"));
        assert!(!prompt.contains("never-send-this-token"));
        assert!(!prompt.contains("never-send-this-body"));
        assert!(!prompt.contains("request_body"));
        assert!(!prompt.contains("repairPreview"));
    }
}
