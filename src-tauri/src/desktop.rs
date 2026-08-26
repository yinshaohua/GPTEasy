use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::consumer::{ConsumerIdentity, ConsumerScanner, ConsumerStatus, WindowsConsumerScanner};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopBoundaryError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPackage {
    pub name: String,
    pub family_name: String,
    pub application_id: String,
    pub install_location: std::path::PathBuf,
}

pub trait DesktopPackageDiscovery: Send + Sync {
    fn discover(&self) -> Result<Vec<DesktopPackage>, DesktopBoundaryError>;
}

pub trait DesktopActivator: Send + Sync {
    fn activate(&self, aumid: &str) -> Result<(), DesktopBoundaryError>;
}

pub trait DesktopProcessController: Send + Sync {
    fn request_close(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError>;
}

pub trait DesktopClock: Send + Sync {
    fn now_epoch_millis(&self) -> u64;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopAction {
    Start,
    Restart,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSnapshot {
    pub status: ConsumerStatus,
    pub action: DesktopAction,
    pub message_id: &'static str,
    pub roots: Vec<ConsumerIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopFailureCategory {
    ActionUnavailable,
    IdentityChanged,
    CloseFailed,
    CloseTimedOut,
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
    process_controller: Arc<dyn DesktopProcessController>,
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
            Arc::new(WindowsDesktopProcessController),
            Arc::new(SystemDesktopClock),
            20,
            Duration::from_millis(250),
        )
    }

    pub fn with_boundaries(
        discovery: Arc<dyn DesktopPackageDiscovery>,
        scanner: Arc<dyn ConsumerScanner>,
        activator: Arc<dyn DesktopActivator>,
        process_controller: Arc<dyn DesktopProcessController>,
        clock: Arc<dyn DesktopClock>,
        scan_attempts: usize,
        scan_delay: Duration,
    ) -> Self {
        Self {
            discovery,
            scanner,
            activator,
            process_controller,
            clock,
            scan_attempts,
            scan_delay,
        }
    }

    pub fn inspect(&self) -> DesktopSnapshot {
        match self.discover_package() {
            Ok(package) => {
                let scan = self.scan_for_package(&package);
                match scan.desktop {
                    ConsumerStatus::Running => desktop_snapshot(
                        ConsumerStatus::Running,
                        DesktopAction::Restart,
                        "desktop.ready_to_restart",
                        scan.desktop_roots,
                    ),
                    ConsumerStatus::Stopped => desktop_snapshot(
                        ConsumerStatus::Stopped,
                        DesktopAction::Start,
                        "desktop.ready_to_start",
                        Vec::new(),
                    ),
                    ConsumerStatus::Unknown => desktop_snapshot(
                        ConsumerStatus::Unknown,
                        DesktopAction::Unavailable,
                        "desktop.identity_untrusted",
                        Vec::new(),
                    ),
                }
            }
            Err(message_id) => desktop_snapshot(
                if message_id == "desktop.not_installed" {
                    ConsumerStatus::Stopped
                } else {
                    ConsumerStatus::Unknown
                },
                DesktopAction::Unavailable,
                message_id,
                Vec::new(),
            ),
        }
    }

    pub fn start(&self) -> Result<DesktopSnapshot, DesktopFailure> {
        let package = self.discover_package().map_err(action_unavailable)?;
        let before = self.scan_for_package(&package);
        if before.desktop != ConsumerStatus::Stopped {
            return Err(action_unavailable("desktop.action_unavailable"));
        }
        self.activate_and_observe(&package, &before.desktop_roots)
    }

    pub fn restart(
        &self,
        expected_roots: &[ConsumerIdentity],
    ) -> Result<DesktopSnapshot, DesktopFailure> {
        let package = self.discover_package().map_err(action_unavailable)?;
        let before = self.scan_for_package(&package);
        if before.desktop != ConsumerStatus::Running
            || before.desktop_roots.is_empty()
            || before.desktop_roots != expected_roots
        {
            return Err(desktop_failure(
                DesktopFailureCategory::IdentityChanged,
                "desktop.identity_changed",
            ));
        }
        self.process_controller
            .request_close(&before.desktop_roots)
            .map_err(|_| {
                desktop_failure(DesktopFailureCategory::CloseFailed, "desktop.close_failed")
            })?;
        if !self.wait_for_roots_to_exit(&package, expected_roots) {
            return Err(desktop_failure(
                DesktopFailureCategory::CloseTimedOut,
                "desktop.close_timed_out",
            ));
        }
        self.activate_and_observe(&package, expected_roots)
    }

    fn discover_package(&self) -> Result<DesktopPackage, &'static str> {
        let packages = self
            .discovery
            .discover()
            .map_err(|_| "desktop.discovery_failed")?;
        resolve_desktop_package(packages)
    }

