use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use toml_edit::DocumentMut;
use uuid::Uuid;

use crate::provider::ProviderSummary;
use crate::state::StateStore;

const MANAGED_START: &str = "# >>> GPTEasy managed provider >>>";
const MANAGED_END: &str = "# <<< GPTEasy managed provider <<<";
const PROVIDER_ID_PREFIX: &str = "# GPTEasy provider-id:";
const BACKUP_LIMIT: usize = 5;
const BACKUP_FORMAT_VERSION: u8 = 1;
const BACKUP_COMPLETION_FILE: &str = "completed";
const BACKUP_COMPLETION_CONTENT: &[u8] = b"gpteasy-config-backup-v1\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentState {
    External,
    Managed,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Config,
    Credentials,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactAction {
    Create,
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactImpact {
    pub artifact: ArtifactKind,
    pub action: ArtifactAction,
    pub fields: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSnapshot {
    pub state: EnvironmentState,
    pub message_id: &'static str,
    pub revision: String,
    pub requires_takeover_confirmation: bool,
    pub impacts: Vec<ArtifactImpact>,
    pub current_provider: Option<ProviderSummary>,
    pub restore_availability: RestoreAvailability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreAvailability {
    Available,
    NoBackup,
    ArtifactsChanged,
    InvalidBackup,
    RecoveryPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentFailureCategory {
    StateUnavailable,
    ProviderNotFound,
    TakeoverConfirmationRequired,
    ManagedConflict,
    UnsupportedCredentialStore,
    InvalidConfig,
    InvalidCredentials,
    BackupFailed,
    ConcurrentModification,
    ArtifactRedirected,
    ArtifactWriteFailed,
    RollbackFailed,
    OperationInterrupted,
    RestoreConfirmationRequired,
    RestoreUnavailable,
    BackupInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentFailure {
    pub category: EnvironmentFailureCategory,
    pub message_id: &'static str,
}

impl EnvironmentFailure {
    fn new(category: EnvironmentFailureCategory, message_id: &'static str) -> Self {
        Self {
            category,
            message_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentFailurePoint {
    AfterBackupCompleted,
    AfterPendingRegistered,
    BeforeConfigReplace,
    AfterConfigReplaced,
    BeforeCredentialsReplace,
    AfterAllArtifactsReplaced,
    BeforeDatabaseCommit,
    AfterDatabaseCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentRecovery {
    NoPendingOperation,
    KeptOldState,
    CompletedNewState,
    Conflict,
}

pub trait EnvironmentFaultInjector: Send + Sync {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool;

    fn interrupts_at(&self, _point: EnvironmentFailurePoint) -> bool {
        false
    }

    fn fails_backup_creation(&self) -> bool {
        false
    }

    fn fails_rollback(&self) -> bool {
        false
    }
}

struct NoFaults;

impl EnvironmentFaultInjector for NoFaults {
    fn fails_at(&self, _point: EnvironmentFailurePoint) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct EnvironmentApplication {
    state_store: StateStore,
    codex_home: PathBuf,
    operation_lock: Arc<Mutex<()>>,
    faults: Arc<dyn EnvironmentFaultInjector>,
}

impl EnvironmentApplication {
    pub fn new(state_store: StateStore, codex_home: impl AsRef<Path>) -> Self {
        Self::with_fault_injector(state_store, codex_home, Arc::new(NoFaults))
    }

    #[doc(hidden)]
    pub fn with_fault_injector(
        state_store: StateStore,
        codex_home: impl AsRef<Path>,
        faults: Arc<dyn EnvironmentFaultInjector>,
    ) -> Self {
        Self {
            state_store,
            codex_home: codex_home.as_ref().to_path_buf(),
            operation_lock: Arc::new(Mutex::new(())),
            faults,
        }
    }

    pub fn inspect(&self) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let connection = self.open_state()?;
        inspect_environment(&connection, &self.codex_home)
    }

    pub fn recover_pending(&self) -> Result<EnvironmentRecovery, EnvironmentFailure> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let mut connection = self.open_state()?;
        let pending = connection
            .query_row(
                "SELECT operation_id, operation_kind, old_config_fingerprint,
                        new_config_fingerprint, old_credentials_fingerprint,
                        new_credentials_fingerprint, backup_reference, target_snapshot_json
                 FROM pending_config_operation WHERE singleton = 1",
                [],
                |row| {
                    Ok(PendingRecovery {
                        operation_id: row.get(0)?,
                        operation_kind: row.get(1)?,
                        old_config_fingerprint: row.get(2)?,
                        new_config_fingerprint: row.get(3)?,
                        old_credentials_fingerprint: row.get(4)?,
                        new_credentials_fingerprint: row.get(5)?,
                        backup_reference: PathBuf::from(row.get::<_, String>(6)?),
                        target_snapshot_json: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|_| state_unavailable())?;
        let Some(pending) = pending else {
            return Ok(EnvironmentRecovery::NoPendingOperation);
        };
        let backup = match pending_backup_path(&self.codex_home, &pending.backup_reference) {
            Ok(backup) => backup,
            Err(_) => {
                mark_pending_conflict(&mut connection, &pending.operation_id)?;
                return Ok(EnvironmentRecovery::Conflict);
            }
        };
        let config = read_artifact(&self.codex_home.join("config.toml"))?;
        let credentials = read_artifact(&self.codex_home.join("auth.json"))?;
        let current_config = config.fingerprint(ArtifactKind::Config);
        let current_credentials = credentials.fingerprint(ArtifactKind::Credentials);
        if current_config == pending.old_config_fingerprint
            && current_credentials == pending.old_credentials_fingerprint
        {
            unmark_backup_completed(&backup)?;
            clear_pending(&connection, &pending.operation_id)?;
            return Ok(EnvironmentRecovery::KeptOldState);
        }
        if current_config == pending.new_config_fingerprint
            && current_credentials == pending.new_credentials_fingerprint
        {
            if mark_backup_completed(&backup).is_ok()
                && commit_recovered_state(&mut connection, &pending, &config, &credentials).is_ok()
            {
                return Ok(EnvironmentRecovery::CompletedNewState);
            }
            mark_pending_conflict(&mut connection, &pending.operation_id)?;
            return Ok(EnvironmentRecovery::Conflict);
        }
        mark_pending_conflict(&mut connection, &pending.operation_id)?;
        Ok(EnvironmentRecovery::Conflict)
    }

    pub fn restore_last_config(
        &self,
        confirm_restore: bool,
        expected_revision: &str,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        if !confirm_restore {
            return Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::RestoreConfirmationRequired,
                "environment.restore_confirmation_required",
            ));
        }
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let mut connection = self.open_state()?;
        if has_pending_operation(&connection)? {
            return Err(restore_unavailable());
        }
        let config = read_artifact(&self.codex_home.join("config.toml"))?;
        let credentials = read_artifact(&self.codex_home.join("auth.json"))?;
        if environment_revision(&config, &credentials) != expected_revision {
            return Err(concurrent_modification());
        }
        let backup = latest_completed_backup(&self.codex_home)?.ok_or_else(restore_unavailable)?;
        if !backup.matches_current(&config, &credentials) {
            return Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::ManagedConflict,
                "environment.restore_conflict",
            ));
        }
        let prepared = PreparedRestore::new(&self.codex_home, config, credentials, backup);
        if self.faults.fails_backup_creation() {
            return Err(backup_failed());
        }
        let rollback_backup = create_restore_backup(&self.codex_home, &prepared)?;
        self.check_interruption(EnvironmentFailurePoint::AfterBackupCompleted)?;
        persist_pending_restore(&mut connection, &prepared, &rollback_backup)?;
        self.check_interruption(EnvironmentFailurePoint::AfterPendingRegistered)?;

        let mut config_applied = false;
        let mut credentials_applied = false;
        let mut interrupted = false;
        let mut backup_completed = false;
        let result = (|| {
            self.check_fault(EnvironmentFailurePoint::BeforeConfigReplace)?;
            prepared.config.commit()?;
            config_applied = true;
            update_pending_stage(&connection, &prepared.operation_id, "config_replaced")?;
            self.check_interruption(EnvironmentFailurePoint::AfterConfigReplaced)
                .map_err(|failure| {
                    interrupted = true;
                    failure
                })?;
            self.check_fault(EnvironmentFailurePoint::BeforeCredentialsReplace)?;
            prepared.credentials.commit()?;
            credentials_applied = true;
            prepared.verify_committed()?;
            update_pending_stage(&connection, &prepared.operation_id, "artifacts_replaced")?;
            mark_backup_completed(&rollback_backup)?;
            backup_completed = true;
            self.check_interruption(EnvironmentFailurePoint::AfterAllArtifactsReplaced)
                .map_err(|failure| {
                    interrupted = true;
                    failure
                })?;
            self.check_interruption(EnvironmentFailurePoint::BeforeDatabaseCommit)
                .map_err(|failure| {
                    interrupted = true;
                    failure
                })?;
            self.check_fault(EnvironmentFailurePoint::BeforeDatabaseCommit)?;
            commit_restored_state(&mut connection, &prepared)?;
            self.check_interruption(EnvironmentFailurePoint::AfterDatabaseCommit)
                .map_err(|failure| {
                    interrupted = true;
                    failure
                })?;
            Ok(())
        })();

        if let Err(failure) = result {
            if interrupted {
                return Err(failure);
            }
            if backup_completed {
                unmark_backup_completed(&rollback_backup)?;
            }
            if self.faults.fails_rollback()
                || rollback_restore(&prepared, config_applied, credentials_applied).is_err()
            {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::RollbackFailed,
                    "environment.rollback_failed",
                ));
            }
            clear_pending(&connection, &prepared.operation_id)?;
            return Err(failure);
        }

        inspect_environment(&connection, &self.codex_home)
    }

    pub fn apply_provider(
        &self,
        provider_id: &str,
        confirm_takeover: bool,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let revision = self.inspect()?.revision;
        self.apply_provider_at_revision(provider_id, confirm_takeover, &revision)
    }

    pub fn apply_provider_at_revision(
        &self,
        provider_id: &str,
        confirm_takeover: bool,
        expected_revision: &str,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let mut connection = self.open_state()?;
        let provider = load_provider(&connection, provider_id)?;
        self.apply_target(
            &mut connection,
            provider,
            confirm_takeover,
            Some(expected_revision),
            None,
        )
    }

    pub(crate) fn save_and_apply_provider_update(
        &self,
        update: VerifiedProviderUpdate,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let mut connection = self.open_state()?;
        let before = inspect_environment(&connection, &self.codex_home)?;
        if before.state != EnvironmentState::Managed
            || before
                .current_provider
                .as_ref()
                .map(|provider| provider.id.as_str())
                != Some(update.provider.id.as_str())
        {
            return Err(managed_conflict());
        }
        let guard = UpdateGuard {
            original_name: update.original_name,
            original_verification_fingerprint: update.original_verification_fingerprint,
        };
        self.apply_target(&mut connection, update.provider, false, None, Some(guard))
    }

    fn apply_target(
        &self,
        connection: &mut Connection,
        provider: ProviderTarget,
        confirm_takeover: bool,
        expected_revision: Option<&str>,
        update_guard: Option<UpdateGuard>,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let before = inspect_environment(connection, &self.codex_home)?;
        if expected_revision.is_some_and(|expected| before.revision != expected) {
            return Err(concurrent_modification());
        }
        match before.state {
            EnvironmentState::Conflict if !confirm_takeover => {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::TakeoverConfirmationRequired,
                    "environment.takeover_confirmation_required",
                ));
            }
            EnvironmentState::External if !confirm_takeover => {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::TakeoverConfirmationRequired,
                    "environment.takeover_confirmation_required",
                ));
            }
            EnvironmentState::Conflict | EnvironmentState::External | EnvironmentState::Managed => {
            }
        }

        let prepared = PreparedSwitch::prepare(&self.codex_home, provider, update_guard)?;
        if expected_revision.is_some_and(|expected| prepared.old_revision() != expected) {
            return Err(concurrent_modification());
        }
        if self.faults.fails_backup_creation() {
            return Err(backup_failed());
        }
        let backup = create_backup(&self.codex_home, &prepared)?;
        self.check_interruption(EnvironmentFailurePoint::AfterBackupCompleted)?;
        persist_pending(connection, &prepared, &backup)?;
        self.check_interruption(EnvironmentFailurePoint::AfterPendingRegistered)?;

        let mut config_applied = false;
        let mut credentials_applied = false;
        let mut interrupted = false;
        let mut backup_completed = false;
        let result = (|| {
            self.check_fault(EnvironmentFailurePoint::BeforeConfigReplace)?;
            prepared.config.commit()?;
            config_applied = true;
            update_pending_stage(connection, &prepared.operation_id, "config_replaced")?;
            self.check_interruption(EnvironmentFailurePoint::AfterConfigReplaced)
                .map_err(|failure| {
                    interrupted = true;
                    failure
                })?;
            self.check_fault(EnvironmentFailurePoint::BeforeCredentialsReplace)?;
            prepared.credentials.commit()?;
            credentials_applied = true;
            prepared.verify_committed()?;
            update_pending_stage(connection, &prepared.operation_id, "artifacts_replaced")?;
            mark_backup_completed(&backup)?;
            backup_completed = true;
            self.check_interruption(EnvironmentFailurePoint::AfterAllArtifactsReplaced)
                .map_err(|failure| {
                    interrupted = true;
                    failure
                })?;
            self.check_interruption(EnvironmentFailurePoint::BeforeDatabaseCommit)
                .map_err(|failure| {
                    interrupted = true;
                    failure
                })?;
            self.check_fault(EnvironmentFailurePoint::BeforeDatabaseCommit)?;
            commit_applied_state(connection, &prepared)?;
            self.check_interruption(EnvironmentFailurePoint::AfterDatabaseCommit)
                .map_err(|failure| {
                    interrupted = true;
                    failure
                })?;
            Ok(())
        })();

        if let Err(failure) = result {
            if interrupted {
                return Err(failure);
            }
            if backup_completed {
                unmark_backup_completed(&backup)?;
            }
            if self.faults.fails_rollback()
                || rollback_switch(&prepared, config_applied, credentials_applied).is_err()
            {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::RollbackFailed,
                    "environment.rollback_failed",
                ));
            }
            clear_pending(connection, &prepared.operation_id)?;
            return Err(failure);
        }

        inspect_environment(connection, &self.codex_home)
    }

    fn open_state(&self) -> Result<Connection, EnvironmentFailure> {
        if !self.state_store.bootstrap().is_ready() {
            return Err(state_unavailable());
        }
        let connection = Connection::open(self.state_store.paths().database())
            .map_err(|_| state_unavailable())?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA synchronous = FULL;",
            )
            .map_err(|_| state_unavailable())?;
        Ok(connection)
    }

    fn check_fault(&self, point: EnvironmentFailurePoint) -> Result<(), EnvironmentFailure> {
        if self.faults.fails_at(point) {
            Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::ArtifactWriteFailed,
                "environment.artifact_write_failed",
            ))
        } else {
            Ok(())
        }
    }

    fn check_interruption(&self, point: EnvironmentFailurePoint) -> Result<(), EnvironmentFailure> {
        if self.faults.interrupts_at(point) {
            Err(operation_interrupted())
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderTarget {
    id: String,
    name: String,
    base_url: String,
    api_key: String,
    default_model: String,
    verified_at_epoch_seconds: u64,
    verification_fingerprint: String,
}

impl ProviderTarget {
    pub(crate) fn new(
        id: String,
        name: String,
        base_url: String,
        api_key: String,
        default_model: String,
        verified_at_epoch_seconds: u64,
        verification_fingerprint: String,
    ) -> Self {
        Self {
            id,
            name,
            base_url,
            api_key,
            default_model,
            verified_at_epoch_seconds,
            verification_fingerprint,
        }
    }
}

struct PendingRecovery {
    operation_id: String,
    operation_kind: String,
    old_config_fingerprint: Option<String>,
    new_config_fingerprint: Option<String>,
    old_credentials_fingerprint: Option<String>,
    new_credentials_fingerprint: Option<String>,
    backup_reference: PathBuf,
    target_snapshot_json: String,
}

pub(crate) struct VerifiedProviderUpdate {
    provider: ProviderTarget,
    original_name: String,
    original_verification_fingerprint: String,
}

impl VerifiedProviderUpdate {
    pub(crate) fn new(
        provider: ProviderTarget,
        original_name: String,
        original_verification_fingerprint: String,
    ) -> Self {
        Self {
            provider,
            original_name,
            original_verification_fingerprint,
        }
    }
}

impl ProviderTarget {
    fn summary(&self, is_current: bool) -> ProviderSummary {
        ProviderSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            base_url: self.base_url.clone(),
            default_model: self.default_model.clone(),
            verified_at_epoch_seconds: self.verified_at_epoch_seconds,
            is_current,
        }
    }
}

fn load_provider(
    connection: &Connection,
    provider_id: &str,
) -> Result<ProviderTarget, EnvironmentFailure> {
    connection
        .query_row(
            "SELECT id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint
             FROM providers WHERE id = ?1",
            [provider_id],
            |row| {
                let verified_at = row.get::<_, String>(5)?;
                Ok(ProviderTarget {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    base_url: row.get(2)?,
                    api_key: row.get(3)?,
                    default_model: row.get(4)?,
                    verified_at_epoch_seconds: verified_at.parse().map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    verification_fingerprint: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|_| state_unavailable())?
        .ok_or_else(|| {
            EnvironmentFailure::new(
                EnvironmentFailureCategory::ProviderNotFound,
                "environment.provider_not_found",
            )
        })
}

fn has_pending_operation(connection: &Connection) -> Result<bool, EnvironmentFailure> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pending_config_operation WHERE singleton = 1)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| state_unavailable())
}

fn inspect_restore_availability(
    connection: &Connection,
    codex_home: &Path,
    config: &ArtifactBytes,
    credentials: &ArtifactBytes,
) -> RestoreAvailability {
    match has_pending_operation(connection) {
        Ok(true) => return RestoreAvailability::RecoveryPending,
        Err(_) => return RestoreAvailability::InvalidBackup,
        Ok(false) => {}
    }
    match latest_completed_backup(codex_home) {
        Ok(Some(backup)) if backup.matches_current(config, credentials) => {
            RestoreAvailability::Available
        }
        Ok(Some(_)) => RestoreAvailability::ArtifactsChanged,
        Ok(None) => RestoreAvailability::NoBackup,
        Err(_) => RestoreAvailability::InvalidBackup,
    }
}

fn inspect_environment(
    connection: &Connection,
    codex_home: &Path,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let config = read_artifact(&codex_home.join("config.toml"))?;
    let credentials = read_artifact(&codex_home.join("auth.json"))?;
    let revision = environment_revision(&config, &credentials);
    let restore_availability =
        inspect_restore_availability(connection, codex_home, &config, &credentials);
    let impacts = vec![
        ArtifactImpact {
            artifact: ArtifactKind::Config,
            action: action_for(&config),
            fields: vec!["model", "model_provider", "model_providers.<provider-id>"],
        },
        ArtifactImpact {
            artifact: ArtifactKind::Credentials,
            action: action_for(&credentials),
            fields: vec!["auth_mode", "OPENAI_API_KEY"],
        },
    ];
    let recovery_conflict = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pending_config_operation
                WHERE singleton = 1 AND stage = 'conflict'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| state_unavailable())?;
    if recovery_conflict {
        return Ok(conflict_snapshot(impacts, revision, restore_availability));
    }
    let last_applied = connection
        .query_row(
            "SELECT provider_id, config_fingerprint, credentials_fingerprint
             FROM last_applied_state WHERE singleton = 1 AND mode = 'provider'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|_| state_unavailable())?;

    let Some(config_bytes) = config.bytes.as_deref() else {
        return Ok(if last_applied.is_some() {
            conflict_snapshot(impacts, revision, restore_availability)
        } else {
            external_snapshot(impacts, revision, restore_availability)
        });
    };
    let config_text = match std::str::from_utf8(config_bytes) {
        Ok(text) => text,
        Err(_) => return Ok(conflict_snapshot(impacts, revision, restore_availability)),
    };
    let document = match config_text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(_) => {
            return Ok(conflict_snapshot(impacts, revision, restore_availability));
        }
    };
    if ensure_file_credential_store(Some(config_bytes)).is_err() {
        return Ok(conflict_snapshot(impacts, revision, restore_availability));
    }
    let managed = match managed_block(config_text) {
        ManagedBlock::None => {
            return Ok(if last_applied.is_some() {
                conflict_snapshot(impacts, revision, restore_availability)
            } else {
                external_snapshot(impacts, revision, restore_availability)
            });
        }
        ManagedBlock::Conflict => {
            return Ok(conflict_snapshot(impacts, revision, restore_availability));
        }
        ManagedBlock::Valid(block) => block,
    };
    if !managed_block_is_root_scoped(&document, config_text, &managed) {
        return Ok(conflict_snapshot(impacts, revision, restore_availability));
    }
    let provider = match load_provider(connection, &managed.provider_id) {
        Ok(provider) => provider,
        Err(failure) if failure.category == EnvironmentFailureCategory::ProviderNotFound => {
            return Ok(if last_applied.is_some() {
                conflict_snapshot(impacts, revision, restore_availability)
            } else {
                external_snapshot(impacts, revision, restore_availability)
            });
        }
        Err(failure) => return Err(failure),
    };
    if !managed_config_matches(&document, &provider) {
        return Ok(conflict_snapshot(impacts, revision, restore_availability));
    }
    if !credentials_match(&credentials, &provider.api_key)? {
        return Ok(conflict_snapshot(impacts, revision, restore_availability));
    }

    let Some((applied_provider, applied_config, _applied_credentials)) = last_applied else {
        return Ok(external_snapshot(impacts, revision, restore_availability));
    };
    if applied_provider != provider.id || applied_config != managed_config_fingerprint(config_bytes)
    {
        return Ok(conflict_snapshot(impacts, revision, restore_availability));
    }

    Ok(EnvironmentSnapshot {
        state: EnvironmentState::Managed,
        message_id: "environment.managed",
        revision,
        requires_takeover_confirmation: false,
        impacts,
        current_provider: Some(provider.summary(true)),
        restore_availability,
    })
}

