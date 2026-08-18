use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures_util::StreamExt;
use minisign_verify::{PublicKey, Signature};
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub const MANUAL_DOWNLOAD_URL: &str = "https://github.com/yinshaohua/GPTEasy/releases/latest";
pub const GITCODE_RELEASES_URL: &str =
    "https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag";
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    Idle,
    Checking,
    Downloading,
    UpToDate,
    Pending,
    Incomplete,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFailureCategory {
    CheckFailed,
    ManifestInvalid,
    DownloadFailed,
    SignatureInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    pub current_version: String,
    pub state: UpdateState,
    pub available_version: Option<String>,
    pub notes: Option<String>,
    pub published_at: Option<String>,
    pub checked_at_epoch_seconds: Option<u64>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub progress_percent: Option<u8>,
    pub failure_category: Option<UpdateFailureCategory>,
    pub error_message: Option<String>,
    pub manual_download_url: String,
    pub release_notes_url: Option<String>,
}

impl UpdateSnapshot {
    fn new(current_version: &str) -> Self {
        Self {
            current_version: current_version.to_owned(),
            state: UpdateState::Idle,
            available_version: None,
            notes: None,
            published_at: None,
            checked_at_epoch_seconds: None,
            downloaded_bytes: 0,
            total_bytes: None,
            progress_percent: None,
            failure_category: None,
            error_message: None,
            manual_download_url: MANUAL_DOWNLOAD_URL.to_owned(),
            release_notes_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    pub_date: Option<String>,
    platforms: std::collections::HashMap<String, ManifestPlatform>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestPlatform {
    url: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct TauriConfiguration {
    plugins: TauriPlugins,
}

#[derive(Debug, Deserialize)]
struct TauriPlugins {
    updater: TauriUpdater,
}

#[derive(Debug, Deserialize)]
struct TauriUpdater {
    endpoints: Vec<String>,
    pubkey: String,
}

fn configured_trust_root() -> (String, String) {
    let config: TauriConfiguration = serde_json::from_str(include_str!("../tauri.conf.json"))
        .expect("Tauri updater trust root configuration is invalid");
    let endpoint = config
        .plugins
        .updater
        .endpoints
        .into_iter()
        .next()
        .expect("Tauri updater endpoint is missing");
    (endpoint, config.plugins.updater.pubkey)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StableVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl StableVersion {
    fn parse(value: &str) -> Result<Self, String> {
        let mut parts = value.split('.');
        let major = parse_component(parts.next(), value)?;
        let minor = parse_component(parts.next(), value)?;
        let patch = parse_component(parts.next(), value)?;
        if parts.next().is_some() {
            return Err("版本必须是稳定 SemVer".to_owned());
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

fn parse_component(value: Option<&str>, version: &str) -> Result<u64, String> {
    let value = value.ok_or_else(|| format!("版本不是稳定 SemVer：{version}"))?;
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return Err(format!("版本不是稳定 SemVer：{version}"));
    }
    value
        .parse::<u64>()
        .map_err(|_| format!("版本不是稳定 SemVer：{version}"))
}

fn is_prerelease_version(value: &str) -> bool {
    value
        .split_once('-')
        .map(|(stable, _)| StableVersion::parse(stable).is_ok())
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
struct PendingUpdate {
    version: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstallAttempt {
    target_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateInstallFailureCategory {
    NoPendingUpdate,
    Busy,
    UnsupportedPlatform,
    StateUnavailable,
    LaunchFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstallFailure {
    pub category: UpdateInstallFailureCategory,
    pub message_id: &'static str,
}

impl UpdateInstallFailure {
    fn new(category: UpdateInstallFailureCategory, message_id: &'static str) -> Self {
        Self {
            category,
            message_id,
        }
    }
}

#[derive(Clone, Default)]
pub struct UpdateActivityGate {
    active: Arc<Mutex<UpdateActivityState>>,
}

pub struct UpdateActivityGuard {
    gate: UpdateActivityGate,
    operation: Option<String>,
}

#[derive(Default)]
struct UpdateActivityState {
    operations: Vec<String>,
    installing: bool,
}

impl UpdateActivityGate {
    pub fn try_begin(&self, operation: impl Into<String>) -> Option<UpdateActivityGuard> {
        let mut active = self.active.lock().ok()?;
        if active.installing {
            return None;
        }
        let operation = operation.into();
        active.operations.push(operation.clone());
        Some(UpdateActivityGuard {
            gate: self.clone(),
            operation: Some(operation),
        })
    }

    pub fn try_begin_install(&self) -> Option<UpdateActivityGuard> {
        let mut active = self.active.lock().ok()?;
        if active.installing || !active.operations.is_empty() {
            return None;
        }
        active.installing = true;
        Some(UpdateActivityGuard {
            gate: self.clone(),
            operation: None,
        })
    }

    pub fn active_operation(&self) -> Option<String> {
        self.active
            .lock()
            .ok()
            .and_then(|active| active.operations.first().cloned())
    }
}

impl Drop for UpdateActivityGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.gate.active.lock() {
            if let Some(operation) = self.operation.as_ref() {
                if let Some(index) = active.operations.iter().position(|item| item == operation) {
                    active.operations.remove(index);
                }
            } else {
                active.installing = false;
            }
        }
    }
}

#[derive(Debug)]
struct CoordinatorState {
    snapshot: UpdateSnapshot,
    pending: Option<PendingUpdate>,
    installing: bool,
}

#[derive(Clone)]
pub struct UpdateCoordinator {
    client: Client,
    current_version: String,
    endpoint: String,
    public_key: String,
    state: Arc<Mutex<CoordinatorState>>,
    install_attempt_path: Option<PathBuf>,
}

impl UpdateCoordinator {
    pub fn new(current_version: impl Into<String>) -> Self {
        let (endpoint, public_key) = configured_trust_root();
        Self::with_endpoint(current_version, endpoint, public_key)
    }

    pub(crate) fn with_endpoint(
        current_version: impl Into<String>,
        endpoint: impl Into<String>,
        public_key: impl Into<String>,
    ) -> Self {
        let current_version = current_version.into();
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30 * 60))
                .build()
                .expect("更新 HTTP 客户端初始化失败"),
            endpoint: endpoint.into(),
            public_key: public_key.into(),
            state: Arc::new(Mutex::new(CoordinatorState {
                snapshot: UpdateSnapshot::new(&current_version),
                pending: None,
                installing: false,
            })),
            current_version,
            install_attempt_path: None,
        }
    }

    pub(crate) fn with_state_path(
        current_version: impl Into<String>,
        install_attempt_path: impl AsRef<Path>,
    ) -> Self {
        let (endpoint, public_key) = configured_trust_root();
        let mut coordinator = Self::with_endpoint(current_version, endpoint, public_key);
        coordinator.install_attempt_path = Some(install_attempt_path.as_ref().to_path_buf());
        coordinator.restore_install_attempt();
        coordinator
    }

    pub fn snapshot(&self) -> UpdateSnapshot {
        self.state
            .lock()
            .map(|state| state.snapshot.clone())
            .unwrap_or_else(|_| UpdateSnapshot::new(&self.current_version))
    }

    fn restore_install_attempt(&self) {
        let Some(path) = self.install_attempt_path.as_ref() else {
            return;
        };
        let Ok(bytes) = fs::read(path) else {
            return;
        };
        let Ok(attempt) = serde_json::from_slice::<InstallAttempt>(&bytes) else {
            let _ = fs::remove_file(path);
            return;
        };
        let Ok(target) = StableVersion::parse(&attempt.target_version) else {
            let _ = fs::remove_file(path);
            return;
        };
        let Ok(current) = StableVersion::parse(&self.current_version) else {
            return;
        };
        if current >= target {
            let _ = fs::remove_file(path);
            self.set_up_to_date();
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.snapshot.state = UpdateState::Incomplete;
            state.snapshot.available_version = Some(attempt.target_version);
            state.snapshot.progress_percent = None;
            state.snapshot.checked_at_epoch_seconds = Some(now_epoch_seconds());
        }
    }

    pub(crate) async fn check_and_download<F>(&self, mut progress: F) -> UpdateSnapshot
    where
        F: FnMut(UpdateSnapshot) + Send,
    {
        if !self.begin_check() {
            return self.snapshot();
        }

        let result = self.fetch_manifest().await;
        let manifest = match result {
            Ok(manifest) => manifest,
            Err(category) => return self.fail(category),
        };
        let version = match StableVersion::parse(&manifest.version) {
            Ok(version) => version,
            Err(_) if is_prerelease_version(&manifest.version) => {
                self.set_up_to_date();
                return self.snapshot();
            }
            Err(_) => return self.fail(UpdateFailureCategory::ManifestInvalid),
        };
        let current = match StableVersion::parse(&self.current_version) {
            Ok(current) => current,
            Err(_) => return self.fail(UpdateFailureCategory::ManifestInvalid),
        };
        if version.cmp(&current) != Ordering::Greater {
            self.set_up_to_date();
            return self.snapshot();
        }

        let Some(platform) = manifest.platforms.get("windows-x86_64") else {
            return self.fail(UpdateFailureCategory::ManifestInvalid);
        };
        if !platform.url.starts_with("https://") {
            return self.fail(UpdateFailureCategory::ManifestInvalid);
        }
        if self.is_pending_version(&manifest.version) {
            self.restore_pending();
            return self.snapshot();
        }
        self.begin_download(&manifest);
        progress(self.snapshot());

        let response = match self.client.get(&platform.url).send().await {
            Ok(response) if response.status().is_success() => response,
            Ok(_) => {
                return self.fail(UpdateFailureCategory::DownloadFailed);
            }
            Err(_) => return self.fail(UpdateFailureCategory::DownloadFailed),
        };
        let total = response.content_length();
        let initial_capacity = total.unwrap_or_default().min(16 * 1024 * 1024) as usize;
        let mut bytes = Vec::with_capacity(initial_capacity);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(_) => return self.fail(UpdateFailureCategory::DownloadFailed),
            };
            bytes.extend_from_slice(&chunk);
            self.set_progress(bytes.len() as u64, total);
            progress(self.snapshot());
        }
        if verify_signature(&bytes, &platform.signature, &self.public_key).is_err() {
            return self.fail(UpdateFailureCategory::SignatureInvalid);
        }
        self.mark_ready(manifest, bytes);
        self.snapshot()
    }

    fn begin_check(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if matches!(
            state.snapshot.state,
            UpdateState::Checking | UpdateState::Downloading
        ) || state.installing
        {
            return false;
        }
        state.snapshot.state = UpdateState::Checking;
        state.snapshot.failure_category = None;
        state.snapshot.error_message = None;
        true
    }

    async fn fetch_manifest(&self) -> Result<Manifest, UpdateFailureCategory> {
        let response = self
            .client
            .get(&self.endpoint)
            .send()
            .await
            .map_err(|_| UpdateFailureCategory::CheckFailed)?;
        if !response.status().is_success() {
            return Err(UpdateFailureCategory::CheckFailed);
        }
        response
            .json::<Manifest>()
            .await
            .map_err(|_| UpdateFailureCategory::ManifestInvalid)
    }

    fn is_pending_version(&self, version: &str) -> bool {
        self.state
            .lock()
            .ok()
            .and_then(|state| {
                state
                    .pending
                    .as_ref()
                    .map(|pending| pending.version == version)
            })
            .unwrap_or(false)
    }

    fn begin_download(&self, manifest: &Manifest) {
        if let Ok(mut state) = self.state.lock() {
            state.snapshot.state = UpdateState::Downloading;
            state.snapshot.available_version = Some(manifest.version.clone());
            state.snapshot.notes = manifest.notes.clone();
            state.snapshot.published_at = manifest.pub_date.clone();
            state.snapshot.downloaded_bytes = 0;
            state.snapshot.total_bytes = None;
            state.snapshot.progress_percent = None;
            state.snapshot.failure_category = None;
            state.snapshot.error_message = None;
        }
    }

    fn set_progress(&self, downloaded: u64, total: Option<u64>) {
        if let Ok(mut state) = self.state.lock() {
            state.snapshot.downloaded_bytes = downloaded;
            state.snapshot.total_bytes = total;
            state.snapshot.progress_percent = total
                .filter(|total| *total > 0)
                .map(|total| ((downloaded.saturating_mul(100) / total).min(100)) as u8);
        }
    }

    fn mark_ready(&self, manifest: Manifest, bytes: Vec<u8>) {
        if let Ok(mut state) = self.state.lock() {
            let version = manifest.version;
            state.pending = Some(PendingUpdate {
                version: version.clone(),
                bytes,
            });
            state.snapshot.state = UpdateState::Pending;
            state.snapshot.available_version = Some(version.clone());
            state.snapshot.notes = manifest.notes;
            state.snapshot.published_at = manifest.pub_date;
            state.snapshot.progress_percent = Some(100);
            state.snapshot.failure_category = None;
            state.snapshot.error_message = None;
            state.snapshot.release_notes_url = Some(release_notes_url(&version));
            state.snapshot.checked_at_epoch_seconds = Some(now_epoch_seconds());
        }
    }

    fn set_up_to_date(&self) {
        if let Ok(mut state) = self.state.lock() {
            if state.pending.is_some() {
                state.snapshot.state = UpdateState::Pending;
                state.snapshot.progress_percent = Some(100);
                state.snapshot.failure_category = None;
                state.snapshot.error_message = None;
                state.snapshot.checked_at_epoch_seconds = Some(now_epoch_seconds());
                return;
            }
            if state.snapshot.state == UpdateState::Incomplete {
                state.snapshot.checked_at_epoch_seconds = Some(now_epoch_seconds());
                return;
            }
            state.pending = None;
            state.snapshot.state = UpdateState::UpToDate;
            state.snapshot.available_version = None;
            state.snapshot.release_notes_url = None;
            state.snapshot.notes = None;
            state.snapshot.published_at = None;
            state.snapshot.downloaded_bytes = 0;
            state.snapshot.total_bytes = None;
            state.snapshot.progress_percent = None;
            state.snapshot.failure_category = None;
            state.snapshot.error_message = None;
            state.snapshot.checked_at_epoch_seconds = Some(now_epoch_seconds());
        }
    }

    fn restore_pending(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.snapshot.state = UpdateState::Pending;
            state.snapshot.progress_percent = Some(100);
            state.snapshot.failure_category = None;
            state.snapshot.error_message = None;
            state.snapshot.checked_at_epoch_seconds = Some(now_epoch_seconds());
        }
    }

    pub(crate) fn confirm_install(&self) -> Result<UpdateSnapshot, UpdateInstallFailure> {
        let (version, bytes) = self.begin_install()?;
        let prepared = self
            .persist_install_attempt(&version)
            .and_then(|_| self.write_installer(&version, &bytes));
        let path = match prepared {
            Ok(path) => path,
            Err(failure) => {
                self.cancel_install();
                return Err(failure);
            }
        };
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        let launched = std::process::Command::new(&path).arg("/P").spawn().is_ok();
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        let launched = false;
        if !launched {
            let _ = fs::remove_file(&path);
            self.clear_install_attempt();
            self.cancel_install();
            return Err(UpdateInstallFailure::new(
                if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
                    UpdateInstallFailureCategory::LaunchFailed
                } else {
                    UpdateInstallFailureCategory::UnsupportedPlatform
                },
                if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
                    "update.install_launch_failed"
                } else {
                    "update.unsupported_platform"
                },
            ));
        }
        Ok(self.snapshot())
    }

    fn begin_install(&self) -> Result<(String, Vec<u8>), UpdateInstallFailure> {
        let mut state = self.state.lock().map_err(|_| {
            UpdateInstallFailure::new(
                UpdateInstallFailureCategory::StateUnavailable,
                "update.state_unavailable",
            )
        })?;
        if state.installing {
            return Err(UpdateInstallFailure::new(
                UpdateInstallFailureCategory::Busy,
                "update.busy",
            ));
        }
        let pending = state.pending.as_ref().ok_or_else(|| {
            UpdateInstallFailure::new(
                UpdateInstallFailureCategory::NoPendingUpdate,
                "update.no_pending_update",
            )
        })?;
        let result = (pending.version.clone(), pending.bytes.clone());
        state.installing = true;
        Ok(result)
    }

    fn cancel_install(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.installing = false;
        }
    }

