use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;
use sha2::{Digest, Sha256};
use toml_edit::DocumentMut;

use crate::environment::{ManagedConfigState, managed_config_evidence, managed_config_state};

#[derive(Debug, Clone)]
pub struct CodexInspector {
    codex_home: PathBuf,
    login_command: LoginStatusCommand,
}

impl CodexInspector {
    pub fn new(codex_home: impl AsRef<Path>, login_command: LoginStatusCommand) -> Self {
        Self {
            codex_home: codex_home.as_ref().to_path_buf(),
            login_command,
        }
    }

    pub fn inspect(&self) -> CodexSnapshot {
        self.inspect_with_credentials(false)
    }

    pub(crate) fn inspect_for_provider_mode(&self) -> CodexSnapshot {
        self.inspect_with_credentials(true)
    }

    pub(crate) fn config_matches_fingerprint(&self, expected: &str) -> bool {
        let config_path = self.codex_home.join("config.toml");
        let Ok(bytes) = fs::read(config_path) else {
            return false;
        };
        sha256_hex(&bytes) == expected
            || crate::environment::managed_config_matches_applied_evidence(&bytes, Some(expected))
    }

    fn inspect_with_credentials(&self, inspect_file_content: bool) -> CodexSnapshot {
        let (
            config_status,
            config_fingerprint,
            credential_store,
            recovered_desktop_rewrite,
            managed_config_state,
        ) = self.inspect_config();
        let credential_file_status = credential_file_status(&self.codex_home, credential_store);
        let login_status = self.login_command.status();
        CodexSnapshot {
            config_status,
            config_fingerprint,
            credential_file_status,
            credential_store,
            login_status,
            recovered_desktop_rewrite,
            managed_config_state,
            credential_fingerprint: credential_fingerprint(
                &self.codex_home,
                credential_store,
                credential_file_status,
                login_status,
                inspect_file_content,
            ),
        }
    }

    fn inspect_config(
        &self,
    ) -> (
        CodexConfigStatus,
        Option<String>,
        CredentialStore,
        bool,
        ManagedConfigState,
    ) {
        let config_path = self.codex_home.join("config.toml");
        match fs::metadata(&config_path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return (
                    CodexConfigStatus::Unreadable,
                    None,
                    CredentialStore::Unknown,
                    false,
                    ManagedConfigState::Conflict,
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (
                    CodexConfigStatus::Missing,
                    None,
                    CredentialStore::Unknown,
                    false,
                    ManagedConfigState::Absent,
                );
            }
            Err(_) => {
                return (
                    CodexConfigStatus::Unreadable,
                    None,
                    CredentialStore::Unknown,
                    false,
                    ManagedConfigState::Conflict,
                );
            }
        }
        let bytes = match fs::read(config_path) {
            Ok(bytes) => bytes,
            Err(_) => {
                return (
                    CodexConfigStatus::Unreadable,
                    None,
                    CredentialStore::Unknown,
                    false,
                    ManagedConfigState::Conflict,
                );
            }
        };
        let config_management = managed_config_state(&bytes);
        let (fingerprint, recovered_desktop_rewrite) = match managed_config_evidence(&bytes) {
            Some(evidence) => (
                Some(evidence.fingerprint),
                evidence.recovered_desktop_rewrite,
            ),
            None => (Some(sha256_hex(&bytes)), false),
        };
        let document = match std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| text.parse::<DocumentMut>().ok())
        {
            Some(document) => document,
            None => {
                return (
                    CodexConfigStatus::Invalid,
                    fingerprint,
                    CredentialStore::Unknown,
                    recovered_desktop_rewrite,
                    config_management,
                );
            }
        };
        (
            CodexConfigStatus::Valid,
            fingerprint,
            credential_store_from_document(&document),
            recovered_desktop_rewrite,
            config_management,
        )
    }
}

#[derive(Debug, Clone)]
pub struct LoginStatusCommand {
    program: OsString,
    arguments: Vec<OsString>,
    codex_home: Option<OsString>,
}

impl LoginStatusCommand {
    pub fn new<I, S>(program: impl AsRef<OsStr>, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Self {
            program: program.as_ref().to_os_string(),
            arguments: arguments
                .into_iter()
                .map(|argument| argument.as_ref().to_os_string())
                .collect(),
            codex_home: None,
        }
    }

