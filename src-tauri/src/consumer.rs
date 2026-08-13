use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerStatus {
    Running,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerRole {
    Desktop,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerIdentity {
    pub role: ConsumerRole,
    pub pid: u32,
    pub started_at_epoch_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerScan {
    pub desktop: ConsumerStatus,
    pub cli: ConsumerStatus,
    pub identities: Vec<ConsumerIdentity>,
    pub desktop_roots: Vec<ConsumerIdentity>,
}

impl ConsumerScan {
    pub fn unknown() -> Self {
        Self {
            desktop: ConsumerStatus::Unknown,
            cli: ConsumerStatus::Unknown,
            identities: Vec::new(),
            desktop_roots: Vec::new(),
        }
    }

    pub fn is_trustworthy(&self) -> bool {
        !matches!(self.desktop, ConsumerStatus::Unknown)
            && !matches!(self.cli, ConsumerStatus::Unknown)
    }

    pub fn has_live_identity_from(&self, previous: &[ConsumerIdentity]) -> bool {
        self.identities
            .iter()
            .any(|identity| previous.contains(identity))
    }

    pub fn has_consumer_started_before(&self, cutoff_epoch_millis: u64) -> bool {
        self.identities
            .iter()
            .any(|identity| identity.started_at_epoch_millis <= cutoff_epoch_millis)
    }
}

pub trait ConsumerScanner: Send + Sync {
    fn scan(&self) -> ConsumerScan;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPackage {
    pub name: String,
    pub family_name: String,
    pub application_id: String,
    pub install_location: PathBuf,
}

impl DesktopPackage {
    fn aumid(&self) -> String {
        format!("{}!{}", self.family_name, self.application_id)
    }
}

pub trait DesktopPackageDiscovery: Send + Sync {
    fn discover(&self) -> Result<Vec<DesktopPackage>, ()>;
}

pub trait DesktopActivator: Send + Sync {
    fn activate(&self, aumid: &str) -> Result<(), ()>;
}

pub trait DesktopClock: Send + Sync {
    fn now_epoch_millis(&self) -> u64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopAction {
    Start,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSnapshot {
    pub status: ConsumerStatus,
    pub action: DesktopAction,
    pub message_id: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopFailureCategory {
    ActionUnavailable,
    ActivationFailed,
    LaunchNotObserved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopFailure {
    pub category: DesktopFailureCategory,
    pub message_id: &'static str,
}

#[derive(Clone)]
pub struct DesktopApplication {
    discovery: Arc<dyn DesktopPackageDiscovery>,
    scanner: Arc<dyn ConsumerScanner>,
    activator: Arc<dyn DesktopActivator>,
    clock: Arc<dyn DesktopClock>,
    scan_attempts: usize,
    scan_delay: Duration,
}

impl DesktopApplication {
    pub fn new() -> Self {
        Self::with_boundaries(
            Arc::new(WindowsDesktopPackageDiscovery),
            Arc::new(WindowsConsumerScanner::new()),
            Arc::new(WindowsDesktopActivator),
            Arc::new(SystemDesktopClock),
            20,
            Duration::from_millis(250),
        )
    }

    pub fn with_boundaries(
        discovery: Arc<dyn DesktopPackageDiscovery>,
        scanner: Arc<dyn ConsumerScanner>,
        activator: Arc<dyn DesktopActivator>,
        clock: Arc<dyn DesktopClock>,
        scan_attempts: usize,
        scan_delay: Duration,
    ) -> Self {
        Self {
            discovery,
            scanner,
            activator,
            clock,
            scan_attempts,
            scan_delay,
        }
    }

    pub fn inspect(&self) -> DesktopSnapshot {
        let scan = self.scanner.scan();
        if scan.desktop == ConsumerStatus::Unknown {
            return desktop_snapshot(
                ConsumerStatus::Unknown,
                DesktopAction::Unavailable,
                "desktop.identity_untrusted",
            );
        }
        if scan.desktop == ConsumerStatus::Running {
            return desktop_snapshot(
                ConsumerStatus::Running,
                DesktopAction::Unavailable,
                "desktop.running",
            );
        }
        match self.discovery.discover() {
            Ok(packages) if packages.is_empty() => desktop_snapshot(
                ConsumerStatus::Stopped,
                DesktopAction::Unavailable,
                "desktop.not_installed",
            ),
            Ok(packages) if packages.len() == 1 => desktop_snapshot(
                ConsumerStatus::Stopped,
                DesktopAction::Start,
                "desktop.ready_to_start",
            ),
            Ok(_) => desktop_snapshot(
                ConsumerStatus::Stopped,
                DesktopAction::Unavailable,
                "desktop.ambiguous_installation",
            ),
            Err(()) => desktop_snapshot(
                ConsumerStatus::Unknown,
                DesktopAction::Unavailable,
                "desktop.discovery_failed",
            ),
        }
    }

    pub fn start(&self) -> Result<DesktopSnapshot, DesktopFailure> {
        let before = self.scanner.scan();
        if before.desktop != ConsumerStatus::Stopped {
            return Err(desktop_failure(
                DesktopFailureCategory::ActionUnavailable,
                "desktop.action_unavailable",
            ));
        }
        let packages = self.discovery.discover().map_err(|()| {
            desktop_failure(
                DesktopFailureCategory::ActionUnavailable,
                "desktop.discovery_failed",
            )
        })?;
        let [package] = packages.as_slice() else {
            return Err(desktop_failure(
                DesktopFailureCategory::ActionUnavailable,
                if packages.is_empty() {
                    "desktop.not_installed"
                } else {
                    "desktop.ambiguous_installation"
                },
            ));
        };
        let activation_started_at = self.clock.now_epoch_millis();
        self.activator.activate(&package.aumid()).map_err(|()| {
            desktop_failure(
                DesktopFailureCategory::ActivationFailed,
                "desktop.activation_failed",
            )
        })?;
        for attempt in 0..self.scan_attempts.max(1) {
            if attempt > 0 && !self.scan_delay.is_zero() {
                thread::sleep(self.scan_delay);
            }
            let after = self.scanner.scan();
            if after.desktop == ConsumerStatus::Running
                && after.desktop_roots.iter().any(|root| {
                    root.started_at_epoch_millis >= activation_started_at
                        && !before.desktop_roots.contains(root)
                })
            {
                return Ok(desktop_snapshot(
                    ConsumerStatus::Running,
                    DesktopAction::Unavailable,
                    "desktop.running",
                ));
            }
        }
        Err(desktop_failure(
            DesktopFailureCategory::LaunchNotObserved,
            "desktop.launch_not_observed",
        ))
    }
}

impl Default for DesktopApplication {
    fn default() -> Self {
        Self::new()
    }
}

fn desktop_snapshot(
    status: ConsumerStatus,
    action: DesktopAction,
    message_id: &'static str,
) -> DesktopSnapshot {
    DesktopSnapshot {
        status,
        action,
        message_id,
    }
}

fn desktop_failure(category: DesktopFailureCategory, message_id: &'static str) -> DesktopFailure {
    DesktopFailure {
        category,
        message_id,
    }
}

#[derive(Debug, Default)]
struct SystemDesktopClock;

impl DesktopClock for SystemDesktopClock {
    fn now_epoch_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

#[derive(Debug, Default)]
struct WindowsDesktopPackageDiscovery;

impl DesktopPackageDiscovery for WindowsDesktopPackageDiscovery {
    fn discover(&self) -> Result<Vec<DesktopPackage>, ()> {
        #[cfg(windows)]
        {
            discover_windows_desktop_packages()
        }
        #[cfg(not(windows))]
        {
            Err(())
        }
    }
}

#[derive(Debug, Default)]
struct WindowsDesktopActivator;

impl DesktopActivator for WindowsDesktopActivator {
    fn activate(&self, aumid: &str) -> Result<(), ()> {
        #[cfg(windows)]
        {
            Command::new("explorer.exe")
                .arg(format!(r"shell:AppsFolder\{aumid}"))
                .spawn()
                .map(|_| ())
                .map_err(|_| ())
        }
        #[cfg(not(windows))]
        {
            let _ = aumid;
            Err(())
        }
    }
}

#[derive(Debug, Default)]
pub struct WindowsConsumerScanner;

impl WindowsConsumerScanner {
    pub fn new() -> Self {
        Self
    }
}

impl ConsumerScanner for WindowsConsumerScanner {
    fn scan(&self) -> ConsumerScan {
        #[cfg(windows)]
        {
            scan_windows().unwrap_or_else(|_| ConsumerScan::unknown())
        }
        #[cfg(not(windows))]
        {
            ConsumerScan::unknown()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAccess {
    Available,
    Denied,
    OtherUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureProcess {
    pub pid: u32,
    pub parent_pid: u32,
    pub started_at_epoch_millis: u64,
    pub name: String,
    pub executable: PathBuf,
    pub access: ProcessAccess,
    pub electron_helper: bool,
}

pub fn classify_fixture(processes: &[FixtureProcess]) -> ConsumerScan {
    classify_processes(processes)
}

fn classify_processes(processes: &[FixtureProcess]) -> ConsumerScan {
    let available = processes
        .iter()
        .filter(|process| process.access == ProcessAccess::Available)
        .collect::<Vec<_>>();
    let by_pid = available
        .iter()
        .map(|process| (process.pid, *process))
        .collect::<HashMap<_, _>>();
    let desktop_roots = available
        .iter()
        .filter(|process| is_desktop_root(process))
        .map(|process| process.pid)
        .collect::<HashSet<_>>();
    let desktop_children = available
        .iter()
        .filter(|process| {
            is_bundled_codex(process) && has_desktop_ancestor(process, &by_pid, &desktop_roots)
        })
        .map(|process| process.pid)
        .collect::<HashSet<_>>();
    let orphaned_bundled = available.iter().any(|process| {
        is_bundled_codex(process)
            && !desktop_roots.contains(&process.pid)
            && !has_desktop_ancestor(process, &by_pid, &desktop_roots)
    });
    let cli = available
        .iter()
        .filter(|process| {
            is_codex_name(&process.name)
                && !is_bundled_codex(process)
                && !process.electron_helper
                && !desktop_roots.contains(&process.pid)
                && !has_desktop_ancestor(process, &by_pid, &desktop_roots)
                && is_trusted_cli_path(&process.executable)
        })
        .collect::<Vec<_>>();
    let untrusted_cli = available.iter().any(|process| {
        is_codex_name(&process.name)
            && !is_bundled_codex(process)
            && !process.electron_helper
            && !desktop_roots.contains(&process.pid)
            && !has_desktop_ancestor(process, &by_pid, &desktop_roots)
            && !is_trusted_cli_path(&process.executable)
    });

    let desktop_denied = processes
        .iter()
        .any(|process| process.access == ProcessAccess::Denied && is_desktop_name(&process.name));
    let cli_denied = processes
        .iter()
        .any(|process| process.access == ProcessAccess::Denied && is_codex_name(&process.name));

    let desktop_running = !desktop_roots.is_empty() || !desktop_children.is_empty();
    let cli_running = !cli.is_empty();
    let mut identities = desktop_roots
        .iter()
        .filter_map(|pid| by_pid.get(pid))
        .map(|process| ConsumerIdentity {
            role: ConsumerRole::Desktop,
            pid: process.pid,
            started_at_epoch_millis: process.started_at_epoch_millis,
        })
        .chain(
            desktop_children
                .iter()
                .filter_map(|pid| by_pid.get(pid))
                .map(|process| ConsumerIdentity {
                    role: ConsumerRole::Desktop,
                    pid: process.pid,
                    started_at_epoch_millis: process.started_at_epoch_millis,
                }),
        )
        .chain(cli.into_iter().map(|process| ConsumerIdentity {
            role: ConsumerRole::Cli,
            pid: process.pid,
            started_at_epoch_millis: process.started_at_epoch_millis,
        }))
        .collect::<Vec<_>>();
    identities.sort_by_key(|identity| (identity.role as u8, identity.pid));

    ConsumerScan {
        desktop: if desktop_denied || orphaned_bundled || untrusted_cli {
            ConsumerStatus::Unknown
        } else if desktop_running {
            ConsumerStatus::Running
        } else {
            ConsumerStatus::Stopped
        },
        cli: if cli_denied || untrusted_cli {
            ConsumerStatus::Unknown
        } else if cli_running {
            ConsumerStatus::Running
        } else {
            ConsumerStatus::Stopped
        },
        identities,
        desktop_roots: desktop_roots
            .iter()
            .filter_map(|pid| by_pid.get(pid))
            .map(|process| ConsumerIdentity {
                role: ConsumerRole::Desktop,
                pid: process.pid,
                started_at_epoch_millis: process.started_at_epoch_millis,
            })
            .collect(),
    }
}

#[cfg(windows)]
fn discover_windows_desktop_packages() -> Result<Vec<DesktopPackage>, ()> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct PackageRecord {
        name: String,
        family_name: String,
        application_id: String,
        install_location: PathBuf,
    }

    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$packages = @(
  Get-AppxPackage -PackageTypeFilter Main |
    Where-Object { $_.Name -in @('OpenAI.Codex', 'OpenAI.ChatGPT') } |
    ForEach-Object {
      $package = $_
      $manifest = Get-AppxPackageManifest -Package $package.PackageFullName
      @($manifest.Package.Applications.Application) | ForEach-Object {
        [pscustomobject]@{
          Name = $package.Name
          FamilyName = $package.PackageFamilyName
          ApplicationId = $_.Id
          InstallLocation = $package.InstallLocation
        }
      }
    }
)
ConvertTo-Json -Compress -InputObject $packages
"#;
    let system_root = std::env::var_os("SystemRoot").ok_or(())?;
    let powershell = PathBuf::from(system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let output = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let records = serde_json::from_slice::<Vec<PackageRecord>>(&output.stdout).map_err(|_| ())?;
    Ok(records
        .into_iter()
        .filter(|record| {
            !record.family_name.trim().is_empty()
                && !record.application_id.trim().is_empty()
                && record.install_location.is_absolute()
        })
        .map(|record| DesktopPackage {
            name: record.name,
            family_name: record.family_name,
            application_id: record.application_id,
            install_location: record.install_location,
        })
        .collect())
}

fn is_desktop_root(process: &&FixtureProcess) -> bool {
    is_desktop_name(&process.name)
        && is_packaged_openai_desktop(&process.executable)
        && !is_bundled_codex(process)
        && !process.electron_helper
}

fn is_desktop_name(name: &str) -> bool {
    matches!(
        normalized_stem(name).as_deref(),
        Some("chatgpt") | Some("codex")
    )
}

fn is_codex_name(name: &str) -> bool {
    normalized_stem(name).as_deref() == Some("codex")
}

fn normalized_stem(name: &str) -> Option<String> {
    Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_ascii_lowercase())
}

fn is_packaged_openai_desktop(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.contains("\\windowsapps\\openai.codex_")
        || normalized.contains("\\windowsapps\\openai.chatgpt_")
}

fn is_bundled_codex(process: &FixtureProcess) -> bool {
    let normalized = process
        .executable
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.contains("\\resources\\codex\\") && is_codex_name(&process.name)
}

fn is_trusted_cli_path(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.contains("\\node_modules\\@openai\\codex\\")
        || normalized.contains("\\codex-win32-")
        || normalized.ends_with("\\.local\\bin\\codex.exe")
}

fn has_desktop_ancestor(
    process: &&FixtureProcess,
    by_pid: &HashMap<u32, &FixtureProcess>,
    desktop_roots: &HashSet<u32>,
) -> bool {
    let mut parent = process.parent_pid;
    let mut visited = HashSet::new();
    while parent != 0 && visited.insert(parent) {
        if desktop_roots.contains(&parent) {
            return true;
        }
        let Some(ancestor) = by_pid.get(&parent) else {
            return false;
        };
        parent = ancestor.parent_pid;
    }
    false
}

#[cfg(windows)]
fn scan_windows() -> Result<ConsumerScan, ()> {
    use std::mem::size_of;

    use sysinfo::{Pid, System, get_current_pid};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    let system = System::new_all();
    let current_user = system
        .process(get_current_pid().map_err(|_| ())?)
        .and_then(|process| process.user_id())
        .cloned()
        .ok_or(())?;
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(());
    }
    let mut processes = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { Process32FirstW(snapshot, &mut entry) } == 0 {
        unsafe {
            CloseHandle(snapshot);
        }
        return Err(());
    }
    loop {
        let name_length = entry
            .szExeFile
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..name_length]);
        let mut process =
            read_windows_process(entry.th32ProcessID, entry.th32ParentProcessID, name);
        if is_desktop_name(&process.name) {
            match system.process(Pid::from_u32(process.pid)) {
                Some(metadata) if metadata.user_id() != Some(&current_user) => {
                    process.access = metadata
                        .user_id()
                        .map(|_| ProcessAccess::OtherUser)
                        .unwrap_or(ProcessAccess::Denied);
                }
                Some(metadata) if metadata.cmd().is_empty() => {
                    process.access = ProcessAccess::Denied;
                }
                Some(metadata) => {
                    process.electron_helper = metadata
                        .cmd()
                        .iter()
                        .skip(1)
                        .any(|argument| argument.to_string_lossy().starts_with("--type="));
                }
                None => process.access = ProcessAccess::Denied,
            }
        }
        processes.push(process);
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if unsafe { Process32NextW(snapshot, &mut entry) } != 0 {
            continue;
        }
        if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
            unsafe {
                CloseHandle(snapshot);
            }
            return Err(());
        }
        break;
    }
    unsafe {
        CloseHandle(snapshot);
    }
    Ok(classify_processes(&processes))
}

#[cfg(windows)]
fn read_windows_process(pid: u32, parent_pid: u32, name: String) -> FixtureProcess {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return FixtureProcess {
            pid,
            parent_pid,
            started_at_epoch_millis: 0,
            name,
            executable: PathBuf::new(),
            access: ProcessAccess::Denied,
            electron_helper: false,
        };
    }
    let mut path = vec![0u16; 32_768];
    let mut path_length = path.len() as u32;
    let mut created = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exited = created;
    let mut kernel = created;
    let mut user = created;
    let readable = unsafe {
        QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut path_length) != 0
            && GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0
    };
    unsafe {
        CloseHandle(handle);
    }
    if !readable {
        return FixtureProcess {
            pid,
            parent_pid,
            started_at_epoch_millis: 0,
            name,
            executable: PathBuf::new(),
            access: ProcessAccess::Denied,
            electron_helper: false,
        };
    }
    let file_time = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
    let started_at_epoch_millis = file_time
        .checked_div(10_000)
        .and_then(|milliseconds| milliseconds.checked_sub(11_644_473_600_000))
        .unwrap_or_default();
    FixtureProcess {
        pid,
        parent_pid,
        started_at_epoch_millis,
        name,
        executable: PathBuf::from(String::from_utf16_lossy(&path[..path_length as usize])),
        access: ProcessAccess::Available,
        electron_helper: false,
    }
}