fn external_snapshot(
    impacts: Vec<ArtifactImpact>,
    revision: String,
    restore_availability: RestoreAvailability,
) -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        state: EnvironmentState::External,
        message_id: "environment.external",
        revision,
        requires_takeover_confirmation: true,
        impacts,
        current_provider: None,
        restore_availability,
    }
}

fn conflict_snapshot(
    impacts: Vec<ArtifactImpact>,
    revision: String,
    restore_availability: RestoreAvailability,
) -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        state: EnvironmentState::Conflict,
        message_id: "environment.managed_conflict",
        revision,
        requires_takeover_confirmation: true,
        impacts,
        current_provider: None,
        restore_availability,
    }
}

fn action_for(artifact: &ArtifactBytes) -> ArtifactAction {
    if artifact.bytes.is_some() {
        ArtifactAction::Update
    } else {
        ArtifactAction::Create
    }
}

#[derive(Debug, Clone)]
struct PreparedSwitch {
    operation_id: String,
    provider: ProviderTarget,
    config: PreparedArtifact,
    credentials: PreparedArtifact,
    update_guard: Option<UpdateGuard>,
}

#[derive(Debug, Clone)]
struct UpdateGuard {
    original_name: String,
    original_verification_fingerprint: String,
}

impl PreparedSwitch {
    fn prepare(
        codex_home: &Path,
        provider: ProviderTarget,
        update_guard: Option<UpdateGuard>,
    ) -> Result<Self, EnvironmentFailure> {
        if Uuid::parse_str(&provider.id).is_err() {
            return Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::InvalidConfig,
                "environment.provider_id_invalid",
            ));
        }
        let config_path = codex_home.join("config.toml");
        let credentials_path = codex_home.join("auth.json");
        let config = read_artifact(&config_path)?;
        ensure_file_credential_store(config.bytes.as_deref())?;
        let rendered_config = render_config(config.bytes.as_deref(), &provider)?;
        let credentials = read_artifact(&credentials_path)?;
        let rendered_credentials =
            render_credentials(credentials.bytes.as_deref(), &provider.api_key)?;
        Ok(Self {
            operation_id: Uuid::new_v4().to_string(),
            provider,
            config: PreparedArtifact::new(
                config_path,
                config,
                rendered_config,
                ArtifactKind::Config,
            ),
            credentials: PreparedArtifact::new(
                credentials_path,
                credentials,
                rendered_credentials,
                ArtifactKind::Credentials,
            ),
            update_guard,
        })
    }

    fn verify_committed(&self) -> Result<(), EnvironmentFailure> {
        self.config.verify_new()?;
        self.credentials.verify_new()
    }

    fn old_revision(&self) -> String {
        environment_revision(&self.config.old, &self.credentials.old)
    }
}

