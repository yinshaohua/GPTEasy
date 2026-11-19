//! Only the official managed-daemon control surface is used. Never enumerate/kill consumers.
use crate::diagnostics::{IssueLogLevel, IssueLogStore};
use crate::session::CodexCliSource;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogRefreshStatus {
    Refreshed,
    NotRunning,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedDaemonStatus {
    Managed,
    Unavailable,
    Unsafe,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonIdentity {
    pub pid: Option<u32>,
    pub version: Option<String>,
    pub cli_version: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogRefreshResult {
    pub operation_id: String,
    pub status: ModelCatalogRefreshStatus,
    pub daemon: ManagedDaemonStatus,
    pub before: Option<DaemonIdentity>,
    pub after: Option<DaemonIdentity>,
    pub message_id: String,
    pub restart_state: DaemonRestartState,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonRestartState {
    Unknown,
    #[default]
    NotStarted,
    Started,
    Completed,
}
impl DaemonRestartState {
    fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::NotStarted => "not_started",
            Self::Started => "started",
            Self::Completed => "completed",
        }
    }
}
#[derive(Default)]
struct RefreshEvidence {
    cli_source: Option<CodexCliSource>,
    restart_state: DaemonRestartState,
}
#[derive(Debug, Clone, Copy)]
pub enum RefreshSource {
    ProviderSwitch,
    ManualButton,
    DesktopRestart,
}
impl RefreshSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderSwitch => "provider_switch",
            Self::ManualButton => "manual_button",
            Self::DesktopRestart => "desktop_restart",
        }
    }
}
#[derive(Debug, Clone, Copy)]
enum ProbeError {
    Unavailable,
    Unsafe,
    Protocol,
    Command,
    CommandExit {
        code: Option<i32>,
        stderr: StderrClass,
    },
    Timeout,
}
impl ProbeError {
    fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "model_catalog_refresh.unavailable",
            Self::Unsafe => "model_catalog_refresh.daemon_unsafe",
            Self::Protocol => "model_catalog_refresh.protocol_incompatible",
            Self::CommandExit {
                stderr: StderrClass::ConnectFailed,
                ..
            } => "model_catalog_refresh.connection_failed",
            Self::Command | Self::CommandExit { .. } => "model_catalog_refresh.command_failed",
            Self::Timeout => "model_catalog_refresh.timeout",
        }
    }
    fn daemon(self) -> ManagedDaemonStatus {
        match self {
            Self::Unavailable => ManagedDaemonStatus::Unavailable,
            Self::Unsafe => ManagedDaemonStatus::Unsafe,
            _ => ManagedDaemonStatus::Unknown,
        }
    }
}
#[derive(Debug, Clone, Copy)]
enum RefreshStage {
    Inspect,
    RestartCheck,
    Restart,
    ReadyCheck,
}
impl RefreshStage {
    fn failure(self, cause: ProbeError) -> RefreshFailure {
        RefreshFailure { stage: self, cause }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::RestartCheck => "restart_check",
            Self::Restart => "restart",
            Self::ReadyCheck => "ready_check",
        }
    }
}
#[derive(Debug, Clone, Copy)]
struct RefreshFailure {
    stage: RefreshStage,
    cause: ProbeError,
}
impl RefreshFailure {
    fn message(self) -> &'static str {
        match (self.stage, self.cause) {
            (RefreshStage::Inspect | RefreshStage::RestartCheck, ProbeError::Timeout) => {
                "model_catalog_refresh.inspect_timeout"
            }
            (RefreshStage::Restart, ProbeError::Timeout) => "model_catalog_refresh.restart_timeout",
            (
                RefreshStage::Inspect | RefreshStage::RestartCheck | RefreshStage::ReadyCheck,
                ProbeError::Command
                | ProbeError::CommandExit {
                    stderr: StderrClass::Other | StderrClass::Unavailable,
                    ..
                },
            ) => "model_catalog_refresh.inspect_failed",
            (_, cause) => cause.message(),
        }
    }
}
trait DaemonControl: Send + Sync {
    fn inspect(&self, evidence: &mut RefreshEvidence)
    -> Result<Option<DaemonIdentity>, ProbeError>;
    fn restart(&self, evidence: &mut RefreshEvidence) -> Result<(), RefreshFailure>;
}
#[derive(Clone)]
pub struct ModelCatalogRefresher {
    lock: Arc<Mutex<()>>,
    control: Arc<dyn DaemonControl>,
    ready_timeout: Duration,
}
impl ModelCatalogRefresher {
    pub fn new(home: impl AsRef<Path>) -> Self {
        Self {
            lock: Arc::new(Mutex::new(())),
            control: Arc::new(OfficialDaemon {
                home: home.as_ref().to_owned(),
            }),
            ready_timeout: Duration::from_secs(5),
        }
    }
    pub fn refresh(
        &self,
        source: RefreshSource,
        logs: &IssueLogStore,
    ) -> ModelCatalogRefreshResult {
        let mut result = ModelCatalogRefreshResult {
            operation_id: Uuid::new_v4().to_string(),
            status: ModelCatalogRefreshStatus::Failed,
            daemon: ManagedDaemonStatus::Unknown,
            before: None,
            after: None,
            message_id: "model_catalog_refresh.failed".into(),
            restart_state: DaemonRestartState::NotStarted,
        };
        let mut evidence = RefreshEvidence::default();
        log(logs, source, "start", &result, &evidence, None);
        let lock_deadline = Instant::now() + Duration::from_secs(30);
        let guard = loop {
            match self.lock.try_lock() {
                Ok(guard) => break Some(guard),
                Err(std::sync::TryLockError::Poisoned(_)) => break None,
                Err(_) if Instant::now() >= lock_deadline => break None,
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        };
        if guard.is_none() {
            result.message_id = "model_catalog_refresh.busy".into();
            log(logs, source, "failed", &result, &evidence, None);
            return result;
        }
        log(logs, source, "inspect", &result, &evidence, None);
        let failure = self.execute(&mut result, source, logs, &mut evidence).err();
        result.restart_state = evidence.restart_state;
        if let Some(failure) = failure {
            result.message_id = failure.message().into();
            result.daemon = failure.cause.daemon();
            if matches!(failure.cause, ProbeError::Unavailable) && result.before.is_none() {
                result.status = ModelCatalogRefreshStatus::NotRunning;
            }
        }
        let phase = match result.status {
            ModelCatalogRefreshStatus::Refreshed => "ready",
            ModelCatalogRefreshStatus::NotRunning => "not_running",
            ModelCatalogRefreshStatus::Failed => "failed",
        };
        log(logs, source, phase, &result, &evidence, failure);
        result
    }
    fn execute(
        &self,
        result: &mut ModelCatalogRefreshResult,
        source: RefreshSource,
        logs: &IssueLogStore,
        evidence: &mut RefreshEvidence,
    ) -> Result<(), RefreshFailure> {
        result.before = self
            .control
            .inspect(evidence)
            .map_err(|cause| RefreshStage::Inspect.failure(cause))?;
        result.daemon = ManagedDaemonStatus::Managed;
        if result.before.is_none() {
            result.status = ModelCatalogRefreshStatus::NotRunning;
            result.message_id = "model_catalog_refresh.not_running".into();
            return Ok(());
        }
        log(logs, source, "restart_check", result, evidence, None);
        self.control.restart(evidence)?;
        evidence.restart_state = DaemonRestartState::Completed;
        log(logs, source, "restart_completed", result, evidence, None);
        let deadline = Instant::now() + self.ready_timeout;
        loop {
            if let Some(identity) = self
                .control
                .inspect(evidence)
                .map_err(|cause| RefreshStage::ReadyCheck.failure(cause))?
            {
                result.after = Some(identity);
                result.status = ModelCatalogRefreshStatus::Refreshed;
                result.message_id = "model_catalog_refresh.refreshed".into();
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(RefreshStage::ReadyCheck.failure(ProbeError::Timeout));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
fn log(
    logs: &IssueLogStore,
    source: RefreshSource,
    phase: &str,
    result: &ModelCatalogRefreshResult,
    evidence: &RefreshEvidence,
    failure: Option<RefreshFailure>,
) {
    let failure_details = failure.map_or_else(String::new, |f| {
        let stage = f.stage.name();
        let reason = f.message();
        let command_details = match f.cause {
            ProbeError::CommandExit { code, stderr } => {
                let exit_code = code.map_or_else(|| "unknown".into(), |code| code.to_string());
                format!(" exit_code={exit_code} stderr_category={}", stderr.name())
            }
            _ => String::new(),
        };
        format!(" failure_stage={stage} failure_reason={reason}{command_details}")
    });
    let cli_source = evidence.cli_source.map_or("unknown", CodexCliSource::name);
    let restart_state = evidence.restart_state.name();
    logs.append(if phase == "failed" { IssueLogLevel::Warn } else { IssueLogLevel::Info }, format!("model_catalog_refresh.{phase}"), &result.message_id,
        Some(format!("operation_id={} source={} phase={phase} status={:?} daemon={:?} before_version={} after_version={} cli_version={} running_tasks=unknown independent_operation={} cli_source={cli_source} restart_state={restart_state}{failure_details}",
            result.operation_id, source.as_str(), result.status, result.daemon,
            result.before.as_ref().and_then(|i| i.version.as_deref()).unwrap_or("unknown"),
            result.after.as_ref().and_then(|i| i.version.as_deref()).unwrap_or("unknown"),
            result.before.as_ref().and_then(|i| i.cli_version.as_deref()).unwrap_or("unknown"),
            if matches!(source, RefreshSource::ManualButton) { "none" } else { "not_blocked" })));
}
struct OfficialDaemon {
    home: PathBuf,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionResponse {
    status: String,
    backend: Option<String>,
    managed_codex_path: Option<PathBuf>,
    socket_path: Option<PathBuf>,
    app_server_version: Option<String>,
    cli_version: Option<String>,
}
impl DaemonControl for OfficialDaemon {
    fn inspect(
        &self,
        evidence: &mut RefreshEvidence,
    ) -> Result<Option<DaemonIdentity>, ProbeError> {
        self.check_home()?;
        self.inspect_control(|| self.run(DaemonCommand::Version, evidence))
    }
    fn restart(&self, evidence: &mut RefreshEvidence) -> Result<(), RefreshFailure> {
        // Recheck immediately before the official restart (which otherwise starts a stopped daemon).
        if self
            .inspect(evidence)
            .map_err(|cause| RefreshStage::RestartCheck.failure(cause))?
            .is_none()
        {
            return Err(RefreshStage::RestartCheck.failure(ProbeError::Command));
        }
        self.run(DaemonCommand::Restart, evidence)
            .map(|_| ())
            .map_err(|cause| RefreshStage::Restart.failure(cause))
    }
}
#[derive(Clone, Copy)]
enum DaemonCommand {
    Version,
    Restart,
}
impl DaemonCommand {
    fn name(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Restart => "restart",
        }
    }
    fn run(self, command: Command, on_spawn: impl FnOnce()) -> Result<Vec<u8>, ProbeError> {
        match self {
            Self::Version => run_bounded(command, Duration::from_secs(3), true, on_spawn),
            // Codex 0.170 on Windows defaults to a 60s graceful shutdown, with up to
            // 10s each for confirming forced exit and starting the new daemon.
            Self::Restart => run_bounded(command, Duration::from_secs(90), false, on_spawn),
        }
    }
}
impl OfficialDaemon {
    fn inspect_control(
        &self,
        version: impl FnOnce() -> Result<Vec<u8>, ProbeError>,
    ) -> Result<Option<DaemonIdentity>, ProbeError> {
        // A missing socket is a no-op. A present but unreachable socket fails closed.
        if !control_socket_present(&self.home)? {
            return Ok(None);
        }
        verify_control_owner(&self.home.join("app-server-control"))?;
        verify_control_owner(&self.home.join("packages/app-server-daemon"))?;
        parse_version(&version()?, &self.home)
    }
    fn check_home(&self) -> Result<(), ProbeError> {
        if let Some(inherited) = std::env::var_os("CODEX_HOME") {
            if !same_path(Path::new(&inherited), &self.home) {
                return Err(ProbeError::Unsafe);
            }
        }
        Ok(())
    }
    fn run(
        &self,
        action: DaemonCommand,
        evidence: &mut RefreshEvidence,
    ) -> Result<Vec<u8>, ProbeError> {
        let (mut command, cli_source) =
            crate::session::managed_daemon_cli_command().ok_or(ProbeError::Unavailable)?;
        evidence.cli_source = Some(cli_source);
        command
            .args(["app-server", "daemon", action.name()])
            .env("CODEX_HOME", &self.home)
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        action.run(command, || {
            if matches!(action, DaemonCommand::Restart) {
                evidence.restart_state = DaemonRestartState::Started;
            }
        })
    }
}
const STDERR_PREFIX_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StderrClass {
    ConnectFailed,
    Other,
    Unavailable,
}
impl StderrClass {
    fn name(self) -> &'static str {
        match self {
            Self::ConnectFailed => "connect_failed",
            Self::Other => "other",
            Self::Unavailable => "unavailable",
        }
    }
}
fn classify_stderr(stderr: &[u8]) -> StderrClass {
    let text = String::from_utf8_lossy(stderr);
    if text.lines().any(|line| {
        let line = line.trim_start();
        let line = line.strip_prefix("Error: ").unwrap_or(line);
        line.starts_with("failed to connect to ")
    }) {
        StderrClass::ConnectFailed
    } else {
        StderrClass::Other
    }
}
fn read_stderr(mut stderr: impl Read, send: std::sync::mpsc::SyncSender<StderrClass>) {
    let mut prefix = Vec::new();
    let kind = match stderr
        .by_ref()
        .take(STDERR_PREFIX_BYTES)
        .read_to_end(&mut prefix)
    {
        Ok(_) => classify_stderr(&prefix),
        Err(_) => StderrClass::Unavailable,
    };
    let _ = send.send(kind);
    // Keep draining so noisy commands can still exit. Raw bytes never leave the reader.
    let _ = std::io::copy(&mut stderr, &mut std::io::sink());
}
fn run_bounded(
    mut command: Command,
    timeout: Duration,
    capture_stdout: bool,
    on_spawn: impl FnOnce(),
) -> Result<Vec<u8>, ProbeError> {
    command.stdout(if capture_stdout {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|_| ProbeError::Command)?;
    on_spawn();
    let stderr_receiver = child.stderr.take().map(|stderr| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || read_stderr(stderr, sender));
        receiver
    });
    let receiver = child.stdout.take().map(|stdout| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut data = Vec::new();
            let output = stdout.take(65537).read_to_end(&mut data).map(|_| data);
            let _ = sender.send(output);
        });
        receiver
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(if result.is_ok() {
                    ProbeError::Timeout
                } else {
                    ProbeError::Command
                });
            }
        }
    };
    let status = status?;
    if !status.success() {
        let stderr = stderr_receiver
            .and_then(|receiver| {
                receiver
                    .recv_timeout(
                        deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(100)),
                    )
                    .ok()
            })
            .unwrap_or(StderrClass::Unavailable);
        return Err(ProbeError::CommandExit {
            code: status.code(),
            stderr,
        });
    }
    let Some(receiver) = receiver else {
        return Ok(Vec::new());
    };
    let data = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| ProbeError::Timeout)?
        .map_err(|_| ProbeError::Command)?;
    if data.len() > 65536 {
        return Err(ProbeError::Protocol);
    }
    Ok(data)
}
#[cfg(windows)]
fn verify_control_owner(path: &Path) -> Result<(), ProbeError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{GetLengthSid, OWNER_SECURITY_INFORMATION};
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut owner = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    // Only the owner SID is read; no socket contents, ACL strings or user identity are logged.
    let error = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(ProbeError::Unsafe);
    }
    let matches = if owner.is_null() {
        false
    } else {
        let length = unsafe { GetLengthSid(owner) } as usize;
        let sid = unsafe { std::slice::from_raw_parts(owner.cast::<u8>(), length) };
        crate::single_instance::current_user_sid().is_ok_and(|current| current == sid)
    };
    unsafe {
        LocalFree(descriptor);
    }
    if matches {
        Ok(())
    } else {
        Err(ProbeError::Unsafe)
    }
}
#[cfg(not(windows))]
fn verify_control_owner(_path: &Path) -> Result<(), ProbeError> {
    // This iteration is Windows-only; do not claim ownership verification on other hosts.
    Err(ProbeError::Unsafe)
}
fn control_socket_present(home: &Path) -> Result<bool, ProbeError> {
    match std::fs::symlink_metadata(home.join("app-server-control/app-server-control.sock")) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ProbeError::Unsafe),
    }
}
fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.is_absolute() && a == b,
    }
}
fn parse_version(data: &[u8], home: &Path) -> Result<Option<DaemonIdentity>, ProbeError> {
    let value: VersionResponse = serde_json::from_slice(data).map_err(|_| ProbeError::Protocol)?;
    match value.status.as_str() {
        "stopped" | "not_running" => return Ok(None),
        "running" => (),
        _ => return Err(ProbeError::Protocol),
    }
    let backend = value.backend.as_deref().ok_or(ProbeError::Protocol)?;
    if !matches!(backend, "pid" | "launchd" | "systemd") {
        return Err(ProbeError::Unsafe);
    }
    let executable = value.managed_codex_path.ok_or(ProbeError::Protocol)?;
    let socket = value.socket_path.ok_or(ProbeError::Protocol)?;
    let owned_package = home
        .join("packages/app-server-daemon")
        .canonicalize()
        .map_err(|_| ProbeError::Unsafe)?;
    let executable = executable.canonicalize().map_err(|_| ProbeError::Unsafe)?;
    if !executable.starts_with(owned_package)
        || !same_path(
            &socket,
            &home.join("app-server-control/app-server-control.sock"),
        )
    {
        return Err(ProbeError::Unsafe);
    }
    let version = value
        .app_server_version
        .filter(|v| safe_version(v))
        .ok_or(ProbeError::Protocol)?;
    if !value.cli_version.as_deref().is_some_and(safe_version) {
        return Err(ProbeError::Protocol);
    }
    // Official version JSON does not expose a PID; never guess by process name or socket contents.
    Ok(Some(DaemonIdentity {
        pid: None,
        version: Some(version),
        cli_version: value.cli_version,
    }))
}
fn safe_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 32
        && v.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Fake {
        running: bool,
        fail: bool,
        restarts: AtomicUsize,
        active: AtomicUsize,
        max: AtomicUsize,
    }
    impl DaemonControl for Fake {
        fn inspect(
            &self,
            evidence: &mut RefreshEvidence,
        ) -> Result<Option<DaemonIdentity>, ProbeError> {
            evidence.cli_source = Some(CodexCliSource::ProcessPath);
            Ok(self.running.then(|| DaemonIdentity {
                pid: Some(42),
                version: Some("0.160.0".into()),
                cli_version: Some("0.160.1".into()),
            }))
        }
        fn restart(&self, evidence: &mut RefreshEvidence) -> Result<(), RefreshFailure> {
            evidence.restart_state = DaemonRestartState::Started;
            self.restarts.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max.fetch_max(active, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            self.active.fetch_sub(1, Ordering::SeqCst);
            if self.fail {
                Err(RefreshStage::Restart.failure(ProbeError::CommandExit {
                    code: Some(23),
                    stderr: StderrClass::Other,
                }))
            } else {
                Ok(())
            }
        }
    }
    fn fake(running: bool, fail: bool) -> (ModelCatalogRefresher, Arc<Fake>) {
        let control = Arc::new(Fake {
            running,
            fail,
            restarts: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            max: AtomicUsize::new(0),
        });
        (
            ModelCatalogRefresher {
                lock: Arc::new(Mutex::new(())),
                control: control.clone(),
                ready_timeout: Duration::from_millis(1),
            },
            control,
        )
    }
    #[test]
    fn stopped_never_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let logs = IssueLogStore::new(dir.path());
        let (service, control) = fake(false, false);
        assert_eq!(
            service.refresh(RefreshSource::ManualButton, &logs).status,
            ModelCatalogRefreshStatus::NotRunning
        );
        assert_eq!(control.restarts.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn failure_evidence_is_redacted_and_does_not_claim_continued_execution() {
        let dir = tempfile::tempdir().unwrap();
        let logs = IssueLogStore::new(dir.path());
        let (service, _) = fake(true, true);
        let result = service.refresh(RefreshSource::DesktopRestart, &logs);
        assert_eq!(result.status, ModelCatalogRefreshStatus::Failed);
        let records = logs.list_all(0, None, None);
        assert!(records.iter().any(|r| r.event.ends_with("restart_check")));
        assert!(records.iter().any(|r| r.event.ends_with("failed")));
        let encoded = serde_json::to_string(&records).unwrap();
        assert!(encoded.contains("not_blocked"));
        assert!(encoded.contains(
            "failure_stage=restart failure_reason=model_catalog_refresh.command_failed exit_code=23"
        ));
        assert!(encoded.contains("cli_source=process_path restart_state=started"));
        assert!(encoded.contains("stderr_category=other"));
        assert!(!encoded.contains("restart_completed"));
        assert!(!encoded.contains("continued=true"));
        assert!(!encoded.contains("socket"));
    }
    #[test]
    fn shared_lock_serializes_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let logs = Arc::new(IssueLogStore::new(dir.path()));
        let (service, control) = fake(true, false);
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let service = service.clone();
                let logs = logs.clone();
                std::thread::spawn(move || service.refresh(RefreshSource::ManualButton, &logs))
            })
            .collect();
        for thread in threads {
            assert_eq!(
                thread.join().unwrap().status,
                ModelCatalogRefreshStatus::Refreshed
            );
        }
        assert_eq!(control.max.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn unknown_status_and_unmanaged_running_daemon_fail_closed() {
        assert!(matches!(
            parse_version(br#"{"status":"future"}"#, Path::new(".")),
            Err(ProbeError::Protocol)
        ));
        assert!(matches!(
            parse_version(br#"{"status":"running"}"#, Path::new(".")),
            Err(ProbeError::Protocol)
        ));
        assert!(
            parse_version(br#"{"status":"stopped"}"#, Path::new("."))
                .unwrap()
                .is_none()
        );
        assert!(!safe_version("secret/socket-path"));
    }
    #[test]
    fn missing_control_socket_is_not_running_without_launching_a_command() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!control_socket_present(dir.path()).unwrap());
        let socket = dir
            .path()
            .join("app-server-control/app-server-control.sock");
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        std::fs::write(socket, b"not a status response").unwrap();
        assert!(control_socket_present(dir.path()).unwrap());
    }
    #[cfg(windows)]
    #[test]
    fn control_directory_owner_is_verified_without_reading_socket_contents() {
        let dir = tempfile::tempdir().unwrap();
        assert!(verify_control_owner(dir.path()).is_ok());
        assert!(matches!(
            verify_control_owner(&dir.path().join("missing")),
            Err(ProbeError::Unsafe)
        ));
    }
    struct ControlDisappears;
    impl DaemonControl for ControlDisappears {
        fn inspect(&self, _: &mut RefreshEvidence) -> Result<Option<DaemonIdentity>, ProbeError> {
            Ok(Some(DaemonIdentity {
                pid: None,
                version: Some("0.160.0".into()),
                cli_version: Some("0.160.1".into()),
            }))
        }
        fn restart(&self, _: &mut RefreshEvidence) -> Result<(), RefreshFailure> {
            Err(RefreshStage::RestartCheck.failure(ProbeError::Unavailable))
        }
    }
    #[test]
    fn control_disappearing_after_running_probe_is_failure_not_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let logs = IssueLogStore::new(dir.path());
        let service = ModelCatalogRefresher {
            lock: Arc::new(Mutex::new(())),
            control: Arc::new(ControlDisappears),
            ready_timeout: Duration::ZERO,
        };
        assert_eq!(
            service.refresh(RefreshSource::ManualButton, &logs).status,
            ModelCatalogRefreshStatus::Failed
        );
    }
    struct StaysStopped {
        inspected: AtomicUsize,
        after_restart_error: Option<ProbeError>,
    }
    impl DaemonControl for StaysStopped {
        fn inspect(&self, _: &mut RefreshEvidence) -> Result<Option<DaemonIdentity>, ProbeError> {
            if self.inspected.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(Some(DaemonIdentity {
                    pid: None,
                    version: Some("0.160.0".into()),
                    cli_version: Some("0.160.1".into()),
                }));
            }
            self.after_restart_error.map_or(Ok(None), Err)
        }
        fn restart(&self, _: &mut RefreshEvidence) -> Result<(), RefreshFailure> {
            Ok(())
        }
    }
    #[test]
    fn ready_timeout_is_a_refresh_failure() {
        let dir = tempfile::tempdir().unwrap();
        let logs = IssueLogStore::new(dir.path());
        let service = ModelCatalogRefresher {
            lock: Arc::new(Mutex::new(())),
            control: Arc::new(StaysStopped {
                inspected: AtomicUsize::new(0),
                after_restart_error: None,
            }),
            ready_timeout: Duration::ZERO,
        };
        let result = service.refresh(RefreshSource::DesktopRestart, &logs);
        assert_eq!(result.status, ModelCatalogRefreshStatus::Failed);
        assert_eq!(result.message_id, "model_catalog_refresh.timeout");
        assert!(result.before.is_some());
        assert!(result.after.is_none());
        assert_eq!(result.restart_state, DaemonRestartState::Completed);
        let records = logs.list_all(0, None, None);
        assert!(
            records
                .iter()
                .any(|r| r.event.ends_with("restart_completed"))
        );
        assert!(records.iter().any(|r| {
            r.details.as_deref().is_some_and(|details| {
                details.contains(
                    "failure_stage=ready_check failure_reason=model_catalog_refresh.timeout",
                )
            })
        }));
    }
    #[test]
    fn refused_connection_after_restart_retains_completed_command_and_failure_stage() {
        let dir = tempfile::tempdir().unwrap();
        let logs = IssueLogStore::new(dir.path());
        let service = ModelCatalogRefresher {
            lock: Arc::new(Mutex::new(())),
            control: Arc::new(StaysStopped {
                inspected: AtomicUsize::new(0),
                after_restart_error: Some(ProbeError::CommandExit {
                    code: Some(1),
                    stderr: StderrClass::ConnectFailed,
                }),
            }),
            ready_timeout: Duration::ZERO,
        };
        let result = service.refresh(RefreshSource::ManualButton, &logs);
        assert_eq!(result.status, ModelCatalogRefreshStatus::Failed);
        assert_eq!(result.restart_state, DaemonRestartState::Completed);
        assert_eq!(result.message_id, "model_catalog_refresh.connection_failed");
        assert!(result.after.is_none());
        let records = logs.list_all(0, None, None);
        let failure = records.last().unwrap().details.as_deref().unwrap();
        assert!(failure.contains("restart_state=completed failure_stage=ready_check"));
        assert!(failure.contains("exit_code=1 stderr_category=connect_failed"));
    }
    #[test]
    fn probe_timeouts_distinguish_inspection_from_restart_and_readiness() {
        for (stage, expected) in [
            (
                RefreshStage::Inspect,
                "model_catalog_refresh.inspect_timeout",
            ),
            (
                RefreshStage::RestartCheck,
                "model_catalog_refresh.inspect_timeout",
            ),
            (
                RefreshStage::Restart,
                "model_catalog_refresh.restart_timeout",
            ),
            (RefreshStage::ReadyCheck, "model_catalog_refresh.timeout"),
        ] {
            assert_eq!(stage.failure(ProbeError::Timeout).message(), expected);
        }
    }
    #[test]
    fn official_version_contract_requires_owned_paths_and_safe_versions() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let executable = home.join("packages/app-server-daemon/current/bin/codex.exe");
        let socket = home.join("app-server-control/app-server-control.sock");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"fixture").unwrap();
        std::fs::write(&socket, b"fixture").unwrap();
        let mut response = serde_json::json!({"status":"running", "backend":"pid", "managedCodexPath":executable, "socketPath":socket, "appServerVersion":"0.160.0", "cliVersion":"0.160.1"});
        let identity = parse_version(&serde_json::to_vec(&response).unwrap(), home)
            .unwrap()
            .unwrap();
        assert_eq!(identity.pid, None);
        assert_eq!(identity.version.as_deref(), Some("0.160.0"));
        response["socketPath"] = serde_json::json!(home.join("other.sock"));
        assert!(matches!(
            parse_version(&serde_json::to_vec(&response).unwrap(), home),
            Err(ProbeError::Unsafe)
        ));
    }
    #[test]
    fn stderr_classification_uses_only_the_bounded_prefix() {
        let mut output = vec![b'x'; STDERR_PREFIX_BYTES as usize];
        output
            .extend_from_slice(b"\nError: failed to connect to private-socket (os error 10061)\n");
        let mut cursor = std::io::Cursor::new(&output);
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        read_stderr(&mut cursor, send);
        assert_eq!(receive.recv().unwrap(), StderrClass::Other);
        assert_eq!(cursor.position() as usize, output.len());
    }
    #[cfg(windows)]
    fn test_control_command(script: &str) -> Command {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("powershell.exe");
        command
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(0x0800_0000)
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        command
    }
    #[cfg(windows)]
    #[test]
    fn restart_control_command_waits_past_old_ten_second_deadline() {
        let command = test_control_command("Start-Sleep -Milliseconds 10500");
        assert!(DaemonCommand::Restart.run(command, || {}).is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn nonzero_control_exit_is_a_command_failure_not_a_protocol_mismatch() {
        let command = test_control_command("exit 1");
        let error = DaemonCommand::Version.run(command, || {}).unwrap_err();
        assert_eq!(error.message(), "model_catalog_refresh.command_failed");
        assert!(matches!(
            error,
            ProbeError::CommandExit {
                code: Some(1),
                stderr: StderrClass::Other
            }
        ));
        assert_eq!(
            RefreshStage::Inspect.failure(error).message(),
            "model_catalog_refresh.inspect_failed"
        );
    }
    #[cfg(windows)]
    #[test]
    fn stale_control_socket_and_connection_refusal_do_not_restart() {
        struct StaleControl(OfficialDaemon);
        impl DaemonControl for StaleControl {
            fn inspect(
                &self,
                evidence: &mut RefreshEvidence,
            ) -> Result<Option<DaemonIdentity>, ProbeError> {
                self.0.inspect_control(|| {
                    evidence.cli_source = Some(CodexCliSource::Npm);
                    let command = test_control_command(
                        "[Console]::Error.WriteLine('Error: failed to connect to private-socket (os error 10061)'); exit 1",
                    );
                    DaemonCommand::Version.run(command, || {})
                })
            }
            fn restart(&self, _: &mut RefreshEvidence) -> Result<(), RefreshFailure> {
                panic!("a failed probe must not dispatch restart")
            }
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("app-server-control")).unwrap();
        std::fs::create_dir_all(dir.path().join("packages/app-server-daemon")).unwrap();
        std::fs::write(
            dir.path()
                .join("app-server-control/app-server-control.sock"),
            b"",
        )
        .unwrap();
        let logs = IssueLogStore::new(dir.path());
        let service = ModelCatalogRefresher {
            lock: Arc::new(Mutex::new(())),
            control: Arc::new(StaleControl(OfficialDaemon {
                home: dir.path().to_owned(),
            })),
            ready_timeout: Duration::ZERO,
        };
        let result = service.refresh(RefreshSource::ManualButton, &logs);
        assert_eq!(result.status, ModelCatalogRefreshStatus::Failed);
        assert_eq!(result.restart_state, DaemonRestartState::NotStarted);
        assert_eq!(result.message_id, "model_catalog_refresh.connection_failed");
        let records = logs.list_all(0, None, None);
        let failure = records.last().unwrap().details.as_deref().unwrap();
        assert!(failure.contains("cli_source=npm restart_state=not_started failure_stage=inspect"));
        assert!(failure.contains("exit_code=1 stderr_category=connect_failed"));
        assert!(!failure.contains("private-socket"));
    }
    #[cfg(windows)]
    #[test]
    fn successful_command_with_invalid_stdout_is_a_protocol_error() {
        let command = test_control_command("Write-Output broken-json");
        let output = DaemonCommand::Version.run(command, || {}).unwrap();
        let error = parse_version(&output, Path::new(".")).unwrap_err();
        assert_eq!(
            error.message(),
            "model_catalog_refresh.protocol_incompatible"
        );
    }
    #[cfg(windows)]
    #[test]
    fn command_timeout_is_bounded_and_does_not_create_visible_console() {
        let command = test_control_command("Start-Sleep -Seconds 5");
        let started = Instant::now();
        assert!(matches!(
            run_bounded(command, Duration::from_millis(100), false, || {}),
            Err(ProbeError::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
