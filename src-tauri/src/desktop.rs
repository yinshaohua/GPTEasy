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
    fn terminate_tree(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError>;
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
    TerminationFailed,
    TerminationTimedOut,
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
    close_scan_attempts: usize,
    launch_scan_attempts: usize,
    scan_delay: Duration,
}

impl DesktopApplication {
    pub fn new() -> Self {
        Self::with_polling_boundaries(
            Arc::new(WindowsDesktopPackageDiscovery),
            Arc::new(WindowsConsumerScanner::new()),
            Arc::new(WindowsDesktopActivator),
            Arc::new(WindowsDesktopProcessController),
            Arc::new(SystemDesktopClock),
            4,
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
        Self::with_polling_boundaries(
            discovery,
            scanner,
            activator,
            process_controller,
            clock,
            scan_attempts,
            scan_attempts,
            scan_delay,
        )
    }

    pub fn with_polling_boundaries(
        discovery: Arc<dyn DesktopPackageDiscovery>,
        scanner: Arc<dyn ConsumerScanner>,
        activator: Arc<dyn DesktopActivator>,
        process_controller: Arc<dyn DesktopProcessController>,
        clock: Arc<dyn DesktopClock>,
        close_scan_attempts: usize,
        launch_scan_attempts: usize,
        scan_delay: Duration,
    ) -> Self {
        Self {
            discovery,
            scanner,
            activator,
            process_controller,
            clock,
            close_scan_attempts,
            launch_scan_attempts,
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
        self.activate_and_observe(&package, &before.desktop_roots, "desktop.running")
    }

    pub fn restart(
        &self,
        expected_roots: &[ConsumerIdentity],
    ) -> Result<DesktopSnapshot, DesktopFailure> {
        self.restart_with_checkpoint(expected_roots, || Ok(()))
    }

    pub fn restart_with_checkpoint<F>(
        &self,
        expected_roots: &[ConsumerIdentity],
        checkpoint: F,
    ) -> Result<DesktopSnapshot, DesktopFailure>
    where
        F: FnOnce() -> Result<(), DesktopFailure>,
    {
        let package = self.discover_package().map_err(action_unavailable)?;
        let before = self.scan_for_package(&package);
        if before.desktop == ConsumerStatus::Stopped {
            return self.activate_after_checkpoint(
                &package,
                expected_roots,
                "desktop.running",
                checkpoint,
            );
        }
        if before.desktop != ConsumerStatus::Running
            || before.desktop_roots.is_empty()
            || before.desktop_roots != expected_roots
        {
            return Err(desktop_failure(
                DesktopFailureCategory::IdentityChanged,
                "desktop.identity_changed",
            ));
        }
        let close_requested = self
            .process_controller
            .request_close(&before.desktop_roots)
            .is_ok();
        if close_requested && self.wait_for_roots_to_exit(&package, expected_roots) {
            return self.activate_after_checkpoint(
                &package,
                expected_roots,
                "desktop.restarted_after_normal_exit",
                checkpoint,
            );
        }

        let before_termination = self.scan_for_package(&package);
        if before_termination.desktop == ConsumerStatus::Stopped {
            return self.activate_after_checkpoint(
                &package,
                expected_roots,
                "desktop.restarted_after_normal_exit",
                checkpoint,
            );
        }
        if before_termination.desktop != ConsumerStatus::Running
            || before_termination.desktop_roots != expected_roots
        {
            return Err(desktop_failure(
                DesktopFailureCategory::IdentityChanged,
                "desktop.identity_changed",
            ));
        }
        self.process_controller
            .terminate_tree(&before_termination.desktop_roots)
            .map_err(|_| {
                desktop_failure(
                    DesktopFailureCategory::TerminationFailed,
                    "desktop.termination_failed",
                )
            })?;
        if !self.wait_for_roots_to_exit(&package, expected_roots) {
            return Err(desktop_failure(
                DesktopFailureCategory::TerminationTimedOut,
                "desktop.termination_timed_out",
            ));
        }
        self.activate_after_checkpoint(
            &package,
            expected_roots,
            "desktop.restarted_after_termination",
            checkpoint,
        )
    }

    fn activate_after_checkpoint<F>(
        &self,
        package: &DesktopPackage,
        previous_roots: &[ConsumerIdentity],
        success_message_id: &'static str,
        checkpoint: F,
    ) -> Result<DesktopSnapshot, DesktopFailure>
    where
        F: FnOnce() -> Result<(), DesktopFailure>,
    {
        checkpoint()?;
        self.activate_and_observe(package, previous_roots, success_message_id)
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
        for attempt in 0..self.close_scan_attempts.max(1) {
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
        success_message_id: &'static str,
    ) -> Result<DesktopSnapshot, DesktopFailure> {
        let activation_started_at = self.clock.now_epoch_millis();
        self.activator.activate(&package.aumid()).map_err(|_| {
            desktop_failure(
                DesktopFailureCategory::ActivationFailed,
                "desktop.activation_failed",
            )
        })?;
        for attempt in 0..self.launch_scan_attempts.max(1) {
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
                    success_message_id,
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

    fn terminate_tree(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
        #[cfg(windows)]
        {
            terminate_windows_process_trees(roots)
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
    use std::collections::HashMap;

    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowThreadProcessId,
        IsWindowVisible, PostMessageW, WM_CLOSE,
    };

    struct CloseContext {
        pids: HashSet<u32>,
        candidates: HashMap<u32, (HWND, u8)>,
    }

    unsafe extern "system" fn collect_root_window(hwnd: HWND, lparam: LPARAM) -> i32 {
        let context = unsafe { &mut *(lparam as *mut CloseContext) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if context.pids.contains(&pid) {
            let mut class_name = [0_u16; 64];
            let class_name_length =
                unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
            if class_name_length > 0
                && is_desktop_close_window_class(&String::from_utf16_lossy(
                    &class_name[..class_name_length as usize],
                ))
            {
                let score = (unsafe { IsWindowVisible(hwnd) } != 0) as u8 * 2
                    + (unsafe { GetWindowTextLengthW(hwnd) } > 0) as u8;
                let current_score = context
                    .candidates
                    .get(&pid)
                    .map(|(_, score)| *score)
                    .unwrap_or_default();
                if score > current_score || !context.candidates.contains_key(&pid) {
                    context.candidates.insert(pid, (hwnd, score));
                }
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
        candidates: HashMap::new(),
    };
    let enumerated = unsafe {
        EnumWindows(
            Some(collect_root_window),
            (&mut context as *mut CloseContext) as LPARAM,
        )
    };
    if enumerated == 0 || context.candidates.len() != roots.len() {
        close_windows_handles(handles);
        return Err(DesktopBoundaryError);
    }
    let failed = context
        .candidates
        .values()
        .any(|(hwnd, _)| unsafe { PostMessageW(*hwnd, WM_CLOSE, 0, 0) } == 0);
    close_windows_handles(handles);
    if failed {
        Err(DesktopBoundaryError)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn terminate_windows_process_trees(roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
    use std::collections::HashMap;
    use std::mem::size_of;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, GetLastError,
        INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        TerminateProcess, WaitForSingleObject,
    };

    if roots.is_empty()
        || roots
            .iter()
            .any(|root| root.role != crate::consumer::ConsumerRole::Desktop)
    {
        return Err(DesktopBoundaryError);
    }

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(DesktopBoundaryError);
    }
    let mut parents = HashMap::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { Process32FirstW(snapshot, &mut entry) } == 0 {
        unsafe {
            CloseHandle(snapshot);
        }
        return Err(DesktopBoundaryError);
    }
    loop {
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if unsafe { Process32NextW(snapshot, &mut entry) } != 0 {
            continue;
        }
        let complete = unsafe { GetLastError() } == ERROR_NO_MORE_FILES;
        unsafe {
            CloseHandle(snapshot);
        }
        if !complete {
            return Err(DesktopBoundaryError);
        }
        break;
    }

    let snapshot_epoch_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);
    let root_pids = roots.iter().map(|root| root.pid).collect::<HashSet<_>>();
    let mut descendants = parents
        .keys()
        .filter(|pid| !root_pids.contains(pid))
        .filter_map(|pid| process_tree_depth(*pid, &parents, &root_pids).map(|depth| (*pid, depth)))
        .collect::<Vec<_>>();
    descendants.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));

    let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE;
    let mut descendant_handles = Vec::with_capacity(descendants.len());
    for (pid, _) in descendants {
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                continue;
            }
            close_windows_handles(descendant_handles);
            return Err(DesktopBoundaryError);
        }
        let Some(created_at) = windows_process_created_at(handle) else {
            unsafe {
                CloseHandle(handle);
            }
            close_windows_handles(descendant_handles);
            return Err(DesktopBoundaryError);
        };
        if created_at > snapshot_epoch_millis {
            unsafe {
                CloseHandle(handle);
            }
            close_windows_handles(descendant_handles);
            return Err(DesktopBoundaryError);
        }
        descendant_handles.push(handle);
    }

    let mut root_handles = Vec::with_capacity(roots.len());
    for root in roots {
        let handle = unsafe { OpenProcess(access, 0, root.pid) };
        if handle.is_null()
            || windows_process_created_at(handle) != Some(root.started_at_epoch_millis)
        {
            if !handle.is_null() {
                unsafe {
                    CloseHandle(handle);
                }
            }
            close_windows_handles(descendant_handles);
            close_windows_handles(root_handles);
            return Err(DesktopBoundaryError);
        }
        root_handles.push(handle);
    }

    let mut failed = false;
    for handle in descendant_handles.iter().chain(root_handles.iter()) {
        let terminated = unsafe { TerminateProcess(*handle, 1) } != 0;
        if !terminated && unsafe { WaitForSingleObject(*handle, 0) } != WAIT_OBJECT_0 {
            failed = true;
        }
    }
    close_windows_handles(descendant_handles);
    close_windows_handles(root_handles);
    if failed {
        Err(DesktopBoundaryError)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn process_tree_depth(
    pid: u32,
    parents: &std::collections::HashMap<u32, u32>,
    roots: &HashSet<u32>,
) -> Option<usize> {
    let mut current = pid;
    let mut visited = HashSet::new();
    let mut depth = 0;
    while visited.insert(current) {
        let parent = *parents.get(&current)?;
        depth += 1;
        if roots.contains(&parent) {
            return Some(depth);
        }
        if parent == 0 {
            return None;
        }
        current = parent;
    }
    None
}

#[cfg(windows)]
fn windows_process_created_at(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<u64> {
    use windows_sys::Win32::System::Threading::GetProcessTimes;

    let mut created = unsafe { std::mem::zeroed() };
    let mut exited = unsafe { std::mem::zeroed() };
    let mut kernel = unsafe { std::mem::zeroed() };
    let mut user = unsafe { std::mem::zeroed() };
    (unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } != 0)
        .then(|| windows_file_time_epoch_millis(created))
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

fn is_desktop_close_window_class(class_name: &str) -> bool {
    class_name == "Chrome_WidgetWin_1"
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::collections::{HashMap, HashSet};

    use super::is_desktop_close_window_class;
    #[cfg(windows)]
    use super::process_tree_depth;

    #[test]
    fn only_the_electron_main_window_is_a_close_target() {
        assert!(is_desktop_close_window_class("Chrome_WidgetWin_1"));
        assert!(!is_desktop_close_window_class("Chrome_WidgetWin_0"));
        assert!(!is_desktop_close_window_class("IME"));
        assert!(!is_desktop_close_window_class("SoPY_Status"));
    }

    #[cfg(windows)]
    #[test]
    fn process_tree_depth_includes_only_descendants_of_the_confirmed_roots() {
        let parents = HashMap::from([(10, 1), (11, 10), (12, 11), (20, 1), (21, 20)]);
        let roots = HashSet::from([10]);

        assert_eq!(process_tree_depth(11, &parents, &roots), Some(1));
        assert_eq!(process_tree_depth(12, &parents, &roots), Some(2));
        assert_eq!(process_tree_depth(20, &parents, &roots), None);
        assert_eq!(process_tree_depth(21, &parents, &roots), None);
    }
}