#[derive(Debug, Clone)]
struct ArtifactBytes {
    bytes: Option<Vec<u8>>,
}

impl ArtifactBytes {
    fn fingerprint(&self, kind: ArtifactKind) -> Option<String> {
        self.bytes
            .as_deref()
            .map(|bytes| artifact_hash(kind, bytes))
    }
}

#[derive(Debug, Clone)]
struct PreparedArtifact {
    path: PathBuf,
    old: ArtifactBytes,
    new_bytes: Vec<u8>,
    old_fingerprint: Option<String>,
    new_fingerprint: String,
}

impl PreparedArtifact {
    fn new(path: PathBuf, old: ArtifactBytes, new_bytes: Vec<u8>, kind: ArtifactKind) -> Self {
        let old_fingerprint = old.fingerprint(kind);
        let new_fingerprint = artifact_hash(kind, &new_bytes);
        Self {
            path,
            old,
            new_bytes,
            old_fingerprint,
            new_fingerprint,
        }
    }

    fn commit(&self) -> Result<(), EnvironmentFailure> {
        if !artifact_matches(&self.path, self.old.bytes.as_deref())? {
            return Err(concurrent_modification());
        }
        let temporary = write_temporary(&self.path, &self.new_bytes)?;
        if !artifact_matches(&self.path, self.old.bytes.as_deref())? {
            let _ = fs::remove_file(&temporary);
            return Err(concurrent_modification());
        }
        atomic_replace(&self.path, &temporary, self.old.bytes.is_some())
    }