    fn persist_install_attempt(&self, version: &str) -> Result<(), UpdateInstallFailure> {
        let Some(path) = self.install_attempt_path.as_ref() else {
            return Ok(());
        };
        let parent = path.parent().ok_or_else(|| {
            UpdateInstallFailure::new(
                UpdateInstallFailureCategory::StateUnavailable,
                "update.state_unavailable",
            )
        })?;
        fs::create_dir_all(parent).map_err(|_| {
            UpdateInstallFailure::new(
                UpdateInstallFailureCategory::StateUnavailable,
                "update.state_unavailable",
            )
        })?;
        let temporary = path.with_extension("tmp");
        let bytes = serde_json::to_vec(&InstallAttempt {
            target_version: version.to_owned(),
        })
        .map_err(|_| {
            UpdateInstallFailure::new(
                UpdateInstallFailureCategory::StateUnavailable,
                "update.state_unavailable",
            )
        })?;
        fs::write(&temporary, bytes).map_err(|_| {
            UpdateInstallFailure::new(
                UpdateInstallFailureCategory::StateUnavailable,
                "update.state_unavailable",
            )
        })?;
        #[cfg(windows)]
        let _ = fs::remove_file(path);
        fs::rename(&temporary, path).map_err(|_| {
            UpdateInstallFailure::new(
                UpdateInstallFailureCategory::StateUnavailable,
                "update.state_unavailable",
            )
        })?;
        Ok(())
    }

