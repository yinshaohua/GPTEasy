use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use thiserror::Error;

use crate::{
    domain::{
        AppSettings, EnvironmentId, EnvironmentKind, Locale, ManagedEnvironment, Provider,
        ProviderId, ProviderKind, ProviderVerification, SecretString, StateSnapshot, Theme,
    },
    state::{StateStore, StoreError},
};

pub const STATE_SMOKE_SCHEMA: &str = "gpteasy.phase1.state-smoke.v1";
const STATE_SMOKE_ROOT: &[&str] = &["contract-smoke", "state"];
const MARKER_FILENAME: &str = "state-smoke-marker.json";
const MARKER_TEMP_FILENAME: &str = "state-smoke-marker.json.tmp";
const EXPECTED_STATE_DIGEST: &str =
    "3a634244209687ed670e40d2ba9a9dc5175e85c00d994708c43bcb17de61365f";
const ALPHA_FINGERPRINT: &str = "89c467dc278dd6a92bd996d7735f9ba8610ad0d930f7d97f4b1e9935614ae416";
const BETA_FINGERPRINT: &str = "27851fe992655e033d9c3ea9e1066b89ce4c79297d110de9dae58ac3c7a725ac";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSmokeMode {
    Seed,
    Verify,
    Cleanup,
}

impl StateSmokeMode {
    pub fn parse(value: &str) -> Result<Self, StateSmokeError> {
        match value {
            "seed" => Ok(Self::Seed),
            "verify" => Ok(Self::Verify),
            "cleanup" => Ok(Self::Cleanup),
            _ => Err(StateSmokeError::InvalidMode),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Verify => "verify",
            Self::Cleanup => "cleanup",
        }
    }
}