    fn verify_new(&self) -> Result<(), EnvironmentFailure> {
        let bytes = fs::read(&self.path).map_err(|_| artifact_write_failed())?;
        let kind = if self.path.file_name().and_then(|name| name.to_str()) == Some("auth.json") {
            ArtifactKind::Credentials
        } else {
            ArtifactKind::Config
        };
        if artifact_hash(kind, &bytes) == self.new_fingerprint {
            Ok(())
        } else {
            Err(artifact_write_failed())
        }
    }

    fn restore(&self) -> Result<(), EnvironmentFailure> {
        if artifact_matches(&self.path, self.old.bytes.as_deref())? {
            return Ok(());
        }
        let current = fs::read(&self.path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                concurrent_modification()
            } else {
                artifact_write_failed()
            }
        })?;
        let kind = if self.path.file_name().and_then(|name| name.to_str()) == Some("auth.json") {
            ArtifactKind::Credentials
        } else {
            ArtifactKind::Config
        };
        if artifact_hash(kind, &current) != self.new_fingerprint {
            return Err(concurrent_modification());
        }
        match self.old.bytes.as_deref() {
            Some(old) => {
                let temporary = write_temporary(&self.path, old)?;
                atomic_replace(&self.path, &temporary, true)
            }
            None => fs::remove_file(&self.path).map_err(|_| artifact_write_failed()),
        }
    }
}

#[derive(Debug, Clone)]
struct CompletedBackup {
    path: PathBuf,
    manifest: BackupManifest,
    config: ArtifactBytes,
    credentials: ArtifactBytes,
}

impl CompletedBackup {
    fn matches_current(&self, config: &ArtifactBytes, credentials: &ArtifactBytes) -> bool {
        config.fingerprint(ArtifactKind::Config) == self.manifest.new_config_fingerprint
            && credentials.fingerprint(ArtifactKind::Credentials)
                == self.manifest.new_credentials_fingerprint
    }
}

#[derive(Debug, Clone)]
struct PreparedRestore {
    operation_id: String,
    config: PreparedRestoreArtifact,
    credentials: PreparedRestoreArtifact,
    source_backup: PathBuf,
}

impl PreparedRestore {
    fn new(
        codex_home: &Path,
        config: ArtifactBytes,
        credentials: ArtifactBytes,
        backup: CompletedBackup,
    ) -> Self {
        Self {
            operation_id: Uuid::new_v4().to_string(),
            config: PreparedRestoreArtifact::new(
                codex_home.join("config.toml"),
                config,
                backup.config,
                ArtifactKind::Config,
            ),
            credentials: PreparedRestoreArtifact::new(
                codex_home.join("auth.json"),
                credentials,
                backup.credentials,
                ArtifactKind::Credentials,
            ),
            source_backup: backup.path,
        }
    }

    fn verify_committed(&self) -> Result<(), EnvironmentFailure> {
        self.config.verify_target()?;
        self.credentials.verify_target()
    }
}

#[derive(Debug, Clone)]
struct PreparedRestoreArtifact {
    path: PathBuf,
    current: ArtifactBytes,
    target: ArtifactBytes,
    current_fingerprint: Option<String>,
    target_fingerprint: Option<String>,
}

impl PreparedRestoreArtifact {
    fn new(
        path: PathBuf,
        current: ArtifactBytes,
        target: ArtifactBytes,
        kind: ArtifactKind,
    ) -> Self {
        let current_fingerprint = current.fingerprint(kind);
        let target_fingerprint = target.fingerprint(kind);
        Self {
            path,
            current,
            target,
            current_fingerprint,
            target_fingerprint,
        }
    }

    fn commit(&self) -> Result<(), EnvironmentFailure> {
        if !artifact_matches(&self.path, self.current.bytes.as_deref())? {
            return Err(concurrent_modification());
        }
        replace_artifact(
            &self.path,
            self.target.bytes.as_deref(),
            self.current.bytes.is_some(),
        )
    }

    fn verify_target(&self) -> Result<(), EnvironmentFailure> {
        if artifact_matches(&self.path, self.target.bytes.as_deref())? {
            Ok(())
        } else {
            Err(artifact_write_failed())
        }
    }

    fn rollback(&self) -> Result<(), EnvironmentFailure> {
        if artifact_matches(&self.path, self.current.bytes.as_deref())? {
            return Ok(());
        }
        if !artifact_matches(&self.path, self.target.bytes.as_deref())? {
            return Err(concurrent_modification());
        }
        replace_artifact(
            &self.path,
            self.current.bytes.as_deref(),
            self.target.bytes.is_some(),
        )
    }
}

fn rollback_switch(
    prepared: &PreparedSwitch,
    config_applied: bool,
    credentials_applied: bool,
) -> Result<(), EnvironmentFailure> {
    if credentials_applied {
        prepared.credentials.restore()?;
    }
    if config_applied {
        prepared.config.restore()?;
    }
    Ok(())
}

fn rollback_restore(
    prepared: &PreparedRestore,
    config_applied: bool,
    credentials_applied: bool,
) -> Result<(), EnvironmentFailure> {
    if credentials_applied {
        prepared.credentials.rollback()?;
    }
    if config_applied {
        prepared.config.rollback()?;
    }
    Ok(())
}

