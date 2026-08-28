use std::fs;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

use super::{DiagnosticApplication, DiagnosticRepairStatus, DiagnosticReport};
use crate::commands::{EnvironmentRuntime, IssueLogRuntime, ProviderRuntime, WslRuntime};
use crate::diagnostic_assistant::{
    self, DiagnosticAssistantResult, DiagnosticChatResult, DiagnosticConversationMessage,
    DiagnosticManagementContext,
};
use crate::diagnostics::{IssueLogLevel, IssueLogRecord};

const DIAGNOSTIC_LOG_WINDOW_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Clone)]
pub(crate) struct DiagnosticRuntime {
    application: DiagnosticApplication,
}

impl DiagnosticRuntime {
    pub(crate) fn new(application: DiagnosticApplication) -> Self {
        Self { application }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticFailure {
    pub message_id: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticRepairExecution {
    status: DiagnosticRepairStatus,
    message_id: &'static str,
    report: DiagnosticReport,
}

#[tauri::command]
pub(crate) async fn get_diagnostic_report(
    runtime: State<'_, DiagnosticRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<DiagnosticReport, DiagnosticFailure> {
    let application = runtime.application.clone();
    let records = recent_error_logs(&logs);
    let result = tauri::async_runtime::spawn_blocking(move || application.inspect(&records))
        .await
        .map_err(|_| DiagnosticFailure {
            message_id: "diagnostics.report_failed",
        });
    log_diagnostic_failure(&logs, "diagnostics.report", &result);
    result
}

#[tauri::command]
pub(crate) async fn analyze_diagnostic_report(
    runtime: State<'_, DiagnosticRuntime>,
    providers: State<'_, ProviderRuntime>,
    environment: State<'_, EnvironmentRuntime>,
    wsl: State<'_, WslRuntime>,
    logs: State<'_, IssueLogRuntime>,
    provider_id: String,
) -> Result<DiagnosticAssistantResult, DiagnosticFailure> {
    let (provider, api_key) =
        providers
            .assistant_provider(&provider_id)
            .map_err(|_| DiagnosticFailure {
                message_id: "diagnostics.assistant_provider_unavailable",
            })?;
    let application = runtime.application.clone();
    let records = recent_error_logs(&logs);
    let report = tauri::async_runtime::spawn_blocking(move || application.inspect(&records))
        .await
        .map_err(|_| DiagnosticFailure {
            message_id: "diagnostics.assistant_failed",
        })?;
    let management = diagnostic_management_context(&providers, &environment, &wsl);
    let result = diagnostic_assistant::analyze(
        provider.id.clone(),
        provider.name,
        provider.base_url,
        api_key,
        provider.default_model,
        &report,
        &management,
    )
    .await
    .map_err(|failure| DiagnosticFailure {
        message_id: failure.message_id,
    });
    log_diagnostic_failure(&logs, "diagnostics.assistant", &result);
    result
}

#[tauri::command]
pub(crate) async fn chat_diagnostic_assistant(
    runtime: State<'_, DiagnosticRuntime>,
    providers: State<'_, ProviderRuntime>,
    environment: State<'_, EnvironmentRuntime>,
    wsl: State<'_, WslRuntime>,
    logs: State<'_, IssueLogRuntime>,
    provider_id: String,
    message: String,
    history: Vec<DiagnosticConversationMessage>,
) -> Result<DiagnosticChatResult, DiagnosticFailure> {
    let (provider, api_key) =
        providers
            .assistant_provider(&provider_id)
            .map_err(|_| DiagnosticFailure {
                message_id: "diagnostics.assistant_provider_unavailable",
            })?;
    let application = runtime.application.clone();
    let records = recent_error_logs(&logs);
    let report = tauri::async_runtime::spawn_blocking(move || application.inspect(&records))
        .await
        .map_err(|_| DiagnosticFailure {
            message_id: "diagnostics.assistant_failed",
        })?;
    let management = diagnostic_management_context(&providers, &environment, &wsl);
    let result = diagnostic_assistant::chat(
        provider.id.clone(),
        provider.name,
        provider.base_url,
        api_key,
        provider.default_model,
        &report,
        &management,
        message,
        &history,
    )
    .await
    .map_err(|failure| DiagnosticFailure {
        message_id: failure.message_id,
    });
    log_diagnostic_failure(&logs, "diagnostics.assistant.chat", &result);
    result
}

#[tauri::command]
pub(crate) async fn repair_diagnostic_custom_provider(
    runtime: State<'_, DiagnosticRuntime>,
    logs: State<'_, IssueLogRuntime>,
    preview_id: String,
) -> Result<DiagnosticRepairExecution, DiagnosticFailure> {
    let application = runtime.application.clone();
    let records = recent_error_logs(&logs);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let repair = application.repair_custom_provider(&preview_id);
        let report = application.inspect(&records);
        DiagnosticRepairExecution {
            status: repair.status,
            message_id: repair.message_id,
            report,
        }
    })
    .await
    .map_err(|_| DiagnosticFailure {
        message_id: "diagnostics.repair_failed",
    });
    log_diagnostic_failure(&logs, "diagnostics.repair_custom_provider", &result);
    result
}

#[tauri::command]
pub(crate) fn choose_diagnostic_export_destination(
    app: AppHandle,
    logs: State<'_, IssueLogRuntime>,
) -> Result<Option<String>, DiagnosticFailure> {
    let selected = app
        .dialog()
        .file()
        .set_file_name("gpteasy-diagnostic-report.md")
        .add_filter("Markdown", &["md"])
        .blocking_save_file();
    let result = selected
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|_| DiagnosticFailure {
                    message_id: "diagnostics.export_destination_invalid",
                })
        })
        .transpose();
    log_diagnostic_failure(&logs, "diagnostics.choose_report_destination", &result);
    result
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticBundleMessage {
    role: String,
    content: String,
}