#[derive(Debug, Error)]
pub enum StateSmokeError {
    #[error("state smoke mode must be seed, verify, or cleanup")]
    InvalidMode,
    #[error("opaque run ID must be 1-64 ASCII letters, digits, or hyphens")]
    InvalidRunId,
    #[error("failed to resolve the application local data root")]
    ResolveStateRoot(#[source] tauri::Error),
    #[error("the fixed state smoke root is unavailable")]
    InvalidStateRoot,
    #[error("failed to access the fixed state smoke root")]
    AccessStateRoot(#[source] io::Error),
    #[error("state smoke marker is missing or does not match")]
    MarkerMismatch,
    #[error("failed to encode the state smoke marker")]
    EncodeMarker(#[source] serde_json::Error),
    #[error("failed to access the state smoke marker")]
    AccessMarker(#[source] io::Error),
    #[error("the fixed state smoke snapshot is invalid")]
    InvalidFixture,
    #[error("the state smoke store is unavailable")]
    Store(#[source] StoreError),
    #[error("state smoke verification did not match the fixed snapshot")]
    VerificationMismatch,
    #[error("state smoke cleanup found an unrecognized artifact")]
    UnexpectedArtifact,
    #[error("state smoke cleanup failed")]
    Cleanup(#[source] io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StateSmokeMarker {
    schema: String,
    run_id: String,
    state_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateSmokeCounts {
    providers: usize,
    verifications: usize,
    native_environments: usize,
    wsl2_environments: usize,
    current_provider_assignments: usize,
    settings_fields: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateSmokeStateReport {
    schema: &'static str,
    mode: &'static str,
    run_id: String,
    counts: StateSmokeCounts,
    state_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateSmokeCleanupReport {
    schema: &'static str,
    mode: &'static str,
    run_id: String,
    cleaned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StateSmokeReport {
    State(StateSmokeStateReport),
    Cleanup(StateSmokeCleanupReport),
}

pub fn validate_run_id(run_id: &str) -> Result<(), StateSmokeError> {
    if (1..=64).contains(&run_id.len())
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Ok(())
    } else {
        Err(StateSmokeError::InvalidRunId)
    }
}

pub fn run_state_smoke<R: Runtime>(
    app: &AppHandle<R>,
    mode: StateSmokeMode,
    run_id: &str,
) -> Result<StateSmokeReport, StateSmokeError> {
    validate_run_id(run_id)?;
    let app_root = app
        .path()
        .app_local_data_dir()
        .map_err(StateSmokeError::ResolveStateRoot)?;

    match mode {
        StateSmokeMode::Seed => seed(&app_root, run_id),
        StateSmokeMode::Verify => verify(&app_root, run_id),
        StateSmokeMode::Cleanup => cleanup(&app_root, run_id),
    }
}

fn seed(app_root: &Path, run_id: &str) -> Result<StateSmokeReport, StateSmokeError> {
    let run_root = create_run_root(app_root, run_id)?;
    ensure_marker(&run_root, run_id)?;
    let expected = fixed_snapshot()?;
    let store = StateStore::open(&run_root).map_err(StateSmokeError::Store)?;
    let stored = store
        .replace_snapshot(&expected)
        .map_err(StateSmokeError::Store)?;
    if stored != expected || stored.digest() != EXPECTED_STATE_DIGEST {
        return Err(StateSmokeError::VerificationMismatch);
    }
    Ok(StateSmokeReport::State(state_report(
        StateSmokeMode::Seed,
        run_id,
        &stored,
    )))
}

fn verify(app_root: &Path, run_id: &str) -> Result<StateSmokeReport, StateSmokeError> {
    let run_root = existing_run_root(app_root, run_id)?;
    read_matching_marker(&run_root, run_id)?;
    let expected = fixed_snapshot()?;
    let store = StateStore::open(&run_root).map_err(StateSmokeError::Store)?;
    let observed = store.snapshot().map_err(StateSmokeError::Store)?;
    if observed != expected || observed.digest() != EXPECTED_STATE_DIGEST {
        return Err(StateSmokeError::VerificationMismatch);
    }
    Ok(StateSmokeReport::State(state_report(
        StateSmokeMode::Verify,
        run_id,
        &observed,
    )))
}

fn cleanup(app_root: &Path, run_id: &str) -> Result<StateSmokeReport, StateSmokeError> {
    let run_root = existing_run_root(app_root, run_id)?;
    read_matching_marker(&run_root, run_id)?;
    validate_cleanup_artifacts(&run_root)?;

    for filename in cleanup_file_allowlist() {
        let path = run_root.join(filename);
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(StateSmokeError::Cleanup(error)),
        }
    }
    fs::remove_dir(&run_root).map_err(StateSmokeError::Cleanup)?;
    prune_empty_fixed_parents(app_root);

    Ok(StateSmokeReport::Cleanup(StateSmokeCleanupReport {
        schema: STATE_SMOKE_SCHEMA,
        mode: StateSmokeMode::Cleanup.as_str(),
        run_id: run_id.to_owned(),
        cleaned: true,
    }))
}

fn state_report(
    mode: StateSmokeMode,
    run_id: &str,
    snapshot: &StateSnapshot,
) -> StateSmokeStateReport {
    StateSmokeStateReport {
        schema: STATE_SMOKE_SCHEMA,
        mode: mode.as_str(),
        run_id: run_id.to_owned(),
        counts: StateSmokeCounts {
            providers: snapshot.providers.len(),
            verifications: snapshot.verifications.len(),
            native_environments: snapshot
                .environments
                .iter()
                .filter(|environment| environment.kind == EnvironmentKind::NativeCodex)
                .count(),
            wsl2_environments: snapshot
                .environments
                .iter()
                .filter(|environment| environment.kind == EnvironmentKind::Wsl2)
                .count(),
            current_provider_assignments: snapshot
                .environments
                .iter()
                .filter(|environment| environment.current_provider_id.is_some())
                .count(),
            settings_fields: 7,
        },
        state_digest: snapshot.digest(),
    }
}

fn fixed_snapshot() -> Result<StateSnapshot, StateSmokeError> {
    let alpha_id = ProviderId::parse("11111111-1111-4111-8111-111111111111")
        .map_err(|_| StateSmokeError::InvalidFixture)?;
    let beta_id = ProviderId::parse("22222222-2222-4222-8222-222222222222")
        .map_err(|_| StateSmokeError::InvalidFixture)?;
    let native_id = EnvironmentId::parse("33333333-3333-4333-8333-333333333333")
        .map_err(|_| StateSmokeError::InvalidFixture)?;
    let wsl_id = EnvironmentId::parse("44444444-4444-4444-8444-444444444444")
        .map_err(|_| StateSmokeError::InvalidFixture)?;

    StateSnapshot::new(
        vec![
            Provider {
                id: alpha_id,
                kind: ProviderKind::BuiltInRecommended,
                built_in_key: Some("dayway".to_owned()),
                display_name: "DayWay".to_owned(),
                base_url: Some("https://dayway.site/v1".to_owned()),
                api_key: Some(SecretString::new(
                    "state-secret-canary-alpha-4D2F9C0E".to_owned(),
                )),
                default_model: Some("dayway-model-a".to_owned()),
                created_at: "2026-08-07T00:00:00.000Z".to_owned(),
                updated_at: "2026-08-07T00:01:00.000Z".to_owned(),
            },
            Provider {
                id: beta_id,
                kind: ProviderKind::Custom,
                built_in_key: None,
                display_name: "Local Compatible".to_owned(),
                base_url: Some("http://127.0.0.1:4010/v1".to_owned()),
                api_key: Some(SecretString::new(
                    "state-secret-canary-beta-7A1E3B8D".to_owned(),
                )),
                default_model: Some("local-model-b".to_owned()),
                created_at: "2026-08-07T00:02:00.000Z".to_owned(),
                updated_at: "2026-08-07T00:03:00.000Z".to_owned(),
            },
        ],
        vec![
            ProviderVerification {
                provider_id: alpha_id,
                combination_fingerprint: ALPHA_FINGERPRINT.to_owned(),
                verified_at: "2026-08-07T00:04:00.000Z".to_owned(),
                contract_version: "gpteasy.provider-validation.v1".to_owned(),
            },
            ProviderVerification {
                provider_id: beta_id,
                combination_fingerprint: BETA_FINGERPRINT.to_owned(),
                verified_at: "2026-08-07T00:05:00.000Z".to_owned(),
                contract_version: "gpteasy.provider-validation.v1".to_owned(),
            },
        ],
        vec![
            ManagedEnvironment {
                id: native_id,
                kind: EnvironmentKind::NativeCodex,
                platform_identity: "native-current-user".to_owned(),
                display_name: "Native Codex".to_owned(),
                current_provider_id: Some(alpha_id),
                first_seen_at: "2026-08-07T00:06:00.000Z".to_owned(),
                last_seen_at: "2026-08-07T00:07:00.000Z".to_owned(),
            },
            ManagedEnvironment {
                id: wsl_id,
                kind: EnvironmentKind::Wsl2,
                platform_identity: "wsl-registration-a1b2c3d4".to_owned(),
                display_name: "Ubuntu-24.04".to_owned(),
                current_provider_id: Some(beta_id),
                first_seen_at: "2026-08-07T00:08:00.000Z".to_owned(),
                last_seen_at: "2026-08-07T00:09:00.000Z".to_owned(),
            },
        ],
        AppSettings {
            locale: Locale::ZhCn,
            theme: Theme::Dark,
            launch_at_login_desired: true,
            close_to_tray_notice_seen: true,
            onboarding_completed: true,
            last_update_check_at: Some("2026-08-07T00:10:00.000Z".to_owned()),
            updated_at: "2026-08-07T00:11:00.000Z".to_owned(),
        },
    )
    .map_err(|_| StateSmokeError::InvalidFixture)
}

fn expected_marker(run_id: &str) -> StateSmokeMarker {
    StateSmokeMarker {
        schema: STATE_SMOKE_SCHEMA.to_owned(),
        run_id: run_id.to_owned(),
        state_digest: EXPECTED_STATE_DIGEST.to_owned(),
    }
}

fn create_run_root(app_root: &Path, run_id: &str) -> Result<PathBuf, StateSmokeError> {
    fs::create_dir_all(app_root).map_err(StateSmokeError::AccessStateRoot)?;
    ensure_plain_directory(app_root)?;
    let mut current = app_root.to_path_buf();
    for component in STATE_SMOKE_ROOT {
        current = create_plain_child(&current, component)?;
    }
    create_plain_child(&current, run_id)
}

fn existing_run_root(app_root: &Path, run_id: &str) -> Result<PathBuf, StateSmokeError> {
    ensure_plain_directory(app_root)?;
    let mut current = app_root.to_path_buf();
    for component in STATE_SMOKE_ROOT {
        current = existing_plain_child(&current, component)?;
    }
    existing_plain_child(&current, run_id)
}

fn create_plain_child(parent: &Path, component: &str) -> Result<PathBuf, StateSmokeError> {
    let child = parent.join(component);
    match fs::create_dir(&child) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(StateSmokeError::AccessStateRoot(error)),
    }
    ensure_plain_directory(&child)?;
    Ok(child)
}

fn existing_plain_child(parent: &Path, component: &str) -> Result<PathBuf, StateSmokeError> {
    let child = parent.join(component);
    ensure_plain_directory(&child)?;
    Ok(child)
}

fn ensure_plain_directory(path: &Path) -> Result<(), StateSmokeError> {
    let metadata = fs::symlink_metadata(path).map_err(StateSmokeError::AccessStateRoot)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(StateSmokeError::InvalidStateRoot);
    }
    Ok(())
}

fn ensure_marker(run_root: &Path, run_id: &str) -> Result<(), StateSmokeError> {
    let marker_path = run_root.join(MARKER_FILENAME);
    match read_matching_marker(run_root, run_id) {
        Ok(()) => return Ok(()),
        Err(StateSmokeError::AccessMarker(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut bytes =
        serde_json::to_vec(&expected_marker(run_id)).map_err(StateSmokeError::EncodeMarker)?;
    bytes.push(b'\n');
    let temporary_path = run_root.join(MARKER_TEMP_FILENAME);
    let mut temporary = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary_path)
        .map_err(StateSmokeError::AccessMarker)?;
    if let Err(error) = temporary
        .write_all(&bytes)
        .and_then(|_| temporary.sync_all())
    {
        drop(temporary);
        let _ = fs::remove_file(&temporary_path);
        return Err(StateSmokeError::AccessMarker(error));
    }
    drop(temporary);
    if let Err(error) = fs::rename(&temporary_path, &marker_path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(StateSmokeError::AccessMarker(error));
    }
    Ok(())
}

fn read_matching_marker(run_root: &Path, run_id: &str) -> Result<(), StateSmokeError> {
    let marker_path = run_root.join(MARKER_FILENAME);
    let metadata = fs::symlink_metadata(&marker_path).map_err(StateSmokeError::AccessMarker)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(StateSmokeError::MarkerMismatch);
    }
    let marker: StateSmokeMarker =
        serde_json::from_slice(&fs::read(marker_path).map_err(StateSmokeError::AccessMarker)?)
            .map_err(|_| StateSmokeError::MarkerMismatch)?;
    if marker != expected_marker(run_id) {
        return Err(StateSmokeError::MarkerMismatch);
    }
    Ok(())
}

fn cleanup_file_allowlist() -> BTreeSet<&'static str> {
    BTreeSet::from([
        MARKER_FILENAME,
        MARKER_TEMP_FILENAME,
        "state-lock-owner.json",
        "state-lock-owner.json.tmp",
        "state.lock",
        "state.sqlite3",
        "state.sqlite3-journal",
        "state.sqlite3-shm",
        "state.sqlite3-wal",
    ])
}

fn validate_cleanup_artifacts(run_root: &Path) -> Result<(), StateSmokeError> {
    let allowed = cleanup_file_allowlist();
    for entry in fs::read_dir(run_root).map_err(StateSmokeError::AccessStateRoot)? {
        let entry = entry.map_err(StateSmokeError::AccessStateRoot)?;
        let file_type = entry
            .file_type()
            .map_err(StateSmokeError::AccessStateRoot)?;
        let filename = entry.file_name();
        let filename = filename
            .to_str()
            .ok_or(StateSmokeError::UnexpectedArtifact)?;
        if !file_type.is_file() || file_type.is_symlink() || !allowed.contains(filename) {
            return Err(StateSmokeError::UnexpectedArtifact);
        }
    }
    Ok(())
}

fn prune_empty_fixed_parents(app_root: &Path) {
    let state_root = app_root.join("contract-smoke").join("state");
    let contract_root = app_root.join("contract-smoke");
    for path in [state_root, contract_root] {
        match fs::remove_dir(path) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(_) => {}
        }
    }
}