fn persist_pending(
    connection: &mut Connection,
    prepared: &PreparedSwitch,
    backup: &Path,
) -> Result<(), EnvironmentFailure> {
    let snapshot = serde_json::to_string(&prepared.provider).map_err(|_| state_unavailable())?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let current_record = transaction
        .query_row(
            "SELECT name, verification_fingerprint FROM providers WHERE id = ?1",
            [&prepared.provider.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| state_unavailable())?;
    let expected_name = prepared
        .update_guard
        .as_ref()
        .map_or(prepared.provider.name.as_str(), |guard| {
            guard.original_name.as_str()
        });
    let expected_fingerprint = prepared.update_guard.as_ref().map_or(
        prepared.provider.verification_fingerprint.as_str(),
        |guard| guard.original_verification_fingerprint.as_str(),
    );
    if current_record
        .as_ref()
        .map(|(name, fingerprint)| (name.as_str(), fingerprint.as_str()))
        != Some((expected_name, expected_fingerprint))
    {
        return Err(state_unavailable());
    }
    transaction
        .execute(
            "INSERT INTO pending_config_operation (
                singleton, operation_id, operation_kind, stage, target_provider_id,
                old_config_fingerprint, new_config_fingerprint,
                old_credentials_fingerprint, new_credentials_fingerprint,
                backup_reference, target_snapshot_json, started_at
             ) VALUES (1, ?1, ?2, 'prepared', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                prepared.operation_id,
                if prepared.update_guard.is_some() {
                    "save_and_apply"
                } else {
                    "switch_provider"
                },
                prepared.provider.id,
                prepared.config.old_fingerprint,
                prepared.config.new_fingerprint,
                prepared.credentials.old_fingerprint,
                prepared.credentials.new_fingerprint,
                backup.to_string_lossy(),
                snapshot,
                epoch_seconds().to_string(),
            ],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn persist_pending_restore(
    connection: &mut Connection,
    prepared: &PreparedRestore,
    backup: &Path,
) -> Result<(), EnvironmentFailure> {
    let target_snapshot = serde_json::json!({
        "sourceBackup": prepared.source_backup.to_string_lossy(),
    })
    .to_string();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    transaction
        .execute(
            "INSERT INTO pending_config_operation (
                singleton, operation_id, operation_kind, stage,
                old_config_fingerprint, new_config_fingerprint,
                old_credentials_fingerprint, new_credentials_fingerprint,
                backup_reference, target_snapshot_json, started_at
             ) VALUES (1, ?1, 'restore_latest', 'prepared', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                prepared.operation_id,
                prepared.config.current_fingerprint,
                prepared.config.target_fingerprint,
                prepared.credentials.current_fingerprint,
                prepared.credentials.target_fingerprint,
                backup.to_string_lossy(),
                target_snapshot,
                epoch_seconds().to_string(),
            ],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn commit_applied_state(
    connection: &mut Connection,
    prepared: &PreparedSwitch,
) -> Result<(), EnvironmentFailure> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    if let Some(guard) = &prepared.update_guard {
        let changed = transaction
            .execute(
                "UPDATE providers SET
                    name = ?1, base_url = ?2, api_key = ?3, default_model = ?4,
                    verified_at = ?5, verification_fingerprint = ?6
                 WHERE id = ?7 AND name = ?8 AND verification_fingerprint = ?9",
                params![
                    prepared.provider.name,
                    prepared.provider.base_url,
                    prepared.provider.api_key,
                    prepared.provider.default_model,
                    prepared.provider.verified_at_epoch_seconds.to_string(),
                    prepared.provider.verification_fingerprint,
                    prepared.provider.id,
                    guard.original_name,
                    guard.original_verification_fingerprint,
                ],
            )
            .map_err(|_| state_unavailable())?;
        if changed != 1 {
            return Err(state_unavailable());
        }
    }
    transaction
        .execute(
            "INSERT INTO last_applied_state (
                singleton, mode, provider_id, config_fingerprint,
                credentials_fingerprint, applied_at
             ) VALUES (1, 'provider', ?1, ?2, ?3, ?4)
             ON CONFLICT(singleton) DO UPDATE SET
                mode = excluded.mode,
                provider_id = excluded.provider_id,
                config_fingerprint = excluded.config_fingerprint,
                credentials_fingerprint = excluded.credentials_fingerprint,
                applied_at = excluded.applied_at",
            params![
                prepared.provider.id,
                managed_config_fingerprint(&prepared.config.new_bytes)
                    .ok_or_else(state_unavailable)?,
                prepared.credentials.new_fingerprint,
                epoch_seconds().to_string(),
            ],
        )
        .map_err(|_| state_unavailable())?;
    transaction
        .execute(
            "DELETE FROM pending_config_operation WHERE singleton = 1 AND operation_id = ?1",
            [&prepared.operation_id],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn clear_pending(connection: &Connection, operation_id: &str) -> Result<(), EnvironmentFailure> {
    connection
        .execute(
            "DELETE FROM pending_config_operation WHERE singleton = 1 AND operation_id = ?1",
            [operation_id],
        )
        .map(|_| ())
        .map_err(|_| state_unavailable())
}

fn update_pending_stage(
    connection: &Connection,
    operation_id: &str,
    stage: &str,
) -> Result<(), EnvironmentFailure> {
    let changed = connection
        .execute(
            "UPDATE pending_config_operation SET stage = ?1
             WHERE singleton = 1 AND operation_id = ?2",
            params![stage, operation_id],
        )
        .map_err(|_| state_unavailable())?;
    if changed == 1 {
        Ok(())
    } else {
        Err(state_unavailable())
    }
}

fn mark_pending_conflict(
    connection: &mut Connection,
    operation_id: &str,
) -> Result<(), EnvironmentFailure> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    transaction
        .execute("DELETE FROM last_applied_state WHERE singleton = 1", [])
        .map_err(|_| state_unavailable())?;
    let changed = transaction
        .execute(
            "UPDATE pending_config_operation SET stage = 'conflict'
             WHERE singleton = 1 AND operation_id = ?1",
            [operation_id],
        )
        .map_err(|_| state_unavailable())?;
    if changed != 1 {
        return Err(state_unavailable());
    }
    transaction.commit().map_err(|_| state_unavailable())
}

fn commit_recovered_state(
    connection: &mut Connection,
    pending: &PendingRecovery,
    config: &ArtifactBytes,
    credentials: &ArtifactBytes,
) -> Result<(), EnvironmentFailure> {
    if pending.operation_kind == "restore_latest" {
        return commit_reconciled_state(connection, &pending.operation_id, config, credentials);
    }
    let provider: ProviderTarget =
        serde_json::from_str(&pending.target_snapshot_json).map_err(|_| state_unavailable())?;
    let config_fingerprint = config
        .bytes
        .as_deref()
        .and_then(managed_config_fingerprint)
        .ok_or_else(state_unavailable)?;
    let credentials_fingerprint = pending
        .new_credentials_fingerprint
        .as_deref()
        .ok_or_else(state_unavailable)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    if pending.operation_kind == "save_and_apply" {
        let changed = transaction
            .execute(
                "UPDATE providers SET
                    name = ?1, base_url = ?2, api_key = ?3, default_model = ?4,
                    verified_at = ?5, verification_fingerprint = ?6
                 WHERE id = ?7",
                params![
                    provider.name,
                    provider.base_url,
                    provider.api_key,
                    provider.default_model,
                    provider.verified_at_epoch_seconds.to_string(),
                    provider.verification_fingerprint,
                    provider.id,
                ],
            )
            .map_err(|_| state_unavailable())?;
        if changed != 1 {
            return Err(state_unavailable());
        }
    } else if pending.operation_kind != "switch_provider" {
        return Err(state_unavailable());
    }
    transaction
        .execute(
            "INSERT INTO last_applied_state (
                singleton, mode, provider_id, config_fingerprint,
                credentials_fingerprint, applied_at
             ) VALUES (1, 'provider', ?1, ?2, ?3, ?4)
             ON CONFLICT(singleton) DO UPDATE SET
                mode = excluded.mode,
                provider_id = excluded.provider_id,
                config_fingerprint = excluded.config_fingerprint,
                credentials_fingerprint = excluded.credentials_fingerprint,
                applied_at = excluded.applied_at",
            params![
                provider.id,
                config_fingerprint,
                credentials_fingerprint,
                epoch_seconds().to_string(),
            ],
        )
        .map_err(|_| state_unavailable())?;
    transaction
        .execute(
            "DELETE FROM pending_config_operation WHERE singleton = 1 AND operation_id = ?1",
            [&pending.operation_id],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn commit_restored_state(
    connection: &mut Connection,
    prepared: &PreparedRestore,
) -> Result<(), EnvironmentFailure> {
    commit_reconciled_state(
        connection,
        &prepared.operation_id,
        &prepared.config.target,
        &prepared.credentials.target,
    )
}

fn commit_reconciled_state(
    connection: &mut Connection,
    operation_id: &str,
    config: &ArtifactBytes,
    credentials: &ArtifactBytes,
) -> Result<(), EnvironmentFailure> {
    let applied = reconciled_applied_provider(connection, config, credentials)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    match applied {
        Some((provider_id, config_fingerprint, credentials_fingerprint)) => {
            transaction
                .execute(
                    "INSERT INTO last_applied_state (
                        singleton, mode, provider_id, config_fingerprint,
                        credentials_fingerprint, applied_at
                     ) VALUES (1, 'provider', ?1, ?2, ?3, ?4)
                     ON CONFLICT(singleton) DO UPDATE SET
                        mode = excluded.mode,
                        provider_id = excluded.provider_id,
                        config_fingerprint = excluded.config_fingerprint,
                        credentials_fingerprint = excluded.credentials_fingerprint,
                        applied_at = excluded.applied_at",
                    params![
                        provider_id,
                        config_fingerprint,
                        credentials_fingerprint,
                        epoch_seconds().to_string(),
                    ],
                )
                .map_err(|_| state_unavailable())?;
        }
        None => {
            transaction
                .execute("DELETE FROM last_applied_state WHERE singleton = 1", [])
                .map_err(|_| state_unavailable())?;
        }
    }
    transaction
        .execute(
            "DELETE FROM pending_config_operation WHERE singleton = 1 AND operation_id = ?1",
            [operation_id],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn reconciled_applied_provider(
    connection: &Connection,
    config: &ArtifactBytes,
    credentials: &ArtifactBytes,
) -> Result<Option<(String, String, String)>, EnvironmentFailure> {
    let Some(config_bytes) = config.bytes.as_deref() else {
        return Ok(None);
    };
    let Ok(text) = std::str::from_utf8(config_bytes) else {
        return Ok(None);
    };
    let Ok(document) = text.parse::<DocumentMut>() else {
        return Ok(None);
    };
    let ManagedBlock::Valid(managed) = managed_block(text) else {
        return Ok(None);
    };
    if !managed_block_is_root_scoped(&document, text, &managed) {
        return Ok(None);
    }
    let provider = match load_provider(connection, &managed.provider_id) {
        Ok(provider) => provider,
        Err(failure) if failure.category == EnvironmentFailureCategory::ProviderNotFound => {
            return Ok(None);
        }
        Err(failure) => return Err(failure),
    };
    if !managed_config_matches(&document, &provider)
        || !credentials_match(credentials, &provider.api_key)?
    {
        return Ok(None);
    }
    let Some(config_fingerprint) = managed_config_fingerprint(config_bytes) else {
        return Ok(None);
    };
    let Some(credentials_fingerprint) = credentials.fingerprint(ArtifactKind::Credentials) else {
        return Ok(None);
    };
    Ok(Some((
        provider.id,
        config_fingerprint,
        credentials_fingerprint,
    )))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    format_version: u8,
    operation_id: String,
    operation_kind: String,
    config_existed: bool,
    credentials_existed: bool,
    old_config_fingerprint: Option<String>,
    new_config_fingerprint: Option<String>,
    old_credentials_fingerprint: Option<String>,
    new_credentials_fingerprint: Option<String>,
}

fn create_backup(
    codex_home: &Path,
    prepared: &PreparedSwitch,
) -> Result<PathBuf, EnvironmentFailure> {
    let root = codex_home.join(".gpteasy-backups");
    reject_redirect(&root)?;
    let operation = root.join(format!(
        "operation-{}-{}",
        epoch_nanos(),
        prepared.operation_id
    ));
    fs::create_dir_all(&operation).map_err(|_| backup_failed())?;
    let result = (|| {
        if let Some(bytes) = prepared.config.old.bytes.as_deref() {
            write_new_synced(&operation.join("config.toml"), bytes).map_err(|_| backup_failed())?;
        }
        if let Some(bytes) = prepared.credentials.old.bytes.as_deref() {
            write_new_synced(&operation.join("auth.json"), bytes).map_err(|_| backup_failed())?;
        }
        let manifest = serde_json::to_vec_pretty(&BackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            operation_id: prepared.operation_id.clone(),
            operation_kind: if prepared.update_guard.is_some() {
                "save_and_apply".to_owned()
            } else {
                "switch_provider".to_owned()
            },
            config_existed: prepared.config.old.bytes.is_some(),
            credentials_existed: prepared.credentials.old.bytes.is_some(),
            old_config_fingerprint: prepared.config.old_fingerprint.clone(),
            new_config_fingerprint: Some(prepared.config.new_fingerprint.clone()),
            old_credentials_fingerprint: prepared.credentials.old_fingerprint.clone(),
            new_credentials_fingerprint: Some(prepared.credentials.new_fingerprint.clone()),
        })
        .map_err(|_| backup_failed())?;
        write_new_synced(&operation.join("manifest.json"), &manifest)
            .map_err(|_| backup_failed())?;
        prune_backups(&root)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&operation);
    }
    result.map(|_| operation)
}

fn create_restore_backup(
    codex_home: &Path,
    prepared: &PreparedRestore,
) -> Result<PathBuf, EnvironmentFailure> {
    let root = codex_home.join(".gpteasy-backups");
    reject_redirect(&root)?;
    let operation = root.join(format!(
        "operation-{}-{}",
        epoch_nanos(),
        prepared.operation_id
    ));
    fs::create_dir_all(&operation).map_err(|_| backup_failed())?;
    let result = (|| {
        if let Some(bytes) = prepared.config.current.bytes.as_deref() {
            write_new_synced(&operation.join("config.toml"), bytes).map_err(|_| backup_failed())?;
        }
        if let Some(bytes) = prepared.credentials.current.bytes.as_deref() {
            write_new_synced(&operation.join("auth.json"), bytes).map_err(|_| backup_failed())?;
        }
        let manifest = serde_json::to_vec_pretty(&BackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            operation_id: prepared.operation_id.clone(),
            operation_kind: "restore_latest".to_owned(),
            config_existed: prepared.config.current.bytes.is_some(),
            credentials_existed: prepared.credentials.current.bytes.is_some(),
            old_config_fingerprint: prepared.config.current_fingerprint.clone(),
            new_config_fingerprint: prepared.config.target_fingerprint.clone(),
            old_credentials_fingerprint: prepared.credentials.current_fingerprint.clone(),
            new_credentials_fingerprint: prepared.credentials.target_fingerprint.clone(),
        })
        .map_err(|_| backup_failed())?;
        write_new_synced(&operation.join("manifest.json"), &manifest)
            .map_err(|_| backup_failed())?;
        prune_backups(&root)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&operation);
    }
    result.map(|_| operation)
}

fn mark_backup_completed(backup: &Path) -> Result<(), EnvironmentFailure> {
    reject_redirect(backup).map_err(|_| backup_failed())?;
    let marker = backup.join(BACKUP_COMPLETION_FILE);
    reject_redirect(&marker).map_err(|_| backup_failed())?;
    match fs::read(&marker) {
        Ok(contents) if contents == BACKUP_COMPLETION_CONTENT => Ok(()),
        Ok(_) => Err(backup_failed()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_new_synced(&marker, BACKUP_COMPLETION_CONTENT).map_err(|_| backup_failed())
        }
        Err(_) => Err(backup_failed()),
    }
}

fn unmark_backup_completed(backup: &Path) -> Result<(), EnvironmentFailure> {
    let marker = backup.join(BACKUP_COMPLETION_FILE);
    reject_redirect(&marker).map_err(|_| backup_failed())?;
    match fs::remove_file(marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(backup_failed()),
    }
}

fn latest_completed_backup(
    codex_home: &Path,
) -> Result<Option<CompletedBackup>, EnvironmentFailure> {
    let root = codex_home.join(".gpteasy-backups");
    reject_redirect(&root).map_err(|_| backup_invalid())?;
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(backup_invalid()),
    };
    let mut completed = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| backup_invalid())?;
        let file_type = entry.file_type().map_err(|_| backup_invalid())?;
        let path = entry.path();
        if !file_type.is_dir()
            || !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("operation-"))
        {
            continue;
        }
        let marker = path.join(BACKUP_COMPLETION_FILE);
        let marker_type = match fs::symlink_metadata(&marker) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(backup_invalid()),
        };
        if !marker_type.file_type().is_file()
            || fs::read(&marker).map_err(|_| backup_invalid())? != BACKUP_COMPLETION_CONTENT
        {
            return Err(backup_invalid());
        }
        completed.push(path);
    }
    completed.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    completed
        .into_iter()
        .next()
        .map(load_completed_backup)
        .transpose()
}

