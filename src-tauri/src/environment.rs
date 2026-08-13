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

use crate::codex::{LoginStatus, LoginStatusCommand};
pub use crate::consumer::ConsumerStatus;
use crate::consumer::{ConsumerIdentity, ConsumerScan, ConsumerScanner, WindowsConsumerScanner};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationMode {
    Provider,
    OpenaiLogin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerStatuses {
    pub desktop: ConsumerStatus,
    pub cli: ConsumerStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingRestartContext {
    consumers: Vec<ConsumerIdentity>,
    switched_at_epoch_millis: u64,
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
    pub mode: Option<AuthenticationMode>,
    pub message_id: &'static str,
    pub revision: String,
    pub requires_takeover_confirmation: bool,
    pub takeover_available: bool,
    pub impacts: Vec<ArtifactImpact>,
    pub current_provider: Option<ProviderSummary>,
    pub restore_availability: RestoreAvailability,
    pub restore_preview: Option<RestorePreview>,
    pub login_status: LoginStatus,
    pub pending_restart: bool,
    pub requires_consumer_confirmation: bool,
    pub consumers: ConsumerStatuses,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    pub artifacts: Vec<ArtifactKind>,
    pub target_mode: Option<AuthenticationMode>,
    pub target_provider: Option<ProviderSummary>,
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
    ModeSwitchConfirmationRequired,
    ConsumerConfirmationRequired,
    OpenAiLoginRequired,
    OpenAiLoginUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentFailure {
    pub category: EnvironmentFailureCategory,
    pub message_id: &'static str,
}

impl EnvironmentFailure {
    pub(crate) fn new(category: EnvironmentFailureCategory, message_id: &'static str) -> Self {
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

pub trait OpenAiLoginProbe: Send + Sync {
    fn status(&self) -> LoginStatus;
}

impl OpenAiLoginProbe for LoginStatusCommand {
    fn status(&self) -> LoginStatus {
        self.status()
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
    login_probe: Arc<dyn OpenAiLoginProbe>,
    consumer_scanner: Arc<dyn ConsumerScanner>,
}

impl EnvironmentApplication {
    pub fn new(state_store: StateStore, codex_home: impl AsRef<Path>) -> Self {
        Self::with_dependencies_and_scanner(
            state_store,
            codex_home,
            Arc::new(NoFaults),
            Arc::new(LoginStatusCommand::codex_default()),
            Arc::new(WindowsConsumerScanner::new()),
        )
    }

    #[doc(hidden)]
    pub fn with_fault_injector(
        state_store: StateStore,
        codex_home: impl AsRef<Path>,
        faults: Arc<dyn EnvironmentFaultInjector>,
    ) -> Self {
        Self::with_dependencies_and_scanner(
            state_store,
            codex_home,
            faults,
            Arc::new(LoginStatusCommand::codex_default()),
            Arc::new(WindowsConsumerScanner::new()),
        )
    }

    #[doc(hidden)]
    pub fn with_login_probe(
        state_store: StateStore,
        codex_home: impl AsRef<Path>,
        login_probe: Arc<dyn OpenAiLoginProbe>,
    ) -> Self {
        Self::with_dependencies_and_scanner(
            state_store,
            codex_home,
            Arc::new(NoFaults),
            login_probe,
            Arc::new(WindowsConsumerScanner::new()),
        )
    }

    #[doc(hidden)]
    pub fn with_dependencies(
        state_store: StateStore,
        codex_home: impl AsRef<Path>,
        faults: Arc<dyn EnvironmentFaultInjector>,
        login_probe: Arc<dyn OpenAiLoginProbe>,
    ) -> Self {
        Self::with_dependencies_and_scanner(
            state_store,
            codex_home,
            faults,
            login_probe,
            Arc::new(WindowsConsumerScanner::new()),
        )
    }

    #[doc(hidden)]
    pub fn with_consumer_scanner(
        state_store: StateStore,
        codex_home: impl AsRef<Path>,
        scanner: Arc<dyn ConsumerScanner>,
    ) -> Self {
        Self::with_dependencies_and_scanner(
            state_store,
            codex_home,
            Arc::new(NoFaults),
            Arc::new(LoginStatusCommand::codex_default()),
            scanner,
        )
    }

    #[doc(hidden)]
    pub fn with_runtime_probes(
        state_store: StateStore,
        codex_home: impl AsRef<Path>,
        login_probe: Arc<dyn OpenAiLoginProbe>,
        consumer_scanner: Arc<dyn ConsumerScanner>,
    ) -> Self {
        Self::with_dependencies_and_scanner(
            state_store,
            codex_home,
            Arc::new(NoFaults),
            login_probe,
            consumer_scanner,
        )
    }

    #[doc(hidden)]
    pub fn with_runtime_dependencies(
        state_store: StateStore,
        codex_home: impl AsRef<Path>,
        faults: Arc<dyn EnvironmentFaultInjector>,
        login_probe: Arc<dyn OpenAiLoginProbe>,
        consumer_scanner: Arc<dyn ConsumerScanner>,
    ) -> Self {
        Self::with_dependencies_and_scanner(
            state_store,
            codex_home,
            faults,
            login_probe,
            consumer_scanner,
        )
    }

    fn with_dependencies_and_scanner(
        state_store: StateStore,
        codex_home: impl AsRef<Path>,
        faults: Arc<dyn EnvironmentFaultInjector>,
        login_probe: Arc<dyn OpenAiLoginProbe>,
        consumer_scanner: Arc<dyn ConsumerScanner>,
    ) -> Self {
        Self {
            state_store,
            codex_home: codex_home.as_ref().to_path_buf(),
            operation_lock: Arc::new(Mutex::new(())),
            faults,
            login_probe,
            consumer_scanner,
        }
    }

    pub fn inspect(&self) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let connection = self.open_state()?;
        self.inspect_environment(&connection)
    }

    pub fn has_pending_restart(&self) -> Result<bool, EnvironmentFailure> {
        let connection = self.open_state()?;
        connection
            .query_row(
                "SELECT pending_restart FROM app_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| state_unavailable())
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
                        new_credentials_fingerprint, backup_reference, target_snapshot_json,
                        restart_context
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
                        restart_context: row.get::<_, Option<String>>(8)?,
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
        let credentials_affected = pending.old_credentials_fingerprint.is_some()
            || pending.new_credentials_fingerprint.is_some();
        let credentials = if credentials_affected {
            read_artifact(&self.codex_home.join("auth.json"))?
        } else {
            ArtifactBytes { bytes: None }
        };
        let current_config = config.fingerprint(ArtifactKind::Config);
        let current_credentials = credentials.fingerprint(ArtifactKind::Credentials);
        if fingerprints_match(&current_config, &pending.old_config_fingerprint, true)
            && fingerprints_match(
                &current_credentials,
                &pending.old_credentials_fingerprint,
                credentials_affected,
            )
        {
            unmark_backup_completed(&backup)?;
            clear_pending(&connection, &pending.operation_id)?;
            return Ok(EnvironmentRecovery::KeptOldState);
        }
        if fingerprints_match(&current_config, &pending.new_config_fingerprint, true)
            && fingerprints_match(
                &current_credentials,
                &pending.new_credentials_fingerprint,
                credentials_affected,
            )
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
        let before = self.inspect_environment(&connection)?;
        let config = read_artifact(&self.codex_home.join("config.toml"))?;
        let credentials = read_artifact(&self.codex_home.join("auth.json"))?;
        if environment_revision(&config, &credentials) != expected_revision {
            return Err(concurrent_modification());
        }
        let consumer_scan = self.consumer_scanner.scan();
        let mut restart_context = pending_restart_context(&consumer_scan);
        let backup = latest_completed_backup(&self.codex_home)?.ok_or_else(restore_unavailable)?;
        let restore_target = backup.restore_target(&connection)?;
        let managed_current =
            reconciled_applied_provider(&connection, &config, Some(&credentials))?.is_some();
        if !backup.matches_current(&config, &credentials, managed_current) {
            return Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::ManagedConflict,
                "environment.restore_conflict",
            ));
        }
        let prepared = PreparedRestore::new(
            &self.codex_home,
            config,
            credentials,
            backup,
            restore_target,
        )?;
        if self.faults.fails_backup_creation() {
            return Err(backup_failed());
        }
        let rollback_backup = create_restore_backup(&self.codex_home, &prepared, &before)?;
        self.check_interruption(EnvironmentFailurePoint::AfterBackupCompleted)?;
        persist_pending_restore(
            &mut connection,
            &prepared,
            &rollback_backup,
            restart_context.as_ref(),
        )?;
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
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            if let Some(credentials) = &prepared.credentials {
                self.check_fault(EnvironmentFailurePoint::BeforeCredentialsReplace)?;
                credentials.commit()?;
                credentials_applied = true;
            }
            prepared.verify_committed()?;
            update_pending_stage(&connection, &prepared.operation_id, "artifacts_replaced")?;
            finalize_restart_context(&connection, &prepared.operation_id, &mut restart_context)?;
            mark_backup_completed(&rollback_backup)?;
            backup_completed = true;
            self.check_interruption(EnvironmentFailurePoint::AfterAllArtifactsReplaced)
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            self.check_interruption(EnvironmentFailurePoint::BeforeDatabaseCommit)
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            self.check_fault(EnvironmentFailurePoint::BeforeDatabaseCommit)?;
            commit_restored_state(&mut connection, &prepared, restart_context.as_ref())?;
            self.check_interruption(EnvironmentFailurePoint::AfterDatabaseCommit)
                .inspect_err(|_| {
                    interrupted = true;
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

        self.inspect_environment_after_write(&connection, &consumer_scan)
    }

    pub fn apply_provider(
        &self,
        provider_id: &str,
        confirm_switch_risk: bool,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let revision = self.inspect()?.revision;
        self.apply_provider_at_revision(provider_id, confirm_switch_risk, &revision)
    }

    pub fn switch_to_openai_login(
        &self,
        confirm_switch: bool,
        expected_revision: &str,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        if !confirm_switch {
            return Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::ModeSwitchConfirmationRequired,
                "environment.mode_switch_confirmation_required",
            ));
        }
        match self.login_probe.status() {
            LoginStatus::LoggedIn => {}
            LoginStatus::NotLoggedIn => {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::OpenAiLoginRequired,
                    "environment.openai_login_required",
                ));
            }
            LoginStatus::Unavailable => {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::OpenAiLoginUnavailable,
                    "environment.openai_login_unavailable",
                ));
            }
        }
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let mut connection = self.open_state()?;
        let before = self.inspect_environment(&connection)?;
        match before.login_status {
            LoginStatus::LoggedIn => {}
            LoginStatus::NotLoggedIn => {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::OpenAiLoginRequired,
                    "environment.openai_login_required",
                ));
            }
            LoginStatus::Unavailable => {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::OpenAiLoginUnavailable,
                    "environment.openai_login_unavailable",
                ));
            }
        }
        if before.revision != expected_revision {
            return Err(concurrent_modification());
        }
        let consumer_scan = self.consumer_scanner.scan();
        let mut restart_context = pending_restart_context(&consumer_scan);
        let prepared = PreparedOpenAiSwitch::prepare(&self.codex_home)?;
        if self.faults.fails_backup_creation() {
            return Err(backup_failed());
        }
        let backup = create_openai_backup(&self.codex_home, &prepared, &before)?;
        self.check_interruption(EnvironmentFailurePoint::AfterBackupCompleted)?;
        persist_pending_openai(
            &mut connection,
            &prepared,
            &backup,
            restart_context.as_ref(),
        )?;
        self.check_interruption(EnvironmentFailurePoint::AfterPendingRegistered)?;

        let mut config_applied = false;
        let mut interrupted = false;
        let mut backup_completed = false;
        let result = (|| {
            self.check_fault(EnvironmentFailurePoint::BeforeConfigReplace)?;
            prepared.config.commit()?;
            config_applied = true;
            update_pending_stage(&connection, &prepared.operation_id, "config_replaced")?;
            self.check_interruption(EnvironmentFailurePoint::AfterConfigReplaced)
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            prepared.config.verify_target()?;
            update_pending_stage(&connection, &prepared.operation_id, "artifacts_replaced")?;
            finalize_restart_context(&connection, &prepared.operation_id, &mut restart_context)?;
            mark_backup_completed(&backup)?;
            backup_completed = true;
            self.check_interruption(EnvironmentFailurePoint::AfterAllArtifactsReplaced)
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            self.check_interruption(EnvironmentFailurePoint::BeforeDatabaseCommit)
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            self.check_fault(EnvironmentFailurePoint::BeforeDatabaseCommit)?;
            commit_openai_state(&mut connection, &prepared, restart_context.as_ref())?;
            self.check_interruption(EnvironmentFailurePoint::AfterDatabaseCommit)
                .inspect_err(|_| {
                    interrupted = true;
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
                || (config_applied && prepared.config.rollback().is_err())
            {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::RollbackFailed,
                    "environment.rollback_failed",
                ));
            }
            clear_pending(&connection, &prepared.operation_id)?;
            return Err(failure);
        }

        self.inspect_environment_after_write(&connection, &consumer_scan)
    }

    pub fn apply_provider_at_revision(
        &self,
        provider_id: &str,
        confirm_switch_risk: bool,
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
            confirm_switch_risk,
            Some(expected_revision),
            None,
        )
    }

    pub(crate) fn save_and_apply_provider_update(
        &self,
        update: VerifiedProviderUpdate,
        confirm_consumer_risk: bool,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let mut connection = self.open_state()?;
        let before = self.inspect_environment(&connection)?;
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
        self.apply_target(
            &mut connection,
            update.provider,
            confirm_consumer_risk,
            None,
            Some(guard),
        )
    }

    fn apply_target(
        &self,
        connection: &mut Connection,
        provider: ProviderTarget,
        confirm_switch_risk: bool,
        expected_revision: Option<&str>,
        update_guard: Option<UpdateGuard>,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let before = self.inspect_environment(connection)?;
        let consumer_scan = self.consumer_scanner.scan();
        if expected_revision.is_some_and(|expected| before.revision != expected) {
            return Err(concurrent_modification());
        }
        if before.mode == Some(AuthenticationMode::OpenaiLogin) && !confirm_switch_risk {
            return Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::ModeSwitchConfirmationRequired,
                "environment.mode_switch_confirmation_required",
            ));
        }
        match before.state {
            EnvironmentState::Conflict if !confirm_switch_risk => {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::TakeoverConfirmationRequired,
                    "environment.takeover_confirmation_required",
                ));
            }
            EnvironmentState::External if !confirm_switch_risk => {
                return Err(EnvironmentFailure::new(
                    EnvironmentFailureCategory::TakeoverConfirmationRequired,
                    "environment.takeover_confirmation_required",
                ));
            }
            EnvironmentState::Conflict | EnvironmentState::External | EnvironmentState::Managed => {
            }
        }
        if !confirm_switch_risk {
            return Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::ConsumerConfirmationRequired,
                "environment.consumer_confirmation_required",
            ));
        }

        let prepared = PreparedSwitch::prepare(&self.codex_home, provider, update_guard)?;
        if expected_revision.is_some_and(|expected| prepared.old_revision() != expected) {
            return Err(concurrent_modification());
        }
        if self.faults.fails_backup_creation() {
            return Err(backup_failed());
        }
        let backup = create_backup(&self.codex_home, &prepared, &before)?;
        self.check_interruption(EnvironmentFailurePoint::AfterBackupCompleted)?;
        let mut restart_context = pending_restart_context(&consumer_scan);
        persist_pending(connection, &prepared, &backup, restart_context.as_ref())?;
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
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            self.check_fault(EnvironmentFailurePoint::BeforeCredentialsReplace)?;
            prepared.credentials.commit()?;
            credentials_applied = true;
            prepared.verify_committed()?;
            update_pending_stage(connection, &prepared.operation_id, "artifacts_replaced")?;
            finalize_restart_context(connection, &prepared.operation_id, &mut restart_context)?;
            mark_backup_completed(&backup)?;
            backup_completed = true;
            self.check_interruption(EnvironmentFailurePoint::AfterAllArtifactsReplaced)
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            self.check_interruption(EnvironmentFailurePoint::BeforeDatabaseCommit)
                .inspect_err(|_| {
                    interrupted = true;
                })?;
            self.check_fault(EnvironmentFailurePoint::BeforeDatabaseCommit)?;
            commit_applied_state(connection, &prepared, restart_context.as_ref())?;
            self.check_interruption(EnvironmentFailurePoint::AfterDatabaseCommit)
                .inspect_err(|_| {
                    interrupted = true;
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

        self.inspect_environment_after_write(connection, &consumer_scan)
    }

    fn inspect_environment(
        &self,
        connection: &Connection,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let scan = self.consumer_scanner.scan();
        reconcile_pending_restart(connection, &scan)?;
        self.snapshot_with_consumer_scan(connection, &scan)
    }

    fn inspect_environment_after_write(
        &self,
        connection: &Connection,
        scan: &ConsumerScan,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        self.snapshot_with_consumer_scan(connection, scan)
    }

    fn snapshot_with_consumer_scan(
        &self,
        connection: &Connection,
        scan: &ConsumerScan,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        let mut snapshot =
            inspect_environment(connection, &self.codex_home, self.login_probe.status())?;
        snapshot.requires_consumer_confirmation = true;
        snapshot.consumers = consumer_statuses(scan);
        Ok(snapshot)
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
    #[serde(default)]
    recommendation_id: Option<String>,
    #[serde(default)]
    recommendation_template_base_url: Option<String>,
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
        recommendation_id: Option<String>,
        recommendation_template_base_url: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            base_url,
            api_key,
            default_model,
            verified_at_epoch_seconds,
            verification_fingerprint,
            recommendation_id,
            recommendation_template_base_url,
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
    restart_context: Option<String>,
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
        let mut summary = ProviderSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            base_url: self.base_url.clone(),
            default_model: self.default_model.clone(),
            verified_at_epoch_seconds: self.verified_at_epoch_seconds,
            is_current,
            recommendation_id: self.recommendation_id.clone(),
            has_recommendation_update: false,
            recommendation_template_base_url: self.recommendation_template_base_url.clone(),
        };
        summary.refresh_recommendation_update();
        summary
    }
}

