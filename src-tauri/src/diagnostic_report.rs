use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use toml_edit::DocumentMut;

pub use crate::codex::CredentialStore;
use crate::codex::{LoginStatus, LoginStatusCommand, credential_store_from_document};
use crate::consumer::{ConsumerScanner, ConsumerStatus, WindowsConsumerScanner};
use crate::diagnostics::IssueLogRecord;
use crate::environment::{
    CustomProviderRepairSource, CustomProviderRepairStatus, EnvironmentApplication,
};

mod commands;
mod export;

use commands::inspect_codex_cli_version;
pub(crate) use commands::{
    DiagnosticRuntime, choose_diagnostic_export_destination, export_diagnostic_report,
    get_diagnostic_report, repair_diagnostic_custom_provider,
};

#[derive(Clone)]
pub struct DiagnosticApplication {
    codex_home: PathBuf,
    codex_home_override: Option<PathBuf>,
    environment: Option<EnvironmentApplication>,
}

impl DiagnosticApplication {
    pub fn new(codex_home: impl AsRef<Path>, codex_home_override: Option<PathBuf>) -> Self {
        Self {
            codex_home: codex_home.as_ref().to_path_buf(),
            codex_home_override,
            environment: None,
        }
    }

    pub fn with_environment(
        codex_home: impl AsRef<Path>,
        codex_home_override: Option<PathBuf>,
        environment: EnvironmentApplication,
    ) -> Self {
        Self {
            codex_home: codex_home.as_ref().to_path_buf(),
            codex_home_override,
            environment: Some(environment),
        }
    }

    pub fn inspect_with(
        &self,
        observations: &DiagnosticObservations,
        issue_logs: &[IssueLogRecord],
    ) -> DiagnosticReport {
        let codex_home_override_status = match self.codex_home_override.as_ref() {
            None => CodexHomeOverrideStatus::Unset,
            Some(overridden) if paths_match(overridden, &self.codex_home) => {
                CodexHomeOverrideStatus::Matches
            }
            Some(_) => CodexHomeOverrideStatus::Differs,
        };
        let config = inspect_config(&self.codex_home.join("config.toml"));
        let repair_preview = (codex_home_override_status != CodexHomeOverrideStatus::Differs)
            .then(|| {
                self.environment.as_ref().and_then(|environment| {
                    environment.preview_custom_provider_repair().ok().flatten()
                })
            })
            .flatten()
            .map(DiagnosticRepairPreview::from);
        let mut findings: Vec<DiagnosticFinding> =
            config_status_finding(config.status).into_iter().collect();
        if let Some(active_provider) = config.active_provider.as_deref()
            && !config
                .declared_providers
                .iter()
                .any(|declared| declared == active_provider)
        {
            findings.push(DiagnosticFinding {
                code: "model_provider_missing_definition",
                origin: DiagnosticOrigin::Local,
                severity: DiagnosticSeverity::Error,
                title: "模型供应商定义缺失".to_owned(),
                summary: format!(
                    "config.toml 使用模型供应商“{active_provider}”，但没有声明同名 model_providers 配置。"
                ),
                repairable: active_provider == "custom" && repair_preview.is_some(),
            });
        }
        if codex_home_override_status == CodexHomeOverrideStatus::Differs {
            findings.push(DiagnosticFinding {
                code: "codex_home_mismatch",
                origin: DiagnosticOrigin::Local,
                severity: DiagnosticSeverity::Warning,
                title: "CODEX_HOME 指向另一环境".to_owned(),
                summary: "当前进程的 CODEX_HOME 与 GPTEasy 管理的当前用户默认 Codex 环境不一致。"
                    .to_owned(),
                repairable: false,
            });
        }
        let (errors, log_findings) = classify_issue_logs(issue_logs);
        findings.extend(log_findings);
        DiagnosticReport {
            schema_version: 2,
            environment: DiagnosticEnvironment {
                scope: DiagnosticScope::CurrentUser,
                codex_home: "~/.codex",
                codex_home_override_status,
                config_status: config.status,
                active_provider: config.active_provider,
                declared_providers: config.declared_providers,
            },
            authentication: DiagnosticAuthentication {
                login_status: observations.login_status,
                auth_file_status: auth_file_status(&self.codex_home.join("auth.json")),
                credential_store: config.credential_store,
            },
            consumers: DiagnosticConsumers {
                desktop: observations.desktop_status,
                cli: observations.cli_status,
            },
            versions: DiagnosticVersions {
                gpteasy: env!("CARGO_PKG_VERSION"),
                codex_cli: observations.codex_cli_version.clone(),
            },
            findings,
            errors,
            repair_preview,
        }
    }

    pub fn inspect(&self, issue_logs: &[IssueLogRecord]) -> DiagnosticReport {
        let consumers = WindowsConsumerScanner::new().scan();
        let observations = DiagnosticObservations {
            login_status: LoginStatusCommand::codex_for_home(&self.codex_home).status(),
            desktop_status: consumers.desktop,
            cli_status: consumers.cli,
            codex_cli_version: inspect_codex_cli_version(),
        };
        self.inspect_with(&observations, issue_logs)
    }