fn pending_backup_path(codex_home: &Path, backup: &Path) -> Result<PathBuf, EnvironmentFailure> {
    let root = codex_home.join(".gpteasy-backups");
    if backup.parent() != Some(root.as_path())
        || !backup
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("operation-"))
    {
        return Err(backup_invalid());
    }
    reject_redirect(&root).map_err(|_| backup_invalid())?;
    reject_redirect(backup).map_err(|_| backup_invalid())?;
    Ok(backup.to_path_buf())
}

fn load_completed_backup(path: PathBuf) -> Result<CompletedBackup, EnvironmentFailure> {
    reject_redirect(&path).map_err(|_| backup_invalid())?;
    let manifest_path = path.join("manifest.json");
    reject_redirect(&manifest_path).map_err(|_| backup_invalid())?;
    let manifest: BackupManifest =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|_| backup_invalid())?)
            .map_err(|_| backup_invalid())?;
    let valid_kind = matches!(
        manifest.operation_kind.as_str(),
        "switch_provider" | "save_and_apply" | "restore_latest"
    );
    let operation_name_matches = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(&manifest.operation_id));
    if manifest.format_version != BACKUP_FORMAT_VERSION
        || !valid_kind
        || Uuid::parse_str(&manifest.operation_id).is_err()
        || !operation_name_matches
        || manifest.config_existed != manifest.old_config_fingerprint.is_some()
        || manifest.credentials_existed != manifest.old_credentials_fingerprint.is_some()
    {
        return Err(backup_invalid());
    }
    let config = read_backup_artifact(
        &path.join("config.toml"),
        manifest.config_existed,
        manifest.old_config_fingerprint.as_deref(),
        ArtifactKind::Config,
    )?;
    let credentials = read_backup_artifact(
        &path.join("auth.json"),
        manifest.credentials_existed,
        manifest.old_credentials_fingerprint.as_deref(),
        ArtifactKind::Credentials,
    )?;
    Ok(CompletedBackup {
        path,
        manifest,
        config,
        credentials,
    })
}

