//! Read-only discovery and identity checking for the Codex shared daemon.
//!
//! This module deliberately does not own, start, stop, or restart a service.  The
//! fixture detector is the contract boundary used by tests and future callers;
//! the Windows adapter only supplies candidates, a fixed probe, and process data.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const DEFAULT_PROBE_TIMEOUT_MILLIS: u64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedDaemonStatus {
    Running,
    Stopped,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedDaemonCapability {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedDaemonEntrySource {
    DesktopBundled,
    Npm,
    Standalone,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedDaemonVersionVisibility {
    Visible,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedDaemonEnvironmentMatch {
    Match,
    Mismatch,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedDaemonBackend {
    Pid,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedDaemonProcessKind {
    SharedDaemon,
    GpteasyOwned,
    DesktopBundled,
    InteractiveCli,
    OtherAppServer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDaemonCandidate {
    pub source: SharedDaemonEntrySource,
    pub path: PathBuf,
    pub exists: bool,
    pub native_identity_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDaemonIdentityEvidence {
    pub pid: u32,
    pub started_at_epoch_millis: u64,
    pub executable: PathBuf,
    pub user: String,
    pub codex_home: PathBuf,
    pub backend: SharedDaemonBackend,
    pub socket_control_owner: String,
    pub kind: SharedDaemonProcessKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDaemonReport {
    pub pid: Option<u32>,
    pub started_at_epoch_millis: Option<u64>,
    pub executable: Option<PathBuf>,
    pub user: Option<String>,
    pub codex_home: Option<PathBuf>,
    pub backend: SharedDaemonBackend,
    pub socket_control_owner: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedDaemonProbe {
    Running(SharedDaemonReport),
    Stopped { version: Option<String> },
    Unsupported { reason: &'static str },
    Unknown { reason: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedDaemonFixture {
    pub current_user: String,
    pub default_codex_home: PathBuf,
    pub candidates: Vec<SharedDaemonCandidate>,
    pub probes: Vec<(PathBuf, SharedDaemonProbe)>,
    pub reported_processes: Vec<SharedDaemonIdentityEvidence>,
    pub actual_processes: Vec<SharedDaemonIdentityEvidence>,
    pub owned_processes: Vec<SharedDaemonIdentityEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedDaemonIdentitySummary {
    pub pid: u32,
    pub started_at_epoch_millis: u64,
    pub executable_identity: String,
    pub user_match: bool,
    pub home_match: bool,
    pub backend: SharedDaemonBackend,
    pub socket_control_match: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedDaemonSnapshot {
    pub status: SharedDaemonStatus,
    pub capability: SharedDaemonCapability,
    pub entry_source: SharedDaemonEntrySource,
    pub version: Option<String>,
    pub version_visibility: SharedDaemonVersionVisibility,
    pub environment_match: SharedDaemonEnvironmentMatch,
    pub identity: Option<SharedDaemonIdentitySummary>,
    pub reason: String,
}

impl SharedDaemonSnapshot {
    fn unknown(reason: impl Into<String>) -> Self {
        Self {
            status: SharedDaemonStatus::Unknown,
            capability: SharedDaemonCapability::Unknown,
            entry_source: SharedDaemonEntrySource::Unknown,
            version: None,
            version_visibility: SharedDaemonVersionVisibility::Unknown,
            environment_match: SharedDaemonEnvironmentMatch::Unknown,
            identity: None,
            reason: reason.into(),
        }
    }
}

pub fn detect_shared_daemon(fixture: &SharedDaemonFixture) -> SharedDaemonSnapshot {
    let mut candidates = fixture
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.path.is_absolute()
                && candidate.exists
                && candidate.native_identity_verified
                && candidate.source != SharedDaemonEntrySource::DesktopBundled
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| normalized_path(&candidate.path));
    candidates.dedup_by(|left, right| normalized_path(&left.path) == normalized_path(&right.path));

    if candidates.is_empty() {
        return SharedDaemonSnapshot::unknown("missing_trusted_entry");
    }

    let mut stopped: Option<SharedDaemonSnapshot> = None;
    let mut saw_unknown = false;
    for candidate in candidates {
        let Some((_, probe)) = fixture
            .probes
            .iter()
            .find(|(path, _)| normalized_path(path) == normalized_path(&candidate.path))
        else {
            saw_unknown = true;
            continue;
        };
        match probe {
            SharedDaemonProbe::Running(report) => {
                let Some(identity) = match_running_identity(fixture, report) else {
                    saw_unknown = true;
                    continue;
                };
                let version = sanitize_version(report.version.as_deref());
                return SharedDaemonSnapshot {
                    status: SharedDaemonStatus::Running,
                    capability: SharedDaemonCapability::Supported,
                    entry_source: candidate.source,
                    version_visibility: version_visibility(&version),
                    version,
                    environment_match: SharedDaemonEnvironmentMatch::Match,
                    identity: Some(identity),
                    reason: "running_identity_verified".to_owned(),
                };
            }
            SharedDaemonProbe::Stopped { version } => {
                stopped = Some(SharedDaemonSnapshot {
                    status: SharedDaemonStatus::Stopped,
                    capability: SharedDaemonCapability::Supported,
                    entry_source: candidate.source,
                    version_visibility: version_visibility(&sanitize_version(version.as_deref())),
                    version: sanitize_version(version.as_deref()),
                    environment_match: SharedDaemonEnvironmentMatch::Match,
                    identity: None,
                    reason: "probe_confirmed_stopped".to_owned(),
                });
            }
            SharedDaemonProbe::Unsupported { reason } => {
                stopped = Some(SharedDaemonSnapshot {
                    status: SharedDaemonStatus::Unsupported,
                    capability: SharedDaemonCapability::Unsupported,
                    entry_source: candidate.source,
                    version: None,
                    version_visibility: SharedDaemonVersionVisibility::Unknown,
                    environment_match: SharedDaemonEnvironmentMatch::Unknown,
                    identity: None,
                    reason: (*reason).to_owned(),
                });
            }
            SharedDaemonProbe::Unknown { .. } => saw_unknown = true,
        }
    }

    if !saw_unknown {
        if let Some(snapshot) = stopped {
            return snapshot;
        }
    }
    SharedDaemonSnapshot::unknown(if saw_unknown {
        "probe_or_identity_unknown"
    } else {
        "no_verified_running_identity"
    })
}

fn match_running_identity(
    fixture: &SharedDaemonFixture,
    report: &SharedDaemonReport,
) -> Option<SharedDaemonIdentitySummary> {
    if report.backend != SharedDaemonBackend::Pid {
        return None;
    }
    let pid = report.pid?;
    let started_at_epoch_millis = report.started_at_epoch_millis?;
    let executable = report.executable.as_ref()?;
    let user = report.user.as_ref()?;
    let codex_home = report.codex_home.as_ref()?;
    let socket_control_owner = report.socket_control_owner.as_ref()?;
    if user != &fixture.current_user
        || normalized_path(codex_home) != normalized_path(&fixture.default_codex_home)
    {
        return None;
    }
    if fixture
        .owned_processes
        .iter()
        .any(|owned| same_identity(owned, report))
    {
        return None;
    }
    let process = fixture.actual_processes.iter().find(|process| {
        process.pid == pid
            && process.started_at_epoch_millis == started_at_epoch_millis
            && normalized_path(&process.executable) == normalized_path(executable)
            && process.user == *user
            && normalized_path(&process.codex_home) == normalized_path(codex_home)
            && process.backend == report.backend
            && process.socket_control_owner == *socket_control_owner
            && process.kind == SharedDaemonProcessKind::SharedDaemon
    })?;
    if !fixture
        .reported_processes
        .iter()
        .any(|reported| same_identity(reported, report))
    {
        return None;
    }
    Some(SharedDaemonIdentitySummary {
        pid,
        started_at_epoch_millis,
        executable_identity: executable_identity(executable),
        user_match: process.user == fixture.current_user,
        home_match: normalized_path(&process.codex_home)
            == normalized_path(&fixture.default_codex_home),
        backend: report.backend,
        socket_control_match: process.socket_control_owner == *socket_control_owner,
    })
}

fn same_identity(left: &SharedDaemonIdentityEvidence, right: &SharedDaemonReport) -> bool {
    Some(left.pid) == right.pid
        && Some(left.started_at_epoch_millis) == right.started_at_epoch_millis
        && right
            .executable
            .as_ref()
            .is_some_and(|path| normalized_path(&left.executable) == normalized_path(path))
        && right.user.as_ref().is_some_and(|user| left.user == *user)
        && right
            .codex_home
            .as_ref()
            .is_some_and(|home| normalized_path(&left.codex_home) == normalized_path(home))
        && left.backend == right.backend
        && right
            .socket_control_owner
            .as_ref()
            .is_some_and(|owner| left.socket_control_owner == *owner)
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn executable_identity(path: &Path) -> String {
    let normalized = normalized_path(path);
    if normalized.contains("\\node_modules\\") {
        "npm_native_codex".to_owned()
    } else if normalized.contains("\\windowsapps\\openai.") {
        "desktop_bundled_codex".to_owned()
    } else if normalized.contains("\\.codex\\packages\\app-server-daemon\\") {
        "codex_home_daemon".to_owned()
    } else {
        "trusted_standalone_codex".to_owned()
    }
}

fn sanitize_version(version: Option<&str>) -> Option<String> {
    let value = version?.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return None;
    }
    Some(value.to_owned())
}

fn version_visibility(version: &Option<String>) -> SharedDaemonVersionVisibility {
    if version.is_some() {
        SharedDaemonVersionVisibility::Visible
    } else {
        SharedDaemonVersionVisibility::Unavailable
    }
}

#[cfg(windows)]
pub fn detect_windows_shared_daemon() -> SharedDaemonSnapshot {
    let current_user = std::env::var("USERNAME").unwrap_or_else(|_| "unknown".to_owned());
    let user_profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let Some(profile) = user_profile else {
        return SharedDaemonSnapshot::unknown("missing_user_profile");
    };
    let fixture = build_windows_fixture(current_user, profile.join(".codex"));
    detect_shared_daemon(&fixture)
}

#[cfg(not(windows))]
pub fn detect_windows_shared_daemon() -> SharedDaemonSnapshot {
    SharedDaemonSnapshot {
        status: SharedDaemonStatus::Unsupported,
        capability: SharedDaemonCapability::Unsupported,
        entry_source: SharedDaemonEntrySource::Unknown,
        version: None,
        version_visibility: SharedDaemonVersionVisibility::Unknown,
        environment_match: SharedDaemonEnvironmentMatch::Unknown,
        identity: None,
        reason: "windows_only".to_owned(),
    }
}

#[cfg(windows)]
fn build_windows_fixture(current_user: String, default_codex_home: PathBuf) -> SharedDaemonFixture {
    let candidates = discover_windows_candidates(&default_codex_home);
    let mut probes = Vec::new();
    for candidate in &candidates {
        if candidate.source == SharedDaemonEntrySource::DesktopBundled
            || candidate.source == SharedDaemonEntrySource::Unknown
            || !candidate.native_identity_verified
        {
            continue;
        }
        probes.push((
            candidate.path.clone(),
            probe_windows_candidate(&candidate.path),
        ));
    }
    let reported_processes = probes
        .iter()
        .filter_map(|(_, probe)| match probe {
            SharedDaemonProbe::Running(report) => report_to_evidence(report),
            _ => None,
        })
        .collect::<Vec<_>>();
    let actual_processes = scan_windows_processes(&current_user, &reported_processes);
    SharedDaemonFixture {
        current_user,
        default_codex_home,
        candidates,
        probes,
        reported_processes,
        actual_processes,
        owned_processes: Vec::new(),
    }
}

#[cfg(windows)]
fn discover_windows_candidates(default_codex_home: &Path) -> Vec<SharedDaemonCandidate> {
    let directories = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let package_root = default_codex_home
        .join("packages")
        .join("app-server-daemon");
    let mut candidates = Vec::new();
    collect_windows_candidates(&package_root, 0, &mut candidates);
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let packages = PathBuf::from(local_app_data).join("Packages");
        collect_windows_candidates(&packages, 0, &mut candidates);
    }
    for directory in directories {
        for name in ["codex.exe", "codex.cmd"] {
            let path = directory.join(name);
            if let Some(candidate) = windows_candidate(path) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

#[cfg(windows)]
fn collect_windows_candidates(
    root: &Path,
    depth: usize,
    candidates: &mut Vec<SharedDaemonCandidate>,
) {
    if depth > 7 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_windows_candidates(&path, depth + 1, candidates);
        } else if path.file_name().is_some_and(|name| {
            name.eq_ignore_ascii_case("codex.exe") || name.eq_ignore_ascii_case("codex.cmd")
        }) {
            if let Some(candidate) = windows_candidate(path) {
                candidates.push(candidate);
            }
        }
    }
}

#[cfg(windows)]
fn windows_candidate(path: PathBuf) -> Option<SharedDaemonCandidate> {
    let native_path = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"))
    {
        npm_native_codex(&path)?
    } else {
        path
    };
    let normalized = normalized_path(&native_path);
    let source = if normalized.contains("\\windowsapps\\openai.") {
        SharedDaemonEntrySource::DesktopBundled
    } else if normalized.contains("\\node_modules\\@openai\\codex")
        || normalized.contains("\\codex-win32-")
    {
        SharedDaemonEntrySource::Npm
    } else if normalized.contains("\\.codex\\packages\\app-server-daemon\\")
        || normalized.ends_with("\\.local\\bin\\codex.exe")
    {
        SharedDaemonEntrySource::Standalone
    } else {
        SharedDaemonEntrySource::Unknown
    };
    Some(SharedDaemonCandidate {
        exists: native_path.is_file(),
        native_identity_verified: source != SharedDaemonEntrySource::Unknown,
        source,
        path: native_path,
    })
}

#[cfg(windows)]
fn npm_native_codex(command: &Path) -> Option<PathBuf> {
    let root = command.parent()?;
    let package = if cfg!(target_arch = "aarch64") {
        "codex-win32-arm64"
    } else {
        "codex-win32-x64"
    };
    let triple = if cfg!(target_arch = "aarch64") {
        "aarch64-pc-windows-msvc"
    } else {
        "x86_64-pc-windows-msvc"
    };
    let candidate = root
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("node_modules")
        .join("@openai")
        .join(package)
        .join("vendor")
        .join(triple)
        .join("bin")
        .join("codex.exe");
    candidate.is_file().then_some(candidate)
}

#[cfg(windows)]
fn report_to_evidence(report: &SharedDaemonReport) -> Option<SharedDaemonIdentityEvidence> {
    Some(SharedDaemonIdentityEvidence {
        pid: report.pid?,
        started_at_epoch_millis: report.started_at_epoch_millis?,
        executable: report.executable.clone()?,
        user: report.user.clone()?,
        codex_home: report.codex_home.clone()?,
        backend: report.backend,
        socket_control_owner: report.socket_control_owner.clone()?,
        kind: SharedDaemonProcessKind::SharedDaemon,
    })
}

#[cfg(windows)]
fn probe_windows_candidate(path: &Path) -> SharedDaemonProbe {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let mut command = Command::new(path);
    command
        .args(["app-server", "daemon", "status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
    let Ok(mut child) = command.spawn() else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_spawn_failed",
        };
    };
    let deadline = Instant::now() + Duration::from_millis(DEFAULT_PROBE_TIMEOUT_MILLIS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return SharedDaemonProbe::Unsupported {
                        reason: "daemon_status_unsupported",
                    };
                }
                let Some(mut stdout) = child.stdout.take() else {
                    return SharedDaemonProbe::Unknown {
                        reason: "probe_missing_output",
                    };
                };
                use std::io::Read;
                let mut body = String::new();
                if stdout.read_to_string(&mut body).is_err() {
                    return SharedDaemonProbe::Unknown {
                        reason: "probe_read_failed",
                    };
                }
                return parse_windows_probe(&body);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                return SharedDaemonProbe::Unknown {
                    reason: "probe_timed_out",
                };
            }
            Err(_) => {
                return SharedDaemonProbe::Unknown {
                    reason: "probe_wait_failed",
                };
            }
        }
    }
}

#[cfg(windows)]
fn parse_windows_probe(body: &str) -> SharedDaemonProbe {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_malformed_json",
        };
    };
    let Some(status) = value.get("status").and_then(serde_json::Value::as_str) else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_missing_status",
        };
    };
    if status.eq_ignore_ascii_case("stopped") {
        return SharedDaemonProbe::Stopped {
            version: value
                .get("version")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        };
    }
    let Some(identity) = value.get("identity") else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_missing_identity",
        };
    };
    let Some(pid) = identity
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_missing_pid",
        };
    };
    let Some(started) = identity
        .get("startedAtEpochMillis")
        .and_then(serde_json::Value::as_u64)
    else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_missing_creation_time",
        };
    };
    let Some(executable) = identity
        .get("executable")
        .and_then(serde_json::Value::as_str)
    else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_missing_executable",
        };
    };
    let Some(user) = identity.get("user").and_then(serde_json::Value::as_str) else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_missing_user",
        };
    };
    let Some(home) = identity
        .get("codexHome")
        .and_then(serde_json::Value::as_str)
    else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_missing_home",
        };
    };
    let Some(socket) = identity
        .get("socketControlOwner")
        .and_then(serde_json::Value::as_str)
    else {
        return SharedDaemonProbe::Unknown {
            reason: "probe_missing_socket_owner",
        };
    };
    let backend = match identity.get("backend").and_then(serde_json::Value::as_str) {
        Some("pid") => SharedDaemonBackend::Pid,
        _ => SharedDaemonBackend::Unknown,
    };
    SharedDaemonProbe::Running(SharedDaemonReport {
        pid: Some(pid),
        started_at_epoch_millis: Some(started),
        executable: Some(PathBuf::from(executable)),
        user: Some(user.to_owned()),
        codex_home: Some(PathBuf::from(home)),
        backend,
        socket_control_owner: Some(socket.to_owned()),
        version: value
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(windows)]
fn scan_windows_processes(
    current_user: &str,
    reported_processes: &[SharedDaemonIdentityEvidence],
) -> Vec<SharedDaemonIdentityEvidence> {
    use sysinfo::{Pid, System, get_current_pid};

    let system = System::new_all();
    let current_user_id = get_current_pid()
        .ok()
        .and_then(|pid| system.process(pid))
        .and_then(|process| process.user_id())
        .cloned();
    reported_processes
        .iter()
        .filter_map(|reported| {
            let process = system.process(Pid::from_u32(reported.pid))?;
            let executable = process.exe()?.to_path_buf();
            let started_at_epoch_millis = process.start_time().saturating_mul(1_000);
            let user_matches = current_user_id
                .as_ref()
                .zip(process.user_id())
                .is_some_and(|(current, process_user)| current == process_user);
            let kind = if user_matches
                && started_at_epoch_millis == reported.started_at_epoch_millis
                && normalized_path(&executable) == normalized_path(&reported.executable)
            {
                SharedDaemonProcessKind::SharedDaemon
            } else {
                SharedDaemonProcessKind::OtherAppServer
            };
            Some(SharedDaemonIdentityEvidence {
                pid: reported.pid,
                started_at_epoch_millis,
                executable,
                user: if user_matches {
                    current_user.to_owned()
                } else {
                    "other_user".to_owned()
                },
                codex_home: reported.codex_home.clone(),
                backend: reported.backend,
                socket_control_owner: reported.socket_control_owner.clone(),
                kind,
            })
        })
        .collect()
}
