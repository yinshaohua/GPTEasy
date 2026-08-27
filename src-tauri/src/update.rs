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
pub const GITEE_RELEASES_URL: &str =
    "https://gitee.com/ericshaohua/gpteasy-releases/releases/tag";
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;
const NSIS_INSTALL_ARGUMENTS: [&str; 2] = ["/P", "/R"];

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
    committed: bool,
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
            committed: false,
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
            committed: false,
        })
    }

    pub fn active_operation(&self) -> Option<String> {
        self.active
            .lock()
            .ok()
            .and_then(|active| active.operations.first().cloned())
    }
}

impl UpdateActivityGuard {
    pub fn commit_install(mut self) {
        self.committed = true;
    }
}

impl Drop for UpdateActivityGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.gate.active.lock() {
            if let Some(operation) = self.operation.as_ref() {
                if let Some(index) = active.operations.iter().position(|item| item == operation) {
                    active.operations.remove(index);
                }
            } else if !self.committed {
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
        let user_agent = format!("Mozilla/5.0 (compatible; GPTEasy updater/{current_version})");
        Self {
            client: Client::builder()
                .user_agent(user_agent)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
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

    pub fn with_state_path(
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

    pub(crate) async fn check_and_download<F>(&self, progress: F) -> UpdateSnapshot
    where
        F: FnMut(UpdateSnapshot) + Send,
    {
        self.check_and_download_with_policy(progress, true).await
    }

    pub(crate) async fn scheduled_check_and_download<F>(&self, progress: F) -> UpdateSnapshot
    where
        F: FnMut(UpdateSnapshot) + Send,
    {
        self.check_and_download_with_policy(progress, false).await
    }

    async fn check_and_download_with_policy<F>(
        &self,
        mut progress: F,
        retry_incomplete: bool,
    ) -> UpdateSnapshot
    where
        F: FnMut(UpdateSnapshot) + Send,
    {
        if !self.begin_check(retry_incomplete) {
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
        if !is_permitted_update_url(&platform.url) {
            return self.fail(UpdateFailureCategory::ManifestInvalid);
        }
        if self.is_pending_version(&manifest.version) {
            self.restore_pending();
            return self.snapshot();
        }
        self.begin_download(&manifest);
        progress(self.snapshot());

        let bytes = match self.download_once(&platform.url, &mut progress).await {
            Ok(bytes) => bytes,
            Err(()) => return self.fail(UpdateFailureCategory::DownloadFailed),
        };
        if verify_signature(&bytes, &platform.signature, &self.public_key).is_err() {
            return self.fail(UpdateFailureCategory::SignatureInvalid);
        }
        self.mark_ready(manifest, bytes);
        self.snapshot()
    }

    fn begin_check(&self, retry_incomplete: bool) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if matches!(
            state.snapshot.state,
            UpdateState::Checking | UpdateState::Downloading
        ) || state.installing
            || (state.snapshot.state == UpdateState::Incomplete && !retry_incomplete)
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
            .map_err(|_| UpdateFailureCategory::CheckFailed)
    }

    async fn download_once<F>(&self, url: &str, progress: &mut F) -> Result<Vec<u8>, ()>
    where
        F: FnMut(UpdateSnapshot) + Send,
    {
        let response = self.client.get(url).send().await.map_err(|_| ())?;
        if !response.status().is_success() {
            return Err(());
        }
        let total = response.content_length();
        let initial_capacity = total.unwrap_or_default().min(16 * 1024 * 1024) as usize;
        let mut bytes = Vec::with_capacity(initial_capacity);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| ())?;
            bytes.extend_from_slice(&chunk);
            self.set_progress(bytes.len() as u64, total);
            progress(self.snapshot());
        }
        Ok(bytes)
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
        let prepared = self.write_installer(&version, &bytes).and_then(|path| {
            if let Err(failure) = self.persist_install_attempt(&version) {
                let _ = fs::remove_file(&path);
                return Err(failure);
            }
            Ok(path)
        });
        let path = match prepared {
            Ok(path) => path,
            Err(failure) => {
                self.cancel_install();
                return Err(failure);
            }
        };
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        let launched = std::process::Command::new(&path)
            .args(NSIS_INSTALL_ARGUMENTS)
            .spawn()
            .is_ok();
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
                    UpdateFailureCategory::CheckFailed => "暂时无法检查应用更新，请手动重试。",
                    UpdateFailureCategory::ManifestInvalid => "更新清单无效，已停止本次更新。",
                    UpdateFailureCategory::DownloadFailed => "应用更新下载失败，请手动重试。",
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

fn is_permitted_update_url(url: &str) -> bool {
    url.starts_with("https://")
        || (cfg!(test)
            && (url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")))
}

fn release_notes_url(version: &str) -> String {
    format!("{GITEE_RELEASES_URL}/v{version}")
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::TempDir;

    const TEST_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXkgRTc2MjBGMTg0MkI0RTgxRgpSV1FmNkxSQ0dBOWk1M21sWWVjTzRJelQ1MVRHUHB2V3VjTlNDaDFDQk0wUVRhTG43M1k3R0ZPMw==";
    const TEST_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIG1pbmlzaWduIHNlY3JldCBrZXkKUlVRZjZMUkNHQTlpNTU5cjNnN1YxcU55SkRBcEdpcDhNZnFjYWRJZ1Q5Q3VoVjNFTWhIb04xbUdUa1VpZEYvejdTcmxRZ1hkeThvZmpiN2JOSkp5bERPb2NyQ284S0x6WndvPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNTU2MTkzMzM1CWZpbGU6dGVzdAp5L3JVdzJ5OC9oT1VZalpVNzFlSHAvV28xS1o0MGZHeTJWSkVEbDM0WE1KTStUWDQ4U3MvMTd1M0l2SWZiVlIxRmtaWlNOQ2lzUWJ1UVkrYkh3aEVCZz09";

    struct ScriptedHttpServer {
        base_url: String,
    }

    impl ScriptedHttpServer {
        fn start(responses: Vec<(u16, String)>) -> Self {
            Self::start_with(move |_| responses)
        }

        fn start_with(build_responses: impl FnOnce(&str) -> Vec<(u16, String)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind update server");
            let address = listener.local_addr().expect("update server address");
            let base_url = format!("http://{address}");
            let responses = build_responses(&base_url);
            thread::spawn(move || {
                for (status, body) in responses {
                    let (mut stream, _) = listener.accept().expect("accept update request");
                    let mut request = [0_u8; 4096];
                    let _ = stream.read(&mut request);
                    let reason = if status == 200 {
                        "OK"
                    } else {
                        "Temporary failure"
                    };
                    write!(
                        stream,
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len(),
                    )
                    .expect("write update response");
                }
            });
            Self { base_url }
        }

        fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base_url)
        }
    }

    #[tokio::test]
    async fn transient_manifest_failure_is_exposed_until_the_user_retries() {
        let server = ScriptedHttpServer::start(vec![(418, String::new())]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");

        let snapshot = coordinator.check_and_download(|_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::CheckFailed)
        );
    }

    #[tokio::test]
    async fn throttled_manifest_check_is_not_retried_automatically() {
        let server = ScriptedHttpServer::start(vec![(429, String::new())]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");

        let snapshot = coordinator.check_and_download(|_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::CheckFailed)
        );
    }

    #[tokio::test]
    async fn repeated_non_json_manifest_remains_a_recoverable_check_failure() {
        let server = ScriptedHttpServer::start(vec![(
            200,
            "<html>temporary WAF response</html>".to_owned(),
        )]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");

        let snapshot = coordinator.check_and_download(|_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::CheckFailed)
        );
    }

    #[tokio::test]
    async fn transient_download_failure_is_exposed_until_the_user_retries() {
        let server = ScriptedHttpServer::start_with(|base_url| {
            let manifest = serde_json::json!({
                "version": "1.1.6",
                "platforms": {
                    "windows-x86_64": {
                        "url": format!("{base_url}/setup.exe"),
                        "signature": TEST_SIGNATURE
                    }
                }
            })
            .to_string();
            vec![(200, manifest), (503, String::new())]
        });
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), TEST_PUBLIC_KEY);

        let snapshot = coordinator.check_and_download(|_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(snapshot.available_version.as_deref(), Some("1.1.6"));
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::DownloadFailed)
        );
    }

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
        assert!(coordinator.begin_check(true));
        assert!(!coordinator.begin_check(true));
        coordinator.set_up_to_date();
        assert!(coordinator.begin_check(true));
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
    fn pending_snapshot_links_to_the_versioned_gitee_release() {
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
            Some("https://gitee.com/ericshaohua/gpteasy-releases/releases/tag/v1.1.0")
        );
    }

    #[test]
    fn duplicate_install_confirmation_is_rejected_until_the_first_attempt_finishes() {
        let coordinator = UpdateCoordinator::with_endpoint("1.0.0", "http://127.0.0.1", "key");
        coordinator.mark_ready(
            Manifest {
                version: "1.1.0".to_owned(),
                notes: None,
                pub_date: None,
                platforms: std::collections::HashMap::new(),
            },
            vec![1, 2, 3],
        );

        assert!(coordinator.begin_install().is_ok());
        let repeated = coordinator.begin_install().expect_err("duplicate install");
        assert_eq!(repeated.category, UpdateInstallFailureCategory::Busy);
        assert_eq!(repeated.message_id, "update.busy");
    }

    #[test]
    fn failed_install_preparation_clears_attempt_state_and_keeps_the_package_pending() {
        let root = TempDir::new().expect("update state root");
        let blocking_file = root.path().join("not-a-directory");
        fs::write(&blocking_file, b"block parent creation").expect("blocking file");
        let attempt = blocking_file.join("update-install-attempt.json");
        let coordinator = UpdateCoordinator::with_state_path("1.0.0", &attempt);
        coordinator.mark_ready(
            Manifest {
                version: "9.9.7".to_owned(),
                notes: None,
                pub_date: None,
                platforms: std::collections::HashMap::new(),
            },
            b"not-an-installer".to_vec(),
        );

        let failure = coordinator
            .confirm_install()
            .expect_err("preparation fails");

        assert_eq!(
            failure.category,
            UpdateInstallFailureCategory::StateUnavailable
        );
        assert_eq!(coordinator.snapshot().state, UpdateState::Pending);
        assert!(!attempt.exists());
        assert!(
            !std::env::temp_dir()
                .join("GPTEasy-update-9.9.7.exe")
                .exists()
        );
    }

    #[test]
    fn process_restart_discards_the_pending_package_and_allows_a_fresh_download() {
        let root = TempDir::new().expect("update state root");
        let attempt = root.path().join("update-install-attempt.json");
        let coordinator = UpdateCoordinator::with_state_path("1.0.0", &attempt);
        coordinator.mark_ready(
            Manifest {
                version: "1.1.0".to_owned(),
                notes: None,
                pub_date: None,
                platforms: std::collections::HashMap::new(),
            },
            vec![1, 2, 3],
        );
        assert!(coordinator.is_pending_version("1.1.0"));
        drop(coordinator);

        let restarted = UpdateCoordinator::with_state_path("1.0.0", &attempt);

        assert_eq!(restarted.snapshot().state, UpdateState::Idle);
        assert!(!restarted.is_pending_version("1.1.0"));
        assert!(restarted.begin_check(true));
    }

    #[test]
    fn automatic_check_preserves_incomplete_state_until_the_user_retries() {
        let root = TempDir::new().expect("update state root");
        let path = root.path().join("update-install-attempt.json");
        fs::write(&path, br#"{"target_version":"1.1.0"}"#).expect("install attempt");
        let coordinator = UpdateCoordinator::with_state_path("1.0.0", &path);

        assert!(!coordinator.begin_check(false));
        assert_eq!(coordinator.snapshot().state, UpdateState::Incomplete);
        assert!(coordinator.begin_check(true));
    }

    #[test]
    fn committed_install_gate_rejects_new_write_operations() {
        let gate = UpdateActivityGate::default();
        let install = gate.try_begin_install().expect("begin install");
        install.commit_install();

        assert!(gate.try_begin("配置写入").is_none());
        assert!(gate.try_begin_install().is_none());
    }

    #[test]
    fn install_attempt_record_contains_only_the_target_version() {
        let root = TempDir::new().expect("update state root");
        let path = root.path().join("update-install-attempt.json");
        let coordinator = UpdateCoordinator::with_state_path("1.0.0", &path);

        coordinator
            .persist_install_attempt("1.1.0")
            .expect("persist attempt");

        assert_eq!(
            fs::read_to_string(path).expect("read attempt"),
            r#"{"target_version":"1.1.0"}"#
        );
    }

    #[test]
    fn nsis_install_is_passive_and_restarts_the_new_version() {
        assert_eq!(NSIS_INSTALL_ARGUMENTS, ["/P", "/R"]);
    }
}
