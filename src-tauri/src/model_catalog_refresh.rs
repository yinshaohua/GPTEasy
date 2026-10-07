//! Only the official managed-daemon control surface is used. Never enumerate/kill consumers.
use crate::diagnostics::{IssueLogLevel, IssueLogStore};
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
    Timeout,
}
impl ProbeError {
    fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "model_catalog_refresh.unavailable",
            Self::Unsafe => "model_catalog_refresh.daemon_unsafe",
            Self::Protocol => "model_catalog_refresh.protocol_incompatible",
            Self::Command => "model_catalog_refresh.command_failed",
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
trait DaemonControl: Send + Sync {
    fn inspect(&self) -> Result<Option<DaemonIdentity>, ProbeError>;
    fn restart(&self) -> Result<(), ProbeError>;
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
        };
        log(logs, source, "start", &result);
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
            log(logs, source, "failed", &result);
            return result;
        }
        log(logs, source, "inspect", &result);
        let outcome = self.execute(&mut result, source, logs);
        if let Err(error) = outcome {
            result.message_id = error.message().into();
            result.daemon = error.daemon();
            if matches!(error, ProbeError::Unavailable) && result.before.is_none() {
                result.status = ModelCatalogRefreshStatus::NotRunning;
            }
        }
        let phase = match result.status {
            ModelCatalogRefreshStatus::Refreshed => "ready",
            ModelCatalogRefreshStatus::NotRunning => "not_running",
            ModelCatalogRefreshStatus::Failed => "failed",
        };
        log(logs, source, phase, &result);
        result
    }
    fn execute(
        &self,
        result: &mut ModelCatalogRefreshResult,
        source: RefreshSource,
        logs: &IssueLogStore,
    ) -> Result<(), ProbeError> {
        result.before = self.control.inspect()?;
        result.daemon = ManagedDaemonStatus::Managed;
        if result.before.is_none() {
            result.status = ModelCatalogRefreshStatus::NotRunning;
            result.message_id = "model_catalog_refresh.not_running".into();
            return Ok(());
        }
        log(logs, source, "restart_requested", result);
        self.control.restart()?;
        let deadline = Instant::now() + self.ready_timeout;
        loop {
            if let Some(identity) = self.control.inspect()? {
                result.after = Some(identity);
                result.status = ModelCatalogRefreshStatus::Refreshed;
                result.message_id = "model_catalog_refresh.refreshed".into();
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(ProbeError::Timeout);
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
) {
    logs.append(if phase == "failed" { IssueLogLevel::Warn } else { IssueLogLevel::Info }, format!("model_catalog_refresh.{phase}"), &result.message_id,
        Some(format!("operation_id={} source={} phase={phase} status={:?} daemon={:?} before_version={} after_version={} cli_version={} running_tasks=unknown independent_operation={}",
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
    fn inspect(&self) -> Result<Option<DaemonIdentity>, ProbeError> {
        self.check_home()?;
        // version is not a status API: 0.160 exits nonzero when its control socket is absent.
        // Check absence without reading the socket or starting a daemon; stale sockets fail closed.
        if !control_socket_present(&self.home)? {
            return Ok(None);
        }
        verify_control_owner(&self.home.join("app-server-control"))?;
        verify_control_owner(&self.home.join("packages/app-server-daemon"))?;
        parse_version(&self.run("version", Duration::from_secs(3))?, &self.home)
    }
    fn restart(&self) -> Result<(), ProbeError> {
        // Recheck immediately before the official restart (which otherwise starts a stopped daemon).
        self.check_home()?;
        if self.inspect()?.is_none() {
            return Err(ProbeError::Command);
        }
        self.run("restart", Duration::from_secs(10)).map(|_| ())
    }
}
impl OfficialDaemon {
    fn check_home(&self) -> Result<(), ProbeError> {
        if let Some(inherited) = std::env::var_os("CODEX_HOME") {
            if !same_path(Path::new(&inherited), &self.home) {
                return Err(ProbeError::Unsafe);
            }
        }
        Ok(())
    }
    fn run(&self, action: &str, timeout: Duration) -> Result<Vec<u8>, ProbeError> {
        let mut command =
            crate::session::managed_daemon_cli_command().ok_or(ProbeError::Unavailable)?;
        command
            .args(["app-server", "daemon", action])
            .env("CODEX_HOME", &self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        run_bounded(command, timeout)
    }
}
fn run_bounded(mut command: Command, timeout: Duration) -> Result<Vec<u8>, ProbeError> {
    let mut child = command.spawn().map_err(|_| ProbeError::Command)?;
    let stdout = child.stdout.take().ok_or(ProbeError::Command)?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let output = stdout.take(65537).read_to_end(&mut data).map(|_| data);
        let _ = sender.send(output);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(ProbeError::Timeout);
            }
        }
    };
    let status = status?;
    let data = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| ProbeError::Timeout)?
        .map_err(|_| ProbeError::Command)?;
    if !status.success() {
        return Err(ProbeError::Protocol);
    }
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
    if !matches!(
        value.backend.as_deref(),
        Some("pid" | "launchd" | "systemd")
    ) {
        return Err(ProbeError::Unsafe);
    }
    let executable = value.managed_codex_path.ok_or(ProbeError::Unsafe)?;
    let socket = value.socket_path.ok_or(ProbeError::Unsafe)?;
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
        fn inspect(&self) -> Result<Option<DaemonIdentity>, ProbeError> {
            Ok(self.running.then(|| DaemonIdentity {
                pid: Some(42),
                version: Some("0.160.0".into()),
                cli_version: Some("0.160.1".into()),
            }))
        }
        fn restart(&self) -> Result<(), ProbeError> {
            self.restarts.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max.fetch_max(active, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            self.active.fetch_sub(1, Ordering::SeqCst);
            if self.fail {
                Err(ProbeError::Command)
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
        assert!(
            records
                .iter()
                .any(|r| r.event.ends_with("restart_requested"))
        );
        assert!(records.iter().any(|r| r.event.ends_with("failed")));
        let encoded = serde_json::to_string(&records).unwrap();
        assert!(encoded.contains("not_blocked"));
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
            Err(ProbeError::Unsafe)
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
        fn inspect(&self) -> Result<Option<DaemonIdentity>, ProbeError> {
            Ok(Some(DaemonIdentity {
                pid: None,
                version: Some("0.160.0".into()),
                cli_version: Some("0.160.1".into()),
            }))
        }
        fn restart(&self) -> Result<(), ProbeError> {
            Err(ProbeError::Unavailable)
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
    }
    impl DaemonControl for StaysStopped {
        fn inspect(&self) -> Result<Option<DaemonIdentity>, ProbeError> {
            Ok(
                (self.inspected.fetch_add(1, Ordering::SeqCst) == 0).then(|| DaemonIdentity {
                    pid: None,
                    version: Some("0.160.0".into()),
                    cli_version: Some("0.160.1".into()),
                }),
            )
        }
        fn restart(&self) -> Result<(), ProbeError> {
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
            }),
            ready_timeout: Duration::ZERO,
        };
        let result = service.refresh(RefreshSource::DesktopRestart, &logs);
        assert_eq!(result.status, ModelCatalogRefreshStatus::Failed);
        assert_eq!(result.message_id, "model_catalog_refresh.timeout");
        assert!(result.before.is_some());
        assert!(result.after.is_none());
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
    #[cfg(windows)]
    #[test]
    fn command_timeout_is_bounded_and_does_not_create_visible_console() {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 5",
        ]);
        use std::os::windows::process::CommandExt;
        command
            .creation_flags(0x0800_0000)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        let started = Instant::now();
        assert!(matches!(
            run_bounded(command, Duration::from_millis(100)),
            Err(ProbeError::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