fn load_provider(
    connection: &Connection,
    provider_id: &str,
) -> Result<ProviderTarget, EnvironmentFailure> {
    connection
        .query_row(
            "SELECT id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint, recommendation_id, recommendation_template_base_url
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
                    recommendation_id: row.get(7)?,
                    recommendation_template_base_url: row.get(8)?,
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

fn fingerprints_match(current: &Option<String>, expected: &Option<String>, affected: bool) -> bool {
    !affected || current == expected
}

fn inspect_restore(
    connection: &Connection,
    codex_home: &Path,
    config: &ArtifactBytes,
    credentials: &ArtifactBytes,
) -> (RestoreAvailability, Option<RestorePreview>) {
    match has_pending_operation(connection) {
        Ok(true) => return (RestoreAvailability::RecoveryPending, None),
        Err(_) => return (RestoreAvailability::InvalidBackup, None),
        Ok(false) => {}
    }
    match latest_completed_backup(codex_home) {
        Ok(Some(backup)) => {
            let managed_current =
                reconciled_applied_provider(connection, config, Some(credentials))
                    .ok()
                    .flatten()
                    .is_some();
            if backup.matches_current(config, credentials, managed_current) {
                match backup.restore_preview(connection) {
                    Ok(preview) => (RestoreAvailability::Available, Some(preview)),
                    Err(_) => (RestoreAvailability::InvalidBackup, None),
                }
            } else {
                (RestoreAvailability::ArtifactsChanged, None)
            }
        }
        Ok(None) => (RestoreAvailability::NoBackup, None),
        Err(_) => (RestoreAvailability::InvalidBackup, None),
    }
}

fn inspect_environment(
    connection: &Connection,
    codex_home: &Path,
    login_status: LoginStatus,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let config = read_artifact(&codex_home.join("config.toml"))?;
    let credentials = read_artifact(&codex_home.join("auth.json"))?;
    let revision = environment_revision(&config, &credentials);
    let (restore_availability, restore_preview) =
        inspect_restore(connection, codex_home, &config, &credentials);
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
    let pending_restart = connection
        .query_row(
            "SELECT pending_restart FROM app_state WHERE singleton = 1",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| state_unavailable())?;
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
        return Ok(unsafe_conflict_snapshot(
            impacts,
            revision,
            restore_availability,
            &restore_preview,
            login_status,
            pending_restart,
        ));
    }
    let last_applied = connection
        .query_row(
            "SELECT mode, provider_id, config_fingerprint, credentials_fingerprint
             FROM last_applied_state WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|_| state_unavailable())?;

    if last_applied.as_ref().map(|applied| applied.0.as_str()) == Some("openai_login") {
        let openai_config_state = match config.bytes.as_deref() {
            None => OpenAiConfigState::OpenAi,
            Some(bytes) => match std::str::from_utf8(bytes).ok().and_then(|text| {
                text.parse::<DocumentMut>()
                    .ok()
                    .map(|document| (text, document))
            }) {
                Some((text, document)) => match managed_block(text) {
                    ManagedBlock::None if document.get("model_provider").is_some() => {
                        OpenAiConfigState::External
                    }
                    ManagedBlock::None => OpenAiConfigState::OpenAi,
                    ManagedBlock::Valid(_) => OpenAiConfigState::Conflict,
                    ManagedBlock::Conflict => OpenAiConfigState::UnsafeConflict,
                },
                None => OpenAiConfigState::UnsafeConflict,
            },
        };
        if openai_config_state == OpenAiConfigState::External {
            return Ok(external_snapshot(
                impacts,
                revision,
                restore_availability,
                &restore_preview,
                login_status,
                pending_restart,
            ));
        }
        if matches!(
            openai_config_state,
            OpenAiConfigState::Conflict | OpenAiConfigState::UnsafeConflict
        ) {
            let snapshot = if openai_config_state == OpenAiConfigState::UnsafeConflict {
                unsafe_conflict_snapshot(
                    impacts,
                    revision,
                    restore_availability,
                    &restore_preview,
                    login_status,
                    pending_restart,
                )
            } else {
                conflict_snapshot(
                    impacts,
                    revision,
                    restore_availability,
                    &restore_preview,
                    login_status,
                    pending_restart,
                )
            };
            return Ok(snapshot);
        }
        return Ok(EnvironmentSnapshot {
            state: EnvironmentState::Managed,
            mode: Some(AuthenticationMode::OpenaiLogin),
            message_id: match login_status {
                LoginStatus::LoggedIn => "environment.openai_login",
                LoginStatus::NotLoggedIn => "environment.openai_login_missing",
                LoginStatus::Unavailable => "environment.openai_login_unavailable",
            },
            revision,
            requires_takeover_confirmation: true,
            takeover_available: true,
            impacts,
            current_provider: None,
            restore_availability,
            restore_preview,
            login_status,
            pending_restart,
            requires_consumer_confirmation: false,
            consumers: unknown_consumers(),
        });
    }

    let last_applied_provider = last_applied.and_then(|applied| {
        if applied.0 == "provider" {
            applied
                .1
                .map(|provider_id| (provider_id, applied.2, applied.3))
        } else {
            None
        }
    });

    let Some(config_bytes) = config.bytes.as_deref() else {
        return Ok(if last_applied_provider.is_some() {
            conflict_snapshot(
                impacts,
                revision,
                restore_availability,
                &restore_preview,
                login_status,
                pending_restart,
            )
        } else {
            external_snapshot(
                impacts,
                revision,
                restore_availability,
                &restore_preview,
                login_status,
                pending_restart,
            )
        });
    };
    let config_text = match std::str::from_utf8(config_bytes) {
        Ok(text) => text,
        Err(_) => {
            return Ok(unsafe_conflict_snapshot(
                impacts,
                revision,
                restore_availability,
                &restore_preview,
                login_status,
                pending_restart,
            ));
        }
    };
    let document = match config_text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(_) => {
            return Ok(unsafe_conflict_snapshot(
                impacts,
                revision,
                restore_availability,
                &restore_preview,
                login_status,
                pending_restart,
            ));
        }
    };
    if ensure_file_credential_store(Some(config_bytes)).is_err() {
        return Ok(unsafe_conflict_snapshot(
            impacts,
            revision,
            restore_availability,
            &restore_preview,
            login_status,
            pending_restart,
        ));
    }
    let managed = match managed_block(config_text) {
        ManagedBlock::None => {
            return Ok(if last_applied_provider.is_some() {
                conflict_snapshot(
                    impacts,
                    revision,
                    restore_availability,
                    &restore_preview,
                    login_status,
                    pending_restart,
                )
            } else {
                external_snapshot(
                    impacts,
                    revision,
                    restore_availability,
                    &restore_preview,
                    login_status,
                    pending_restart,
                )
            });
        }
        ManagedBlock::Conflict => {
            return Ok(unsafe_conflict_snapshot(
                impacts,
                revision,
                restore_availability,
                &restore_preview,
                login_status,
                pending_restart,
            ));
        }
        ManagedBlock::Valid(block) => block,
    };
    if managed.recovered_desktop_rewrite && last_applied_provider.is_none() {
        return Ok(unsafe_conflict_snapshot(
            impacts,
            revision,
            restore_availability,
            &restore_preview,
            login_status,
            pending_restart,
        ));
    }
    if !managed_block_is_root_scoped(&document, config_text, &managed) {
        return Ok(unsafe_conflict_snapshot(
            impacts,
            revision,
            restore_availability,
            &restore_preview,
            login_status,
            pending_restart,
        ));
    }
    let provider = match load_provider(connection, &managed.provider_id) {
        Ok(provider) => provider,
        Err(failure) if failure.category == EnvironmentFailureCategory::ProviderNotFound => {
            return Ok(if last_applied_provider.is_some() {
                conflict_snapshot(
                    impacts,
                    revision,
                    restore_availability,
                    &restore_preview,
                    login_status,
                    pending_restart,
                )
            } else {
                external_snapshot(
                    impacts,
                    revision,
                    restore_availability,
                    &restore_preview,
                    login_status,
                    pending_restart,
                )
            });
        }
        Err(failure) => return Err(failure),
    };
    if !managed_config_matches(&document, &provider) {
        return Ok(conflict_snapshot(
            impacts,
            revision,
            restore_availability,
            &restore_preview,
            login_status,
            pending_restart,
        ));
    }
    if !credentials_match(&credentials, &provider.api_key)? {
        return Ok(conflict_snapshot(
            impacts,
            revision,
            restore_availability,
            &restore_preview,
            login_status,
            pending_restart,
        ));
    }

    let Some((applied_provider, applied_config, _applied_credentials)) = last_applied_provider
    else {
        return Ok(external_snapshot(
            impacts,
            revision,
            restore_availability,
            &restore_preview,
            login_status,
            pending_restart,
        ));
    };
    if applied_provider != provider.id || applied_config != managed_config_fingerprint(config_bytes)
    {
        return Ok(conflict_snapshot(
            impacts,
            revision,
            restore_availability,
            &restore_preview,
            login_status,
            pending_restart,
        ));
    }

    Ok(EnvironmentSnapshot {
        state: EnvironmentState::Managed,
        mode: Some(AuthenticationMode::Provider),
        message_id: "environment.managed",
        revision,
        requires_takeover_confirmation: false,
        takeover_available: true,
        impacts,
        current_provider: Some(provider.summary(true)),
        restore_availability,
        restore_preview,
        login_status,
        pending_restart,
        requires_consumer_confirmation: false,
        consumers: unknown_consumers(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenAiConfigState {
    OpenAi,
    External,
    Conflict,
    UnsafeConflict,
}

fn external_snapshot(
    impacts: Vec<ArtifactImpact>,
    revision: String,
    restore_availability: RestoreAvailability,
    restore_preview: &Option<RestorePreview>,
    login_status: LoginStatus,
    pending_restart: bool,
) -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        state: EnvironmentState::External,
        mode: None,
        message_id: "environment.external",
        revision,
        requires_takeover_confirmation: true,
        takeover_available: true,
        impacts,
        current_provider: None,
        restore_availability,
        restore_preview: restore_preview.clone(),
        login_status,
        pending_restart,
        requires_consumer_confirmation: false,
        consumers: unknown_consumers(),
    }
}

fn conflict_snapshot(
    impacts: Vec<ArtifactImpact>,
    revision: String,
    restore_availability: RestoreAvailability,
    restore_preview: &Option<RestorePreview>,
    login_status: LoginStatus,
    pending_restart: bool,
) -> EnvironmentSnapshot {
    EnvironmentSnapshot {
        state: EnvironmentState::Conflict,
        mode: None,
        message_id: "environment.managed_conflict",
        revision,
        requires_takeover_confirmation: true,
        takeover_available: true,
        impacts,
        current_provider: None,
        restore_availability,
        restore_preview: restore_preview.clone(),
        login_status,
        pending_restart,
        requires_consumer_confirmation: false,
        consumers: unknown_consumers(),
    }
}

fn unsafe_conflict_snapshot(
    impacts: Vec<ArtifactImpact>,
    revision: String,
    restore_availability: RestoreAvailability,
    restore_preview: &Option<RestorePreview>,
    login_status: LoginStatus,
    pending_restart: bool,
) -> EnvironmentSnapshot {
    let mut snapshot = conflict_snapshot(
        impacts,
        revision,
        restore_availability,
        restore_preview,
        login_status,
        pending_restart,
    );
    snapshot.takeover_available = false;
    snapshot
}

fn unknown_consumers() -> ConsumerStatuses {
    ConsumerStatuses {
        desktop: ConsumerStatus::Unknown,
        cli: ConsumerStatus::Unknown,
    }
}

fn consumer_statuses(scan: &ConsumerScan) -> ConsumerStatuses {
    ConsumerStatuses {
        desktop: scan.desktop,
        cli: scan.cli,
    }
}

fn pending_restart_context(scan: &ConsumerScan) -> Option<PendingRestartContext> {
    Some(PendingRestartContext {
        consumers: scan.identities.clone(),
        switched_at_epoch_millis: u64::MAX,
    })
}

fn finalize_restart_context(
    connection: &Connection,
    operation_id: &str,
    context: &mut Option<PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    let context = context.as_mut().ok_or_else(state_unavailable)?;
    context.switched_at_epoch_millis = epoch_millis();
    let context_json = serde_json::to_string(context).map_err(|_| state_unavailable())?;
    let changed = connection
        .execute(
            "UPDATE pending_config_operation SET restart_context = ?1
             WHERE singleton = 1 AND operation_id = ?2",
            params![context_json, operation_id],
        )
        .map_err(|_| state_unavailable())?;
    if changed == 1 {
        Ok(())
    } else {
        Err(state_unavailable())
    }
}

fn serialize_restart_context(
    context: Option<&PendingRestartContext>,
) -> Result<Option<String>, EnvironmentFailure> {
    context
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| state_unavailable())
}

fn update_pending_restart(
    transaction: &rusqlite::Transaction<'_>,
    context: Option<&PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    let context_json = serialize_restart_context(context)?;
    transaction
        .execute(
            "UPDATE app_state SET pending_restart = ?1,
                pending_restart_context = ?2 WHERE singleton = 1",
            params![context.is_some(), context_json],
        )
        .map_err(|_| state_unavailable())?;
    Ok(())
}

fn reconcile_pending_restart(
    connection: &Connection,
    scan: &ConsumerScan,
) -> Result<(), EnvironmentFailure> {
    let Some((pending_restart, context)) = connection
        .query_row(
            "SELECT pending_restart, pending_restart_context
             FROM app_state WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|_| state_unavailable())?
    else {
        return Err(state_unavailable());
    };
    if !pending_restart || !scan.is_trustworthy() {
        return Ok(());
    }
    let Some(context_json) = context.as_deref() else {
        return Ok(());
    };
    let Ok(context) = serde_json::from_str::<PendingRestartContext>(context_json) else {
        return Ok(());
    };
    let old_consumer_alive = scan.has_live_identity_from(&context.consumers)
        || scan.has_consumer_started_before(context.switched_at_epoch_millis);
    if !old_consumer_alive {
        connection
            .execute(
                "UPDATE app_state SET pending_restart = 0,
                    pending_restart_context = NULL WHERE singleton = 1",
                [],
            )
            .map_err(|_| state_unavailable())?;
    }
    Ok(())
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
struct PreparedOpenAiSwitch {
    operation_id: String,
    config: PreparedRestoreArtifact,
}

impl PreparedOpenAiSwitch {
    fn prepare(codex_home: &Path) -> Result<Self, EnvironmentFailure> {
        let current = read_artifact(&codex_home.join("config.toml"))?;
        let target = ArtifactBytes {
            bytes: render_openai_config(current.bytes.as_deref())?,
        };
        Ok(Self {
            operation_id: Uuid::new_v4().to_string(),
            config: PreparedRestoreArtifact::new(
                codex_home.join("config.toml"),
                current,
                target,
                ArtifactKind::Config,
            ),
        })
    }
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
    manifest: BackupManifest,
    config: ArtifactBytes,
    credentials: ArtifactBytes,
}

impl CompletedBackup {
    fn matches_current(
        &self,
        config: &ArtifactBytes,
        credentials: &ArtifactBytes,
        managed_current: bool,
    ) -> bool {
        artifact_matches_completed_operation(
            config.fingerprint(ArtifactKind::Config),
            self.manifest.old_config_fingerprint.as_deref(),
            self.manifest.new_config_fingerprint.as_deref(),
            self.manifest.config_affected,
            managed_current,
        ) && artifact_matches_completed_operation(
            credentials.fingerprint(ArtifactKind::Credentials),
            self.manifest.old_credentials_fingerprint.as_deref(),
            self.manifest.new_credentials_fingerprint.as_deref(),
            self.manifest.credentials_affected,
            managed_current,
        )
    }

    fn restore_preview(
        &self,
        connection: &Connection,
    ) -> Result<RestorePreview, EnvironmentFailure> {
        let artifacts = [
            self.manifest
                .config_affected
                .then_some(ArtifactKind::Config),
            self.manifest
                .credentials_affected
                .then_some(ArtifactKind::Credentials),
        ]
        .into_iter()
        .flatten()
        .collect();
        let target = self.restore_target(connection)?;
        let target_provider = match (target.mode, target.provider_id.as_deref()) {
            (Some(AuthenticationMode::Provider), Some(provider_id)) => Some(
                load_provider(connection, provider_id)
                    .map_err(|_| backup_invalid())?
                    .summary(false),
            ),
            _ => None,
        };
        Ok(RestorePreview {
            artifacts,
            target_mode: target.mode,
            target_provider,
        })
    }

    fn restore_target(&self, connection: &Connection) -> Result<RestoreTarget, EnvironmentFailure> {
        if self.manifest.restore_target_recorded {
            let target = RestoreTarget {
                mode: self.manifest.previous_mode,
                provider_id: self.manifest.previous_provider_id.clone(),
            };
            if let Some(provider_id) = target.provider_id.as_deref() {
                load_provider(connection, provider_id).map_err(|_| backup_invalid())?;
            }
            return Ok(target);
        }
        infer_legacy_restore_target(connection, self)
    }
}

fn infer_legacy_restore_target(
    connection: &Connection,
    backup: &CompletedBackup,
) -> Result<RestoreTarget, EnvironmentFailure> {
    if let Some(config_bytes) = backup.config.bytes.as_deref() {
        let text = std::str::from_utf8(config_bytes).map_err(|_| backup_invalid())?;
        let document = text.parse::<DocumentMut>().map_err(|_| backup_invalid())?;
        match managed_block(text) {
            ManagedBlock::Valid(managed) => {
                if !managed_block_is_root_scoped(&document, text, &managed) {
                    return Err(backup_invalid());
                }
                let provider = load_provider(connection, &managed.provider_id)
                    .map_err(|_| backup_invalid())?;
                if !managed_config_matches(&document, &provider) {
                    return Err(backup_invalid());
                }
                return Ok(RestoreTarget {
                    mode: Some(AuthenticationMode::Provider),
                    provider_id: Some(provider.id),
                });
            }
            ManagedBlock::Conflict => return Err(backup_invalid()),
            ManagedBlock::None => {}
        }
    }
    let field_auth_mode = backup
        .manifest
        .credential_fields
        .as_ref()
        .and_then(|fields| fields.auth_mode.as_ref())
        .and_then(Value::as_str);
    let artifact_auth_mode = backup
        .credentials
        .bytes
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .and_then(|value| value.get("auth_mode").cloned())
        .and_then(|value| value.as_str().map(str::to_owned));
    Ok(RestoreTarget {
        mode: (field_auth_mode == Some("chatgpt")
            || artifact_auth_mode.as_deref() == Some("chatgpt"))
        .then_some(AuthenticationMode::OpenaiLogin),
        provider_id: None,
    })
}

fn artifact_matches_completed_operation(
    current: Option<String>,
    old: Option<&str>,
    new: Option<&str>,
    affected: bool,
    managed_current: bool,
) -> bool {
    !affected || current.as_deref() == new || (managed_current && old == new)
}

#[derive(Debug, Clone)]
struct PreparedRestore {
    operation_id: String,
    config: PreparedRestoreArtifact,
    credentials: Option<PreparedRestoreArtifact>,
    target: RestoreTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreTarget {
    mode: Option<AuthenticationMode>,
    provider_id: Option<String>,
}

impl PreparedRestore {
    fn new(
        codex_home: &Path,
        config: ArtifactBytes,
        credentials: ArtifactBytes,
        backup: CompletedBackup,
        target: RestoreTarget,
    ) -> Result<Self, EnvironmentFailure> {
        let preserve_current_config =
            backup.manifest.old_config_fingerprint == backup.manifest.new_config_fingerprint;
        let preserve_current_credentials = backup.manifest.old_credentials_fingerprint
            == backup.manifest.new_credentials_fingerprint;
        let credential_target = if !backup.manifest.credentials_affected {
            None
        } else if preserve_current_credentials {
            Some(credentials.clone())
        } else if let Some(fields) = backup.manifest.credential_fields.as_ref() {
            Some(restore_credential_fields(
                &credentials,
                backup.manifest.credentials_existed,
                fields,
            )?)
        } else {
            Some(backup.credentials.clone())
        };
        Ok(Self {
            operation_id: Uuid::new_v4().to_string(),
            config: PreparedRestoreArtifact::new(
                codex_home.join("config.toml"),
                config.clone(),
                if preserve_current_config {
                    config
                } else {
                    backup.config
                },
                ArtifactKind::Config,
            ),
            credentials: credential_target.map(|target| {
                PreparedRestoreArtifact::new(
                    codex_home.join("auth.json"),
                    credentials,
                    target,
                    ArtifactKind::Credentials,
                )
            }),
            target,
        })
    }

    fn verify_committed(&self) -> Result<(), EnvironmentFailure> {
        self.config.verify_target()?;
        if let Some(credentials) = &self.credentials {
            credentials.verify_target()?;
        }
        Ok(())
    }
}

fn restore_credential_fields(
    current: &ArtifactBytes,
    existed: bool,
    fields: &CredentialFieldsBackup,
) -> Result<ArtifactBytes, EnvironmentFailure> {
    if !existed {
        return Ok(ArtifactBytes { bytes: None });
    }
    let bytes = current.bytes.as_deref().ok_or_else(backup_invalid)?;
    let mut object = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| invalid_credentials())?
        .as_object()
        .cloned()
        .ok_or_else(invalid_credentials)?;
    match &fields.auth_mode {
        Some(value) => {
            object.insert("auth_mode".to_owned(), value.clone());
        }
        None => {
            object.remove("auth_mode");
        }
    }
    match &fields.openai_api_key {
        Some(value) => {
            object.insert("OPENAI_API_KEY".to_owned(), value.clone());
        }
        None => {
            object.remove("OPENAI_API_KEY");
        }
    }
    let mut bytes =
        serde_json::to_vec_pretty(&Value::Object(object)).map_err(|_| invalid_credentials())?;
    bytes.push(b'\n');
    Ok(ArtifactBytes { bytes: Some(bytes) })
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
        if self.current.bytes == self.target.bytes {
            return Ok(());
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
        prepared
            .credentials
            .as_ref()
            .ok_or_else(state_unavailable)?
            .rollback()?;
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
    restart_context: Option<&PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    let snapshot = serde_json::to_string(&prepared.provider).map_err(|_| state_unavailable())?;
    let restart_context = serialize_restart_context(restart_context)?;
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
                backup_reference, target_snapshot_json, started_at, restart_context
             ) VALUES (1, ?1, ?2, 'prepared', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                restart_context,
            ],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn persist_pending_openai(
    connection: &mut Connection,
    prepared: &PreparedOpenAiSwitch,
    backup: &Path,
    restart_context: Option<&PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    let restart_context = serialize_restart_context(restart_context)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    transaction
        .execute(
            "INSERT INTO pending_config_operation (
                singleton, operation_id, operation_kind, stage,
                old_config_fingerprint, new_config_fingerprint,
                old_credentials_fingerprint, new_credentials_fingerprint,
                backup_reference, target_snapshot_json, started_at, restart_context
             ) VALUES (1, ?1, 'switch_openai_login', 'prepared', ?2, ?3,
                       NULL, NULL, ?4, '{}', ?5, ?6)",
            params![
                prepared.operation_id,
                prepared.config.current_fingerprint,
                prepared.config.target_fingerprint,
                backup.to_string_lossy(),
                epoch_seconds().to_string(),
                restart_context,
            ],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn persist_pending_restore(
    connection: &mut Connection,
    prepared: &PreparedRestore,
    backup: &Path,
    restart_context: Option<&PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    let target_snapshot =
        serde_json::to_string(&prepared.target).map_err(|_| state_unavailable())?;
    let restart_context = serialize_restart_context(restart_context)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    transaction
        .execute(
            "INSERT INTO pending_config_operation (
                singleton, operation_id, operation_kind, stage,
                old_config_fingerprint, new_config_fingerprint,
                old_credentials_fingerprint, new_credentials_fingerprint,
                backup_reference, target_snapshot_json, started_at, restart_context
             ) VALUES (1, ?1, 'restore_latest', 'prepared', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                prepared.operation_id,
                prepared.config.current_fingerprint,
                prepared.config.target_fingerprint,
                prepared
                    .credentials
                    .as_ref()
                    .and_then(|credentials| credentials.current_fingerprint.clone()),
                prepared
                    .credentials
                    .as_ref()
                    .and_then(|credentials| credentials.target_fingerprint.clone()),
                backup.to_string_lossy(),
                target_snapshot,
                epoch_seconds().to_string(),
                restart_context,
            ],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn commit_applied_state(
    connection: &mut Connection,
    prepared: &PreparedSwitch,
    restart_context: Option<&PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    if let Some(guard) = &prepared.update_guard {
        let changed = transaction
            .execute(
                "UPDATE providers SET
                    name = ?1, base_url = ?2, api_key = ?3, default_model = ?4,
                    verified_at = ?5, verification_fingerprint = ?6,
                    recommendation_template_base_url = ?10
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
                    prepared.provider.recommendation_template_base_url,
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
    update_pending_restart(&transaction, restart_context)?;
    transaction
        .execute(
            "DELETE FROM pending_config_operation WHERE singleton = 1 AND operation_id = ?1",
            [&prepared.operation_id],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn commit_openai_state(
    connection: &mut Connection,
    prepared: &PreparedOpenAiSwitch,
    restart_context: Option<&PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    transaction
        .execute(
            "INSERT INTO last_applied_state (
                singleton, mode, provider_id, config_fingerprint,
                credentials_fingerprint, applied_at
             ) VALUES (1, 'openai_login', NULL, NULL, NULL, ?1)
             ON CONFLICT(singleton) DO UPDATE SET
                mode = excluded.mode,
                provider_id = NULL,
                config_fingerprint = NULL,
                credentials_fingerprint = NULL,
                applied_at = excluded.applied_at",
            [epoch_seconds().to_string()],
        )
        .map_err(|_| state_unavailable())?;
    update_pending_restart(&transaction, restart_context)?;
    transaction
        .execute(
            "DELETE FROM pending_config_operation WHERE singleton = 1 AND operation_id = ?1",
            [&prepared.operation_id],
        )
        .map_err(|_| state_unavailable())?;
    transaction.commit().map_err(|_| state_unavailable())
}

fn commit_recovered_openai_state(
    connection: &mut Connection,
    operation_id: &str,
    restart_context: Option<&PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    transaction
        .execute(
            "INSERT INTO last_applied_state (
                singleton, mode, provider_id, config_fingerprint,
                credentials_fingerprint, applied_at
             ) VALUES (1, 'openai_login', NULL, NULL, NULL, ?1)
             ON CONFLICT(singleton) DO UPDATE SET
                mode = excluded.mode,
                provider_id = NULL,
                config_fingerprint = NULL,
                credentials_fingerprint = NULL,
                applied_at = excluded.applied_at",
            [epoch_seconds().to_string()],
        )
        .map_err(|_| state_unavailable())?;
    update_pending_restart(&transaction, restart_context)?;
    transaction
        .execute(
            "DELETE FROM pending_config_operation WHERE singleton = 1 AND operation_id = ?1",
            [operation_id],
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
    let restart_context = pending
        .restart_context
        .as_deref()
        .map(serde_json::from_str::<PendingRestartContext>)
        .transpose()
        .map_err(|_| state_unavailable())?;
    if pending.operation_kind == "restore_latest" {
        let target: RestoreTarget =
            serde_json::from_str(&pending.target_snapshot_json).map_err(|_| state_unavailable())?;
        let credentials = (pending.old_credentials_fingerprint.is_some()
            || pending.new_credentials_fingerprint.is_some())
        .then_some(credentials);
        return commit_reconciled_state(
            connection,
            &pending.operation_id,
            config,
            credentials,
            restart_context.as_ref(),
            Some(&target),
        );
    }
    if pending.operation_kind == "switch_openai_login" {
        return commit_recovered_openai_state(
            connection,
            &pending.operation_id,
            restart_context.as_ref(),
        );
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
                    verified_at = ?5, verification_fingerprint = ?6,
                    recommendation_template_base_url = ?8
                 WHERE id = ?7",
                params![
                    provider.name,
                    provider.base_url,
                    provider.api_key,
                    provider.default_model,
                    provider.verified_at_epoch_seconds.to_string(),
                    provider.verification_fingerprint,
                    provider.id,
                    provider.recommendation_template_base_url,
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
    update_pending_restart(&transaction, restart_context.as_ref())?;
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
    restart_context: Option<&PendingRestartContext>,
) -> Result<(), EnvironmentFailure> {
    commit_reconciled_state(
        connection,
        &prepared.operation_id,
        &prepared.config.target,
        prepared
            .credentials
            .as_ref()
            .map(|credentials| &credentials.target),
        restart_context,
        Some(&prepared.target),
    )
}

fn commit_reconciled_state(
    connection: &mut Connection,
    operation_id: &str,
    config: &ArtifactBytes,
    credentials: Option<&ArtifactBytes>,
    restart_context: Option<&PendingRestartContext>,
    restore_target: Option<&RestoreTarget>,
) -> Result<(), EnvironmentFailure> {
    if restore_target.is_some_and(|target| target.mode == Some(AuthenticationMode::OpenaiLogin)) {
        return commit_recovered_openai_state(connection, operation_id, restart_context);
    }
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
    update_pending_restart(&transaction, restart_context)?;
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
    credentials: Option<&ArtifactBytes>,
) -> Result<Option<(String, String, String)>, EnvironmentFailure> {
    let Some(credentials) = credentials else {
        return Ok(None);
    };
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
    if managed.recovered_desktop_rewrite {
        let evidence_matches = connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM last_applied_state
                    WHERE singleton = 1 AND mode = 'provider'
                      AND provider_id = ?1 AND config_fingerprint = ?2
                      AND credentials_fingerprint = ?3
                 )",
                params![provider.id, config_fingerprint, credentials_fingerprint],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|_| state_unavailable())?;
        if !evidence_matches {
            return Ok(None);
        }
    }
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
    #[serde(default = "default_true")]
    config_affected: bool,
    #[serde(default = "default_true")]
    credentials_affected: bool,
    config_existed: bool,
    credentials_existed: bool,
    old_config_fingerprint: Option<String>,
    new_config_fingerprint: Option<String>,
    old_credentials_fingerprint: Option<String>,
    new_credentials_fingerprint: Option<String>,
    #[serde(default)]
    credential_fields: Option<CredentialFieldsBackup>,
    #[serde(default)]
    previous_mode: Option<AuthenticationMode>,
    #[serde(default)]
    previous_provider_id: Option<String>,
    #[serde(default)]
    restore_target_recorded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialFieldsBackup {
    auth_mode: Option<Value>,
    openai_api_key: Option<Value>,
}

fn default_true() -> bool {
    true
}

fn credential_fields_backup(
    credentials: &ArtifactBytes,
) -> Result<Option<CredentialFieldsBackup>, EnvironmentFailure> {
    let Some(bytes) = credentials.bytes.as_deref() else {
        return Ok(None);
    };
    let object = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| invalid_credentials())?
        .as_object()
        .cloned()
        .ok_or_else(invalid_credentials)?;
    let contains_openai_login_material = object.keys().any(|key| {
        let key = key.to_ascii_lowercase();
        key.contains("token") || key == "last_refresh"
    });
    Ok(
        contains_openai_login_material.then(|| CredentialFieldsBackup {
            auth_mode: object.get("auth_mode").cloned(),
            openai_api_key: object.get("OPENAI_API_KEY").cloned(),
        }),
    )
}

fn create_backup(
    codex_home: &Path,
    prepared: &PreparedSwitch,
    before: &EnvironmentSnapshot,
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
        let credential_fields = credential_fields_backup(&prepared.credentials.old)?;
        if let Some(bytes) = prepared.config.old.bytes.as_deref() {
            write_new_synced(&operation.join("config.toml"), bytes).map_err(|_| backup_failed())?;
        }
        if credential_fields.is_none() {
            if let Some(bytes) = prepared.credentials.old.bytes.as_deref() {
                write_new_synced(&operation.join("auth.json"), bytes)
                    .map_err(|_| backup_failed())?;
            }
        }
        let manifest = serde_json::to_vec_pretty(&BackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            operation_id: prepared.operation_id.clone(),
            operation_kind: if prepared.update_guard.is_some() {
                "save_and_apply".to_owned()
            } else {
                "switch_provider".to_owned()
            },
            config_affected: true,
            credentials_affected: true,
            config_existed: prepared.config.old.bytes.is_some(),
            credentials_existed: prepared.credentials.old.bytes.is_some(),
            old_config_fingerprint: prepared.config.old_fingerprint.clone(),
            new_config_fingerprint: Some(prepared.config.new_fingerprint.clone()),
            old_credentials_fingerprint: prepared.credentials.old_fingerprint.clone(),
            new_credentials_fingerprint: Some(prepared.credentials.new_fingerprint.clone()),
            credential_fields,
            previous_mode: before.mode,
            previous_provider_id: before
                .current_provider
                .as_ref()
                .map(|provider| provider.id.clone()),
            restore_target_recorded: true,
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

fn create_openai_backup(
    codex_home: &Path,
    prepared: &PreparedOpenAiSwitch,
    before: &EnvironmentSnapshot,
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
        let manifest = serde_json::to_vec_pretty(&BackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            operation_id: prepared.operation_id.clone(),
            operation_kind: "switch_openai_login".to_owned(),
            config_affected: true,
            credentials_affected: false,
            config_existed: prepared.config.current.bytes.is_some(),
            credentials_existed: false,
            old_config_fingerprint: prepared.config.current_fingerprint.clone(),
            new_config_fingerprint: prepared.config.target_fingerprint.clone(),
            old_credentials_fingerprint: None,
            new_credentials_fingerprint: None,
            credential_fields: None,
            previous_mode: before.mode,
            previous_provider_id: before
                .current_provider
                .as_ref()
                .map(|provider| provider.id.clone()),
            restore_target_recorded: true,
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
    before: &EnvironmentSnapshot,
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
        let credential_fields = prepared
            .credentials
            .as_ref()
            .map(|credentials| credential_fields_backup(&credentials.current))
            .transpose()?
            .flatten();
        if let Some(bytes) = prepared.config.current.bytes.as_deref() {
            write_new_synced(&operation.join("config.toml"), bytes).map_err(|_| backup_failed())?;
        }
        if credential_fields.is_none() {
            if let Some(bytes) = prepared
                .credentials
                .as_ref()
                .and_then(|credentials| credentials.current.bytes.as_deref())
            {
                write_new_synced(&operation.join("auth.json"), bytes)
                    .map_err(|_| backup_failed())?;
            }
        }
        let manifest = serde_json::to_vec_pretty(&BackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            operation_id: prepared.operation_id.clone(),
            operation_kind: "restore_latest".to_owned(),
            config_affected: true,
            credentials_affected: prepared.credentials.is_some(),
            config_existed: prepared.config.current.bytes.is_some(),
            credentials_existed: prepared
                .credentials
                .as_ref()
                .is_some_and(|credentials| credentials.current.bytes.is_some()),
            old_config_fingerprint: prepared.config.current_fingerprint.clone(),
            new_config_fingerprint: prepared.config.target_fingerprint.clone(),
            old_credentials_fingerprint: prepared
                .credentials
                .as_ref()
                .and_then(|credentials| credentials.current_fingerprint.clone()),
            new_credentials_fingerprint: prepared
                .credentials
                .as_ref()
                .and_then(|credentials| credentials.target_fingerprint.clone()),
            credential_fields,
            previous_mode: before.mode,
            previous_provider_id: before
                .current_provider
                .as_ref()
                .map(|provider| provider.id.clone()),
            restore_target_recorded: true,
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
        "switch_provider" | "save_and_apply" | "restore_latest" | "switch_openai_login"
    );
    let operation_name_matches = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(&manifest.operation_id));
    let previous_target_valid = match (
        manifest.restore_target_recorded,
        manifest.previous_mode,
        manifest.previous_provider_id.as_deref(),
    ) {
        (true, Some(AuthenticationMode::Provider), Some(provider_id)) => {
            Uuid::parse_str(provider_id).is_ok()
        }
        (true, Some(AuthenticationMode::OpenaiLogin) | None, None) => true,
        (false, None, None) => true,
        _ => false,
    };
    if manifest.format_version != BACKUP_FORMAT_VERSION
        || !valid_kind
        || Uuid::parse_str(&manifest.operation_id).is_err()
        || !operation_name_matches
        || !previous_target_valid
        || !manifest.config_affected
        || manifest.config_existed != manifest.old_config_fingerprint.is_some()
        || (manifest.credentials_affected
            && manifest.credentials_existed != manifest.old_credentials_fingerprint.is_some())
        || (!manifest.credentials_affected
            && (manifest.credentials_existed
                || manifest.old_credentials_fingerprint.is_some()
                || manifest.new_credentials_fingerprint.is_some()
                || manifest.credential_fields.is_some()))
        || (manifest.credential_fields.is_some()
            && (!manifest.credentials_affected || !manifest.credentials_existed))
    {
        return Err(backup_invalid());
    }
    let config = read_backup_artifact(
        &path.join("config.toml"),
        manifest.config_existed,
        manifest.old_config_fingerprint.as_deref(),
        ArtifactKind::Config,
    )?;
    let credentials = if manifest.credential_fields.is_some() {
        read_backup_artifact(
            &path.join("auth.json"),
            false,
            None,
            ArtifactKind::Credentials,
        )?
    } else if manifest.credentials_affected {
        read_backup_artifact(
            &path.join("auth.json"),
            manifest.credentials_existed,
            manifest.old_credentials_fingerprint.as_deref(),
            ArtifactKind::Credentials,
        )?
    } else {
        read_backup_artifact(
            &path.join("auth.json"),
            false,
            None,
            ArtifactKind::Credentials,
        )?
    };
    Ok(CompletedBackup {
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
            replace_managed_block(text, &existing, Some(&block)).ok_or_else(managed_conflict)?
        }
        ManagedBlock::Conflict => return Err(managed_conflict()),
    };
    rendered
        .parse::<DocumentMut>()
        .map_err(|_| invalid_config())?;
    Ok(rendered.into_bytes())
}

fn render_openai_config(original: Option<&[u8]>) -> Result<Option<Vec<u8>>, EnvironmentFailure> {
    let Some(original) = original else {
        return Ok(None);
    };
    let text = std::str::from_utf8(original).map_err(|_| invalid_config())?;
    let mut document = text.parse::<DocumentMut>().map_err(|_| invalid_config())?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let rendered = match managed_block(text) {
        ManagedBlock::None if document.get("model_provider").is_some() => {
            document.remove("model");
            document.remove("model_provider");
            normalize_newlines(&document.to_string(), newline)
        }
        ManagedBlock::None => text.to_owned(),
        ManagedBlock::Valid(block) => {
            if !managed_block_is_root_scoped(&document, text, &block) {
                return Err(managed_conflict());
            }
            replace_managed_block(text, &block, None).ok_or_else(managed_conflict)?
        }
        ManagedBlock::Conflict => return Err(managed_conflict()),
    };
    rendered
        .parse::<DocumentMut>()
        .map_err(|_| invalid_config())?;
    Ok(Some(rendered.into_bytes()))
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
    recovered_desktop_rewrite: bool,
    relocated_end_marker: Option<(usize, usize)>,
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
        ([(start, start_line_end)], [(end_start, end)]) if start < end_start => {
            let managed = validated_managed_block(text, *start, *end).or_else(|| {
                if *start == 0 {
                    recover_desktop_managed_block(
                        text,
                        *start,
                        *start_line_end,
                        Some((*end_start, *end)),
                    )
                } else {
                    None
                }
            });
            managed.map_or(ManagedBlock::Conflict, ManagedBlock::Valid)
        }
        // Desktop Codex can keep the owned prefix while dropping or relocating its sentinel.
        ([(start, start_line_end)], []) if *start == 0 => {
            recover_desktop_managed_block(text, *start, *start_line_end, None)
                .map_or(ManagedBlock::Conflict, ManagedBlock::Valid)
        }
        _ => ManagedBlock::Conflict,
    }
}

fn replace_managed_block(
    text: &str,
    managed: &ManagedBlockRange,
    replacement: Option<&str>,
) -> Option<String> {
    let mut rendered = String::with_capacity(text.len() + replacement.map_or(0, str::len));
    rendered.push_str(text.get(..managed.start)?);
    if let Some(replacement) = replacement {
        rendered.push_str(replacement);
    }
    if let Some((marker_start, marker_end)) = managed.relocated_end_marker {
        rendered.push_str(text.get(managed.end..marker_start)?);
        rendered.push_str(text.get(marker_end..)?);
    } else {
        rendered.push_str(text.get(managed.end..)?);
    }
    Some(rendered)
}

fn canonical_managed_block(text: &str, managed: &ManagedBlockRange) -> Option<String> {
    let bytes = text.as_bytes().get(managed.start..managed.end)?;
    if !managed.recovered_desktop_rewrite {
        return std::str::from_utf8(bytes).ok().map(str::to_owned);
    }
    String::from_utf8(reconstructed_managed_block(bytes)?).ok()
}

fn reconstructed_managed_block(prefix: &[u8]) -> Option<Vec<u8>> {
    std::str::from_utf8(prefix).ok()?;
    let newline = if prefix.windows(2).any(|window| window == b"\r\n") {
        b"\r\n".as_slice()
    } else {
        b"\n".as_slice()
    };
    let mut reconstructed = prefix.to_vec();
    if !reconstructed.ends_with(newline) {
        reconstructed.extend_from_slice(newline);
    }
    reconstructed.extend_from_slice(MANAGED_END.as_bytes());
    reconstructed.extend_from_slice(newline);
    Some(reconstructed)
}

fn recover_desktop_managed_block(
    text: &str,
    start: usize,
    start_line_end: usize,
    relocated_end_marker: Option<(usize, usize)>,
) -> Option<ManagedBlockRange> {
    let mut end = start_line_end;
    let mut remaining_lines = 7;
    for line in text.get(start_line_end..)?.split_inclusive('\n') {
        end += line.len();
        remaining_lines -= 1;
        if remaining_lines == 0 {
            break;
        }
    }
    if remaining_lines != 0 {
        return None;
    }
    if let Some((marker_start, marker_end)) = relocated_end_marker {
        if marker_start < end
            || !text.get(marker_end..)?.trim().is_empty()
            || text
                .get(end..marker_start)?
                .lines()
                .any(|line| line.starts_with(PROVIDER_ID_PREFIX))
        {
            return None;
        }
    }
    let block = reconstructed_managed_block(text.as_bytes().get(start..end)?)?;
    let provider_id = validated_provider_id(std::str::from_utf8(&block).ok()?)?;
    Some(ManagedBlockRange {
        start,
        end,
        provider_id,
        recovered_desktop_rewrite: true,
        relocated_end_marker,
    })
}

fn validated_managed_block(text: &str, start: usize, end: usize) -> Option<ManagedBlockRange> {
    let block = text.get(start..end)?;
    let provider_id = validated_provider_id(block)?;
    Some(ManagedBlockRange {
        start,
        end,
        provider_id,
        recovered_desktop_rewrite: false,
        relocated_end_marker: None,
    })
}

fn validated_provider_id(block: &str) -> Option<String> {
    let provider_ids = block
        .lines()
        .filter_map(|line| line.strip_prefix(PROVIDER_ID_PREFIX))
        .map(str::trim)
        .collect::<Vec<_>>();
    let [provider_id] = provider_ids.as_slice() else {
        return None;
    };
    if provider_id.is_empty()
        || Uuid::parse_str(provider_id).is_err()
        || !managed_block_has_expected_shape(block, provider_id)
    {
        return None;
    }
    Some((*provider_id).to_owned())
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
    let Some(block) = canonical_managed_block(text, managed) else {
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

pub(crate) struct ManagedConfigEvidence {
    pub(crate) fingerprint: String,
    pub(crate) recovered_desktop_rewrite: bool,
}

pub(crate) fn managed_config_evidence(bytes: &[u8]) -> Option<ManagedConfigEvidence> {
    let text = std::str::from_utf8(bytes).ok()?;
    let ManagedBlock::Valid(block) = managed_block(text) else {
        return None;
    };
    let canonical = canonical_managed_block(text, &block)?;
    Some(ManagedConfigEvidence {
        fingerprint: artifact_hash(ArtifactKind::Config, canonical.as_bytes()),
        recovered_desktop_rewrite: block.recovered_desktop_rewrite,
    })
}

pub(crate) fn managed_config_fingerprint(bytes: &[u8]) -> Option<String> {
    managed_config_evidence(bytes).map(|evidence| evidence.fingerprint)
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

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
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

#[cfg(test)]
mod tests {
    use super::ProviderTarget;

    #[test]
    fn legacy_pending_provider_snapshot_defaults_to_no_recommendation_identity() {
        let provider: ProviderTarget = serde_json::from_str(
            r#"{
                "id":"provider-id",
                "name":"Provider",
                "baseUrl":"https://provider.example/v1",
                "apiKey":"key",
                "defaultModel":"model-a",
                "verifiedAtEpochSeconds":1,
                "verificationFingerprint":"fingerprint"
            }"#,
        )
        .expect("legacy pending provider snapshot remains readable");

        assert_eq!(provider.recommendation_id, None);
    }
}