fn read_backup_artifact(
    path: &Path,
    existed: bool,
    expected_fingerprint: Option<&str>,
    kind: ArtifactKind,
) -> Result<ArtifactBytes, EnvironmentFailure> {
    reject_redirect(path).map_err(|_| backup_invalid())?;
    if !existed {
        if path.exists() || expected_fingerprint.is_some() {
            return Err(backup_invalid());
        }
        return Ok(ArtifactBytes { bytes: None });
    }
    let bytes = fs::read(path).map_err(|_| backup_invalid())?;
    if expected_fingerprint != Some(artifact_hash(kind, &bytes).as_str()) {
        return Err(backup_invalid());
    }
    match kind {
        ArtifactKind::Config => {
            let text = std::str::from_utf8(&bytes).map_err(|_| backup_invalid())?;
            text.parse::<DocumentMut>().map_err(|_| backup_invalid())?;
        }
        ArtifactKind::Credentials => {
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| backup_invalid())?;
            if !value.is_object() {
                return Err(backup_invalid());
            }
        }
    }
    Ok(ArtifactBytes { bytes: Some(bytes) })
}

fn prune_backups(root: &Path) -> Result<(), EnvironmentFailure> {
    let mut operations = fs::read_dir(root)
        .map_err(|_| backup_failed())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("operation-"))
        })
        .collect::<Vec<_>>();
    operations.sort();
    let remove_count = operations.len().saturating_sub(BACKUP_LIMIT);
    for obsolete in operations.into_iter().take(remove_count) {
        fs::remove_dir_all(obsolete).map_err(|_| backup_failed())?;
    }
    Ok(())
}

fn render_config(
    original: Option<&[u8]>,
    provider: &ProviderTarget,
) -> Result<Vec<u8>, EnvironmentFailure> {
    let original = original.unwrap_or_default();
    let text = std::str::from_utf8(original).map_err(|_| invalid_config())?;
    let document = text.parse::<DocumentMut>().map_err(|_| invalid_config())?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let block = render_managed_block(provider, newline);
    let rendered = match managed_block(text) {
        ManagedBlock::None => {
            let body = migrate_external_config(text, &provider.id, newline)?;
            format!("{block}{body}")
        }
        ManagedBlock::Valid(existing) => {
            if !managed_block_is_root_scoped(&document, text, &existing) {
                return Err(managed_conflict());
            }
            let mut rendered = String::with_capacity(text.len() + block.len());
            rendered.push_str(&text[..existing.start]);
            rendered.push_str(&block);
            rendered.push_str(&text[existing.end..]);
            rendered
        }
        ManagedBlock::Conflict => return Err(managed_conflict()),
    };
    rendered
        .parse::<DocumentMut>()
        .map_err(|_| invalid_config())?;
    Ok(rendered.into_bytes())
}

fn migrate_external_config(
    original: &str,
    target_provider_id: &str,
    newline: &str,
) -> Result<String, EnvironmentFailure> {
    let mut document = original
        .parse::<DocumentMut>()
        .map_err(|_| invalid_config())?;
    document.remove("model");
    document.remove("model_provider");
    let mut remove_parent = false;
    if let Some(item) = document.get_mut("model_providers") {
        let providers = item.as_table_mut().ok_or_else(invalid_config)?;
        providers.remove(target_provider_id);
        if providers.iter().any(|(_, value)| !value.is_table()) {
            return Err(invalid_config());
        }
        if providers.is_empty() {
            remove_parent = true;
        } else {
            providers.set_implicit(true);
        }
    }
    if remove_parent {
        document.remove("model_providers");
    }
    Ok(normalize_newlines(&document.to_string(), newline))
}

fn render_managed_block(provider: &ProviderTarget, newline: &str) -> String {
    let string = |value: &str| toml_edit::Value::from(value).to_string();
    let table = format!("model_providers.{}", provider.id);
    [
        MANAGED_START.to_owned(),
        format!("{PROVIDER_ID_PREFIX} {}", provider.id),
        format!("model = {}", string(&provider.default_model)),
        format!("model_provider = {}", string(&provider.id)),
        format!("{table}.name = {}", string(&provider.name)),
        format!("{table}.base_url = {}", string(&provider.base_url)),
        format!("{table}.wire_api = \"responses\""),
        format!("{table}.requires_openai_auth = true"),
        MANAGED_END.to_owned(),
        String::new(),
    ]
    .join(newline)
}

fn render_credentials(
    original: Option<&[u8]>,
    api_key: &str,
) -> Result<Vec<u8>, EnvironmentFailure> {
    let mut object = match original {
        Some(bytes) => serde_json::from_slice::<Value>(bytes)
            .map_err(|_| invalid_credentials())?
            .as_object()
            .cloned()
            .ok_or_else(invalid_credentials)?,
        None => Map::new(),
    };
    object.insert("auth_mode".to_owned(), Value::String("apikey".to_owned()));
    object.insert(
        "OPENAI_API_KEY".to_owned(),
        Value::String(api_key.to_owned()),
    );
    let mut bytes =
        serde_json::to_vec_pretty(&Value::Object(object)).map_err(|_| invalid_credentials())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn ensure_file_credential_store(config: Option<&[u8]>) -> Result<(), EnvironmentFailure> {
    let Some(bytes) = config else {
        return Ok(());
    };
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_config())?;
    let document = text.parse::<DocumentMut>().map_err(|_| invalid_config())?;
    let Some(item) = document.get("cli_auth_credentials_store") else {
        return Ok(());
    };
    let Some(value) = item.as_str() else {
        return Err(invalid_config());
    };
    match value {
        "file" => Ok(()),
        _ => Err(EnvironmentFailure::new(
            EnvironmentFailureCategory::UnsupportedCredentialStore,
            "environment.file_credentials_required",
        )),
    }
}

#[derive(Debug, Clone)]
struct ManagedBlockRange {
    start: usize,
    end: usize,
    provider_id: String,
}

enum ManagedBlock {
    None,
    Valid(ManagedBlockRange),
    Conflict,
}

fn managed_block(text: &str) -> ManagedBlock {
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if content == MANAGED_START {
            starts.push((offset, offset + line.len()));
        }
        if content == MANAGED_END {
            ends.push((offset, offset + line.len()));
        }
        offset += line.len();
    }
    if offset < text.len() {
        let content = text[offset..].trim_end_matches('\r');
        if content == MANAGED_START {
            starts.push((offset, text.len()));
        }
        if content == MANAGED_END {
            ends.push((offset, text.len()));
        }
    }
    if starts
        .iter()
        .chain(ends.iter())
        .any(|(start, _)| is_inside_multiline_string(text, *start))
    {
        return ManagedBlock::Conflict;
    }
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => ManagedBlock::None,
        ([(start, _)], [(_, end)]) if start < end => {
            let block = &text[*start..*end];
            let provider_ids = block
                .lines()
                .filter_map(|line| line.strip_prefix(PROVIDER_ID_PREFIX))
                .map(str::trim)
                .collect::<Vec<_>>();
            match provider_ids.as_slice() {
                [provider_id]
                    if !provider_id.is_empty()
                        && Uuid::parse_str(provider_id).is_ok()
                        && managed_block_has_expected_shape(block, provider_id) =>
                {
                    ManagedBlock::Valid(ManagedBlockRange {
                        start: *start,
                        end: *end,
                        provider_id: (*provider_id).to_owned(),
                    })
                }
                _ => ManagedBlock::Conflict,
            }
        }
        _ => ManagedBlock::Conflict,
    }
}

fn managed_block_has_expected_shape(block: &str, provider_id: &str) -> bool {
    let Ok(document) = block.parse::<DocumentMut>() else {
        return false;
    };
    if document.iter().count() != 3
        || document
            .get("model")
            .and_then(|item| item.as_str())
            .is_none()
        || document
            .get("model_provider")
            .and_then(|item| item.as_str())
            != Some(provider_id)
    {
        return false;
    }
    let Some(providers) = document
        .get("model_providers")
        .and_then(|item| item.as_table())
    else {
        return false;
    };
    if providers.len() != 1 {
        return false;
    }
    let Some(provider) = providers.get(provider_id).and_then(|item| item.as_table()) else {
        return false;
    };
    managed_provider_fields(provider).is_some()
}

fn managed_block_is_root_scoped(
    document: &DocumentMut,
    text: &str,
    managed: &ManagedBlockRange,
) -> bool {
    let Some(block) = text.get(managed.start..managed.end) else {
        return false;
    };
    let Ok(block_document) = block.parse::<DocumentMut>() else {
        return false;
    };
    if document.get("model").and_then(|item| item.as_str())
        != block_document.get("model").and_then(|item| item.as_str())
        || document
            .get("model_provider")
            .and_then(|item| item.as_str())
            != Some(&managed.provider_id)
    {
        return false;
    }
    let (Some(actual), Some(expected)) = (
        managed_provider_table(document, &managed.provider_id),
        managed_provider_table(&block_document, &managed.provider_id),
    ) else {
        return false;
    };
    managed_provider_fields(actual) == managed_provider_fields(expected)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ManagedProviderFields<'a> {
    name: &'a str,
    base_url: &'a str,
    wire_api: &'a str,
    requires_openai_auth: bool,
}