    fn scan_for_package(&self, package: &DesktopPackage) -> crate::consumer::ConsumerScan {
        self.scanner
            .scan_for_install_locations(std::slice::from_ref(&package.install_location))
    }

    fn wait_for_roots_to_exit(
        &self,
        package: &DesktopPackage,
        expected_roots: &[ConsumerIdentity],
    ) -> bool {
        for attempt in 0..self.scan_attempts.max(1) {
            self.wait_between_scans(attempt);
            let scan = self.scan_for_package(package);
            if scan.desktop != ConsumerStatus::Unknown
                && !scan
                    .desktop_roots
                    .iter()
                    .any(|root| expected_roots.contains(root))
            {
                return true;
            }
        }
        false
    }

    fn activate_and_observe(
        &self,
        package: &DesktopPackage,
        previous_roots: &[ConsumerIdentity],
    ) -> Result<DesktopSnapshot, DesktopFailure> {
        let activation_started_at = self.clock.now_epoch_millis();
        self.activator.activate(&package.aumid()).map_err(|_| {
            desktop_failure(
                DesktopFailureCategory::ActivationFailed,
                "desktop.activation_failed",
            )
        })?;
        for attempt in 0..self.scan_attempts.max(1) {
            self.wait_between_scans(attempt);
            let scan = self.scan_for_package(package);
            if scan.desktop == ConsumerStatus::Running
                && scan.desktop_roots.iter().any(|root| {
                    root.started_at_epoch_millis >= activation_started_at
                        && !previous_roots.contains(root)
                })
            {
                return Ok(desktop_snapshot(
                    ConsumerStatus::Running,
                    DesktopAction::Restart,
                    "desktop.running",
                    scan.desktop_roots,
                ));
            }
        }
        Err(desktop_failure(
            DesktopFailureCategory::LaunchNotObserved,
            "desktop.launch_not_observed",
        ))
    }

