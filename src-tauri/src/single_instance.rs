use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, SetEvent,
    WaitForMultipleObjects,
};
use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

const PRIMARY_PID_RETRY_COUNT: usize = 50;
const PRIMARY_PID_RETRY_DELAY: Duration = Duration::from_millis(20);

pub enum InstanceRole {
    Primary(PrimaryInstance),
    Secondary,
}

pub struct PrimaryInstance {
    handles: Arc<InstanceHandles>,
}

pub struct InstanceListener {
    handles: Arc<InstanceHandles>,
    thread: Option<JoinHandle<()>>,
}

struct InstanceHandles {
    _mutex: OwnedHandle,
    activation: OwnedHandle,
    shutdown: OwnedHandle,
    pid_file: PathBuf,
    owner_pid: u32,
}

struct OwnedHandle(HANDLE);

// Win32 kernel handles may be waited on and closed from any thread while owned here.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    fn new(handle: HANDLE) -> io::Result<Self> {
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl Drop for InstanceHandles {
    fn drop(&mut self) {
        remove_pid_file_if_owned(&self.pid_file, self.owner_pid);
    }
}

impl Drop for InstanceListener {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.handles.shutdown.raw());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl PrimaryInstance {
    pub fn listen(
        self,
        mut on_activate: impl FnMut() + Send + 'static,
    ) -> io::Result<InstanceListener> {
        let handles = Arc::clone(&self.handles);
        let listener_handles = Arc::clone(&handles);
        let thread = thread::Builder::new()
            .name("gpteasy-single-instance".to_owned())
            .spawn(move || {
                let wait_handles = [
                    listener_handles.activation.raw(),
                    listener_handles.shutdown.raw(),
                ];
                loop {
                    let result = unsafe {
                        WaitForMultipleObjects(
                            wait_handles.len() as u32,
                            wait_handles.as_ptr(),
                            false.into(),
                            u32::MAX,
                        )
                    };
                    match result {
                        WAIT_OBJECT_0 => on_activate(),
                        value if value == WAIT_OBJECT_0 + 1 || value == WAIT_FAILED => break,
                        _ => break,
                    }
                }
            })?;
        Ok(InstanceListener {
            handles,
            thread: Some(thread),
        })
    }
}

pub fn acquire(executable: &Path) -> io::Result<InstanceRole> {
    let canonical_executable = fs::canonicalize(executable)?;
    let key = installation_key(&canonical_executable, &current_user_sid()?);
    let mutex_name = wide_name(&format!("Global\\GPTEasy-{key}-instance"));
    let activation_name = wide_name(&format!("Global\\GPTEasy-{key}-activate"));
    let mutex = OwnedHandle::new(unsafe {
        CreateMutexW(std::ptr::null(), false.into(), mutex_name.as_ptr())
    })?;
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let activation = OwnedHandle::new(unsafe {
        CreateEventW(
            std::ptr::null(),
            false.into(),
            false.into(),
            activation_name.as_ptr(),
        )
    })?;
    let pid_file = std::env::temp_dir().join(format!("gpteasy-{key}.pid"));

    if already_running {
        if let Some(primary_pid) = wait_for_primary_pid(&pid_file, &canonical_executable) {
            unsafe {
                AllowSetForegroundWindow(primary_pid);
            }
        }
        if unsafe { SetEvent(activation.raw()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        return Ok(InstanceRole::Secondary);
    }

    let shutdown = OwnedHandle::new(unsafe {
        CreateEventW(
            std::ptr::null(),
            false.into(),
            false.into(),
            std::ptr::null(),
        )
    })?;
    let owner_pid = std::process::id();
    fs::write(&pid_file, owner_pid.to_string())?;

    Ok(InstanceRole::Primary(PrimaryInstance {
        handles: Arc::new(InstanceHandles {
            _mutex: mutex,
            activation,
            shutdown,
            pid_file,
            owner_pid,
        }),
    }))
}

fn installation_key(executable: &Path, user_sid: &[u8]) -> String {
    let normalized = executable.to_string_lossy().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-single-instance-v2\0");
    hasher.update(user_sid);
    hasher.update(b"\0");
    for unit in normalized.encode_utf16() {
        hasher.update(unit.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn current_user_sid() -> io::Result<Vec<u8>> {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle::new(token)?;
    let mut required_length = 0;
    unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut required_length,
        );
    }
    if required_length == 0 {
        return Err(io::Error::last_os_error());
    }
    let word_count = (required_length as usize).div_ceil(size_of::<usize>());
    let mut buffer = vec![0usize; word_count];
    if unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required_length,
            &mut required_length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let token_user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let sid_length = unsafe { GetLengthSid(token_user.User.Sid) } as usize;
    if sid_length == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { std::slice::from_raw_parts(token_user.User.Sid.cast(), sid_length) }.to_vec())
}

fn wait_for_primary_pid(pid_file: &Path, executable: &Path) -> Option<u32> {
    for _ in 0..PRIMARY_PID_RETRY_COUNT {
        if let Some(pid) = read_primary_pid(pid_file)
            && process_executable(pid).is_some_and(|path| same_executable(&path, executable))
        {
            return Some(pid);
        }
        thread::sleep(PRIMARY_PID_RETRY_DELAY);
    }
    None
}

fn read_primary_pid(pid_file: &Path) -> Option<u32> {
    fs::read_to_string(pid_file).ok()?.trim().parse().ok()
}

fn process_executable(pid: u32) -> Option<PathBuf> {
    let process = OwnedHandle::new(unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false.into(), pid)
    })
    .ok()?;
    let mut path = vec![0u16; 32_768];
    let mut path_length = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process.raw(), 0, path.as_mut_ptr(), &mut path_length) }
        == 0
    {
        return None;
    }
    path.truncate(path_length as usize);
    Some(PathBuf::from(String::from_utf16_lossy(&path)))
}

fn same_executable(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy()),
        _ => false,
    }
}

fn remove_pid_file_if_owned(pid_file: &Path, owner_pid: u32) {
    if read_primary_pid(pid_file) == Some(owner_pid) {
        let _ = fs::remove_file(pid_file);
    }
}

fn wide_name(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