    pub fn codex_default() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::new("cmd.exe", ["/D", "/S", "/C", "codex login status"])
        }
        #[cfg(not(target_os = "windows"))]
        {
            Self::new("codex", ["login", "status"])
        }
    }

    pub fn codex_for_home(codex_home: impl AsRef<OsStr>) -> Self {
        let mut command = Self::codex_default();
        command.codex_home = Some(codex_home.as_ref().to_os_string());
        command
    }

    pub(crate) fn status(&self) -> LoginStatus {
        self.inspect().status
    }

    pub(crate) fn inspect(&self) -> LoginInspection {
        let mut command = Command::new(&self.program);
        command
            .args(&self.arguments)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        if let Some(codex_home) = self.codex_home.as_ref() {
            command.env("CODEX_HOME", codex_home);
        }
        configure_hidden_process(&mut command);
        match command.output() {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
                let method = if text.contains("logged in using chatgpt") {
                    LoginMethod::ChatGpt
                } else if text.contains("api key") {
                    LoginMethod::ApiKey
                } else {
                    LoginMethod::Unknown
                };
                LoginInspection {
                    status: LoginStatus::LoggedIn,
                    method,
                }
            }
            Ok(_) => LoginInspection {
                status: LoginStatus::NotLoggedIn,
                method: LoginMethod::Unknown,
            },
            Err(_) => LoginInspection {
                status: LoginStatus::Unavailable,
                method: LoginMethod::Unknown,
            },
        }
    }
}

