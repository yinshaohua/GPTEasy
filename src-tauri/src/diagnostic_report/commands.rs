use std::fs;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::{DiagnosticApplication, DiagnosticReport};
use crate::commands::IssueLogRuntime;
use crate::diagnostics::{IssueLogLevel, IssueLogRecord};

const DIAGNOSTIC_LOG_WINDOW_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone)]
pub(crate) struct DiagnosticRuntime {
    application: DiagnosticApplication,
}

impl DiagnosticRuntime {
    pub(crate) fn new(application: DiagnosticApplication) -> Self {
        Self { application }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiagnosticExportFormat {
    Json,
    Markdown,
}

impl DiagnosticExportFormat {
    fn file_name(self) -> &'static str {
        match self {
            Self::Json => "gpteasy-diagnostic-report.json",
            Self::Markdown => "gpteasy-diagnostic-report.md",
        }
    }

    fn dialog_filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Json => ("JSON", &["json"]),
            Self::Markdown => ("Markdown", &["md"]),
        }
    }

    fn render(self, report: &DiagnosticReport) -> String {
        match self {
            Self::Json => report.redacted_json(),
            Self::Markdown => report.redacted_markdown(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticFailure {
    pub message_id: &'static str,
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
pub(crate) fn choose_diagnostic_export_destination(
    app: AppHandle,
    logs: State<'_, IssueLogRuntime>,
    format: DiagnosticExportFormat,
) -> Result<Option<String>, DiagnosticFailure> {
    let (filter_name, extensions) = format.dialog_filter();
    let selected = app
        .dialog()
        .file()
        .set_file_name(format.file_name())
        .add_filter(filter_name, extensions)
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

#[tauri::command]
pub(crate) async fn export_diagnostic_report(
    runtime: State<'_, DiagnosticRuntime>,
    logs: State<'_, IssueLogRuntime>,
    format: DiagnosticExportFormat,
    destination: String,
) -> Result<(), DiagnosticFailure> {
    let application = runtime.application.clone();
    let records = recent_error_logs(&logs);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let report = application.inspect(&records);
        fs::write(destination, format.render(&report)).map_err(|_| DiagnosticFailure {
            message_id: "diagnostics.export_failed",
        })
    })
    .await
    .map_err(|_| DiagnosticFailure {
        message_id: "diagnostics.export_failed",
    })
    .and_then(|result| result);
    log_diagnostic_failure(&logs, "diagnostics.export_report", &result);
    result
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