fn managed_provider_fields(table: &toml_edit::Table) -> Option<ManagedProviderFields<'_>> {
    if table.len() != 4 {
        return None;
    }
    Some(ManagedProviderFields {
        name: table.get("name")?.as_str()?,
        base_url: table.get("base_url")?.as_str()?,
        wire_api: table.get("wire_api")?.as_str()?,
        requires_openai_auth: table.get("requires_openai_auth")?.as_bool()?,
    })
    .filter(|fields| fields.wire_api == "responses" && fields.requires_openai_auth)
}

fn managed_provider_table<'a>(
    document: &'a DocumentMut,
    provider_id: &str,
) -> Option<&'a toml_edit::Table> {
    document
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(provider_id))
        .and_then(|item| item.as_table())
}

pub(crate) fn managed_config_fingerprint(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let ManagedBlock::Valid(block) = managed_block(text) else {
        return None;
    };
    Some(artifact_hash(
        ArtifactKind::Config,
        text.as_bytes().get(block.start..block.end)?,
    ))
}

fn is_inside_multiline_string(text: &str, target: usize) -> bool {
    #[derive(Clone, Copy)]
    enum StringKind {
        Basic,
        Literal,
    }

    let bytes = text.as_bytes();
    let mut kind = None;
    let mut index = 0;
    while index < target {
        match kind {
            None => {
                if bytes[index] == b'#' {
                    while index < target && bytes[index] != b'\n' {
                        index += 1;
                    }
                    continue;
                }
                if bytes[index] == b'"' {
                    if bytes.get(index..index + 3) == Some(b"\"\"\"") {
                        kind = Some(StringKind::Basic);
                        index += 3;
                    } else {
                        index += 1;
                        while index < target {
                            if bytes[index] == b'\\' {
                                index = (index + 2).min(target);
                            } else if bytes[index] == b'"' {
                                index += 1;
                                break;
                            } else {
                                index += 1;
                            }
                        }
                    }
                } else if bytes[index] == b'\'' {
                    if bytes.get(index..index + 3) == Some(b"'''") {
                        kind = Some(StringKind::Literal);
                        index += 3;
                    } else {
                        index += 1;
                        while index < target && bytes[index] != b'\'' {
                            index += 1;
                        }
                        if index < target {
                            index += 1;
                        }
                    }
                } else {
                    index += 1;
                }
            }
            Some(StringKind::Basic) => {
                if bytes.get(index..index + 3) == Some(b"\"\"\"") {
                    kind = None;
                    index += 3;
                } else if bytes[index] == b'\\' {
                    index = (index + 2).min(target);
                } else {
                    index += 1;
                }
            }
            Some(StringKind::Literal) => {
                if bytes.get(index..index + 3) == Some(b"'''") {
                    kind = None;
                    index += 3;
                } else {
                    index += 1;
                }
            }
        }
    }
    kind.is_some()
}

fn managed_config_matches(document: &DocumentMut, provider: &ProviderTarget) -> bool {
    document.get("model").and_then(|item| item.as_str()) == Some(&provider.default_model)
        && document
            .get("model_provider")
            .and_then(|item| item.as_str())
            == Some(&provider.id)
        && document["model_providers"][&provider.id]["base_url"].as_str()
            == Some(&provider.base_url)
        && document["model_providers"][&provider.id]["wire_api"].as_str() == Some("responses")
        && document["model_providers"][&provider.id]["requires_openai_auth"].as_bool() == Some(true)
}

fn credentials_match(
    credentials: &ArtifactBytes,
    expected_api_key: &str,
) -> Result<bool, EnvironmentFailure> {
    let Some(bytes) = credentials.bytes.as_deref() else {
        return Ok(false);
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid_credentials())?;
    Ok(
        value.get("auth_mode").and_then(Value::as_str) == Some("apikey")
            && value.get("OPENAI_API_KEY").and_then(Value::as_str) == Some(expected_api_key),
    )
}

fn read_artifact(path: &Path) -> Result<ArtifactBytes, EnvironmentFailure> {
    reject_redirect(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(ArtifactBytes { bytes: Some(bytes) }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ArtifactBytes { bytes: None })
        }
        Err(_) => Err(artifact_write_failed()),
    }
}

fn artifact_matches(path: &Path, expected: Option<&[u8]>) -> Result<bool, EnvironmentFailure> {
    reject_redirect(path)?;
    match (fs::read(path), expected) {
        (Ok(actual), Some(expected)) => Ok(actual == expected),
        (Err(error), None) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        (Ok(_), None) | (Err(_), Some(_)) => Ok(false),
        (Err(_), None) => Err(artifact_write_failed()),
    }
}

fn reject_redirect(path: &Path) -> Result<(), EnvironmentFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(EnvironmentFailure::new(
            EnvironmentFailureCategory::ArtifactRedirected,
            "environment.artifact_redirected",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(artifact_write_failed()),
    }
}

fn replace_artifact(
    path: &Path,
    target: Option<&[u8]>,
    target_existed: bool,
) -> Result<(), EnvironmentFailure> {
    match target {
        Some(bytes) => {
            let temporary = write_temporary(path, bytes)?;
            atomic_replace(path, &temporary, target_existed)
        }
        None if target_existed => fs::remove_file(path).map_err(|_| artifact_write_failed()),
        None => Ok(()),
    }
}

fn write_temporary(path: &Path, bytes: &[u8]) -> Result<PathBuf, EnvironmentFailure> {
    let parent = path.parent().ok_or_else(artifact_write_failed)?;
    fs::create_dir_all(parent).map_err(|_| artifact_write_failed())?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    let temporary = parent.join(format!(
        ".{file_name}.gpteasy-{}-{}.tmp",
        epoch_nanos(),
        Uuid::new_v4()
    ));
    write_new_synced(&temporary, bytes).map_err(|_| artifact_write_failed())?;
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temporary, metadata.permissions())
            .map_err(|_| artifact_write_failed())?;
    }
    Ok(temporary)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(windows)]
fn atomic_replace(
    target: &Path,
    replacement: &Path,
    target_existed: bool,
) -> Result<(), EnvironmentFailure> {
    if !target_existed {
        return fs::rename(replacement, target).map_err(|_| artifact_write_failed());
    }
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
    let replaced = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            replacement_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        let _ = fs::remove_file(replacement);
        Err(artifact_write_failed())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(
    target: &Path,
    replacement: &Path,
    _target_existed: bool,
) -> Result<(), EnvironmentFailure> {
    fs::rename(replacement, target).map_err(|_| artifact_write_failed())?;
    if let Some(parent) = target.parent() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| artifact_write_failed())?;
    }
    Ok(())
}

fn normalize_newlines(value: &str, newline: &str) -> String {
    let value = value.replace("\r\n", "\n");
    if newline == "\r\n" {
        value.replace('\n', "\r\n")
    } else {
        value
    }
}

fn artifact_hash(kind: ArtifactKind, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    if kind == ArtifactKind::Credentials {
        hasher.update(b"file:present:");
    }
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn environment_revision(config: &ArtifactBytes, credentials: &ArtifactBytes) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-environment-revision-v1\0");
    for (label, artifact) in [
        (b"config".as_slice(), config),
        (b"credentials", credentials),
    ] {
        hasher.update(label);
        match artifact.bytes.as_deref() {
            Some(bytes) => {
                hasher.update(b":present:");
                hasher.update(bytes);
            }
            None => hasher.update(b":missing"),
        }
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn epoch_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn state_unavailable() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::StateUnavailable,
        "environment.state_unavailable",
    )
}

fn invalid_config() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::InvalidConfig,
        "environment.config_invalid",
    )
}

fn invalid_credentials() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::InvalidCredentials,
        "environment.credentials_invalid",
    )
}

fn managed_conflict() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::ManagedConflict,
        "environment.managed_conflict",
    )
}

fn backup_failed() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::BackupFailed,
        "environment.backup_failed",
    )
}

fn backup_invalid() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::BackupInvalid,
        "environment.backup_invalid",
    )
}

fn restore_unavailable() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::RestoreUnavailable,
        "environment.restore_unavailable",
    )
}

fn concurrent_modification() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::ConcurrentModification,
        "environment.concurrent_modification",
    )
}

fn artifact_write_failed() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::ArtifactWriteFailed,
        "environment.artifact_write_failed",
    )
}

fn operation_interrupted() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::OperationInterrupted,
        "environment.operation_interrupted",
    )
}
