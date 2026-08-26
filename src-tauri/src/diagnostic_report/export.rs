use super::{
    AuthFileStatus, CodexHomeOverrideStatus, CredentialStore, DiagnosticConfigStatus,
    DiagnosticOrigin, DiagnosticReport,
};
use crate::codex::LoginStatus;
use crate::consumer::ConsumerStatus;

impl DiagnosticReport {
    pub fn redacted_json(&self) -> String {
        let mut report = serde_json::to_value(self).unwrap_or_default();
        if let Some(object) = report.as_object_mut() {
            object.remove("repairPreview");
        }
        serde_json::to_string_pretty(&report)
            .unwrap_or_else(|_| "{\"schemaVersion\":2,\"findings\":[]}".to_owned())
    }

    pub fn redacted_markdown(&self) -> String {
        let active_provider = self
            .environment
            .active_provider
            .as_deref()
            .map(markdown_text)
            .unwrap_or_else(|| "未设置".to_owned());
        let declared_providers = if self.environment.declared_providers.is_empty() {
            "无".to_owned()
        } else {
            self.environment
                .declared_providers
                .iter()
                .map(|provider| markdown_text(provider))
                .collect::<Vec<_>>()
                .join("、")
        };
        let codex_cli_version = self
            .versions
            .codex_cli
            .as_deref()
            .map(markdown_text)
            .unwrap_or_else(|| "无法确认".to_owned());
        let mut output = format!(
            concat!(
                "# GPTEasy 本机诊断报告\n\n",
                "报告格式版本：{}\n\n",
                "## 当前用户 Codex 环境\n\n",
                "| 项目 | 状态 |\n| --- | --- |\n",
                "| 范围 | 当前用户 |\n",
                "| CODEX_HOME | {} |\n",
                "| config.toml | {} |\n",
                "| 当前 provider | {} |\n",
                "| 已声明 provider | {} |\n",
                "| 登录状态 | {} |\n",
                "| auth.json | {} |\n",
                "| 凭据存储 | {} |\n",
                "| ChatGPT/Codex 桌面版 | {} |\n",
                "| Codex CLI | {} |\n\n",
                "## 版本\n\n",
                "- GPTEasy：{}\n",
                "- Codex CLI：{}\n\n",
                "## Findings\n\n",
            ),
            self.schema_version,
            codex_home_override_status_name(self.environment.codex_home_override_status),
            config_status_name(self.environment.config_status),
            active_provider,
            declared_providers,
            login_status_name(self.authentication.login_status),
            auth_file_status_name(self.authentication.auth_file_status),
            credential_store_name(self.authentication.credential_store),
            consumer_status_name(self.consumers.desktop),
            consumer_status_name(self.consumers.cli),
            self.versions.gpteasy,
            codex_cli_version,
        );
        if self.findings.is_empty() {
            output.push_str("未发现诊断项。\n");
        } else {
            for finding in &self.findings {
                output.push_str(&format!(
                    "- **{}** (`{}` / {}): {}\n",
                    markdown_text(&finding.title),
                    finding.code,
                    origin_name(finding.origin),
                    markdown_text(&finding.summary),
                ));
            }
        }
        if !self.errors.is_empty() {
            output.push_str("\n## 相关错误元数据\n\n");
            for error in &self.errors {
                output.push_str(&format!(
                    "- `{}` / {}：{} 次，最后时间戳 {}\n",
                    error.error_code,
                    origin_name(error.origin),
                    error.occurrences,
                    error.last_seen_epoch_seconds,
                ));
            }
        }
        output
    }
}

fn markdown_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\r', '\n'], " ")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn codex_home_override_status_name(status: CodexHomeOverrideStatus) -> &'static str {
    match status {
        CodexHomeOverrideStatus::Unset => "未设置",
        CodexHomeOverrideStatus::Matches => "与当前环境一致",
        CodexHomeOverrideStatus::Differs => "指向另一环境",
    }
}

fn config_status_name(status: DiagnosticConfigStatus) -> &'static str {
    match status {
        DiagnosticConfigStatus::Missing => "缺失",
        DiagnosticConfigStatus::Unreadable => "无法读取",
        DiagnosticConfigStatus::EncodingError => "编码错误",
        DiagnosticConfigStatus::TomlSyntaxError => "TOML 语法错误",
        DiagnosticConfigStatus::Valid => "有效",
    }
}

fn login_status_name(status: LoginStatus) -> &'static str {
    match status {
        LoginStatus::LoggedIn => "已认证",
        LoginStatus::NotLoggedIn => "未认证",
        LoginStatus::Unavailable => "无法确认",
    }
}

fn auth_file_status_name(status: AuthFileStatus) -> &'static str {
    match status {
        AuthFileStatus::Present => "存在",
        AuthFileStatus::Missing => "缺失",
        AuthFileStatus::Unreadable => "无法读取",
    }
}

fn credential_store_name(store: CredentialStore) -> &'static str {
    match store {
        CredentialStore::File => "文件",
        CredentialStore::Keyring => "系统密钥环",
        CredentialStore::Auto => "自动",
        CredentialStore::Unsupported => "不支持的值",
        CredentialStore::Unknown => "无法确认",
    }
}

fn consumer_status_name(status: ConsumerStatus) -> &'static str {
    match status {
        ConsumerStatus::Running => "运行中",
        ConsumerStatus::Stopped => "已停止",
        ConsumerStatus::Unknown => "无法确认",
    }
}

fn origin_name(origin: DiagnosticOrigin) -> &'static str {
    match origin {
        DiagnosticOrigin::Local => "local",
        DiagnosticOrigin::Remote => "remote",
    }
}