    pub fn repair_custom_provider(&self, preview_id: &str) -> DiagnosticRepairResult {
        if self
            .codex_home_override
            .as_ref()
            .is_some_and(|override_home| !paths_match(override_home, &self.codex_home))
        {
            return DiagnosticRepairResult {
                status: DiagnosticRepairStatus::ManualRequired,
                message_id: "diagnostics.repair_manual_required",
            };
        }
        let Some(environment) = self.environment.as_ref() else {
            return DiagnosticRepairResult {
                status: DiagnosticRepairStatus::ManualRequired,
                message_id: "diagnostics.repair_manual_required",
            };
        };
        let result = environment.repair_custom_provider(preview_id);
        DiagnosticRepairResult {
            status: result.status.into(),
            message_id: result.message_id,
        }
    }
}

fn paths_match(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => {
            #[cfg(target_os = "windows")]
            {
                left.to_string_lossy().replace('/', "\\").to_lowercase()
                    == right.to_string_lossy().replace('/', "\\").to_lowercase()
            }
            #[cfg(not(target_os = "windows"))]
            {
                left == right
            }
        }
    }
}

fn config_status_finding(status: DiagnosticConfigStatus) -> Option<DiagnosticFinding> {
    let (code, title, summary) = match status {
        DiagnosticConfigStatus::Missing => (
            "config_missing",
            "Codex 配置缺失",
            "当前用户 Codex 环境中没有 config.toml。",
        ),
        DiagnosticConfigStatus::Unreadable => (
            "config_unreadable",
            "Codex 配置无法读取",
            "无法把 config.toml 作为普通文件读取。",
        ),
        DiagnosticConfigStatus::EncodingError => (
            "config_encoding_error",
            "Codex 配置编码错误",
            "config.toml 不是有效的 UTF-8 文本。",
        ),
        DiagnosticConfigStatus::TomlSyntaxError => (
            "config_toml_syntax_error",
            "Codex 配置语法错误",
            "config.toml 不是有效的 TOML，未继续分析其中的供应商。",
        ),
        DiagnosticConfigStatus::Valid => return None,
    };
    Some(DiagnosticFinding {
        code,
        origin: DiagnosticOrigin::Local,
        severity: DiagnosticSeverity::Error,
        title: title.to_owned(),
        summary: summary.to_owned(),
        repairable: false,
    })
}