    fn wait_between_scans(&self, attempt: usize) {
        if attempt > 0 && !self.scan_delay.is_zero() {
            thread::sleep(self.scan_delay);
        }
    }
}

impl Default for DesktopApplication {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopPackage {
    fn aumid(&self) -> String {
        format!("{}!{}", self.family_name, self.application_id)
    }
}

const OPENAI_WINDOWS_PUBLISHER_ID: &str = "2p2nqsd0c76g0";

fn resolve_desktop_package(packages: Vec<DesktopPackage>) -> Result<DesktopPackage, &'static str> {
    if packages.is_empty() {
        return Err("desktop.not_installed");
    }
    if packages.iter().any(|package| {
        !matches!(package.name.as_str(), "OpenAI.Codex" | "OpenAI.ChatGPT")
            || package.application_id.trim().is_empty()
            || package.install_location.as_os_str().is_empty()
            || package
                .family_name
                .rsplit_once('_')
                .map(|(_, publisher_id)| {
                    !publisher_id.eq_ignore_ascii_case(OPENAI_WINDOWS_PUBLISHER_ID)
                })
                .unwrap_or(true)
    }) {
        return Err("desktop.discovery_failed");
    }
    let family_count = packages
        .iter()
        .map(|package| package.family_name.to_ascii_lowercase())
        .collect::<HashSet<_>>()
        .len();
    match packages.as_slice() {
        [package] => Ok(package.clone()),
        _ if family_count > 1 => Err("desktop.ambiguous_installation"),
        _ => Err("desktop.discovery_failed"),
    }
}

fn desktop_snapshot(
    status: ConsumerStatus,
    action: DesktopAction,
    message_id: &'static str,
    roots: Vec<ConsumerIdentity>,
) -> DesktopSnapshot {
    DesktopSnapshot {
        status,
        action,
        message_id,
        roots,
    }
}

fn desktop_failure(category: DesktopFailureCategory, message_id: &'static str) -> DesktopFailure {
    DesktopFailure {
        category,
        message_id,
    }
}

fn action_unavailable(message_id: &'static str) -> DesktopFailure {
    desktop_failure(DesktopFailureCategory::ActionUnavailable, message_id)
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
    fn discover(&self) -> Result<Vec<DesktopPackage>, DesktopBoundaryError> {
        #[cfg(windows)]
        {
            discover_windows_desktop_packages()
        }
        #[cfg(not(windows))]
        {
            Err(DesktopBoundaryError)
        }
    }
}

#[derive(Debug, Default)]
struct WindowsDesktopActivator;

impl DesktopActivator for WindowsDesktopActivator {
    fn activate(&self, aumid: &str) -> Result<(), DesktopBoundaryError> {
        #[cfg(windows)]
        {
            let system_root = std::env::var_os("SystemRoot").ok_or(DesktopBoundaryError)?;
            let explorer = PathBuf::from(system_root).join("explorer.exe");
            Command::new(explorer)
                .arg(format!(r"shell:AppsFolder\{aumid}"))
                .spawn()
                .map(|_| ())
                .map_err(|_| DesktopBoundaryError)
        }
        #[cfg(not(windows))]
        {
            let _ = aumid;
            Err(DesktopBoundaryError)
        }
    }
}

#[derive(Debug, Default)]
struct WindowsDesktopProcessController;

impl DesktopProcessController for WindowsDesktopProcessController {
    fn request_close(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
        #[cfg(windows)]
        {
            request_windows_close(roots)
        }
        #[cfg(not(windows))]
        {
            let _ = roots;
            Err(DesktopBoundaryError)
        }
    }
}

#[cfg(windows)]
fn discover_windows_desktop_packages() -> Result<Vec<DesktopPackage>, DesktopBoundaryError> {
    use std::os::windows::process::CommandExt;

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct PackageRecord {
        name: String,
        family_name: String,
        application_id: String,
        install_location: PathBuf,
    }

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$packages = @(
  Get-AppxPackage -PackageTypeFilter Main |
    Where-Object {
      $_.Name -in @('OpenAI.Codex', 'OpenAI.ChatGPT') -and
      $_.PublisherId -eq '2p2nqsd0c76g0'
    } |
    ForEach-Object {
      $package = $_
      $manifest = Get-AppxPackageManifest -Package $package.PackageFullName
      @($manifest.Package.Applications.Application) | Where-Object {
        [IO.Path]::GetFileNameWithoutExtension([string]$_.Executable) -in @('ChatGPT', 'Codex')
      } | ForEach-Object {
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
    let system_root = std::env::var_os("SystemRoot").ok_or(DesktopBoundaryError)?;
    let powershell = PathBuf::from(system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let output = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|_| DesktopBoundaryError)?;
    if !output.status.success() {
        return Err(DesktopBoundaryError);
    }
    let records = serde_json::from_slice::<Vec<PackageRecord>>(&output.stdout)
        .map_err(|_| DesktopBoundaryError)?;
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

#[cfg(windows)]
fn request_windows_close(roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };

    struct CloseContext {
        pids: HashSet<u32>,
        posted: bool,
        failed: bool,
    }

    unsafe extern "system" fn close_root_window(hwnd: HWND, lparam: LPARAM) -> i32 {
        let context = unsafe { &mut *(lparam as *mut CloseContext) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if context.pids.contains(&pid) {
            context.posted = true;
            if unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) } == 0 {
                context.failed = true;
            }
        }
        1
    }

    let mut handles = Vec::with_capacity(roots.len());
    for root in roots {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, root.pid) };
        if handle.is_null() {
            close_windows_handles(handles);
            return Err(DesktopBoundaryError);
        }
        let mut created = unsafe { std::mem::zeroed() };
        let mut exited = unsafe { std::mem::zeroed() };
        let mut kernel = unsafe { std::mem::zeroed() };
        let mut user = unsafe { std::mem::zeroed() };
        let readable = unsafe {
            GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0
        };
        if !readable || windows_file_time_epoch_millis(created) != root.started_at_epoch_millis {
            unsafe {
                CloseHandle(handle);
            }
            close_windows_handles(handles);
            return Err(DesktopBoundaryError);
        }
        handles.push(handle);
    }

    let mut context = CloseContext {
        pids: roots.iter().map(|root| root.pid).collect(),
        posted: false,
        failed: false,
    };
    let enumerated = unsafe {
        EnumWindows(
            Some(close_root_window),
            (&mut context as *mut CloseContext) as LPARAM,
        )
    };
    close_windows_handles(handles);
    if enumerated == 0 || !context.posted || context.failed {
        Err(DesktopBoundaryError)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn close_windows_handles(handles: Vec<windows_sys::Win32::Foundation::HANDLE>) {
    use windows_sys::Win32::Foundation::CloseHandle;

    for handle in handles {
        unsafe {
            CloseHandle(handle);
        }
    }
}

#[cfg(windows)]
fn windows_file_time_epoch_millis(created: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    let file_time = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
    file_time
        .checked_div(10_000)
        .and_then(|milliseconds| milliseconds.checked_sub(11_644_473_600_000))
        .unwrap_or_default()
}
