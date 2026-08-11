use serde::Serialize;

use crate::codex::{CodexConfigStatus, CodexInspector, CodexSnapshot, CredentialStore};
use crate::state::{
    AppliedMode, DatabaseSnapshot, DatabaseStatus, PendingConfigOperationSnapshot, StateStore,
};

#[derive(Debug, Clone)]
pub struct StartupCoordinator {
    state_store: StateStore,
    codex_inspector: CodexInspector,
}

impl StartupCoordinator {
    pub fn new(state_store: StateStore, codex_inspector: CodexInspector) -> Self {
        Self {
            state_store,
            codex_inspector,
        }
    }

    pub fn inspect(&self) -> StartupSnapshot {
        let database = self.state_store.bootstrap();
        let inspect_credentials = database.contents.as_ref().is_some_and(|contents| {
            contents.last_applied_mode == Some(AppliedMode::Provider)
                || contents
                    .pending_config_operation
                    .as_ref()
                    .is_some_and(|operation| {
                        operation.old_credentials_fingerprint.is_some()
                            || operation.new_credentials_fingerprint.is_some()
                    })
        });
        let codex = if inspect_credentials {
            self.codex_inspector.inspect_for_provider_mode()
        } else {
            self.codex_inspector.inspect()
        };
        let pending_operation_resolution = database
            .contents
            .as_ref()
            .and_then(|contents| contents.pending_config_operation.as_ref())
            .map(|operation| pending_operation_resolution(operation, &codex));
        let block_reason = startup_block_reason(&database, &codex);
        let mode = if block_reason.is_some() {
            ApplicationMode::Blocked
        } else {
            ApplicationMode::Ready
        };
        let message_id = startup_message_id(&database, block_reason);
        StartupSnapshot {
            mode,
            message_id,
            block_reason,
            pending_operation_resolution,
            database,
            codex,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationMode {
    Ready,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupBlockReason {
    DatabaseUnavailable,
    CodexConfigInvalid,
    CodexConfigUnreadable,
    PendingConfigOperation,
    ManagedConfigConflict,
    UnsupportedCredentialStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingOperationResolution {
    MatchesOldState,
    MatchesNewState,
    Conflict,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupSnapshot {
    pub mode: ApplicationMode,
    pub message_id: &'static str,
    pub block_reason: Option<StartupBlockReason>,
    pub pending_operation_resolution: Option<PendingOperationResolution>,
    pub database: DatabaseSnapshot,
    pub codex: CodexSnapshot,
}

fn startup_block_reason(
    database: &DatabaseSnapshot,
    codex: &CodexSnapshot,
) -> Option<StartupBlockReason> {
    if !database.is_ready() {
        return Some(StartupBlockReason::DatabaseUnavailable);
    }
    match codex.config_status {
        CodexConfigStatus::Invalid => return Some(StartupBlockReason::CodexConfigInvalid),
        CodexConfigStatus::Unreadable => return Some(StartupBlockReason::CodexConfigUnreadable),
        CodexConfigStatus::Missing | CodexConfigStatus::Valid => {}
    }
    if codex.credential_store == CredentialStore::Unsupported {
        return Some(StartupBlockReason::UnsupportedCredentialStore);
    }

    let Some(contents) = database.contents.as_ref() else {
        return Some(StartupBlockReason::DatabaseUnavailable);
    };
    if contents
        .pending_config_operation
        .as_ref()
        .is_some_and(|operation| operation.stage == "conflict")
    {
        return Some(StartupBlockReason::ManagedConfigConflict);
    }
    if contents.has_pending_config_operation {
        return Some(StartupBlockReason::PendingConfigOperation);
    }
    if codex.recovered_desktop_rewrite
        && (contents.last_applied_mode != Some(AppliedMode::Provider)
            || contents.last_applied_config_fingerprint.as_ref() != Some(&codex.config_fingerprint))
    {
        return Some(StartupBlockReason::ManagedConfigConflict);
    }
    if let Some(expected) = &contents.last_applied_config_fingerprint {
        if expected.as_ref() != codex.config_fingerprint.as_ref() {
            return Some(StartupBlockReason::ManagedConfigConflict);
        }
    }
    match (
        &contents.last_applied_credentials_fingerprint,
        contents.last_applied_mode,
    ) {
        (Some(Some(expected)), _) => {
            if codex.credential_fingerprint.as_ref() != Some(expected) {
                return Some(StartupBlockReason::ManagedConfigConflict);
            }
        }
        (Some(None), Some(AppliedMode::Provider)) | (None, Some(AppliedMode::Provider)) => {
            return Some(StartupBlockReason::ManagedConfigConflict);
        }
        _ => {}
    }
    None
}

fn pending_operation_resolution(
    operation: &PendingConfigOperationSnapshot,
    codex: &CodexSnapshot,
) -> PendingOperationResolution {
    if pending_fingerprints_match(
        &operation.old_config_fingerprint,
        &operation.old_credentials_fingerprint,
        codex,
    ) {
        return PendingOperationResolution::MatchesOldState;
    }
    if pending_fingerprints_match(
        &operation.new_config_fingerprint,
        &operation.new_credentials_fingerprint,
        codex,
    ) {
        return PendingOperationResolution::MatchesNewState;
    }
    if operation.old_config_fingerprint.is_some()
        || operation.new_config_fingerprint.is_some()
        || operation.old_credentials_fingerprint.is_some()
        || operation.new_credentials_fingerprint.is_some()
    {
        PendingOperationResolution::Conflict
    } else {
        PendingOperationResolution::Unknown
    }
}

fn pending_fingerprints_match(
    expected_config: &Option<String>,
    expected_credentials: &Option<String>,
    codex: &CodexSnapshot,
) -> bool {
    let mut checked = false;
    if let Some(expected) = expected_config {
        checked = true;
        if codex.config_fingerprint.as_ref() != Some(expected) {
            return false;
        }
    }
    if let Some(expected) = expected_credentials {
        checked = true;
        if codex.credential_fingerprint.as_ref() != Some(expected) {
            return false;
        }
    }
    checked
}

fn startup_message_id(
    database: &DatabaseSnapshot,
    block_reason: Option<StartupBlockReason>,
) -> &'static str {
    if let Some(reason) = block_reason {
        return match reason {
            StartupBlockReason::DatabaseUnavailable => "startup.database_blocked",
            StartupBlockReason::CodexConfigInvalid => "startup.codex_config_invalid",
            StartupBlockReason::CodexConfigUnreadable => "startup.codex_config_unreadable",
            StartupBlockReason::PendingConfigOperation => "startup.pending_config_operation",
            StartupBlockReason::ManagedConfigConflict => "startup.managed_config_conflict",
            StartupBlockReason::UnsupportedCredentialStore => {
                "startup.unsupported_credential_store"
            }
        };
    }
    match database.status {
        DatabaseStatus::Initialized => "startup.database_initialized",
        DatabaseStatus::Recovered => "startup.database_recovered",
        DatabaseStatus::Ready => "startup.ready",
        DatabaseStatus::Blocked => "startup.database_blocked",
    }
}
