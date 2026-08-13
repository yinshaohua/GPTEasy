use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
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

    fn scan_for_packages(&self, _packages: &[DesktopPackage]) -> ConsumerScan {
        self.scan()
    }
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
    fn discover(&self) -> Result<Vec<DesktopPackage>, DesktopBoundaryError>;
}

pub trait DesktopActivator: Send + Sync {
    fn activate(&self, aumid: &str) -> Result<(), DesktopBoundaryError>;
}

pub trait DesktopProcessController: Send + Sync {
    fn request_close(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError>;
    fn force_terminate(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopBoundaryError;

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
pub enum DesktopRestartStatus {
    Restarted,
    CloseTimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopRestartResult {
    pub status: DesktopRestartStatus,
    pub message_id: &'static str,
    pub desktop_identities: Vec<ConsumerIdentity>,
    pub force_authorization: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopFailureCategory {
    ActionUnavailable,
    CloseFailed,
    ForceTerminateFailed,
    IdentityChanged,
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
    force_authorizations: Arc<Mutex<HashMap<String, Vec<ConsumerIdentity>>>>,
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
            force_authorizations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn inspect(&self) -> DesktopSnapshot {
        match self.discover_package() {
            Ok(package) => {
                let packages = [package];
                let scan = self.scanner.scan_for_packages(&packages);
                match scan.desktop {
                    ConsumerStatus::Unknown => desktop_snapshot(
                        ConsumerStatus::Unknown,
                        DesktopAction::Unavailable,
                        "desktop.identity_untrusted",
                        Vec::new(),
                    ),
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
                }
            }
            Err(message_id) => desktop_snapshot(
                if message_id == "desktop.discovery_failed" {
                    ConsumerStatus::Unknown
                } else {
                    ConsumerStatus::Stopped
                },
                DesktopAction::Unavailable,
                message_id,
                Vec::new(),
            ),
        }
    }

    pub fn start(&self) -> Result<DesktopSnapshot, DesktopFailure> {
        let package = self.discover_package().map_err(|message_id| {
            desktop_failure(DesktopFailureCategory::ActionUnavailable, message_id)
        })?;
        let packages = [package];
        let before = self.scanner.scan_for_packages(&packages);
        if before.desktop != ConsumerStatus::Stopped {
            return Err(desktop_failure(
                DesktopFailureCategory::ActionUnavailable,
                "desktop.action_unavailable",
            ));
        }
        let activation_started_at = self.clock.now_epoch_millis();
        self.activator.activate(&packages[0].aumid()).map_err(|_| {
            desktop_failure(
                DesktopFailureCategory::ActivationFailed,
                "desktop.activation_failed",
            )
        })?;
        for attempt in 0..self.scan_attempts.max(1) {
            if attempt > 0 && !self.scan_delay.is_zero() {
                thread::sleep(self.scan_delay);
            }
            let after = self.scanner.scan_for_packages(&packages);
            if after.desktop == ConsumerStatus::Running
                && after.desktop_roots.iter().any(|root| {
                    root.started_at_epoch_millis >= activation_started_at
                        && !before.desktop_roots.contains(root)
                })
            {
                return Ok(desktop_snapshot(
                    ConsumerStatus::Running,
                    DesktopAction::Restart,
                    "desktop.running",
                    after.desktop_roots,
                ));
            }
        }
        Err(desktop_failure(
            DesktopFailureCategory::LaunchNotObserved,
            "desktop.launch_not_observed",
        ))
    }

    pub fn restart(
        &self,
        expected_roots: &[ConsumerIdentity],
    ) -> Result<DesktopRestartResult, DesktopFailure> {
        let package = self.discover_package().map_err(|message_id| {
            desktop_failure(DesktopFailureCategory::ActionUnavailable, message_id)
        })?;
        let packages = [package];
        let before = self.scanner.scan_for_packages(&packages);
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
        let desktop_tree = desktop_identities(&before);
        if !self.wait_for_identities_to_exit(&packages, &desktop_tree) {
            let force_authorization = uuid::Uuid::new_v4().to_string();
            self.force_authorizations
                .lock()
                .map_err(|_| {
                    desktop_failure(
                        DesktopFailureCategory::ActionUnavailable,
                        "desktop.state_unavailable",
                    )
                })?
                .insert(force_authorization.clone(), desktop_tree.clone());
            return Ok(desktop_restart_result(
                DesktopRestartStatus::CloseTimedOut,
                "desktop.close_timed_out",
                desktop_tree,
                Some(force_authorization),
            ));
        }
        self.activate_and_observe(&packages)
    }

    pub fn force_restart(
        &self,
        force_authorization: &str,
    ) -> Result<DesktopRestartResult, DesktopFailure> {
        let expected_identities = self
            .force_authorizations
            .lock()
            .map_err(|_| {
                desktop_failure(
                    DesktopFailureCategory::ActionUnavailable,
                    "desktop.state_unavailable",
                )
            })?
            .remove(force_authorization)
            .ok_or_else(|| {
                desktop_failure(
                    DesktopFailureCategory::ActionUnavailable,
                    "desktop.force_not_authorized",
                )
            })?;
        let package = self.discover_package().map_err(|message_id| {
            desktop_failure(DesktopFailureCategory::ActionUnavailable, message_id)
        })?;
        let packages = [package];
        let current = self.scanner.scan_for_packages(&packages);
        let current_desktop_tree = desktop_identities(&current);
        if expected_identities.is_empty()
            || current.desktop != ConsumerStatus::Running
            || current_desktop_tree != expected_identities
        {
            return Err(desktop_failure(
                DesktopFailureCategory::IdentityChanged,
                "desktop.identity_changed",
            ));
        }
        self.process_controller
            .force_terminate(&expected_identities)
            .map_err(|_| {
                desktop_failure(
                    DesktopFailureCategory::ForceTerminateFailed,
                    "desktop.force_terminate_failed",
                )
            })?;
        if !self.wait_for_identities_to_exit(&packages, &expected_identities) {
            return Err(desktop_failure(
                DesktopFailureCategory::ForceTerminateFailed,
                "desktop.force_terminate_failed",
            ));
        }
        self.activate_and_observe(&packages)
    }

    fn wait_for_identities_to_exit(
        &self,
        packages: &[DesktopPackage],
        identities: &[ConsumerIdentity],
    ) -> bool {
        for attempt in 0..self.scan_attempts.max(1) {
            if attempt > 0 && !self.scan_delay.is_zero() {
                thread::sleep(self.scan_delay);
            }
            let scan = self.scanner.scan_for_packages(packages);
            if scan.desktop != ConsumerStatus::Unknown
                && !desktop_identities(&scan)
                    .iter()
                    .any(|identity| identities.contains(identity))
            {
                return true;
            }
        }
        false
    }

    fn activate_and_observe(
        &self,
        packages: &[DesktopPackage],
    ) -> Result<DesktopRestartResult, DesktopFailure> {
        let activation_started_at = self.clock.now_epoch_millis();
        self.activator.activate(&packages[0].aumid()).map_err(|_| {
            desktop_failure(
                DesktopFailureCategory::ActivationFailed,
                "desktop.activation_failed",
            )
        })?;
        for attempt in 0..self.scan_attempts.max(1) {
            if attempt > 0 && !self.scan_delay.is_zero() {
                thread::sleep(self.scan_delay);
            }
            let scan = self.scanner.scan_for_packages(packages);
            if scan.desktop == ConsumerStatus::Running
                && scan
                    .desktop_roots
                    .iter()
                    .any(|root| root.started_at_epoch_millis >= activation_started_at)
            {
                return Ok(desktop_restart_result(
                    DesktopRestartStatus::Restarted,
                    "desktop.restart_succeeded",
                    Vec::new(),
                    None,
                ));
            }
        }
        Err(desktop_failure(
            DesktopFailureCategory::LaunchNotObserved,
            "desktop.launch_not_observed",
        ))
    }

    fn discover_package(&self) -> Result<DesktopPackage, &'static str> {
        let packages = self
            .discovery
            .discover()
            .map_err(|_| "desktop.discovery_failed")?;
        resolve_desktop_package(packages)
    }
}

const OPENAI_WINDOWS_PUBLISHER_ID: &str = "2p2nqsd0c76g0";

fn resolve_desktop_package(packages: Vec<DesktopPackage>) -> Result<DesktopPackage, &'static str> {
    if packages.is_empty() {
        return Err("desktop.not_installed");
    }
    if packages.iter().any(|package| {
        package
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

impl Default for DesktopApplication {
    fn default() -> Self {
        Self::new()
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

fn desktop_restart_result(
    status: DesktopRestartStatus,
    message_id: &'static str,
    desktop_identities: Vec<ConsumerIdentity>,
    force_authorization: Option<String>,
) -> DesktopRestartResult {
    DesktopRestartResult {
        status,
        message_id,
        desktop_identities,
        force_authorization,
    }
}

fn desktop_identities(scan: &ConsumerScan) -> Vec<ConsumerIdentity> {
    scan.identities
        .iter()
        .filter(|identity| {
            identity.role == ConsumerRole::Desktop && !scan.desktop_roots.contains(identity)
        })
        .cloned()
        .chain(scan.desktop_roots.iter().cloned())
        .collect()
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
            Command::new("explorer.exe")
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

    fn force_terminate(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
        #[cfg(windows)]
        {
            force_terminate_windows_processes(roots)
        }
        #[cfg(not(windows))]
        {
            let _ = roots;
            Err(DesktopBoundaryError)
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
            scan_windows(None).unwrap_or_else(|_| ConsumerScan::unknown())
        }
        #[cfg(not(windows))]
        {
            ConsumerScan::unknown()
        }
    }

    fn scan_for_packages(&self, packages: &[DesktopPackage]) -> ConsumerScan {
        #[cfg(windows)]
        {
            let install_locations = packages
                .iter()
                .map(|package| package.install_location.clone())
                .collect::<Vec<_>>();
            scan_windows(Some(&install_locations)).unwrap_or_else(|_| ConsumerScan::unknown())
        }
        #[cfg(not(windows))]
        {
            let _ = packages;
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
    classify_processes(processes, None)
}

pub fn classify_fixture_for_packages(
    processes: &[FixtureProcess],
    install_locations: &[PathBuf],
) -> ConsumerScan {
    classify_processes(processes, Some(install_locations))
}

fn classify_processes(
    processes: &[FixtureProcess],
    install_locations: Option<&[PathBuf]>,
) -> ConsumerScan {
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
        .filter(|process| is_desktop_root(process, install_locations))
        .map(|process| process.pid)
        .collect::<HashSet<_>>();
    let desktop_children = available
        .iter()
        .filter(|process| {
            !desktop_roots.contains(&process.pid)
                && is_discovered_openai_desktop(&process.executable, install_locations)
                && has_desktop_ancestor(process, &by_pid, &desktop_roots)
        })
        .map(|process| process.pid)
        .collect::<HashSet<_>>();
    let orphaned_bundled = available.iter().any(|process| {
        is_bundled_codex(process, install_locations)
            && !desktop_roots.contains(&process.pid)
            && !has_desktop_ancestor(process, &by_pid, &desktop_roots)
    });
    let cli = available
        .iter()
        .filter(|process| {
            is_codex_name(&process.name)
                && !is_bundled_codex(process, install_locations)
                && !process.electron_helper
                && !desktop_roots.contains(&process.pid)
                && !has_desktop_ancestor(process, &by_pid, &desktop_roots)
                && is_trusted_cli_path(&process.executable)
        })
        .collect::<Vec<_>>();
    let untrusted_cli = available.iter().any(|process| {
        is_codex_name(&process.name)
            && !is_bundled_codex(process, install_locations)
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
    let mut root_identities = desktop_roots
        .iter()
        .filter_map(|pid| by_pid.get(pid))
        .map(|process| ConsumerIdentity {
            role: ConsumerRole::Desktop,
            pid: process.pid,
            started_at_epoch_millis: process.started_at_epoch_millis,
        })
        .collect::<Vec<_>>();
    root_identities.sort_by_key(|identity| identity.pid);
    let mut identities = root_identities
        .iter()
        .cloned()
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
        desktop: if desktop_denied || orphaned_bundled {
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
        desktop_roots: root_identities,
    }
}

#[cfg(windows)]
fn discover_windows_desktop_packages() -> Result<Vec<DesktopPackage>, DesktopBoundaryError> {
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
    Where-Object {
      $_.Name -in @('OpenAI.Codex', 'OpenAI.ChatGPT') -and
      $_.PublisherId -eq '2p2nqsd0c76g0'
    } |
    ForEach-Object {
      $package = $_
      $manifest = Get-AppxPackageManifest -Package $package.PackageFullName
      @($manifest.Package.Applications.Application) | ForEach-Object {
        $_
      } | Where-Object {
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

    let handles = open_verified_windows_processes(roots, false)?;
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
    for handle in handles {
        unsafe {
            CloseHandle(handle);
        }
    }
    if enumerated == 0 || !context.posted || context.failed {
        return Err(DesktopBoundaryError);
    }
    Ok(())
}

#[cfg(windows)]
fn force_terminate_windows_processes(
    identities: &[ConsumerIdentity],
) -> Result<(), DesktopBoundaryError> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::TerminateProcess;

    let handles = open_verified_windows_processes(identities, true)?;

    let mut failed = false;
    for handle in handles {
        let terminated = unsafe { TerminateProcess(handle, 1) != 0 };
        unsafe {
            CloseHandle(handle);
        }
        if !terminated {
            failed = true;
        }
    }
    if failed {
        Err(DesktopBoundaryError)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn open_verified_windows_processes(
    identities: &[ConsumerIdentity],
    terminate: bool,
) -> Result<Vec<windows_sys::Win32::Foundation::HANDLE>, DesktopBoundaryError> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    let access = if terminate {
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE
    } else {
        PROCESS_QUERY_LIMITED_INFORMATION
    };
    let mut handles = Vec::with_capacity(identities.len());
    for identity in identities {
        let handle = unsafe { OpenProcess(access, 0, identity.pid) };
        if handle.is_null() {
            close_windows_handles(handles);
            return Err(DesktopBoundaryError);
        }
        let mut created = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        let readable = unsafe {
            GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0
        };
        if !readable || windows_file_time_epoch_millis(created) != identity.started_at_epoch_millis
        {
            unsafe {
                CloseHandle(handle);
            }
            close_windows_handles(handles);
            return Err(DesktopBoundaryError);
        }
        handles.push(handle);
    }
    Ok(handles)
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

fn is_desktop_root(process: &&FixtureProcess, install_locations: Option<&[PathBuf]>) -> bool {
    is_desktop_name(&process.name)
        && is_discovered_openai_desktop(&process.executable, install_locations)
        && !is_bundled_codex(process, install_locations)
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

fn is_discovered_openai_desktop(path: &Path, install_locations: Option<&[PathBuf]>) -> bool {
    if let Some(install_locations) = install_locations {
        return install_locations
            .iter()
            .any(|install_location| path_is_within(path, install_location));
    }
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.contains("\\windowsapps\\openai.codex_")
        || normalized.contains("\\windowsapps\\openai.chatgpt_")
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path = normalized_windows_path(path);
    let mut root = normalized_windows_path(root);
    root.push('\\');
    path.starts_with(&root)
}

fn normalized_windows_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn is_bundled_codex(process: &FixtureProcess, install_locations: Option<&[PathBuf]>) -> bool {
    let normalized = process
        .executable
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.contains("\\resources\\codex\\")
        && is_codex_name(&process.name)
        && is_discovered_openai_desktop(&process.executable, install_locations)
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
fn scan_windows(install_locations: Option<&[PathBuf]>) -> Result<ConsumerScan, ()> {
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
    Ok(classify_processes(&processes, install_locations))
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
    let started_at_epoch_millis = windows_file_time_epoch_millis(created);
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