#[derive(Debug, Clone)]
pub struct DiagnosticObservations {
    pub login_status: LoginStatus,
    pub desktop_status: ConsumerStatus,
    pub cli_status: ConsumerStatus,
    pub codex_cli_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticConfigStatus {
    Missing,
    Unreadable,
    EncodingError,
    TomlSyntaxError,
    Valid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticScope {
    CurrentUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexHomeOverrideStatus {
    Unset,
    Matches,
    Differs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthFileStatus {
    Present,
    Missing,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticOrigin {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticRepairSource {
    CurrentConfig,
    GpteasyBackup,
}

impl From<CustomProviderRepairSource> for DiagnosticRepairSource {
    fn from(source: CustomProviderRepairSource) -> Self {
        match source {
            CustomProviderRepairSource::CurrentConfig => Self::CurrentConfig,
            CustomProviderRepairSource::GpteasyBackup => Self::GpteasyBackup,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticRepairStatus {
    Succeeded,
    NotModified,
    RolledBack,
    ManualRequired,
}

impl From<CustomProviderRepairStatus> for DiagnosticRepairStatus {
    fn from(status: CustomProviderRepairStatus) -> Self {
        match status {
            CustomProviderRepairStatus::Succeeded => Self::Succeeded,
            CustomProviderRepairStatus::NotModified => Self::NotModified,
            CustomProviderRepairStatus::RolledBack => Self::RolledBack,
            CustomProviderRepairStatus::ManualRequired => Self::ManualRequired,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRepairPreview {
    pub preview_id: String,
    pub source: DiagnosticRepairSource,
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub authentication: &'static str,
    pub changes: Vec<&'static str>,
}

impl From<crate::environment::CustomProviderRepairPreview> for DiagnosticRepairPreview {
    fn from(preview: crate::environment::CustomProviderRepairPreview) -> Self {
        Self {
            preview_id: preview.preview_id,
            source: preview.source.into(),
            provider_name: preview.provider_name,
            base_url: preview.base_url,
            model: preview.model,
            authentication: "current_api_key",
            changes: vec![
                "backup_config",
                "add_custom_provider_definition",
                "verify_and_rediagnose",
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRepairResult {
    pub status: DiagnosticRepairStatus,
    pub message_id: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEnvironment {
    pub scope: DiagnosticScope,
    pub codex_home: &'static str,
    pub codex_home_override_status: CodexHomeOverrideStatus,
    pub config_status: DiagnosticConfigStatus,
    pub active_provider: Option<String>,
    pub declared_providers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticAuthentication {
    pub login_status: LoginStatus,
    pub auth_file_status: AuthFileStatus,
    pub credential_store: CredentialStore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticConsumers {
    pub desktop: ConsumerStatus,
    pub cli: ConsumerStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticVersions {
    pub gpteasy: &'static str,
    pub codex_cli: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticFinding {
    pub code: &'static str,
    pub origin: DiagnosticOrigin,
    pub severity: DiagnosticSeverity,
    pub title: String,
    pub summary: String,
    pub repairable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticErrorMetadata {
    pub error_code: &'static str,
    pub origin: DiagnosticOrigin,
    pub occurrences: usize,
    pub last_seen_epoch_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    pub schema_version: u8,
    pub environment: DiagnosticEnvironment,
    pub authentication: DiagnosticAuthentication,
    pub consumers: DiagnosticConsumers,
    pub versions: DiagnosticVersions,
    pub findings: Vec<DiagnosticFinding>,
    pub errors: Vec<DiagnosticErrorMetadata>,
    pub repair_preview: Option<DiagnosticRepairPreview>,
}

fn classify_issue_logs(
    issue_logs: &[IssueLogRecord],
) -> (Vec<DiagnosticErrorMetadata>, Vec<DiagnosticFinding>) {
    let mut local_count = 0;
    let mut local_last_seen = 0;
    let mut remote_count = 0;
    let mut remote_last_seen = 0;
    for record in issue_logs
        .iter()
        .filter(|record| record.level == crate::diagnostics::IssueLogLevel::Error)
    {
        if record.message == "session.model_provider_not_found" {
            local_count += 1;
            local_last_seen = local_last_seen.max(record.timestamp_epoch_seconds);
        }
        if record.message == "provider.invalid_api_key" {
            remote_count += 1;
            remote_last_seen = remote_last_seen.max(record.timestamp_epoch_seconds);
        }
    }

    let mut errors = Vec::new();
    let mut findings = Vec::new();
    if local_count > 0 {
        errors.push(DiagnosticErrorMetadata {
            error_code: "model_provider_not_found",
            origin: DiagnosticOrigin::Local,
            occurrences: local_count,
            last_seen_epoch_seconds: local_last_seen,
        });
        findings.push(DiagnosticFinding {
            code: "historical_provider_missing",
            origin: DiagnosticOrigin::Local,
            severity: DiagnosticSeverity::Error,
            title: "历史会话引用的供应商缺失".to_owned(),
            summary: "本地 Codex 无法找到会话引用的模型供应商；这不是远端 API 认证错误。"
                .to_owned(),
            repairable: false,
        });
    }
    if remote_count > 0 {
        errors.push(DiagnosticErrorMetadata {
            error_code: "invalid_api_key",
            origin: DiagnosticOrigin::Remote,
            occurrences: remote_count,
            last_seen_epoch_seconds: remote_last_seen,
        });
        findings.push(DiagnosticFinding {
            code: "remote_invalid_api_key",
            origin: DiagnosticOrigin::Remote,
            severity: DiagnosticSeverity::Error,
            title: "API Key 认证失败".to_owned(),
            summary:
                "远端服务拒绝 API Key（401 invalid_api_key 类认证失败）；这与本地供应商定义缺失不同。"
                    .to_owned(),
            repairable: false,
        });
    }
    (errors, findings)
}

struct ConfigInspection {
    status: DiagnosticConfigStatus,
    active_provider: Option<String>,
    declared_providers: Vec<String>,
    credential_store: CredentialStore,
}

fn empty_config_inspection(status: DiagnosticConfigStatus) -> ConfigInspection {
    ConfigInspection {
        status,
        active_provider: None,
        declared_providers: Vec::new(),
        credential_store: CredentialStore::Unknown,
    }
}

fn inspect_config(path: &Path) -> ConfigInspection {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return empty_config_inspection(DiagnosticConfigStatus::Missing);
        }
        Err(_) => return empty_config_inspection(DiagnosticConfigStatus::Unreadable),
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(_) => return empty_config_inspection(DiagnosticConfigStatus::EncodingError),
    };
    let document = match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(_) => return empty_config_inspection(DiagnosticConfigStatus::TomlSyntaxError),
    };
    let active_provider = document
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::to_owned);
    let mut declared_providers: Vec<String> = document
        .get("model_providers")
        .and_then(|item| item.as_table_like())
        .map(|table| table.iter().map(|(name, _)| name.to_owned()).collect())
        .unwrap_or_default();
    declared_providers.sort();
    let credential_store = credential_store_from_document(&document);
    ConfigInspection {
        status: DiagnosticConfigStatus::Valid,
        active_provider,
        declared_providers,
        credential_store,
    }
}

fn auth_file_status(path: &Path) -> AuthFileStatus {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => AuthFileStatus::Present,
        Ok(_) => AuthFileStatus::Unreadable,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => AuthFileStatus::Missing,
        Err(_) => AuthFileStatus::Unreadable,
    }
}
