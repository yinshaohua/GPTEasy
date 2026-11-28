use std::cmp::Ordering;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures_util::StreamExt;
use minisign_verify::{PublicKey, Signature};
use reqwest::Client;
use reqwest::header::RETRY_AFTER;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio::time::Instant;

pub const MANUAL_DOWNLOAD_URL: &str = "https://gitee.com/ericshaohua/gpteasy-releases/releases";
pub const GITEE_RELEASES_URL: &str = "https://gitee.com/ericshaohua/gpteasy-releases/releases/tag";
const GITEE_RAW_MANIFEST_URL: &str =
    "https://gitee.com/ericshaohua/gpteasy-releases/raw/main/latest.md";
const GITEE_CONTENTS_MANIFEST_URL: &str =
    "https://gitee.com/api/v5/repos/ericshaohua/gpteasy-releases/contents/latest.md?ref=main";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(600),
];
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
    pub next_retry_at_epoch_seconds: Option<u64>,
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
            next_retry_at_epoch_seconds: None,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManifestSource {
    Api,
    Raw,
    Custom,
}

impl ManifestSource {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Raw => "raw",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone)]
struct ManifestEndpoint {
    url: String,
    source: ManifestSource,
}

#[derive(Debug, Deserialize)]
struct GiteeContentsFile {
    content: String,
    encoding: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateCheckStage {
    ManifestRequest,
    ManifestStatus,
    ManifestBody,
    ManifestValidation,
    DownloadRequest,
    DownloadStatus,
    DownloadBody,
    Signature,
}

impl UpdateCheckStage {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::ManifestRequest => "manifest_request",
            Self::ManifestStatus => "manifest_status",
            Self::ManifestBody => "manifest_body",
            Self::ManifestValidation => "manifest_validation",
            Self::DownloadRequest => "download_request",
            Self::DownloadStatus => "download_status",
            Self::DownloadBody => "download_body",
            Self::Signature => "signature",
        }
    }

    fn is_manifest_read(self) -> bool {
        matches!(
            self,
            Self::ManifestRequest | Self::ManifestStatus | Self::ManifestBody
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateCheckError {
    Timeout,
    ConnectionReset,
    Connection,
    Interrupted,
    Tls,
    Request,
    HttpStatus,
    InvalidJson,
    InvalidVersion,
    MissingPlatform,
    InvalidUrl,
    InvalidSignature,
}

impl UpdateCheckError {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::ConnectionReset => "connection_reset",
            Self::Connection => "connection",
            Self::Interrupted => "interrupted",
            Self::Tls => "tls",
            Self::Request => "request",
            Self::HttpStatus => "http_status",
            Self::InvalidJson => "invalid_json",
            Self::InvalidVersion => "invalid_version",
            Self::MissingPlatform => "missing_platform",
            Self::InvalidUrl => "invalid_url",
            Self::InvalidSignature => "invalid_signature",
        }
    }
}

#[derive(Debug)]
pub(crate) struct UpdateCheckFailure {
    pub(crate) stage: UpdateCheckStage,
    pub(crate) error: UpdateCheckError,
    pub(crate) http_status: Option<u16>,
    pub(crate) os_error: Option<i32>,
    pub(crate) manifest_source: Option<ManifestSource>,
    retry_after: Option<Duration>,
}

impl UpdateCheckFailure {
    pub(crate) fn new(stage: UpdateCheckStage, error: UpdateCheckError) -> Self {
        Self {
            stage,
            error,
            http_status: None,
            os_error: None,
            manifest_source: None,
            retry_after: None,
        }
    }

    fn from_status(stage: UpdateCheckStage, response: &reqwest::Response) -> Self {
        let status = response.status();
        let mut failure = Self::new(stage, UpdateCheckError::HttpStatus);
        failure.http_status = Some(status.as_u16());
        if failure.retryable() {
            failure.retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|header| header.to_str().ok())
                .and_then(|header| retry_after_delay(header, SystemTime::now()));
        }
        failure
    }

    fn from_transport(stage: UpdateCheckStage, error: &reqwest::Error) -> Self {
        let kind = if error.is_timeout() {
            UpdateCheckError::Timeout
        } else if error.is_builder() || error.is_redirect() {
            UpdateCheckError::Request
        } else if error.is_connect() {
            UpdateCheckError::Connection
        } else {
            UpdateCheckError::Interrupted
        };
        let (kind, os_error) = transport_cause(kind, error.source());
        let mut failure = Self::new(stage, kind);
        failure.os_error = os_error;
        failure
    }

    pub(crate) fn category(&self) -> UpdateFailureCategory {
        match self.stage {
            UpdateCheckStage::ManifestValidation => UpdateFailureCategory::ManifestInvalid,
            UpdateCheckStage::Signature => UpdateFailureCategory::SignatureInvalid,
            stage if stage.is_manifest_read() => UpdateFailureCategory::CheckFailed,
            _ => UpdateFailureCategory::DownloadFailed,
        }
    }

    fn retryable(&self) -> bool {
        self.stage.is_manifest_read()
            && match self.error {
                UpdateCheckError::Timeout
                | UpdateCheckError::ConnectionReset
                | UpdateCheckError::Connection
                | UpdateCheckError::Interrupted => true,
                UpdateCheckError::HttpStatus => {
                    matches!(self.http_status, Some(429 | 500 | 502 | 503 | 504))
                }
                _ => false,
            }
    }
}

