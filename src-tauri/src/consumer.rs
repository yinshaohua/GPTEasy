use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
pub struct ConsumerProcessExclusion {
    pub pid: u32,
    pub started_at_epoch_millis: u64,
    pub executable: PathBuf,
}

impl ConsumerProcessExclusion {
    pub fn from_windows_process_creation_time(
        pid: u32,
        process_created_at: i64,
        executable: impl Into<PathBuf>,
    ) -> Option<Self> {
        let file_time = u64::try_from(process_created_at).ok()?;
        let started_at_epoch_millis = file_time
            .checked_div(10_000)?
            .checked_sub(11_644_473_600_000)?;
        Some(Self {
            pid,
            started_at_epoch_millis,
            executable: executable.into(),
        })
    }
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

    fn scan_for_install_locations(&self, _install_locations: &[PathBuf]) -> ConsumerScan {
        self.scan()
    }

    fn scan_excluding(&self, _exclusions: &[ConsumerProcessExclusion]) -> ConsumerScan {
        self.scan()
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
            scan_windows(None, &[]).unwrap_or_else(|_| ConsumerScan::unknown())
        }
        #[cfg(not(windows))]
        {
            ConsumerScan::unknown()
        }
    }

    fn scan_excluding(&self, exclusions: &[ConsumerProcessExclusion]) -> ConsumerScan {
        #[cfg(windows)]
        {
            scan_windows(None, exclusions).unwrap_or_else(|_| ConsumerScan::unknown())
        }
        #[cfg(not(windows))]
        {
            let _ = exclusions;
            ConsumerScan::unknown()
        }
    }

    fn scan_for_install_locations(&self, install_locations: &[PathBuf]) -> ConsumerScan {
        #[cfg(windows)]
        {
            scan_windows(Some(install_locations), &[]).unwrap_or_else(|_| ConsumerScan::unknown())
        }
        #[cfg(not(windows))]
        {
            let _ = install_locations;
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
    classify_processes(processes, None, &[])
}

pub fn classify_fixture_for_packages(
    processes: &[FixtureProcess],
    install_locations: &[PathBuf],
) -> ConsumerScan {
    classify_processes(processes, Some(install_locations), &[])
}

pub fn classify_fixture_with_exclusions(
    processes: &[FixtureProcess],
    exclusions: &[ConsumerProcessExclusion],
) -> ConsumerScan {
    classify_processes(processes, None, exclusions)
}

fn classify_processes(
    processes: &[FixtureProcess],
    install_locations: Option<&[PathBuf]>,
    exclusions: &[ConsumerProcessExclusion],
) -> ConsumerScan {
    let all_available = processes
        .iter()
        .filter(|process| process.access == ProcessAccess::Available)
        .collect::<Vec<_>>();
    let all_by_pid = all_available
        .iter()
        .map(|process| (process.pid, *process))
        .collect::<HashMap<_, _>>();
    let excluded_roots = all_available
        .iter()
        .filter(|process| {
            exclusions.iter().any(|exclusion| {
                process.pid == exclusion.pid
                    && process.started_at_epoch_millis == exclusion.started_at_epoch_millis
                    && normalized_windows_path(&process.executable)
                        == normalized_windows_path(&exclusion.executable)
            })
        })
        .map(|process| process.pid)
        .collect::<HashSet<_>>();
    let available = all_available
        .into_iter()
        .filter(|process| {
            !excluded_roots.contains(&process.pid)
                && !has_ancestor(process, &all_by_pid, &excluded_roots)
        })
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

fn has_ancestor(
    process: &&FixtureProcess,
    by_pid: &HashMap<u32, &FixtureProcess>,
    roots: &HashSet<u32>,
) -> bool {
    let mut parent = process.parent_pid;
    let mut visited = HashSet::new();
    while parent != 0 && visited.insert(parent) {
        if roots.contains(&parent) {
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
fn scan_windows(
    install_locations: Option<&[PathBuf]>,
    exclusions: &[ConsumerProcessExclusion],
) -> Result<ConsumerScan, ()> {
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
    Ok(classify_processes(
        &processes,
        install_locations,
        exclusions,
    ))
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