#[cfg(target_os = "windows")]
fn configure_hidden_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn configure_hidden_process(_command: &mut Command) {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexConfigStatus {
    Missing,
    Valid,
    Invalid,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginStatus {
    LoggedIn,
    NotLoggedIn,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMethod {
    ChatGpt,
    ApiKey,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginInspection {
    pub status: LoginStatus,
    pub method: LoginMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStore {
    Unknown,
    File,
    Keyring,
    Auto,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialFileStatus {
    NotApplicable,
    Missing,
    Present,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSnapshot {
    pub config_status: CodexConfigStatus,
    pub config_fingerprint: Option<String>,
    pub credential_store: CredentialStore,
    pub credential_file_status: CredentialFileStatus,
    pub login_status: LoginStatus,
    #[serde(skip)]
    pub(crate) recovered_desktop_rewrite: bool,
    #[serde(skip)]
    pub(crate) managed_config_state: ManagedConfigState,
    #[serde(skip)]
    pub(crate) credential_fingerprint: Option<String>,
}

pub(crate) fn credential_store_from_document(document: &DocumentMut) -> CredentialStore {
    let Some(item) = document.get("cli_auth_credentials_store") else {
        return CredentialStore::File;
    };
    match item.as_str() {
        Some("file") => CredentialStore::File,
        Some("keyring") => CredentialStore::Keyring,
        Some("auto") => CredentialStore::Auto,
        Some(_) | None => CredentialStore::Unsupported,
    }
}

fn credential_file_status(home: &Path, store: CredentialStore) -> CredentialFileStatus {
    if store != CredentialStore::File {
        return CredentialFileStatus::NotApplicable;
    }
    match fs::metadata(home.join("auth.json")) {
        Ok(metadata) if metadata.is_file() => CredentialFileStatus::Present,
        Ok(_) => CredentialFileStatus::Unreadable,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CredentialFileStatus::Missing,
        Err(_) => CredentialFileStatus::Unreadable,
    }
}

fn credential_fingerprint(
    home: &Path,
    store: CredentialStore,
    file_status: CredentialFileStatus,
    _login_status: LoginStatus,
    inspect_file_content: bool,
) -> Option<String> {
    if !inspect_file_content {
        return None;
    }
    let material = match store {
        CredentialStore::File => match file_status {
            CredentialFileStatus::Present => {
                let bytes = fs::read(home.join("auth.json")).ok()?;
                return Some(sha256_with_prefix(b"file:present:", &bytes));
            }
            CredentialFileStatus::Missing => "file:missing".to_owned(),
            CredentialFileStatus::NotApplicable | CredentialFileStatus::Unreadable => {
                return None;
            }
        },
        CredentialStore::Keyring | CredentialStore::Auto => return None,
        CredentialStore::Unknown | CredentialStore::Unsupported => return None,
    };
    Some(sha256_hex(material.as_bytes()))
}

fn sha256_with_prefix(prefix: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix);
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{
        CredentialFileStatus, CredentialStore, LoginMethod, LoginStatus, LoginStatusCommand,
        credential_file_status, credential_fingerprint,
    };

    #[cfg(target_os = "windows")]
    fn scripted_login_probe(script: &str) -> LoginStatusCommand {
        LoginStatusCommand::new("cmd.exe", ["/D", "/S", "/C", script])
    }

    #[cfg(not(target_os = "windows"))]
    fn scripted_login_probe(script: &str) -> LoginStatusCommand {
        LoginStatusCommand::new("sh", ["-c", script])
    }

    #[test]
    fn file_credential_fingerprint_uses_content_not_metadata() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("auth.json");
        fs::write(&path, b"aaaa").expect("write first auth content");
        let first = credential_fingerprint(
            temp.path(),
            CredentialStore::File,
            credential_file_status(temp.path(), CredentialStore::File),
            LoginStatus::Unavailable,
            true,
        )
        .expect("fingerprint first auth content");
        fs::write(&path, b"bbbb").expect("write second auth content");
        let second = credential_fingerprint(
            temp.path(),
            CredentialStore::File,
            credential_file_status(temp.path(), CredentialStore::File),
            LoginStatus::Unavailable,
            true,
        )
        .expect("fingerprint second auth content");

        assert_ne!(first, second);
    }

    #[test]
    fn keyring_credential_fingerprint_is_unknown_without_identity_evidence() {
        assert_eq!(
            credential_fingerprint(
                std::path::Path::new("."),
                CredentialStore::Keyring,
                CredentialFileStatus::NotApplicable,
                LoginStatus::LoggedIn,
                true,
            ),
            None
        );
    }

    #[test]
    fn default_inspection_does_not_read_file_credentials() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("config.toml"),
            "cli_auth_credentials_store = 'file'\n",
        )
        .expect("write config");
        fs::write(temp.path().join("auth.json"), b"credential-content")
            .expect("write auth content");

        let snapshot = super::CodexInspector::new(
            temp.path(),
            LoginStatusCommand::new("cmd.exe", ["/D", "/S", "/C", "exit 0"]),
        )
        .inspect();

        assert_eq!(snapshot.credential_fingerprint, None);
    }

    #[test]
    fn login_probe_identifies_chatgpt_login() {
        let inspection = scripted_login_probe("echo Logged in using ChatGPT").inspect();

        assert_eq!(inspection.status, LoginStatus::LoggedIn);
        assert_eq!(inspection.method, LoginMethod::ChatGpt);
    }

    #[test]
    fn login_probe_identifies_api_key_login() {
        let inspection = scripted_login_probe("echo Logged in using an API key").inspect();

        assert_eq!(inspection.status, LoginStatus::LoggedIn);
        assert_eq!(inspection.method, LoginMethod::ApiKey);
    }

    #[test]
    fn login_probe_keeps_unknown_success_distinct_from_chatgpt() {
        let inspection = scripted_login_probe("echo Logged in").inspect();

        assert_eq!(inspection.status, LoginStatus::LoggedIn);
        assert_eq!(inspection.method, LoginMethod::Unknown);
    }

    #[test]
    fn login_probe_maps_nonzero_exit_to_not_logged_in() {
        #[cfg(target_os = "windows")]
        let command = scripted_login_probe("exit /b 1");
        #[cfg(not(target_os = "windows"))]
        let command = scripted_login_probe("exit 1");

        let inspection = command.inspect();

        assert_eq!(inspection.status, LoginStatus::NotLoggedIn);
        assert_eq!(inspection.method, LoginMethod::Unknown);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn default_login_probe_uses_cmd_to_resolve_npm_shims() {
        let command = LoginStatusCommand::codex_default();

        assert_eq!(command.program, "cmd.exe");
        assert_eq!(command.arguments, ["/D", "/S", "/C", "codex login status"]);
    }
}