fn transport_cause(
    mut kind: UpdateCheckError,
    mut cause: Option<&(dyn Error + 'static)>,
) -> (UpdateCheckError, Option<i32>) {
    let mut os_error = None;
    while let Some(source) = cause {
        if source.is::<rustls::Error>() {
            kind = UpdateCheckError::Tls;
            break;
        }
        if let Some(io_error) = source.downcast_ref::<io::Error>() {
            os_error = os_error.or(io_error.raw_os_error());
            if io_error.kind() == io::ErrorKind::ConnectionReset {
                kind = UpdateCheckError::ConnectionReset;
            }
            // TLS wraps the socket error in io::Error. Inspect its payload before
            // following source(), which can skip the wrapped error itself.
            if let Some(inner) = io_error.get_ref() {
                cause = Some(inner);
                continue;
            }
        }
        cause = source.source();
    }
    (kind, os_error)
}

fn retry_after_delay(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    let delay = if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Duration::from_secs(value.parse().ok()?)
    } else {
        httpdate::parse_http_date(value)
            .ok()?
            .duration_since(now)
            .ok()?
    };
    now.checked_add(delay)?;
    Some(delay)
}

#[derive(Debug)]
struct CheckSchedule {
    next_check: Instant,
    retries_scheduled: usize,
    retry_at_epoch_seconds: Option<u64>,
}

impl CheckSchedule {
    fn new() -> Self {
        Self {
            next_check: Instant::now(),
            retries_scheduled: 0,
            retry_at_epoch_seconds: None,
        }
    }

    fn record_result(
        &mut self,
        manual: bool,
        result: Result<(), &UpdateCheckFailure>,
        now: Instant,
        epoch: u64,
    ) {
        let retry_after = result
            .as_ref()
            .err()
            .and_then(|failure| failure.retry_after)
            .and_then(|delay| now.checked_add(delay));
        if manual && result.is_err() {
            if let Some(earliest) = retry_after.filter(|earliest| *earliest > self.next_check) {
                self.next_check = earliest;
                if self.retry_at_epoch_seconds.is_some() {
                    self.retry_at_epoch_seconds = Some(retry_epoch(epoch, now, earliest));
                }
            }
            return;
        }
        if let Err(failure) = result {
            if failure.retryable() {
                if let Some(delay) = RETRY_DELAYS.get(self.retries_scheduled) {
                    self.retries_scheduled += 1;
                    self.next_check = retry_after.unwrap_or(now).max(now + *delay);
                    self.retry_at_epoch_seconds = Some(retry_epoch(epoch, now, self.next_check));
                    return;
                }
            }
        }
        self.next_check = retry_after.unwrap_or(now).max(now + CHECK_INTERVAL);
        self.retries_scheduled = 0;
        self.retry_at_epoch_seconds = None;
    }
}

fn retry_epoch(epoch: u64, now: Instant, deadline: Instant) -> u64 {
    let wait = deadline.duration_since(now);
    epoch
        .saturating_add(wait.as_secs())
        .saturating_add(u64::from(wait.subsec_nanos() != 0))
}

#[derive(Debug)]
struct CoordinatorState {
    snapshot: UpdateSnapshot,
    pending: Option<PendingUpdate>,
    installing: bool,
    checking: bool,
    schedule: CheckSchedule,
}

#[derive(Clone)]
pub struct UpdateCoordinator {
    client: Client,
    current_version: String,
    manifest_endpoints: Vec<ManifestEndpoint>,
    public_key: String,
    state: Arc<Mutex<CoordinatorState>>,
    install_attempt_path: Option<PathBuf>,
    schedule_changed: watch::Sender<()>,
}

impl UpdateCoordinator {
    pub fn new(current_version: impl Into<String>) -> Self {
        let (endpoint, public_key) = configured_trust_root();
        let endpoints = if endpoint == GITEE_RAW_MANIFEST_URL {
            vec![
                ManifestEndpoint {
                    url: GITEE_CONTENTS_MANIFEST_URL.to_owned(),
                    source: ManifestSource::Api,
                },
                ManifestEndpoint {
                    url: endpoint,
                    source: ManifestSource::Raw,
                },
            ]
        } else {
            vec![ManifestEndpoint {
                url: endpoint,
                source: ManifestSource::Custom,
            }]
        };
        Self::with_manifest_endpoints(current_version, endpoints, public_key)
    }

