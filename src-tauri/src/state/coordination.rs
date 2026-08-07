use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Write},
    path::Path,
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const LOCK_FILENAME: &str = "state.lock";
pub const OWNER_FILENAME: &str = "state-lock-owner.json";
const OWNER_TEMP_FILENAME: &str = "state-lock-owner.json.tmp";
const OWNER_SCHEMA: &str = "gpteasy.state-coordinator-owner.v1";
const LOCK_WAIT_TIMEOUT: Duration = Duration::from_millis(500);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(25);
const RUN_ID_DIGEST_DOMAIN: &[u8] = b"gpteasy-state-coordinator-run-id-v1\0";

static PROCESS_START_TOKEN: OnceLock<String> = OnceLock::new();

#[derive(Debug, Error)]
pub enum CoordinationError {
    #[error("the local state is busy in another process")]
    Busy,
    #[error("failed to access the state coordination lock")]
    Lock(#[source] io::Error),
    #[error("the state coordination path is not a regular local file")]
    UnsafePath,
    #[error("failed to encode state coordination owner metadata")]
    EncodeOwner(#[source] serde_json::Error),
    #[error("failed to persist state coordination owner metadata")]
    PersistOwner(#[source] io::Error),
}

#[derive(Debug, Serialize)]
struct OwnerMetadata<'a> {
    schema: &'static str,
    pid: u32,
    process_start_token: &'a str,
    run_id_digest: String,
}

pub struct StateCoordinator {
    lock_file: File,
}

impl StateCoordinator {
    pub fn acquire(state_root: &Path, run_id: Option<&str>) -> Result<Self, CoordinationError> {
        let lock_path = state_root.join(LOCK_FILENAME);
        reject_non_file_if_present(&lock_path)?;
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(CoordinationError::Lock)?;
        reject_non_file_if_present(&lock_path)?;

        acquire_with_timeout(&lock_file)?;
        let coordinator = Self { lock_file };
        if let Err(error) = write_owner_metadata(state_root, run_id) {
            drop(coordinator);
            return Err(error);
        }
        Ok(coordinator)
    }
}

impl Drop for StateCoordinator {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
    }
}

fn acquire_with_timeout(file: &File) -> Result<(), CoordinationError> {
    let deadline = Instant::now() + LOCK_WAIT_TIMEOUT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(TryLockError::WouldBlock) => return Err(CoordinationError::Busy),
            Err(TryLockError::Error(error)) => return Err(CoordinationError::Lock(error)),
        }
    }
}

fn process_start_token() -> &'static str {
    PROCESS_START_TOKEN
        .get_or_init(|| Uuid::new_v4().to_string())
        .as_str()
}

fn run_id_digest(run_id: Option<&str>, start_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(RUN_ID_DIGEST_DOMAIN);
    match run_id {
        Some(run_id) => {
            hasher.update([1]);
            hasher.update((run_id.len() as u64).to_be_bytes());
            hasher.update(run_id.as_bytes());
        }
        None => {
            hasher.update([0]);
            hasher.update((start_token.len() as u64).to_be_bytes());
            hasher.update(start_token.as_bytes());
        }
    }
    lowercase_hex(&hasher.finalize())
}

fn write_owner_metadata(state_root: &Path, run_id: Option<&str>) -> Result<(), CoordinationError> {
    let owner_path = state_root.join(OWNER_FILENAME);
    let temporary_path = state_root.join(OWNER_TEMP_FILENAME);
    reject_non_file_if_present(&owner_path)?;
    remove_plain_temporary_if_present(&temporary_path)?;

    let start_token = process_start_token();
    let owner = OwnerMetadata {
        schema: OWNER_SCHEMA,
        pid: std::process::id(),
        process_start_token: start_token,
        run_id_digest: run_id_digest(run_id, start_token),
    };
    let mut bytes = serde_json::to_vec(&owner).map_err(CoordinationError::EncodeOwner)?;
    bytes.push(b'\n');

    let mut temporary = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(CoordinationError::PersistOwner)?;
    if let Err(error) = temporary
        .write_all(&bytes)
        .and_then(|_| temporary.sync_all())
    {
        drop(temporary);
        let _ = fs::remove_file(&temporary_path);
        return Err(CoordinationError::PersistOwner(error));
    }
    drop(temporary);

    let result = if owner_path.exists() {
        atomic_replace(&owner_path, &temporary_path)
    } else {
        fs::rename(&temporary_path, &owner_path)
    };
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary_path);
        return Err(CoordinationError::PersistOwner(error));
    }
    Ok(())
}

fn reject_non_file_if_present(path: &Path) -> Result<(), CoordinationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(())
        }
        Ok(_) => Err(CoordinationError::UnsafePath),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoordinationError::Lock(error)),
    }
}

fn remove_plain_temporary_if_present(path: &Path) -> Result<(), CoordinationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(path).map_err(CoordinationError::PersistOwner)
        }
        Ok(_) => Err(CoordinationError::UnsafePath),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CoordinationError::PersistOwner(error)),
    }
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(windows)]
fn atomic_replace(target: &Path, replacement: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement_wide = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            replacement_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(target: &Path, replacement: &Path) -> io::Result<()> {
    fs::rename(replacement, target)
}