#[tauri::command]
pub(crate) async fn export_diagnostic_bundle(
    runtime: State<'_, DiagnosticRuntime>,
    providers: State<'_, ProviderRuntime>,
    environment: State<'_, EnvironmentRuntime>,
    wsl: State<'_, WslRuntime>,
    logs: State<'_, IssueLogRuntime>,
    destination: String,
    conversation: Vec<DiagnosticBundleMessage>,
) -> Result<(), DiagnosticFailure> {
    let application = runtime.application.clone();
    let records = recent_error_logs(&logs);
    let management = diagnostic_management_context(&providers, &environment, &wsl);
    let conversation = sanitize_bundle_conversation(conversation);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let report = application.inspect(&records);
        let body = render_diagnostic_bundle_markdown(&report, &management, &conversation);
        fs::write(destination, body).map_err(|_| DiagnosticFailure {
            message_id: "diagnostics.export_failed",
        })
    })
    .await
    .map_err(|_| DiagnosticFailure {
        message_id: "diagnostics.export_failed",
    })
    .and_then(|result| result);
    log_diagnostic_failure(&logs, "diagnostics.export_bundle", &result);
    result
}

#[tauri::command]
pub(crate) async fn copy_diagnostic_bundle(
    app: AppHandle,
    runtime: State<'_, DiagnosticRuntime>,
    providers: State<'_, ProviderRuntime>,
    environment: State<'_, EnvironmentRuntime>,
    wsl: State<'_, WslRuntime>,
    logs: State<'_, IssueLogRuntime>,
    conversation: Vec<DiagnosticBundleMessage>,
) -> Result<(), DiagnosticFailure> {
    let application = runtime.application.clone();
    let records = recent_error_logs(&logs);
    let management = diagnostic_management_context(&providers, &environment, &wsl);
    let conversation = sanitize_bundle_conversation(conversation);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let report = application.inspect(&records);
        render_diagnostic_bundle_markdown(&report, &management, &conversation)
    })
    .await
    .map_err(|_| DiagnosticFailure {
        message_id: "diagnostics.copy_failed",
    })
    .and_then(|body| {
        app.clipboard()
            .write_text(body)
            .map_err(|_| DiagnosticFailure {
                message_id: "diagnostics.copy_failed",
            })
    });
    log_diagnostic_failure(&logs, "diagnostics.copy_bundle", &result);
    result
}

fn sanitize_bundle_conversation(
    conversation: Vec<DiagnosticBundleMessage>,
) -> Vec<DiagnosticBundleMessage> {
    conversation
        .into_iter()
        .take(100)
        .filter_map(|message| match message.role.as_str() {
            "user" | "assistant" | "system" => Some(DiagnosticBundleMessage {
                role: message.role,
                content: diagnostic_assistant::redact_user_text(&message.content),
            }),
            _ => None,
        })
        .collect()
}

fn render_diagnostic_bundle_markdown(
    report: &DiagnosticReport,
    management: &DiagnosticManagementContext,
    conversation: &[DiagnosticBundleMessage],
) -> String {
    let mut output = report.redacted_markdown();
    output.push('\n');
    output.push_str(&management.redacted_markdown());
    output.push_str("\n## 诊断助手对话\n\n");
    if conversation.is_empty() {
        output.push_str("未导出对话。\n");
    } else {
        for message in conversation {
            output.push_str(&format!(
                "- **{}**：{}\n",
                message.role,
                markdown_bundle_text(&message.content)
            ));
        }
    }
    output
}

fn diagnostic_management_context(
    providers: &ProviderRuntime,
    environment: &EnvironmentRuntime,
    wsl: &WslRuntime,
) -> DiagnosticManagementContext {
    DiagnosticManagementContext::inspect(
        environment.inspect().map_err(|failure| failure.message_id),
        providers.list().map_err(|failure| failure.message_id),
        wsl.inspect().map_err(|failure| failure.message_id),
    )
}

fn markdown_bundle_text(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn recent_error_logs(logs: &IssueLogRuntime) -> Vec<IssueLogRecord> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    logs.store.list_all(
        now.saturating_sub(DIAGNOSTIC_LOG_WINDOW_SECONDS),
        Some(IssueLogLevel::Error),
        None,
    )
}

fn log_diagnostic_failure<T>(
    logs: &IssueLogRuntime,
    event: &'static str,
    result: &Result<T, DiagnosticFailure>,
) {
    if let Err(failure) = result {
        logs.store.append(
            IssueLogLevel::Error,
            event,
            failure.message_id,
            Some("category=diagnostics".to_owned()),
        );
    }
}

pub(super) fn inspect_codex_cli_version() -> Option<String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/S", "/C", "codex --version"]);
        command
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut command = Command::new("codex");
        command.arg("--version");
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_hidden_process(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .find(|part| {
            part.chars()
                .next()
                .is_some_and(|first| first.is_ascii_digit())
        })
        .filter(|part| {
            part.len() <= 40
                && part
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || ".-+".contains(value))
        })
        .map(str::to_owned)
}

#[cfg(target_os = "windows")]
fn configure_hidden_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn configure_hidden_process(_command: &mut Command) {}