    fn clear_install_attempt(&self) {
        if let Some(path) = self.install_attempt_path.as_ref() {
            let _ = fs::remove_file(path);
        }
    }

    fn write_installer(
        &self,
        version: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, UpdateInstallFailure> {
        let path = std::env::temp_dir().join(format!("GPTEasy-update-{version}.exe"));
        fs::write(&path, bytes).map_err(|_| {
            UpdateInstallFailure::new(
                UpdateInstallFailureCategory::StateUnavailable,
                "update.installer_write_failed",
            )
        })?;
        Ok(path)
    }

    fn fail(&self, category: UpdateFailureCategory) -> UpdateSnapshot {
        if let Ok(mut state) = self.state.lock() {
            state.snapshot.state = UpdateState::Failed;
            state.snapshot.failure_category = Some(category);
            state.snapshot.error_message = Some(
                match category {
                    UpdateFailureCategory::CheckFailed => "暂时无法检查应用更新，请稍后重试。",
                    UpdateFailureCategory::ManifestInvalid => "更新清单无效，已停止本次更新。",
                    UpdateFailureCategory::DownloadFailed => "应用更新下载失败，请重试。",
                    UpdateFailureCategory::SignatureInvalid => {
                        "应用更新未通过签名验证，已拒绝使用。"
                    }
                }
                .to_owned(),
            );
            state.snapshot.checked_at_epoch_seconds = Some(now_epoch_seconds());
        }
        self.snapshot()
    }
}

fn release_notes_url(version: &str) -> String {
    format!("{GITCODE_RELEASES_URL}/v{version}")
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn decode_tauri_value(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.starts_with("untrusted comment: ") {
        return Ok(trimmed.to_owned());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .map_err(|_| "更新签名格式无效".to_owned())?;
    String::from_utf8(bytes).map_err(|_| "更新签名不是 UTF-8".to_owned())
}

fn verify_signature(bytes: &[u8], signature: &str, public_key: &str) -> Result<(), String> {
    let key = PublicKey::decode(&decode_tauri_value(public_key)?)
        .map_err(|_| "更新公钥无效".to_owned())?;
    let signature = Signature::decode(&decode_tauri_value(signature)?)
        .map_err(|_| "更新签名无效".to_owned())?;
    key.verify(bytes, &signature, false)
        .map_err(|_| "更新签名验证失败".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn stable_version_rejects_prerelease_and_build_suffixes() {
        assert!(StableVersion::parse("1.2.3").is_ok());
        assert!(StableVersion::parse("1.2.3-beta.1").is_err());
        assert!(StableVersion::parse("1.2.3+build").is_err());
        assert!(StableVersion::parse("01.2.3").is_err());
    }

    #[test]
    fn progress_is_percent_only_when_total_is_known() {
        let coordinator = UpdateCoordinator::with_endpoint("1.0.0", "http://127.0.0.1", "key");
        coordinator.begin_download(&Manifest {
            version: "1.1.0".to_owned(),
            notes: None,
            pub_date: None,
            platforms: std::collections::HashMap::new(),
        });
        coordinator.set_progress(50, None);
        assert_eq!(coordinator.snapshot().progress_percent, None);
        coordinator.set_progress(50, Some(100));
        assert_eq!(coordinator.snapshot().progress_percent, Some(50));
    }

    #[test]
    fn ready_snapshot_keeps_download_in_memory_and_deduplicates_version() {
        let coordinator = UpdateCoordinator::with_endpoint("1.0.0", "http://127.0.0.1", "key");
        coordinator.mark_ready(
            Manifest {
                version: "1.1.0".to_owned(),
                notes: Some("notes".to_owned()),
                pub_date: None,
                platforms: std::collections::HashMap::new(),
            },
            vec![1, 2, 3],
        );
        assert_eq!(coordinator.snapshot().state, UpdateState::Pending);
        assert!(coordinator.is_pending_version("1.1.0"));
    }

    #[test]
    fn check_and_download_is_mutually_exclusive() {
        let coordinator = UpdateCoordinator::with_endpoint("1.0.0", "http://127.0.0.1", "key");
        assert!(coordinator.begin_check());
        assert!(!coordinator.begin_check());
        coordinator.set_up_to_date();
        assert!(coordinator.begin_check());
    }

    #[test]
    fn startup_reports_an_incomplete_install_without_using_the_business_database() {
        let root = TempDir::new().expect("update state root");
        let path = root.path().join("update-install-attempt.json");
        fs::write(&path, br#"{"target_version":"1.1.0"}"#).expect("install attempt");

        let coordinator = UpdateCoordinator::with_state_path("1.0.0", &path);

        let snapshot = coordinator.snapshot();
        assert_eq!(snapshot.state, UpdateState::Incomplete);
        assert_eq!(snapshot.available_version.as_deref(), Some("1.1.0"));
        assert!(path.exists());
    }

    #[test]
    fn startup_confirms_the_target_version_and_clears_the_install_attempt() {
        let root = TempDir::new().expect("update state root");
        let path = root.path().join("update-install-attempt.json");
        fs::write(&path, br#"{"target_version":"1.1.0"}"#).expect("install attempt");

        let coordinator = UpdateCoordinator::with_state_path("1.1.0", &path);

        assert_eq!(coordinator.snapshot().state, UpdateState::UpToDate);
        assert!(!path.exists());
    }

    #[test]
    fn activity_gate_blocks_install_until_the_user_operation_finishes() {
        let gate = UpdateActivityGate::default();
        let guard = gate.try_begin("供应商验证").expect("begin activity");

        assert_eq!(gate.active_operation().as_deref(), Some("供应商验证"));
        assert!(gate.try_begin_install().is_none());

        drop(guard);
        assert!(gate.try_begin_install().is_some());
    }

    #[test]
    fn pending_snapshot_links_to_the_versioned_gitcode_release() {
        let coordinator = UpdateCoordinator::with_endpoint("1.0.0", "http://127.0.0.1", "key");
        coordinator.mark_ready(
            Manifest {
                version: "1.1.0".to_owned(),
                notes: Some("首段\n\n完整说明".to_owned()),
                pub_date: None,
                platforms: std::collections::HashMap::new(),
            },
            vec![1, 2, 3],
        );

        assert_eq!(
            coordinator.snapshot().release_notes_url.as_deref(),
            Some("https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag/v1.1.0")
        );
    }
}