    #[cfg(test)]
    pub(crate) fn with_endpoint(
        current_version: impl Into<String>,
        endpoint: impl Into<String>,
        public_key: impl Into<String>,
    ) -> Self {
        Self::with_manifest_endpoints(
            current_version,
            vec![ManifestEndpoint {
                url: endpoint.into(),
                source: ManifestSource::Custom,
            }],
            public_key,
        )
    }

    fn with_manifest_endpoints(
        current_version: impl Into<String>,
        manifest_endpoints: Vec<ManifestEndpoint>,
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
            manifest_endpoints,
            public_key: public_key.into(),
            state: Arc::new(Mutex::new(CoordinatorState {
                snapshot: UpdateSnapshot::new(&current_version),
                pending: None,
                installing: false,
                checking: false,
                schedule: CheckSchedule::new(),
            })),
            schedule_changed: watch::channel(()).0,
            current_version,
            install_attempt_path: None,
        }
    }

    pub fn with_state_path(
        current_version: impl Into<String>,
        install_attempt_path: impl AsRef<Path>,
    ) -> Self {
        let mut coordinator = Self::new(current_version);
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

    pub(crate) async fn wait_for_scheduled_check(&self) {
        let mut changes = self.schedule_changed.subscribe();
        loop {
            let deadline = self
                .state
                .lock()
                .ok()
                .and_then(|state| (!state.checking).then_some(state.schedule.next_check));
            if let Some(deadline) = deadline {
                tokio::select! {
                    biased;
                    _ = changes.changed() => continue,
                    _ = tokio::time::sleep_until(deadline) => return,
                }
            }
            let _ = changes.changed().await;
        }
    }

    pub(crate) async fn check_and_download<F>(
        &self,
        manual: bool,
        mut progress: F,
    ) -> (UpdateSnapshot, Option<(usize, UpdateCheckFailure)>)
    where
        F: FnMut(UpdateSnapshot) + Send,
    {
        let Some((attempt, previous_pending)) = self.begin_check(manual) else {
            return (self.snapshot(), None);
        };
        progress(self.snapshot());
        let result = self.run_check(&mut progress).await;
        let snapshot = self.finish_check(manual, result.as_ref().map(|_| ()), previous_pending);
        (snapshot, result.err().map(|failure| (attempt, failure)))
    }

    async fn run_check<F>(&self, progress: &mut F) -> Result<(), UpdateCheckFailure>
    where
        F: FnMut(UpdateSnapshot) + Send,
    {
        use UpdateCheckError as Failure;
        use UpdateCheckStage as Stage;
        let manifest = self.fetch_manifest().await?;
        let invalid = |error| UpdateCheckFailure::new(Stage::ManifestValidation, error);
        let version = match StableVersion::parse(&manifest.version) {
            Ok(version) => version,
            Err(_) if is_prerelease_version(&manifest.version) => {
                self.set_up_to_date();
                return Ok(());
            }
            Err(_) => return Err(invalid(Failure::InvalidVersion)),
        };
        let current = StableVersion::parse(&self.current_version)
            .map_err(|_| invalid(Failure::InvalidVersion))?;
        if version.cmp(&current) != Ordering::Greater {
            self.set_up_to_date();
            return Ok(());
        }

        let platform = manifest
            .platforms
            .get("windows-x86_64")
            .ok_or_else(|| invalid(Failure::MissingPlatform))?;
        if !is_permitted_update_url(&platform.url) {
            return Err(invalid(Failure::InvalidUrl));
        }
        if self.is_pending_version(&manifest.version) {
            self.restore_pending();
            return Ok(());
        }
        self.begin_download(&manifest);
        progress(self.snapshot());

        let bytes = self.download_once(&platform.url, progress).await?;
        verify_signature(&bytes, &platform.signature, &self.public_key)
            .map_err(|_| UpdateCheckFailure::new(Stage::Signature, Failure::InvalidSignature))?;
        self.mark_ready(manifest, bytes);
        Ok(())
    }

    fn begin_check(&self, manual: bool) -> Option<(usize, Option<UpdateSnapshot>)> {
        let mut state = self.state.lock().ok()?;
        if state.checking {
            return None;
        }
        let now = Instant::now();
        if !manual && now < state.schedule.next_check {
            return None;
        }
        if state.installing || (state.snapshot.state == UpdateState::Incomplete && !manual) {
            if !manual {
                state
                    .schedule
                    .record_result(false, Ok(()), now, now_epoch_seconds());
                state.snapshot.next_retry_at_epoch_seconds = None;
            }
            return None;
        }
        let attempt = if manual {
            1
        } else {
            state.schedule.retries_scheduled + 1
        };
        let previous_pending =
            (state.snapshot.state == UpdateState::Pending).then(|| state.snapshot.clone());
        state.checking = true;
        state.snapshot.state = UpdateState::Checking;
        state.snapshot.failure_category = None;
        state.snapshot.error_message = None;
        state.snapshot.next_retry_at_epoch_seconds = None;
        self.schedule_changed.send_replace(());
        Some((attempt, previous_pending))
    }

    fn finish_check(
        &self,
        manual: bool,
        result: Result<(), &UpdateCheckFailure>,
        previous_pending: Option<UpdateSnapshot>,
    ) -> UpdateSnapshot {
        let Ok(mut state) = self.state.lock() else {
            return UpdateSnapshot::new(&self.current_version);
        };
        let checked_at = now_epoch_seconds();
        state
            .schedule
            .record_result(manual, result, Instant::now(), checked_at);
        if let Err(failure) = result {
            if let Some(pending) = previous_pending {
                state.snapshot = pending;
            } else {
                state.snapshot.state = UpdateState::Failed;
            }
            let category = failure.category();
            state.snapshot.failure_category = Some(category);
            state.snapshot.error_message = Some(
                match (category, failure.error) {
                    (UpdateFailureCategory::CheckFailed, UpdateCheckError::Tls) => {
                        "无法建立安全的更新连接，请检查网络后重试。"
                    }
                    (UpdateFailureCategory::CheckFailed, _)
                        if state.schedule.retry_at_epoch_seconds.is_some() =>
                    {
                        "暂时无法检查应用更新。"
                    }
                    (UpdateFailureCategory::CheckFailed, _) => "暂时无法检查应用更新，请手动重试。",
                    (UpdateFailureCategory::ManifestInvalid, _) => "更新清单无效，已停止本次更新。",
                    (UpdateFailureCategory::DownloadFailed, _) => "应用更新下载失败，请手动重试。",
                    (UpdateFailureCategory::SignatureInvalid, _) => {
                        "应用更新未通过签名验证，已拒绝使用。"
                    }
                }
                .to_owned(),
            );
        }
        state.snapshot.checked_at_epoch_seconds = Some(checked_at);
        state.snapshot.next_retry_at_epoch_seconds = state.schedule.retry_at_epoch_seconds;
        state.checking = false;
        self.schedule_changed.send_replace(());
        state.snapshot.clone()
    }

    async fn fetch_manifest(&self) -> Result<Manifest, UpdateCheckFailure> {
        use UpdateCheckStage as Stage;
        let mut last_failure = None;
        for endpoint in &self.manifest_endpoints {
            match self.fetch_manifest_bytes(endpoint).await {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(manifest) => return Ok(manifest),
                    Err(_) => {
                        let mut failure = UpdateCheckFailure::new(
                            Stage::ManifestValidation,
                            UpdateCheckError::InvalidJson,
                        );
                        failure.manifest_source = Some(endpoint.source);
                        return Err(failure);
                    }
                },
                Err(failure) => {
                    let retry_with_next_endpoint =
                        endpoint.source == ManifestSource::Api && self.manifest_endpoints.len() > 1;
                    last_failure = Some(failure);
                    if !retry_with_next_endpoint {
                        break;
                    }
                }
            }
        }
        Err(last_failure.expect("更新清单端点不能为空"))
    }

    async fn fetch_manifest_bytes(
        &self,
        endpoint: &ManifestEndpoint,
    ) -> Result<Vec<u8>, UpdateCheckFailure> {
        use UpdateCheckStage as Stage;
        let response = self
            .client
            .get(&endpoint.url)
            .send()
            .await
            .map_err(|error| {
                let mut failure =
                    UpdateCheckFailure::from_transport(Stage::ManifestRequest, &error);
                failure.manifest_source = Some(endpoint.source);
                failure
            })?;
        if !response.status().is_success() {
            let mut failure = UpdateCheckFailure::from_status(Stage::ManifestStatus, &response);
            failure.manifest_source = Some(endpoint.source);
            return Err(failure);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| UpdateCheckFailure::from_transport(Stage::ManifestBody, &error));
        let mut bytes = match bytes {
            Ok(bytes) => bytes.to_vec(),
            Err(mut failure) => {
                failure.manifest_source = Some(endpoint.source);
                return Err(failure);
            }
        };
        if endpoint.source == ManifestSource::Api {
            let file: GiteeContentsFile = serde_json::from_slice(&bytes).map_err(|_| {
                let mut failure = UpdateCheckFailure::new(
                    Stage::ManifestValidation,
                    UpdateCheckError::InvalidJson,
                );
                failure.manifest_source = Some(endpoint.source);
                failure
            })?;
            if file.encoding != "base64" {
                let mut failure = UpdateCheckFailure::new(
                    Stage::ManifestValidation,
                    UpdateCheckError::InvalidJson,
                );
                failure.manifest_source = Some(endpoint.source);
                return Err(failure);
            }
            bytes = base64::engine::general_purpose::STANDARD
                .decode(file.content.replace(char::is_whitespace, ""))
                .map_err(|_| {
                    let mut failure = UpdateCheckFailure::new(
                        Stage::ManifestValidation,
                        UpdateCheckError::InvalidJson,
                    );
                    failure.manifest_source = Some(endpoint.source);
                    failure
                })?;
        }
        Ok(bytes)
    }

    async fn download_once<F>(
        &self,
        url: &str,
        progress: &mut F,
    ) -> Result<Vec<u8>, UpdateCheckFailure>
    where
        F: FnMut(UpdateSnapshot) + Send,
    {
        use UpdateCheckStage as Stage;
        let response =
            self.client.get(url).send().await.map_err(|error| {
                UpdateCheckFailure::from_transport(Stage::DownloadRequest, &error)
            })?;
        if !response.status().is_success() {
            return Err(UpdateCheckFailure::from_status(
                Stage::DownloadStatus,
                &response,
            ));
        }
        let total = response.content_length();
        let initial_capacity = total.unwrap_or_default().min(16 * 1024 * 1024) as usize;
        let mut bytes = Vec::with_capacity(initial_capacity);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|error| UpdateCheckFailure::from_transport(Stage::DownloadBody, &error))?;
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
        if state.installing || state.checking {
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
    use std::sync::{Arc, Mutex};
    use std::thread;
    use tempfile::TempDir;

    const TEST_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXkgRTc2MjBGMTg0MkI0RTgxRgpSV1FmNkxSQ0dBOWk1M21sWWVjTzRJelQ1MVRHUHB2V3VjTlNDaDFDQk0wUVRhTG43M1k3R0ZPMw==";
    const TEST_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIG1pbmlzaWduIHNlY3JldCBrZXkKUlVRZjZMUkNHQTlpNTU5cjNnN1YxcU55SkRBcEdpcDhNZnFjYWRJZ1Q5Q3VoVjNFTWhIb04xbUdUa1VpZEYvejdTcmxRZ1hkeThvZmpiN2JOSkp5bERPb2NyQ284S0x6WndvPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNTU2MTkzMzM1CWZpbGU6dGVzdAp5L3JVdzJ5OC9oT1VZalpVNzFlSHAvV28xS1o0MGZHeTJWSkVEbDM0WE1KTStUWDQ4U3MvMTd1M0l2SWZiVlIxRmtaWlNOQ2lzUWJ1UVkrYkh3aEVCZz09";

    struct ScriptedHttpServer {
        base_url: String,
        requests: Arc<Mutex<Vec<HttpRequest>>>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct HttpRequest {
        method: String,
        path: String,
        authorization: Option<String>,
    }

    impl ScriptedHttpServer {
        fn start(responses: Vec<(u16, String)>) -> Self {
            Self::start_with(move |_| responses)
        }

        fn start_with(build_responses: impl FnOnce(&str) -> Vec<(u16, String)>) -> Self {
            Self::serve(build_responses, "")
        }

        fn start_with_headers(responses: Vec<(u16, String)>, headers: &'static str) -> Self {
            Self::serve(move |_| responses, headers)
        }

        fn serve(
            build_responses: impl FnOnce(&str) -> Vec<(u16, String)>,
            response_headers: &'static str,
        ) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind update server");
            let address = listener.local_addr().expect("update server address");
            let base_url = format!("http://{address}");
            let responses = build_responses(&base_url);
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);
            thread::spawn(move || {
                for (status, body) in responses {
                    let (mut stream, _) = listener.accept().expect("accept update request");
                    let mut request = [0_u8; 4096];
                    let read = stream.read(&mut request).expect("read update request");
                    let headers = String::from_utf8_lossy(&request[..read]);
                    let mut request_parts = headers
                        .lines()
                        .next()
                        .expect("request line")
                        .split_whitespace();
                    let method = request_parts.next().expect("request method").to_owned();
                    let path = request_parts.next().expect("request path").to_owned();
                    let authorization = headers.lines().find_map(|line| {
                        line.strip_prefix("Authorization: ")
                            .or_else(|| line.strip_prefix("authorization: "))
                            .map(str::to_owned)
                    });
                    server_requests
                        .lock()
                        .expect("update request records")
                        .push(HttpRequest {
                            method,
                            path,
                            authorization,
                        });
                    if status == 0 {
                        continue;
                    }
                    let (status, length) = if status == 1 {
                        (200, body.len() + 10)
                    } else {
                        (status, body.len())
                    };
                    write!(
                        stream,
                        "HTTP/1.1 {status} Test response\r\n{response_headers}Content-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}",
                    )
                    .expect("write update response");
                }
            });
            Self { base_url, requests }
        }

        fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base_url)
        }

        fn requests(&self) -> Vec<HttpRequest> {
            self.requests
                .lock()
                .expect("update request records")
                .clone()
        }
    }

    #[tokio::test]
    async fn canonical_gitee_manifest_prefers_contents_api() {
        let manifest = serde_json::json!({
            "version": "1.1.5",
            "platforms": {}
        });
        let encoded = base64::engine::general_purpose::STANDARD.encode(manifest.to_string());
        let server = ScriptedHttpServer::start_with(|_| {
            vec![(
                200,
                format!(r#"{{"content":"{encoded}","encoding":"base64"}}"#),
            )]
        });
        let coordinator = UpdateCoordinator::with_manifest_endpoints(
            "1.1.5",
            vec![ManifestEndpoint {
                url: server.url("/contents/latest.md?ref=main"),
                source: ManifestSource::Api,
            }],
            "unused",
        );

        let (snapshot, failure) = coordinator.check_and_download(true, |_| {}).await;

        assert_eq!(snapshot.state, UpdateState::UpToDate);
        assert!(failure.is_none());
        assert_eq!(server.requests()[0].path, "/contents/latest.md?ref=main");
    }

    #[tokio::test]
    async fn canonical_gitee_manifest_falls_back_to_raw_after_api_failure() {
        let manifest = serde_json::json!({
            "version": "1.1.5",
            "platforms": {}
        });
        let server =
            ScriptedHttpServer::start(vec![(503, String::new()), (200, manifest.to_string())]);
        let coordinator = UpdateCoordinator::with_manifest_endpoints(
            "1.1.5",
            vec![
                ManifestEndpoint {
                    url: server.url("/contents/latest.md?ref=main"),
                    source: ManifestSource::Api,
                },
                ManifestEndpoint {
                    url: server.url("/raw/main/latest.md"),
                    source: ManifestSource::Raw,
                },
            ],
            "unused",
        );

        let (snapshot, failure) = coordinator.check_and_download(true, |_| {}).await;

        assert_eq!(snapshot.state, UpdateState::UpToDate);
        assert!(failure.is_none());
        assert_eq!(server.requests().len(), 2);
        assert_eq!(server.requests()[0].path, "/contents/latest.md?ref=main");
        assert_eq!(server.requests()[1].path, "/raw/main/latest.md");
    }
    #[tokio::test]
    async fn scheduled_check_recovers_from_a_disconnected_manifest_request() {
        let server = ScriptedHttpServer::start(vec![
            (0, String::new()),
            (200, r#"{"version":"1.1.5","platforms":{}}"#.to_owned()),
        ]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");

        let (failed, failure) = coordinator.check_and_download(false, |_| {}).await;
        assert_eq!(failed.state, UpdateState::Failed);
        assert!(failure.unwrap().1.retryable());
        assert_eq!(
            failed.next_retry_at_epoch_seconds,
            failed.checked_at_epoch_seconds.map(|time| time + 30)
        );

        coordinator.check_and_download(false, |_| {}).await;
        assert_eq!(server.requests().len(), 1);
        tokio::time::pause();
        coordinator.wait_for_scheduled_check().await;
        tokio::time::resume();

        let (recovered, _) = coordinator.check_and_download(false, |_| {}).await;
        assert_eq!(recovered.state, UpdateState::UpToDate);
        assert_eq!(recovered.next_retry_at_epoch_seconds, None);
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn short_retry_schedule_is_bounded_and_restarts_after_the_daily_interval() {
        let failure = UpdateCheckFailure::new(
            UpdateCheckStage::ManifestRequest,
            UpdateCheckError::ConnectionReset,
        );
        let mut schedule = CheckSchedule::new();
        let mut now = schedule.next_check;
        let mut epoch = 1000;
        for (attempt, delay) in [30, 120, 600].into_iter().enumerate() {
            schedule.record_result(false, Err(&failure), now, epoch);
            assert_eq!(schedule.retries_scheduled, attempt + 1);
            assert_eq!(schedule.next_check, now + Duration::from_secs(delay));
            assert_eq!(schedule.retry_at_epoch_seconds, Some(epoch + delay));
            now = schedule.next_check;
            epoch += delay;
        }
        schedule.record_result(false, Err(&failure), now, epoch);
        assert_eq!(schedule.next_check, now + CHECK_INTERVAL);
        assert_eq!(schedule.retry_at_epoch_seconds, None);
        assert_eq!(schedule.retries_scheduled, 0);
        let next_day = schedule.next_check;
        schedule.record_result(
            false,
            Err(&failure),
            next_day,
            epoch + CHECK_INTERVAL.as_secs(),
        );
        assert_eq!(schedule.next_check, next_day + Duration::from_secs(30));
    }

    #[tokio::test(start_paused = true)]
    async fn manual_failure_keeps_the_timer_and_manual_success_moves_it_to_the_next_day() {
        let coordinator = UpdateCoordinator::with_endpoint("1.1.5", "http://127.0.0.1", "unused");
        let failure =
            UpdateCheckFailure::new(UpdateCheckStage::ManifestRequest, UpdateCheckError::Timeout);
        assert_eq!(coordinator.begin_check(false).unwrap().0, 1);
        let scheduled = coordinator.finish_check(false, Err(&failure), None);
        let now = Instant::now();
        let wait = coordinator.wait_for_scheduled_check();
        tokio::pin!(wait);
        assert!(futures_util::poll!(&mut wait).is_pending());
        tokio::time::advance(Duration::from_secs(10)).await;

        assert!(coordinator.begin_check(true).is_some());
        let failed = coordinator.finish_check(true, Err(&failure), None);
        assert_eq!(
            failed.next_retry_at_epoch_seconds,
            scheduled.next_retry_at_epoch_seconds
        );
        assert_eq!(
            coordinator.state.lock().unwrap().schedule.next_check,
            now + RETRY_DELAYS[0]
        );
        assert!(futures_util::poll!(&mut wait).is_pending());

        assert!(coordinator.begin_check(true).is_some());
        tokio::time::advance(Duration::from_secs(20)).await;
        assert!(futures_util::poll!(&mut wait).is_pending());
        assert!(coordinator.begin_check(false).is_none());
        coordinator.set_up_to_date();
        let recovered = coordinator.finish_check(true, Ok(()), None);
        assert_eq!(recovered.next_retry_at_epoch_seconds, None);
        assert_eq!(
            coordinator.state.lock().unwrap().schedule.retries_scheduled,
            0
        );
        assert!(futures_util::poll!(&mut wait).is_pending());
        tokio::time::advance(CHECK_INTERVAL).await;
        wait.await;
        assert_eq!(coordinator.begin_check(false).unwrap().0, 1);
    }

    #[test]
    fn wrapped_socket_error_keeps_the_system_code_and_certificate_failure_is_terminal() {
        #[cfg(windows)]
        let reset_code = 10054;
        #[cfg(not(windows))]
        let reset_code = io::Error::from(io::ErrorKind::ConnectionReset).raw_os_error();
        #[cfg(windows)]
        let socket_error = io::Error::from_raw_os_error(reset_code);
        #[cfg(not(windows))]
        let socket_error = io::Error::from(io::ErrorKind::ConnectionReset);
        let wrapped = io::Error::other(socket_error);
        let (kind, code) = transport_cause(UpdateCheckError::Connection, Some(&wrapped));
        assert_eq!(kind, UpdateCheckError::ConnectionReset);
        #[cfg(windows)]
        assert_eq!(code, Some(reset_code));
        #[cfg(not(windows))]
        assert_eq!(code, reset_code);

        let wrapped = io::Error::other(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ));
        let (kind, code) = transport_cause(UpdateCheckError::Connection, Some(&wrapped));
        assert_eq!(kind, UpdateCheckError::Tls);
        assert!(code.is_none());
        let failure = UpdateCheckFailure::new(UpdateCheckStage::ManifestRequest, kind);
        assert!(!failure.retryable());
    }

    #[test]
    fn retry_after_accepts_http_dates_and_delay_seconds() {
        let now = httpdate::parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap();
        assert_eq!(retry_after_delay("90", now), Some(Duration::from_secs(90)));
        assert_eq!(
            retry_after_delay("Sun, 06 Nov 1994 08:51:07 GMT", now),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            retry_after_delay("Sun, 06 Nov 1994 08:49:36 GMT", now),
            None
        );
        for invalid in ["garbage", "-20", "18446744073709551615"] {
            assert_eq!(retry_after_delay(invalid, now), None);
        }
    }

    #[tokio::test]
    async fn scheduled_rate_limit_uses_retry_after_from_the_response() {
        let server = ScriptedHttpServer::start_with_headers(
            vec![(429, String::new())],
            "Retry-After: 180\r\n",
        );
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");
        let (failed, failure) = coordinator.check_and_download(false, |_| {}).await;
        let (attempt, failure) = failure.unwrap();
        assert_eq!(attempt, 1);
        assert_eq!(failure.http_status, Some(429));
        assert_eq!(
            failed.next_retry_at_epoch_seconds,
            failed.checked_at_epoch_seconds.map(|time| time + 180)
        );
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn an_interrupted_manifest_body_is_eligible_for_retry() {
        let server = ScriptedHttpServer::start(vec![(1, "partial".to_owned())]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");
        let (failed, failure) = coordinator.check_and_download(false, |_| {}).await;
        let (_, failure) = failure.unwrap();
        assert_eq!(failure.stage, UpdateCheckStage::ManifestBody);
        assert!(failure.retryable());
        assert!(failed.next_retry_at_epoch_seconds.is_some());
    }

    #[tokio::test]
    async fn a_scheduled_failure_preserves_an_already_verified_pending_update() {
        let server = ScriptedHttpServer::start(vec![(503, String::new())]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");
        coordinator.mark_ready(
            Manifest {
                version: "1.1.6".to_owned(),
                notes: Some("notes".to_owned()),
                pub_date: None,
                platforms: std::collections::HashMap::new(),
            },
            vec![1, 2, 3],
        );
        let (snapshot, failure) = coordinator.check_and_download(false, |_| {}).await;
        assert_eq!(snapshot.state, UpdateState::Pending);
        assert_eq!(snapshot.available_version.as_deref(), Some("1.1.6"));
        assert!(snapshot.next_retry_at_epoch_seconds.is_some());
        assert_eq!(failure.unwrap().1.http_status, Some(503));
        assert!(coordinator.begin_install().is_ok());
    }

    #[tokio::test]
    async fn permanent_http_failure_is_exposed_without_a_short_retry() {
        let server = ScriptedHttpServer::start(vec![(418, String::new())]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");

        let (snapshot, _) = coordinator.check_and_download(false, |_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(snapshot.next_retry_at_epoch_seconds, None);
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::CheckFailed)
        );
        assert_eq!(
            server.requests(),
            vec![HttpRequest {
                method: "GET".to_owned(),
                path: "/latest.md".to_owned(),
                authorization: None,
            }]
        );
    }

    #[tokio::test]
    async fn throttled_manifest_check_is_not_retried_automatically() {
        let server = ScriptedHttpServer::start(vec![(429, String::new())]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");

        let (snapshot, _) = coordinator.check_and_download(true, |_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::CheckFailed)
        );
    }

    #[tokio::test]
    async fn non_json_manifest_is_invalid_without_a_short_retry() {
        let server = ScriptedHttpServer::start(vec![(
            200,
            "<html>temporary WAF response</html>".to_owned(),
        )]);
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), "unused");

        let (snapshot, _) = coordinator.check_and_download(false, |_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::ManifestInvalid)
        );
        assert_eq!(snapshot.next_retry_at_epoch_seconds, None);
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

        let (snapshot, _) = coordinator.check_and_download(false, |_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(snapshot.next_retry_at_epoch_seconds, None);
        assert_eq!(snapshot.available_version.as_deref(), Some("1.1.6"));
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::DownloadFailed)
        );
    }

    #[tokio::test]
    async fn invalid_download_signature_is_rejected_before_an_update_becomes_ready() {
        let server = ScriptedHttpServer::start_with(|base_url| {
            let manifest = serde_json::json!({
                "version": "1.1.6",
                "platforms": {
                    "windows-x86_64": {
                        "url": format!("{base_url}/setup.exe"),
                        "signature": "invalid signature"
                    }
                }
            })
            .to_string();
            vec![(200, manifest), (200, "installer bytes".to_owned())]
        });
        let coordinator =
            UpdateCoordinator::with_endpoint("1.1.5", server.url("/latest.md"), TEST_PUBLIC_KEY);

        let (snapshot, _) = coordinator.check_and_download(false, |_| {}).await;

        assert_eq!(snapshot.state, UpdateState::Failed);
        assert_eq!(snapshot.next_retry_at_epoch_seconds, None);
        assert_eq!(
            snapshot.failure_category,
            Some(UpdateFailureCategory::SignatureInvalid)
        );
        assert_eq!(
            server.requests(),
            vec![
                HttpRequest {
                    method: "GET".to_owned(),
                    path: "/latest.md".to_owned(),
                    authorization: None,
                },
                HttpRequest {
                    method: "GET".to_owned(),
                    path: "/setup.exe".to_owned(),
                    authorization: None,
                },
            ]
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
        assert!(coordinator.begin_check(true).is_some());
        assert!(coordinator.begin_check(true).is_none());
        coordinator.set_up_to_date();
        coordinator.finish_check(true, Ok(()), None);
        assert!(coordinator.begin_check(true).is_some());
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
    fn update_snapshots_offer_gitee_releases_for_manual_downloads() {
        let coordinator = UpdateCoordinator::with_endpoint("1.0.0", "http://127.0.0.1", "key");

        assert_eq!(
            coordinator.snapshot().manual_download_url,
            "https://gitee.com/ericshaohua/gpteasy-releases/releases"
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
        assert!(restarted.begin_check(true).is_some());
    }

    #[test]
    fn automatic_check_preserves_incomplete_state_until_the_user_retries() {
        let root = TempDir::new().expect("update state root");
        let path = root.path().join("update-install-attempt.json");
        fs::write(&path, br#"{"target_version":"1.1.0"}"#).expect("install attempt");
        let coordinator = UpdateCoordinator::with_state_path("1.0.0", &path);

        assert!(coordinator.begin_check(false).is_none());
        assert_eq!(coordinator.snapshot().state, UpdateState::Incomplete);
        assert!(coordinator.begin_check(true).is_some());
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
