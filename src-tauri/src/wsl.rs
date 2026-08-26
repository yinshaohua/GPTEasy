use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::io::Write;
use std::process::Output;
#[cfg(windows)]
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::provider::ProviderSummary;
use crate::state::StateStore;

#[cfg(windows)]
const WSL_REGISTRY_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";
const HELPER_VERSION: &str = "gpteasy-wsl-guest-writer-v2";
const HELPER_PATH: &str = "$HOME/.local/lib/gpteasy/guest-writer-v2";
const BUNDLE_MAGIC: &str = "GPTEASY_WSL_BUNDLE_V2";
const NATURAL_STOP_TIMEOUT: Duration = Duration::from_secs(10);
const NATURAL_STOP_POLL_INTERVAL: Duration = Duration::from_millis(250);
const CODEX_VERSION_PROBE_PREFIX: &str = "__GPTEASY_CODEX_VERSION__:";
const CODEX_NOT_FOUND_PROBE_RESULT: &str = "__GPTEASY_CODEX_NOT_FOUND__";
const CODEX_VERSION_PROBE_SCRIPT: &str = r#"codex_path=$(type -P codex 2>/dev/null) || {
    printf '%s\n' '__GPTEASY_CODEX_NOT_FOUND__'
    exit 42
}
codex_output=$("$codex_path" --version 2>/dev/null) || exit 43
printf '%s%s\n' '__GPTEASY_CODEX_VERSION__:' "$codex_output"
"#;

const GUEST_LOCK: &str = include_str!("wsl_guest_lock.sh");
const GUEST_CREDENTIAL_CLEANUP: &str = include_str!("wsl_guest_credential_cleanup.sh");
#[cfg(windows)]
const GUEST_PRIVATE_READER: &str = include_str!("wsl_guest_private_reader.sh");
const GUEST_WRITER: &[u8] = include_bytes!("wsl_guest_writer.sh");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WslAvailability {
    Manageable,
    Infrastructure,
    UnsupportedVersion,
    Ambiguous,
    Removed,
    Unavailable,
    DefaultUserChanged,
    NeedsRefresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WslConfigurationState {
    Unknown,
    None,
    Current,
    Updated,
    Legacy,
    ProviderMissing,
    Conflict,
    Busy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslEnvironmentSummary {
    pub environment_id: String,
    pub display_name: String,
    pub command_name: Option<String>,
    pub default_uid: Option<u32>,
    pub running: bool,
    pub availability: WslAvailability,
    pub current_provider: Option<ProviderSummary>,
    pub actual_provider_id: Option<String>,
    pub configuration_state: WslConfigurationState,
    pub requires_attention: bool,
    pub pending_restart: bool,
    pub revision: String,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslApplyResult {
    pub environment: WslEnvironmentSummary,
    pub pending_restart: bool,
    pub lifecycle_outcome: WslLifecycleOutcome,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslRefreshResult {
    pub environment: WslEnvironmentSummary,
    pub lifecycle_outcome: WslLifecycleOutcome,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslLifecycleResult {
    pub environment_id: String,
    pub display_name: String,
    pub outcome: WslLifecycleOutcome,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslDeletionAudit {
    pub lifecycle_results: Vec<WslLifecycleResult>,
}

#[derive(Debug)]
pub(crate) enum WslDeletionAuditError<E> {
    Verification(WslFailure),
    Deletion {
        failure: E,
        lifecycle_results: Vec<WslLifecycleResult>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WslLifecycleOutcome {
    UnchangedRunning,
    StoppedNaturally,
    StillRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WslFailureCategory {
    StateUnavailable,
    UnsupportedPlatform,
    ProbeFailed,
    EnvironmentNotFound,
    EnvironmentChanged,
    ProviderNotFound,
    InvalidEnvironment,
    DefaultUserChanged,
    GuestUnavailable,
    GuestWriteFailed,
    ConcurrentModification,
    RecoveryPending,
    NeedsAttention,
    Busy,
    Interrupted,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslFailure {
    pub category: WslFailureCategory,
    pub message_id: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_outcome: Option<WslLifecycleOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WslFailurePoint {
    AfterRefreshLockAcquired,
    AfterPendingRegistered,
    AfterLockAcquired,
    AfterPrepared,
    AfterArtifactsReplaced,
    AfterStateCommitted,
}

pub trait WslFaultInjector: Send + Sync {
    fn check(&self, point: WslFailurePoint) -> Result<(), WslFailure>;
}

#[derive(Debug)]
struct NoWslFaults;

impl WslFaultInjector for NoWslFaults {
    fn check(&self, _point: WslFailurePoint) -> Result<(), WslFailure> {
        Ok(())
    }
}

impl WslFailure {
    pub(crate) fn new(category: WslFailureCategory, message_id: &'static str) -> Self {
        Self {
            category,
            message_id,
            lifecycle_outcome: None,
        }
    }

    fn with_lifecycle_outcome(mut self, outcome: WslLifecycleOutcome) -> Self {
        self.lifecycle_outcome = Some(outcome);
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WslProbe {
    pub environment_id: String,
    pub display_name: String,
    pub command_name: Option<String>,
    pub default_uid: Option<u32>,
    pub wsl_version: Option<u32>,
    pub running: bool,
    pub availability: WslAvailability,
    pub message_id: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub(crate) struct WslArtifacts {
    pub config: Option<Vec<u8>>,
    pub credentials: Option<Vec<u8>>,
}

pub(crate) trait WslRuntime: Send + Sync {
    fn probe(&self) -> Result<Vec<WslProbe>, WslFailure>;
    fn start(&self, environment: &WslProbe) -> Result<(), WslFailure>;
    fn wait_for_natural_stop(
        &self,
        environment: &WslProbe,
        timeout: Duration,
    ) -> Result<bool, WslFailure>;
    fn acquire_lock(
        &self,
        _environment: &WslProbe,
        _token: &str,
        _operation: &str,
    ) -> Result<(), WslFailure> {
        Ok(())
    }
    fn release_lock(&self, _environment: &WslProbe, _token: &str) -> Result<(), WslFailure> {
        Ok(())
    }
    fn check_codex_version(&self, _environment: &WslProbe) -> Result<(), WslFailure> {
        Ok(())
    }
    fn cleanup_credentials(
        &self,
        _environment: &WslProbe,
        _lock_token: &str,
    ) -> Result<(), WslFailure> {
        Ok(())
    }
    fn read_artifacts(&self, environment: &WslProbe) -> Result<WslArtifacts, WslFailure>;
    fn ensure_helper(&self, environment: &WslProbe) -> Result<(), WslFailure>;
    fn write_bundle(
        &self,
        environment: &WslProbe,
        lock_token: &str,
        old_config_hash: &str,
        bundle: &[u8],
    ) -> Result<String, WslFailure>;
}

#[derive(Clone)]
pub struct WslApplication {
    state_store: StateStore,
    operation_lock: Arc<Mutex<()>>,
    runtime: Arc<dyn WslRuntime>,
    faults: Arc<dyn WslFaultInjector>,
    natural_stop_timeout: Duration,
}

struct DeletionAuditEnvironment {
    probe: WslProbe,
    originally_running: bool,
    lock_token: Option<String>,
}

impl std::fmt::Debug for WslApplication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WslApplication")
            .finish_non_exhaustive()
    }
}

impl WslApplication {
    pub fn new(state_store: StateStore) -> Self {
        Self::with_runtime(state_store, Arc::new(SystemWslRuntime::default()))
    }

    #[cfg(all(windows, feature = "wsl-guest-harness"))]
    #[doc(hidden)]
    pub fn with_wsl_program_for_harness(state_store: StateStore, program: OsString) -> Self {
        Self::with_runtime(
            state_store,
            Arc::new(SystemWslRuntime {
                program,
                distribution_filter: None,
            }),
        )
    }

    #[cfg(all(windows, feature = "wsl-guest-harness"))]
    #[doc(hidden)]
    pub fn with_wsl_program_and_timeout_for_harness(
        state_store: StateStore,
        program: OsString,
        natural_stop_timeout: Duration,
    ) -> Self {
        let mut application = Self::with_runtime(
            state_store,
            Arc::new(SystemWslRuntime {
                program,
                distribution_filter: None,
            }),
        );
        application.natural_stop_timeout = natural_stop_timeout;
        application
    }

    #[cfg(all(windows, feature = "wsl-guest-harness"))]
    #[doc(hidden)]
    pub fn with_isolated_wsl_for_harness(
        state_store: StateStore,
        program: OsString,
        natural_stop_timeout: Duration,
        distribution: String,
    ) -> Self {
        let mut application = Self::with_runtime(
            state_store,
            Arc::new(SystemWslRuntime {
                program,
                distribution_filter: Some(distribution),
            }),
        );
        application.natural_stop_timeout = natural_stop_timeout;
        application
    }

    #[cfg(all(windows, feature = "wsl-guest-harness"))]
    #[doc(hidden)]
    pub fn with_wsl_program_and_fault_for_harness(
        state_store: StateStore,
        program: OsString,
        faults: Arc<dyn WslFaultInjector>,
    ) -> Self {
        Self::with_dependencies(
            state_store,
            Arc::new(SystemWslRuntime {
                program,
                distribution_filter: None,
            }),
            faults,
        )
    }

    #[doc(hidden)]
    pub(crate) fn with_runtime(state_store: StateStore, runtime: Arc<dyn WslRuntime>) -> Self {
        Self::with_dependencies(state_store, runtime, Arc::new(NoWslFaults))
    }

    #[doc(hidden)]
    pub(crate) fn with_dependencies(
        state_store: StateStore,
        runtime: Arc<dyn WslRuntime>,
        faults: Arc<dyn WslFaultInjector>,
    ) -> Self {
        Self {
            state_store,
            operation_lock: Arc::new(Mutex::new(())),
            runtime,
            faults,
            natural_stop_timeout: NATURAL_STOP_TIMEOUT,
        }
    }

    pub fn list(&self) -> Result<Vec<WslEnvironmentSummary>, WslFailure> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let probes = self.runtime.probe()?;
        let mut connection = self.open_state()?;
        reconcile_probes(&mut connection, &probes)?;
        for probe in probes
            .iter()
            .filter(|probe| probe.running && probe.availability == WslAvailability::Manageable)
        {
            if let Err(failure) = self.refresh_running_probe(&connection, probe, false) {
                if failure.category == WslFailureCategory::Busy {
                    mark_wsl_busy(&connection, &probe.environment_id)?;
                } else if failure.message_id == "wsl.credentials_invalid" {
                    mark_wsl_conflict(&connection, &probe.environment_id, failure.message_id)?;
                } else {
                    mark_wsl_attention(&connection, &probe.environment_id, failure.message_id)?;
                }
            }
        }
        load_summaries(&connection, &probes)
    }

    fn refresh_running_probe(
        &self,
        connection: &Connection,
        probe: &WslProbe,
        cleanup_credentials: bool,
    ) -> Result<(), WslFailure> {
        self.recover_refresh_lock(connection, probe)?;
        let token = Uuid::new_v4().to_string();
        set_refresh_lock_token(connection, &probe.environment_id, Some(&token))?;
        if let Err(failure) = self.runtime.acquire_lock(probe, &token, "refresh") {
            set_refresh_lock_token(connection, &probe.environment_id, None)?;
            return Err(failure);
        }
        self.faults
            .check(WslFailurePoint::AfterRefreshLockAcquired)?;
        let result = self
            .runtime
            .read_artifacts(probe)
            .and_then(|artifacts| reconcile_actual_state(connection, probe, &artifacts))
            .and_then(|()| {
                if cleanup_credentials {
                    self.runtime.cleanup_credentials(probe, &token)
                } else {
                    Ok(())
                }
            });
        self.runtime.release_lock(probe, &token)?;
        set_refresh_lock_token(connection, &probe.environment_id, None)?;
        result
    }

    fn recover_refresh_lock(
        &self,
        connection: &Connection,
        probe: &WslProbe,
    ) -> Result<(), WslFailure> {
        let Some(token) = load_refresh_lock_token(connection, &probe.environment_id)? else {
            return Ok(());
        };
        self.runtime.release_lock(probe, &token)?;
        set_refresh_lock_token(connection, &probe.environment_id, None)
    }

    pub fn recover_pending(&self) -> Result<(), WslFailure> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let probes = self.runtime.probe()?;
        let mut connection = self.open_state()?;
        reconcile_probes(&mut connection, &probes)?;
        let pending = load_pending_operations(&connection)?;
        for probe in probes
            .iter()
            .filter(|probe| probe.running && probe.availability == WslAvailability::Manageable)
        {
            self.recover_refresh_lock(&connection, probe)?;
        }
        for operation in pending {
            let Some(probe) = probes
                .iter()
                .find(|probe| probe.environment_id == operation.environment_id)
            else {
                mark_pending_attention(
                    &connection,
                    &operation.environment_id,
                    "wsl.environment_removed",
                )?;
                continue;
            };
            if !probe.running {
                mark_pending_attention(
                    &connection,
                    &operation.environment_id,
                    "wsl.recovery_pending",
                )?;
                continue;
            }
            self.reconcile_pending_for_probe(&connection, &operation, probe, true)?;
        }
        Ok(())
    }

    pub fn refresh_environment(
        &self,
        environment_id: &str,
        expected_revision: &str,
        authorize_start: bool,
    ) -> Result<WslRefreshResult, WslFailure> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let probes = self.runtime.probe()?;
        let probe = probes
            .iter()
            .find(|item| item.environment_id == environment_id)
            .cloned()
            .ok_or_else(|| {
                WslFailure::new(
                    WslFailureCategory::EnvironmentNotFound,
                    "wsl.environment_not_found",
                )
            })?;
        if probe.availability != WslAvailability::Manageable {
            return Err(wsl_availability_failure(probe.availability));
        }
        if revision_for_probe(&probe) != expected_revision {
            return Err(WslFailure::new(
                WslFailureCategory::EnvironmentChanged,
                "wsl.environment_changed",
            ));
        }
        if !probe.running && !authorize_start {
            return Err(WslFailure::new(
                WslFailureCategory::InvalidEnvironment,
                "wsl.start_authorization_required",
            ));
        }

        let mut connection = self.open_state()?;
        reconcile_probes(&mut connection, &probes)?;
        let originally_running = probe.running;
        let active_probe = if originally_running {
            probe
        } else {
            self.runtime.start(&probe)?;
            let refreshed = match self.runtime.probe().and_then(|items| {
                items
                    .into_iter()
                    .find(|item| item.environment_id == environment_id)
                    .ok_or_else(|| {
                        WslFailure::new(
                            WslFailureCategory::EnvironmentNotFound,
                            "wsl.environment_disappeared",
                        )
                    })
            }) {
                Ok(refreshed) => refreshed,
                Err(failure) => {
                    let outcome = self.observe_natural_stop(&connection, &probe)?;
                    return Err(failure.with_lifecycle_outcome(outcome));
                }
            };
            if refreshed.availability != WslAvailability::Manageable
                || !same_environment_identity(&refreshed, &probe)
                || !refreshed.running
            {
                let outcome = self.observe_natural_stop(&connection, &refreshed)?;
                return Err(WslFailure::new(
                    WslFailureCategory::EnvironmentChanged,
                    "wsl.environment_changed",
                )
                .with_lifecycle_outcome(outcome));
            }
            refreshed
        };

        if let Err(failure) = self.refresh_running_probe(&connection, &active_probe, true) {
            if failure.category == WslFailureCategory::Interrupted || originally_running {
                return Err(failure);
            }
            let outcome = self.observe_natural_stop(&connection, &active_probe)?;
            return Err(failure.with_lifecycle_outcome(outcome));
        }
        let lifecycle_outcome = if originally_running {
            WslLifecycleOutcome::UnchangedRunning
        } else {
            self.observe_natural_stop(&connection, &active_probe)?
        };
        let mut final_probe = active_probe;
        final_probe.running = match lifecycle_outcome {
            WslLifecycleOutcome::UnchangedRunning | WslLifecycleOutcome::StillRunning => true,
            WslLifecycleOutcome::StoppedNaturally => false,
        };
        Ok(WslRefreshResult {
            environment: load_summary(&connection, &final_probe)?,
            lifecycle_outcome,
        })
    }

    pub fn audit_provider_deletion(
        &self,
        provider_id: &str,
        authorize_stopped: bool,
    ) -> Result<WslDeletionAudit, WslFailure> {
        match self.audit_provider_deletion_then(provider_id, authorize_stopped, || {
            Ok::<_, std::convert::Infallible>(())
        }) {
            Ok((audit, ())) => Ok(audit),
            Err(WslDeletionAuditError::Verification(failure)) => Err(failure),
            Err(WslDeletionAuditError::Deletion { failure: never, .. }) => match never {},
        }
    }

    pub(crate) fn audit_provider_deletion_then<T, E>(
        &self,
        provider_id: &str,
        authorize_stopped: bool,
        delete: impl FnOnce() -> Result<T, E>,
    ) -> Result<(WslDeletionAudit, T), WslDeletionAuditError<E>> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| WslDeletionAuditError::Verification(state_unavailable()))?;
        let probes = self
            .runtime
            .probe()
            .map_err(WslDeletionAuditError::Verification)?;
        let managed = probes
            .iter()
            .filter(|probe| {
                if probe.availability == WslAvailability::Infrastructure {
                    return false;
                }
                if probe.availability != WslAvailability::Unavailable {
                    return true;
                }
                !probes.iter().any(|candidate| {
                    candidate.availability == WslAvailability::Manageable
                        && candidate
                            .display_name
                            .eq_ignore_ascii_case(&probe.display_name)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        if managed
            .iter()
            .any(|probe| probe.availability != WslAvailability::Manageable)
        {
            return Err(WslDeletionAuditError::Verification(WslFailure::new(
                WslFailureCategory::NeedsAttention,
                "wsl.delete_verification_unavailable",
            )));
        }
        if !authorize_stopped && managed.iter().any(|probe| !probe.running) {
            return Err(WslDeletionAuditError::Verification(WslFailure::new(
                WslFailureCategory::InvalidEnvironment,
                "wsl.delete_start_authorization_required",
            )));
        }

        let mut connection = self
            .open_state()
            .map_err(WslDeletionAuditError::Verification)?;
        reconcile_probes(&mut connection, &probes).map_err(WslDeletionAuditError::Verification)?;
        let mut audited = Vec::with_capacity(managed.len());
        for probe in managed {
            let originally_running = probe.running;
            let active_probe = if originally_running {
                probe.clone()
            } else {
                if let Err(failure) = self.runtime.start(&probe) {
                    return Err(self.deletion_verification_failure(
                        &connection,
                        &audited,
                        failure,
                        None,
                    ));
                }
                let refreshed = match self.runtime.probe().and_then(|items| {
                    items
                        .into_iter()
                        .find(|item| item.environment_id == probe.environment_id)
                        .ok_or_else(|| {
                            WslFailure::new(
                                WslFailureCategory::EnvironmentNotFound,
                                "wsl.environment_disappeared",
                            )
                        })
                }) {
                    Ok(refreshed) => refreshed,
                    Err(failure) => {
                        audited.push(DeletionAuditEnvironment {
                            probe,
                            originally_running,
                            lock_token: None,
                        });
                        return Err(self.deletion_verification_failure(
                            &connection,
                            &audited,
                            failure,
                            None,
                        ));
                    }
                };
                if refreshed.availability != WslAvailability::Manageable
                    || !same_environment_identity(&refreshed, &probe)
                    || !refreshed.running
                {
                    let environment_id = refreshed.environment_id.clone();
                    audited.push(DeletionAuditEnvironment {
                        probe: refreshed,
                        originally_running,
                        lock_token: None,
                    });
                    return Err(self.deletion_verification_failure(
                        &connection,
                        &audited,
                        WslFailure::new(
                            WslFailureCategory::EnvironmentChanged,
                            "wsl.environment_changed",
                        ),
                        Some(&environment_id),
                    ));
                }
                refreshed
            };
            let environment_id = active_probe.environment_id.clone();
            audited.push(DeletionAuditEnvironment {
                probe: active_probe,
                originally_running,
                lock_token: None,
            });
            let index = audited.len() - 1;
            let token = Uuid::new_v4().to_string();
            if let Err(failure) = self.recover_refresh_lock(&connection, &audited[index].probe) {
                return Err(self.deletion_verification_failure(
                    &connection,
                    &audited,
                    failure,
                    Some(&environment_id),
                ));
            }
            if let Err(failure) = set_refresh_lock_token(&connection, &environment_id, Some(&token))
            {
                return Err(self.deletion_verification_failure(
                    &connection,
                    &audited,
                    failure,
                    Some(&environment_id),
                ));
            }
            if let Err(failure) =
                self.runtime
                    .acquire_lock(&audited[index].probe, &token, "delete-audit")
            {
                let _ = set_refresh_lock_token(&connection, &environment_id, None);
                return Err(self.deletion_verification_failure(
                    &connection,
                    &audited,
                    failure,
                    Some(&environment_id),
                ));
            }
            audited[index].lock_token = Some(token);
            let refresh_result =
                self.runtime
                    .read_artifacts(&audited[index].probe)
                    .and_then(|artifacts| {
                        reconcile_actual_state(&connection, &audited[index].probe, &artifacts)
                    });
            if let Err(failure) = refresh_result {
                return Err(self.deletion_verification_failure(
                    &connection,
                    &audited,
                    failure,
                    Some(&environment_id),
                ));
            }
            let actual_provider_id = connection
                .query_row(
                    "SELECT actual_provider_id FROM wsl_environments WHERE environment_id = ?1",
                    [environment_id.as_str()],
                    |row| row.get::<_, Option<String>>(0),
                )
                .map_err(|_| state_unavailable());
            let actual_provider_id = match actual_provider_id {
                Ok(value) => value,
                Err(failure) => {
                    return Err(self.deletion_verification_failure(
                        &connection,
                        &audited,
                        failure,
                        Some(&environment_id),
                    ));
                }
            };
            if actual_provider_id.as_deref() == Some(provider_id) {
                return Err(self.deletion_verification_failure(
                    &connection,
                    &audited,
                    WslFailure::new(
                        WslFailureCategory::NeedsAttention,
                        "provider.wsl_current_delete_forbidden",
                    ),
                    Some(&environment_id),
                ));
            }
            let Some(token) = audited[index].lock_token.as_deref() else {
                return Err(self.deletion_verification_failure(
                    &connection,
                    &audited,
                    state_unavailable(),
                    Some(&environment_id),
                ));
            };
            if let Err(failure) = self
                .runtime
                .cleanup_credentials(&audited[index].probe, token)
            {
                return Err(self.deletion_verification_failure(
                    &connection,
                    &audited,
                    failure,
                    Some(&environment_id),
                ));
            }
        }

        let deleted = match delete() {
            Ok(deleted) => deleted,
            Err(failure) => {
                let lifecycle_results = self
                    .finish_deletion_audit(&connection, &audited)
                    .map_err(WslDeletionAuditError::Verification)?;
                return Err(WslDeletionAuditError::Deletion {
                    failure,
                    lifecycle_results,
                });
            }
        };
        let lifecycle_results = self
            .finish_deletion_audit(&connection, &audited)
            .map_err(WslDeletionAuditError::Verification)?;
        Ok((WslDeletionAudit { lifecycle_results }, deleted))
    }

    fn deletion_verification_failure<E>(
        &self,
        connection: &Connection,
        audited: &[DeletionAuditEnvironment],
        mut failure: WslFailure,
        failed_environment_id: Option<&str>,
    ) -> WslDeletionAuditError<E> {
        match self.finish_deletion_audit(connection, audited) {
            Ok(results) => {
                if failure.lifecycle_outcome.is_none()
                    && let Some(environment_id) = failed_environment_id
                {
                    failure.lifecycle_outcome = results
                        .iter()
                        .find(|result| result.environment_id == environment_id)
                        .map(|result| result.outcome);
                }
                WslDeletionAuditError::Verification(failure)
            }
            Err(unlock_failure) => WslDeletionAuditError::Verification(unlock_failure),
        }
    }

    fn finish_deletion_audit(
        &self,
        connection: &Connection,
        audited: &[DeletionAuditEnvironment],
    ) -> Result<Vec<WslLifecycleResult>, WslFailure> {
        let mut first_failure = None;
        for environment in audited.iter().rev() {
            let Some(token) = environment.lock_token.as_deref() else {
                continue;
            };
            if let Err(failure) = self.runtime.release_lock(&environment.probe, token) {
                first_failure.get_or_insert(failure);
                continue;
            }
            if let Err(failure) =
                set_refresh_lock_token(connection, &environment.probe.environment_id, None)
            {
                first_failure.get_or_insert(failure);
            }
        }
        if let Some(failure) = first_failure {
            return Err(failure);
        }
        let lifecycle_results = audited
            .iter()
            .map(|environment| {
                let outcome = if environment.originally_running {
                    WslLifecycleOutcome::UnchangedRunning
                } else {
                    self.observe_natural_stop(connection, &environment.probe)?
                };
                Ok(WslLifecycleResult {
                    environment_id: environment.probe.environment_id.clone(),
                    display_name: environment.probe.display_name.clone(),
                    outcome,
                })
            })
            .collect::<Result<Vec<_>, WslFailure>>()?;
        Ok(lifecycle_results)
    }

    pub fn apply_provider(
        &self,
        environment_id: &str,
        provider_id: &str,
        expected_revision: &str,
        confirm: bool,
    ) -> Result<WslApplyResult, WslFailure> {
        if !confirm {
            return Err(WslFailure::new(
                WslFailureCategory::InvalidEnvironment,
                "wsl.confirmation_required",
            ));
        }
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| state_unavailable())?;
        let probes = self.runtime.probe()?;
        let probe = probes
            .iter()
            .find(|item| item.environment_id == environment_id)
            .cloned()
            .ok_or_else(|| {
                WslFailure::new(
                    WslFailureCategory::EnvironmentNotFound,
                    "wsl.environment_not_found",
                )
            })?;
        if probe.availability != WslAvailability::Manageable {
            return Err(wsl_availability_failure(probe.availability));
        }
        if revision_for_probe(&probe) != expected_revision {
            return Err(WslFailure::new(
                WslFailureCategory::EnvironmentChanged,
                "wsl.environment_changed",
            ));
        }

        let mut connection = self.open_state()?;
        let provider = load_provider(&connection, provider_id)?;
        let persisted_uid = connection
            .query_row(
                "SELECT default_uid FROM wsl_environments WHERE environment_id = ?1",
                [environment_id],
                |row| row.get::<_, Option<u32>>(0),
            )
            .optional()
            .map_err(|_| state_unavailable())?
            .flatten();
        if persisted_uid.is_some() && persisted_uid != probe.default_uid {
            connection
                .execute(
                    "UPDATE wsl_environments SET default_uid = ?2, availability = 'manageable',
                        requires_attention = 0, last_error = NULL, updated_at = ?3
                     WHERE environment_id = ?1",
                    params![
                        environment_id,
                        probe.default_uid,
                        epoch_seconds().to_string()
                    ],
                )
                .map_err(|_| state_unavailable())?;
        }

        let originally_running = probe.running;
        let active_probe = if originally_running {
            probe.clone()
        } else {
            let pre_start = self
                .runtime
                .probe()?
                .into_iter()
                .find(|item| item.environment_id == environment_id)
                .ok_or_else(|| {
                    WslFailure::new(
                        WslFailureCategory::EnvironmentNotFound,
                        "wsl.environment_disappeared",
                    )
                })?;
            if revision_for_probe(&pre_start) != expected_revision {
                return Err(WslFailure::new(
                    WslFailureCategory::EnvironmentChanged,
                    "wsl.environment_changed",
                ));
            }
            self.runtime.start(&pre_start)?;
            let refreshed = self.runtime.probe().and_then(|items| {
                items
                    .into_iter()
                    .find(|item| item.environment_id == environment_id)
                    .ok_or_else(|| {
                        WslFailure::new(
                            WslFailureCategory::EnvironmentNotFound,
                            "wsl.environment_disappeared",
                        )
                    })
            });
            let refreshed = match refreshed {
                Ok(refreshed) => refreshed,
                Err(failure) => {
                    let outcome = self.observe_natural_stop(&connection, &probe)?;
                    return Err(failure.with_lifecycle_outcome(outcome));
                }
            };
            if refreshed.availability != WslAvailability::Manageable {
                let failure = wsl_availability_failure(refreshed.availability);
                let outcome = self.observe_natural_stop(&connection, &refreshed)?;
                return Err(failure.with_lifecycle_outcome(outcome));
            }
            if !same_environment_identity(&refreshed, &pre_start) || !refreshed.running {
                let outcome = self.observe_natural_stop(&connection, &refreshed)?;
                return Err(WslFailure::new(
                    WslFailureCategory::EnvironmentChanged,
                    "wsl.environment_changed",
                )
                .with_lifecycle_outcome(outcome));
            }
            refreshed
        };

        let mut result = self.apply_started(
            &mut connection,
            &active_probe,
            &provider,
            originally_running,
        );
        if originally_running
            || matches!(&result, Err(failure) if failure.category == WslFailureCategory::Interrupted)
        {
            return result;
        }
        let lifecycle_outcome = self.observe_natural_stop(&connection, &active_probe)?;
        match &mut result {
            Ok(applied) => {
                let mut final_probe = active_probe.clone();
                final_probe.running = lifecycle_outcome == WslLifecycleOutcome::StillRunning;
                applied.environment = load_summary(&connection, &final_probe)?;
                applied.lifecycle_outcome = lifecycle_outcome;
            }
            Err(failure) => failure.lifecycle_outcome = Some(lifecycle_outcome),
        }
        result
    }

    fn observe_natural_stop(
        &self,
        connection: &Connection,
        probe: &WslProbe,
    ) -> Result<WslLifecycleOutcome, WslFailure> {
        let stopped = self
            .runtime
            .wait_for_natural_stop(probe, self.natural_stop_timeout)?;
        if stopped {
            clear_lifecycle_attention(connection, &probe.environment_id)?;
            Ok(WslLifecycleOutcome::StoppedNaturally)
        } else {
            mark_lifecycle_still_running(connection, &probe.environment_id)?;
            Ok(WslLifecycleOutcome::StillRunning)
        }
    }

    fn apply_started(
        &self,
        connection: &mut Connection,
        probe: &WslProbe,
        provider: &WslProvider,
        originally_running: bool,
    ) -> Result<WslApplyResult, WslFailure> {
        self.runtime.check_codex_version(probe)?;
        if let Some(pending) = load_pending_operation(connection, &probe.environment_id)? {
            self.reconcile_pending_for_probe(connection, &pending, probe, false)?;
        }
        self.runtime.ensure_helper(probe)?;
        let token = Uuid::new_v4().to_string();
        let operation_id = Uuid::new_v4().to_string();
        let old_provider_id = connection
            .query_row(
                "SELECT actual_provider_id FROM wsl_environments WHERE environment_id = ?1",
                [probe.environment_id.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|_| state_unavailable())?;
        connection
            .execute(
                "INSERT OR REPLACE INTO wsl_pending_operation(
                    environment_id, operation_id, stage, old_provider_id, target_provider_id,
                    originally_running, expected_default_uid, expected_revision, started_at,
                    lock_token
                 ) VALUES (?1, ?2, 'registered', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    probe.environment_id,
                    operation_id,
                    old_provider_id,
                    provider.id,
                    originally_running,
                    probe.default_uid,
                    revision_for_probe(probe),
                    epoch_seconds().to_string(),
                    token,
                ],
            )
            .map_err(|_| state_unavailable())?;
        self.faults.check(WslFailurePoint::AfterPendingRegistered)?;
        if let Err(failure) = self.runtime.acquire_lock(probe, &token, "switch") {
            connection
                .execute(
                    "DELETE FROM wsl_pending_operation WHERE environment_id = ?1",
                    [probe.environment_id.as_str()],
                )
                .map_err(|_| state_unavailable())?;
            return Err(failure);
        }
        connection
            .execute(
                "UPDATE wsl_pending_operation SET stage = 'locked' WHERE environment_id = ?1",
                [probe.environment_id.as_str()],
            )
            .map_err(|_| state_unavailable())?;
        self.faults.check(WslFailurePoint::AfterLockAcquired)?;
        let result = self.apply_started_locked(
            connection,
            probe,
            provider,
            originally_running,
            &operation_id,
            &token,
        );
        if matches!(&result, Err(failure) if failure.category == WslFailureCategory::Interrupted) {
            return Err(result.expect_err("matched interrupted failure"));
        }
        if let Err(failure) = result {
            if let Some(pending) = load_pending_operation(connection, &probe.environment_id)? {
                self.reconcile_pending_for_probe(connection, &pending, probe, false)?;
            }
            return Err(failure);
        }
        self.runtime.release_lock(probe, &token)?;
        connection
            .execute(
                "DELETE FROM wsl_pending_operation WHERE environment_id = ?1",
                [probe.environment_id.as_str()],
            )
            .map_err(|_| state_unavailable())?;
        self.load_apply_result(connection, probe, originally_running)
    }

    fn apply_started_locked(
        &self,
        connection: &mut Connection,
        probe: &WslProbe,
        provider: &WslProvider,
        originally_running: bool,
        operation_id: &str,
        lock_token: &str,
    ) -> Result<(), WslFailure> {
        let current_probe = self
            .runtime
            .probe()?
            .into_iter()
            .find(|item| item.environment_id == probe.environment_id)
            .ok_or_else(|| {
                WslFailure::new(
                    WslFailureCategory::EnvironmentNotFound,
                    "wsl.environment_disappeared",
                )
            })?;
        if current_probe.availability != WslAvailability::Manageable
            || !current_probe.running
            || !same_environment_identity(&current_probe, probe)
        {
            return Err(WslFailure::new(
                WslFailureCategory::EnvironmentChanged,
                "wsl.environment_changed",
            ));
        }
        let original = self.runtime.read_artifacts(&current_probe)?;
        if matches!(
            inspect_actual_managed_state(
                original.config.as_deref(),
                original.credentials.as_deref()
            ),
            ActualManagedState::Conflict
        ) {
            return Err(WslFailure::new(
                WslFailureCategory::NeedsAttention,
                "wsl.managed_conflict",
            ));
        }
        let source_id = format!("desktop-{operation_id}");
        let config = render_config(original.config.as_deref(), provider, &source_id)?;
        let credentials = render_credentials(&provider.api_key)?;
        let old_config_hash = hash_optional(original.config.as_deref());
        let old_credentials_hash = hash_optional(original.credentials.as_deref());
        let new_config_hash = hash_bytes(&config);
        let new_credentials_hash = hash_bytes(&credentials);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| state_unavailable())?;
        transaction
            .execute(
                "UPDATE wsl_pending_operation SET stage = 'prepared',
                    old_config_fingerprint = ?3, new_config_fingerprint = ?4,
                    old_credentials_fingerprint = ?5, new_credentials_fingerprint = ?6
                 WHERE environment_id = ?1 AND operation_id = ?2",
                params![
                    current_probe.environment_id,
                    operation_id,
                    old_config_hash,
                    new_config_hash,
                    old_credentials_hash,
                    new_credentials_hash,
                ],
            )
            .map_err(|_| state_unavailable())?;
        transaction.commit().map_err(|_| state_unavailable())?;
        self.faults.check(WslFailurePoint::AfterPrepared)?;

        let bundle = bundle_bytes(&config, &credentials);
        let writer_output =
            match self
                .runtime
                .write_bundle(&current_probe, lock_token, &old_config_hash, &bundle)
            {
                Ok(output) => output,
                Err(failure) => {
                    mark_pending_attention(
                        connection,
                        &current_probe.environment_id,
                        failure.message_id,
                    )?;
                    return Err(failure);
                }
            };
        if !writer_output.contains("\"status\":\"written\"")
            || !writer_output.contains(HELPER_VERSION)
        {
            mark_wsl_attention(
                connection,
                &current_probe.environment_id,
                "wsl.guest_write_failed",
            )?;
            return Err(WslFailure::new(
                WslFailureCategory::GuestWriteFailed,
                "wsl.guest_write_failed",
            ));
        }
        connection
            .execute(
                "UPDATE wsl_pending_operation SET stage = 'artifacts_replaced' WHERE environment_id = ?1",
                [current_probe.environment_id.as_str()],
            )
            .map_err(|_| state_unavailable())?;
        self.faults.check(WslFailurePoint::AfterArtifactsReplaced)?;
        let latest = self
            .runtime
            .probe()?
            .into_iter()
            .find(|item| item.environment_id == current_probe.environment_id)
            .ok_or_else(|| {
                WslFailure::new(
                    WslFailureCategory::EnvironmentNotFound,
                    "wsl.environment_disappeared",
                )
            })?;
        if latest.availability != WslAvailability::Manageable
            || !latest.running
            || !same_environment_identity(&latest, &current_probe)
        {
            mark_wsl_attention(
                connection,
                &current_probe.environment_id,
                "wsl.environment_changed",
            )?;
            return Err(WslFailure::new(
                WslFailureCategory::EnvironmentChanged,
                "wsl.environment_changed",
            ));
        }
        let written = self.runtime.read_artifacts(&latest)?;
        if hash_optional(written.config.as_deref()) != new_config_hash
            || hash_optional(written.credentials.as_deref()) != new_credentials_hash
        {
            mark_pending_attention(
                connection,
                &current_probe.environment_id,
                "wsl.guest_reread_failed",
            )?;
            return Err(WslFailure::new(
                WslFailureCategory::GuestWriteFailed,
                "wsl.guest_reread_failed",
            ));
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| state_unavailable())?;
        transaction
            .execute(
                "UPDATE wsl_environments
                 SET current_provider_id = ?2, config_fingerprint = ?3,
                     credentials_fingerprint = ?4, pending_restart = ?5,
                     requires_attention = 0, last_error = NULL,
                     availability = 'manageable', actual_provider_id = ?2,
                     configuration_state = 'current', updated_at = ?6
                 WHERE environment_id = ?1",
                params![
                    current_probe.environment_id,
                    provider.id,
                    new_config_hash,
                    new_credentials_hash,
                    originally_running,
                    epoch_seconds().to_string(),
                ],
            )
            .map_err(|_| state_unavailable())?;
        transaction
            .execute(
                "UPDATE wsl_pending_operation SET stage = 'state_committed' WHERE environment_id = ?1",
                [current_probe.environment_id.as_str()],
            )
            .map_err(|_| state_unavailable())?;
        transaction.commit().map_err(|_| state_unavailable())?;
        self.faults.check(WslFailurePoint::AfterStateCommitted)?;
        self.runtime
            .cleanup_credentials(&current_probe, lock_token)?;
        Ok(())
    }

    fn load_apply_result(
        &self,
        connection: &Connection,
        probe: &WslProbe,
        originally_running: bool,
    ) -> Result<WslApplyResult, WslFailure> {
        Ok(WslApplyResult {
            environment: load_summary(connection, probe)?,
            pending_restart: originally_running,
            lifecycle_outcome: WslLifecycleOutcome::UnchangedRunning,
        })
    }

    fn reconcile_pending_for_probe(
        &self,
        connection: &Connection,
        pending: &PendingWslOperation,
        probe: &WslProbe,
        restore_lifecycle: bool,
    ) -> Result<(), WslFailure> {
        if pending.expected_default_uid != probe.default_uid {
            mark_pending_attention(
                connection,
                &probe.environment_id,
                "wsl.default_user_changed",
            )?;
            return Err(WslFailure::new(
                WslFailureCategory::DefaultUserChanged,
                "wsl.default_user_changed",
            ));
        }
        let token = pending.lock_token.as_deref().ok_or_else(|| {
            WslFailure::new(
                WslFailureCategory::NeedsAttention,
                "wsl.lock_recovery_required",
            )
        })?;
        let hashes = (
            pending.old_config_hash.as_deref(),
            pending.new_config_hash.as_deref(),
            pending.old_credentials_hash.as_deref(),
            pending.new_credentials_hash.as_deref(),
        );
        if pending.stage != "registered" && pending.stage != "locked" {
            if pending.stage != "state_committed" {
                let (
                    Some(old_config),
                    Some(new_config),
                    Some(old_credentials),
                    Some(new_credentials),
                ) = hashes
                else {
                    mark_pending_attention(
                        connection,
                        &probe.environment_id,
                        "wsl.recovery_conflict",
                    )?;
                    return Err(WslFailure::new(
                        WslFailureCategory::NeedsAttention,
                        "wsl.recovery_conflict",
                    ));
                };
                let artifacts = self.runtime.read_artifacts(probe)?;
                let config_hash = hash_optional(artifacts.config.as_deref());
                let credentials_hash = hash_optional(artifacts.credentials.as_deref());
                let matches_old = config_hash == old_config && credentials_hash == old_credentials;
                let matches_new = config_hash == new_config && credentials_hash == new_credentials;
                if !matches_old && !matches_new {
                    mark_pending_attention(
                        connection,
                        &probe.environment_id,
                        "wsl.recovery_conflict",
                    )?;
                    return Err(WslFailure::new(
                        WslFailureCategory::NeedsAttention,
                        "wsl.recovery_conflict",
                    ));
                }
                reconcile_actual_state(connection, probe, &artifacts)?;
                connection
                    .execute(
                        "UPDATE wsl_environments SET pending_restart = ?2 WHERE environment_id = ?1",
                        params![
                            probe.environment_id,
                            matches_new && pending.originally_running
                        ],
                    )
                    .map_err(|_| state_unavailable())?;
            }
            connection
                .execute(
                    "UPDATE wsl_pending_operation SET stage = 'state_committed' WHERE environment_id = ?1",
                    [probe.environment_id.as_str()],
                )
                .map_err(|_| state_unavailable())?;
        }
        if pending.stage != "registered" && pending.stage != "locked" {
            self.runtime.cleanup_credentials(probe, token)?;
        }
        self.runtime.release_lock(probe, token)?;
        connection
            .execute(
                "DELETE FROM wsl_pending_operation WHERE environment_id = ?1",
                [probe.environment_id.as_str()],
            )
            .map_err(|_| state_unavailable())?;
        if restore_lifecycle && !pending.originally_running && probe.running {
            self.observe_natural_stop(connection, probe)?;
        }
        Ok(())
    }

    fn open_state(&self) -> Result<Connection, WslFailure> {
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
}

#[derive(Debug, Clone)]
struct PendingWslOperation {
    environment_id: String,
    stage: String,
    old_config_hash: Option<String>,
    new_config_hash: Option<String>,
    old_credentials_hash: Option<String>,
    new_credentials_hash: Option<String>,
    originally_running: bool,
    expected_default_uid: Option<u32>,
    lock_token: Option<String>,
}

fn load_pending_operations(
    connection: &Connection,
) -> Result<Vec<PendingWslOperation>, WslFailure> {
    let mut statement = connection
        .prepare(
            "SELECT environment_id, stage, old_config_fingerprint, new_config_fingerprint,
                    old_credentials_fingerprint, new_credentials_fingerprint,
                    originally_running, expected_default_uid, lock_token
             FROM wsl_pending_operation",
        )
        .map_err(|_| state_unavailable())?;
    statement
        .query_map([], pending_from_row)
        .map_err(|_| state_unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| state_unavailable())
}

fn load_pending_operation(
    connection: &Connection,
    environment_id: &str,
) -> Result<Option<PendingWslOperation>, WslFailure> {
    connection
        .query_row(
            "SELECT environment_id, stage, old_config_fingerprint, new_config_fingerprint,
                    old_credentials_fingerprint, new_credentials_fingerprint,
                    originally_running, expected_default_uid, lock_token
             FROM wsl_pending_operation WHERE environment_id = ?1",
            [environment_id],
            pending_from_row,
        )
        .optional()
        .map_err(|_| state_unavailable())
}

fn load_refresh_lock_token(
    connection: &Connection,
    environment_id: &str,
) -> Result<Option<String>, WslFailure> {
    connection
        .query_row(
            "SELECT refresh_lock_token FROM wsl_environments WHERE environment_id = ?1",
            [environment_id],
            |row| row.get(0),
        )
        .map_err(|_| state_unavailable())
}

fn set_refresh_lock_token(
    connection: &Connection,
    environment_id: &str,
    token: Option<&str>,
) -> Result<(), WslFailure> {
    connection
        .execute(
            "UPDATE wsl_environments SET refresh_lock_token = ?2 WHERE environment_id = ?1",
            params![environment_id, token],
        )
        .map(|_| ())
        .map_err(|_| state_unavailable())
}

fn pending_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingWslOperation> {
    Ok(PendingWslOperation {
        environment_id: row.get(0)?,
        stage: row.get(1)?,
        old_config_hash: row.get(2)?,
        new_config_hash: row.get(3)?,
        old_credentials_hash: row.get(4)?,
        new_credentials_hash: row.get(5)?,
        originally_running: row.get(6)?,
        expected_default_uid: row.get(7)?,
        lock_token: row.get(8)?,
    })
}

fn mark_pending_attention(
    connection: &Connection,
    environment_id: &str,
    message_id: &str,
) -> Result<(), WslFailure> {
    connection
        .execute(
            "UPDATE wsl_pending_operation SET stage = 'needs_attention' WHERE environment_id = ?1",
            [environment_id],
        )
        .map_err(|_| state_unavailable())?;
    mark_wsl_attention(connection, environment_id, message_id)
}

#[derive(Debug, Clone)]
struct WslProvider {
    id: String,
    name: String,
    base_url: String,
    api_key: String,
    default_model: String,
    verified_at: u64,
    recommendation_id: Option<String>,
}

fn load_provider(connection: &Connection, provider_id: &str) -> Result<WslProvider, WslFailure> {
    connection
        .query_row(
            "SELECT id, name, base_url, api_key, default_model, verified_at, recommendation_id
             FROM providers WHERE id = ?1",
            [provider_id],
            |row| {
                Ok(WslProvider {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    base_url: row.get(2)?,
                    api_key: row.get(3)?,
                    default_model: row.get(4)?,
                    verified_at: row.get::<_, String>(5)?.parse().unwrap_or_default(),
                    recommendation_id: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|_| state_unavailable())?
        .ok_or_else(|| {
            WslFailure::new(
                WslFailureCategory::ProviderNotFound,
                "wsl.provider_not_found",
            )
        })
}

fn load_summaries(
    connection: &Connection,
    probes: &[WslProbe],
) -> Result<Vec<WslEnvironmentSummary>, WslFailure> {
    let mut result = Vec::with_capacity(probes.len());
    for probe in probes {
        result.push(load_summary(connection, probe)?);
    }
    let observed = probes
        .iter()
        .map(|probe| probe.environment_id.as_str())
        .collect::<HashSet<_>>();
    let mut statement = connection
        .prepare(
            "SELECT environment_id, display_name, default_uid, wsl_version, availability, last_error
             FROM wsl_environments ORDER BY display_name COLLATE NOCASE",
        )
        .map_err(|_| state_unavailable())?;
    let persisted = statement
        .query_map([], |row| {
            Ok(WslProbe {
                environment_id: row.get(0)?,
                display_name: row.get(1)?,
                command_name: None,
                default_uid: row.get(2)?,
                wsl_version: row.get(3)?,
                running: false,
                availability: parse_availability(&row.get::<_, String>(4)?),
                message_id: None,
            })
        })
        .map_err(|_| state_unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| state_unavailable())?;
    drop(statement);
    for probe in persisted
        .into_iter()
        .filter(|probe| !observed.contains(probe.environment_id.as_str()))
    {
        result.push(load_summary(connection, &probe)?);
    }
    Ok(result)
}

fn load_summary(
    connection: &Connection,
    probe: &WslProbe,
) -> Result<WslEnvironmentSummary, WslFailure> {
    let row = connection
        .query_row(
            "SELECT current_provider_id, requires_attention, pending_restart, last_error, availability,
                    actual_provider_id, configuration_state
             FROM wsl_environments WHERE environment_id = ?1",
            [probe.environment_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| state_unavailable())?
        .unwrap_or((
            None,
            false,
            false,
            None,
            availability_name(probe.availability).to_owned(),
            None,
            "unknown".to_owned(),
        ));
    let current_provider = row
        .0
        .as_deref()
        .and_then(|id| load_provider(connection, id).ok())
        .map(|provider| ProviderSummary {
            id: provider.id,
            name: provider.name,
            base_url: provider.base_url,
            default_model: provider.default_model,
            verified_at_epoch_seconds: provider.verified_at,
            is_current: true,
            recommendation_id: provider
                .recommendation_id
                .and_then(|id| (id == "dayway").then_some(id)),
            has_recommendation_update: false,
            recommendation_template_base_url: None,
        });
    Ok(WslEnvironmentSummary {
        environment_id: probe.environment_id.clone(),
        display_name: probe.display_name.clone(),
        command_name: probe.command_name.clone(),
        default_uid: probe.default_uid,
        running: probe.running,
        availability: parse_availability(&row.4),
        current_provider,
        actual_provider_id: row.5,
        configuration_state: parse_configuration_state(&row.6),
        requires_attention: row.1 || parse_availability(&row.4) != WslAvailability::Manageable,
        pending_restart: row.2,
        revision: revision_for_probe(probe),
        message_id: probe.message_id.or(row.3.as_deref()).map(str::to_owned),
    })
}

fn parse_configuration_state(value: &str) -> WslConfigurationState {
    match value {
        "none" => WslConfigurationState::None,
        "current" => WslConfigurationState::Current,
        "updated" => WslConfigurationState::Updated,
        "legacy" => WslConfigurationState::Legacy,
        "provider_missing" => WslConfigurationState::ProviderMissing,
        "conflict" => WslConfigurationState::Conflict,
        "busy" => WslConfigurationState::Busy,
        _ => WslConfigurationState::Unknown,
    }
}

#[derive(Debug)]
enum ActualManagedState {
    None,
    Current {
        provider_id: String,
        name: String,
        base_url: String,
        model: String,
    },
    Legacy {
        provider_id: String,
    },
    Conflict,
}

fn reconcile_actual_state(
    connection: &Connection,
    probe: &WslProbe,
    artifacts: &WslArtifacts,
) -> Result<(), WslFailure> {
    let config_hash = hash_optional(artifacts.config.as_deref());
    let credential_hash = hash_optional(artifacts.credentials.as_deref());
    let previous_config_hash = connection
        .query_row(
            "SELECT config_fingerprint FROM wsl_environments WHERE environment_id = ?1",
            [probe.environment_id.as_str()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|_| state_unavailable())?
        .flatten();
    let observed = inspect_actual_managed_state(
        artifacts.config.as_deref(),
        artifacts.credentials.as_deref(),
    );
    let (actual_provider_id, current_provider_id, state, requires_attention, message_id) =
        match observed {
            ActualManagedState::None => (None, None, "none", false, None),
            ActualManagedState::Conflict => {
                (None, None, "conflict", true, Some("wsl.managed_conflict"))
            }
            ActualManagedState::Legacy { provider_id } => {
                let known = provider_exists(connection, &provider_id)?;
                (
                    Some(provider_id.clone()),
                    known.then_some(provider_id),
                    if known { "legacy" } else { "provider_missing" },
                    !known,
                    (!known).then_some("wsl.provider_missing"),
                )
            }
            ActualManagedState::Current {
                provider_id,
                name,
                base_url,
                model,
            } => {
                let provider = load_provider_optional(connection, &provider_id)?;
                match provider {
                    Some(provider) => {
                        let matches = provider.name == name
                            && provider.base_url == base_url
                            && provider.default_model == model
                            && artifacts.credentials.as_deref()
                                == Some(provider.api_key.as_bytes());
                        (
                            Some(provider_id.clone()),
                            Some(provider_id),
                            if matches { "current" } else { "updated" },
                            false,
                            None,
                        )
                    }
                    None => (
                        Some(provider_id),
                        None,
                        "provider_missing",
                        true,
                        Some("wsl.provider_missing"),
                    ),
                }
            }
        };
    let externally_changed = previous_config_hash
        .as_deref()
        .is_some_and(|previous| previous != config_hash);
    connection
        .execute(
            "UPDATE wsl_environments SET actual_provider_id = ?2, current_provider_id = ?3,
                configuration_state = ?4, config_fingerprint = ?5,
                credentials_fingerprint = ?6,
                pending_restart = CASE WHEN ?7 THEN 1 ELSE pending_restart END,
                requires_attention = ?8, last_error = ?9, updated_at = ?10
             WHERE environment_id = ?1",
            params![
                probe.environment_id,
                actual_provider_id,
                current_provider_id,
                state,
                config_hash,
                credential_hash,
                externally_changed,
                requires_attention,
                message_id,
                epoch_seconds().to_string(),
            ],
        )
        .map_err(|_| state_unavailable())?;
    Ok(())
}

fn provider_exists(connection: &Connection, provider_id: &str) -> Result<bool, WslFailure> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM providers WHERE id = ?1)",
            [provider_id],
            |row| row.get(0),
        )
        .map_err(|_| state_unavailable())
}

fn load_provider_optional(
    connection: &Connection,
    provider_id: &str,
) -> Result<Option<WslProvider>, WslFailure> {
    match load_provider(connection, provider_id) {
        Ok(provider) => Ok(Some(provider)),
        Err(failure) if failure.category == WslFailureCategory::ProviderNotFound => Ok(None),
        Err(failure) => Err(failure),
    }
}

fn inspect_actual_managed_state(
    config: Option<&[u8]>,
    credential: Option<&[u8]>,
) -> ActualManagedState {
    let Some(config) = config else {
        return ActualManagedState::None;
    };
    let Ok(text) = std::str::from_utf8(config) else {
        return ActualManagedState::Conflict;
    };
    let start = "# >>> GPTEasy managed provider >>>";
    let end = "# <<< GPTEasy managed provider <<<";
    let lines = text
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .collect::<Vec<_>>();
    let starts = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == start)
        .collect::<Vec<_>>();
    let ends = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == end)
        .collect::<Vec<_>>();
    if starts.is_empty() && ends.is_empty() {
        return ActualManagedState::None;
    }
    if starts.len() != 1 || ends.len() != 1 || starts[0].0 >= ends[0].0 {
        return ActualManagedState::Conflict;
    }
    let block = &lines[starts[0].0..=ends[0].0];
    let metadata = |prefix: &str| {
        block
            .iter()
            .filter_map(|line| line.strip_prefix(prefix).map(str::trim))
            .collect::<Vec<_>>()
    };
    let provider_ids = metadata("# GPTEasy provider-id:");
    if provider_ids.len() != 1 || Uuid::parse_str(provider_ids[0]).is_err() {
        return ActualManagedState::Conflict;
    }
    let provider_id = provider_ids[0].to_owned();
    let schemas = metadata("# GPTEasy schema-version:");
    if schemas.is_empty() {
        return inspect_legacy_managed_state(text, provider_id);
    }
    if !schema_v1_block_has_expected_shape(block) {
        return ActualManagedState::Conflict;
    }
    let sources = metadata("# GPTEasy source-id:");
    let credential_files = metadata("# GPTEasy credential-file:");
    if schemas != ["1"] || sources.len() != 1 || credential_files.len() != 1 || credential.is_none()
    {
        return ActualManagedState::Conflict;
    }
    let expected_credential = format!(
        ".gpteasy-shell/credentials/{}/{}.token",
        sources[0], provider_id
    );
    if credential_files[0] != expected_credential
        || credential_relative_from_config(config) != Some(expected_credential.as_str())
    {
        return ActualManagedState::Conflict;
    }
    let Ok(document) = text.parse::<toml_edit::DocumentMut>() else {
        return ActualManagedState::Conflict;
    };
    let Some(model) = document.get("model").and_then(|value| value.as_str()) else {
        return ActualManagedState::Conflict;
    };
    if document
        .get("model_provider")
        .and_then(|value| value.as_str())
        != Some("gpteasy")
    {
        return ActualManagedState::Conflict;
    }
    let Some(provider) = document
        .get("model_providers")
        .and_then(|value| value.get("gpteasy"))
    else {
        return ActualManagedState::Conflict;
    };
    let (Some(name), Some(base_url)) = (
        provider.get("name").and_then(|value| value.as_str()),
        provider.get("base_url").and_then(|value| value.as_str()),
    ) else {
        return ActualManagedState::Conflict;
    };
    let expected_auth_script =
        format!("cat -- \"${{CODEX_HOME:-$HOME/.codex}}/{expected_credential}\"");
    let auth_args = provider
        .get("auth")
        .and_then(|value| value.get("args"))
        .and_then(|value| value.as_array());
    if provider.get("wire_api").and_then(|value| value.as_str()) != Some("responses")
        || provider
            .get("auth")
            .and_then(|value| value.get("command"))
            .and_then(|value| value.as_str())
            != Some("sh")
        || provider
            .get("supports_websockets")
            .and_then(|value| value.as_bool())
            != Some(false)
        || auth_args.is_none_or(|args| {
            args.len() != 2
                || args.get(0).and_then(|value| value.as_str()) != Some("-c")
                || args.get(1).and_then(|value| value.as_str())
                    != Some(expected_auth_script.as_str())
        })
    {
        return ActualManagedState::Conflict;
    }
    ActualManagedState::Current {
        provider_id,
        name: name.to_owned(),
        base_url: base_url.to_owned(),
        model: model.to_owned(),
    }
}

fn schema_v1_block_has_expected_shape(block: &[&str]) -> bool {
    const EXACT_LINES: [&str; 4] = [
        "model_provider = \"gpteasy\"",
        "model_providers.gpteasy.wire_api = \"responses\"",
        "model_providers.gpteasy.supports_websockets = false",
        "model_providers.gpteasy.auth.command = \"sh\"",
    ];
    const PREFIXES: [&str; 8] = [
        "# GPTEasy schema-version:",
        "# GPTEasy provider-id:",
        "# GPTEasy source-id:",
        "# GPTEasy credential-file:",
        "model = ",
        "model_providers.gpteasy.name = ",
        "model_providers.gpteasy.base_url = ",
        "model_providers.gpteasy.auth.args = ",
    ];

    block.len() == 14
        && block[1..block.len() - 1].iter().all(|line| {
            EXACT_LINES.contains(line) || PREFIXES.iter().any(|prefix| line.starts_with(prefix))
        })
}

fn credential_relative_from_config(config: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(config).ok()?;
    let values = text
        .lines()
        .filter_map(|line| {
            line.trim_end_matches('\r')
                .strip_prefix("# GPTEasy credential-file:")
                .map(str::trim)
        })
        .collect::<Vec<_>>();
    let [relative] = values.as_slice() else {
        return None;
    };
    if relative.contains("..") || relative.contains("//") || relative.contains(['\0', '\r', '\n']) {
        return None;
    }
    let parts = relative.split('/').collect::<Vec<_>>();
    if parts.len() != 4
        || parts[0] != ".gpteasy-shell"
        || parts[1] != "credentials"
        || parts[2].is_empty()
        || !parts[2]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || !parts[3].ends_with(".token")
        || Uuid::parse_str(parts[3].trim_end_matches(".token")).is_err()
    {
        return None;
    }
    Some(relative)
}

fn inspect_legacy_managed_state(text: &str, provider_id: String) -> ActualManagedState {
    let Ok(document) = text.parse::<toml_edit::DocumentMut>() else {
        return ActualManagedState::Conflict;
    };
    let provider_table = document
        .get("model_providers")
        .and_then(|value| value.get(&provider_id));
    if document
        .get("model")
        .and_then(|value| value.as_str())
        .is_some()
        && document
            .get("model_provider")
            .and_then(|value| value.as_str())
            == Some(&provider_id)
        && provider_table
            .and_then(|value| value.get("wire_api"))
            .and_then(|value| value.as_str())
            == Some("responses")
    {
        ActualManagedState::Legacy { provider_id }
    } else {
        ActualManagedState::Conflict
    }
}

fn reconcile_probes(connection: &mut Connection, probes: &[WslProbe]) -> Result<(), WslFailure> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| state_unavailable())?;
    let mut seen = HashSet::new();
    for probe in probes {
        seen.insert(probe.environment_id.clone());
        let old_uid = transaction
            .query_row(
                "SELECT default_uid FROM wsl_environments WHERE environment_id = ?1",
                [probe.environment_id.as_str()],
                |row| row.get::<_, Option<u32>>(0),
            )
            .optional()
            .map_err(|_| state_unavailable())?
            .flatten();
        let default_changed = old_uid.is_some() && old_uid != probe.default_uid;
        let availability = if default_changed {
            "default_user_changed"
        } else {
            availability_name(probe.availability)
        };
        transaction
            .execute(
                "INSERT INTO wsl_environments(
                    environment_id, display_name, command_name, default_uid, wsl_version,
                    availability, requires_attention, last_error, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(environment_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    command_name = excluded.command_name,
                    default_uid = CASE
                        WHEN wsl_environments.default_uid IS NOT NULL
                             AND excluded.default_uid IS NOT wsl_environments.default_uid
                        THEN wsl_environments.default_uid
                        ELSE excluded.default_uid
                    END,
                    wsl_version = excluded.wsl_version,
                    availability = excluded.availability,
                    requires_attention = CASE WHEN excluded.availability = 'manageable'
                        THEN wsl_environments.requires_attention ELSE 1 END,
                    last_error = CASE WHEN excluded.availability = 'manageable'
                        THEN wsl_environments.last_error ELSE excluded.last_error END,
                    updated_at = excluded.updated_at",
                params![
                    probe.environment_id,
                    probe.display_name,
                    probe.command_name,
                    probe.default_uid,
                    probe.wsl_version,
                    availability,
                    default_changed || probe.availability != WslAvailability::Manageable,
                    if default_changed {
                        Some("wsl.default_user_changed")
                    } else {
                        probe.message_id
                    },
                    epoch_seconds().to_string(),
                ],
            )
            .map_err(|_| state_unavailable())?;
    }
    let mut statement = transaction
        .prepare("SELECT environment_id FROM wsl_environments")
        .map_err(|_| state_unavailable())?;
    let existing = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| state_unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| state_unavailable())?;
    drop(statement);
    for environment_id in existing.into_iter().filter(|id| !seen.contains(id)) {
        transaction
            .execute(
                "UPDATE wsl_environments SET availability = 'removed', command_name = NULL,
                    requires_attention = 1, last_error = 'wsl.environment_removed', updated_at = ?2
                 WHERE environment_id = ?1",
                params![environment_id, epoch_seconds().to_string()],
            )
            .map_err(|_| state_unavailable())?;
    }
    transaction.commit().map_err(|_| state_unavailable())
}

fn mark_wsl_attention(
    connection: &Connection,
    environment_id: &str,
    message_id: &str,
) -> Result<(), WslFailure> {
    connection
        .execute(
            "UPDATE wsl_environments SET requires_attention = 1, last_error = ?2,
             availability = 'needs_refresh', updated_at = ?3 WHERE environment_id = ?1",
            params![environment_id, message_id, epoch_seconds().to_string()],
        )
        .map_err(|_| state_unavailable())?;
    Ok(())
}

fn mark_wsl_conflict(
    connection: &Connection,
    environment_id: &str,
    message_id: &str,
) -> Result<(), WslFailure> {
    connection
        .execute(
            "UPDATE wsl_environments SET actual_provider_id = NULL, current_provider_id = NULL,
                configuration_state = 'conflict', requires_attention = 1, last_error = ?2,
                updated_at = ?3 WHERE environment_id = ?1",
            params![environment_id, message_id, epoch_seconds().to_string()],
        )
        .map_err(|_| state_unavailable())?;
    Ok(())
}

fn mark_wsl_busy(connection: &Connection, environment_id: &str) -> Result<(), WslFailure> {
    connection
        .execute(
            "UPDATE wsl_environments SET configuration_state = 'busy',
                last_error = 'wsl.lock_busy', updated_at = ?2 WHERE environment_id = ?1",
            params![environment_id, epoch_seconds().to_string()],
        )
        .map_err(|_| state_unavailable())?;
    Ok(())
}

fn mark_lifecycle_still_running(
    connection: &Connection,
    environment_id: &str,
) -> Result<(), WslFailure> {
    connection
        .execute(
            "UPDATE wsl_environments SET requires_attention = 1,
                last_error = 'wsl.lifecycle_still_running', updated_at = ?2
             WHERE environment_id = ?1",
            params![environment_id, epoch_seconds().to_string()],
        )
        .map_err(|_| state_unavailable())?;
    Ok(())
}

fn clear_lifecycle_attention(
    connection: &Connection,
    environment_id: &str,
) -> Result<(), WslFailure> {
    connection
        .execute(
            "UPDATE wsl_environments SET
                requires_attention = CASE WHEN last_error = 'wsl.lifecycle_still_running'
                    THEN 0 ELSE requires_attention END,
                last_error = CASE WHEN last_error = 'wsl.lifecycle_still_running'
                    THEN NULL ELSE last_error END,
                pending_restart = 0, updated_at = ?2
             WHERE environment_id = ?1",
            params![environment_id, epoch_seconds().to_string()],
        )
        .map_err(|_| state_unavailable())?;
    Ok(())
}

fn availability_name(value: WslAvailability) -> &'static str {
    match value {
        WslAvailability::Manageable => "manageable",
        WslAvailability::Infrastructure => "infrastructure",
        WslAvailability::UnsupportedVersion => "unsupported_version",
        WslAvailability::Ambiguous => "ambiguous",
        WslAvailability::Removed => "removed",
        WslAvailability::Unavailable => "unavailable",
        WslAvailability::DefaultUserChanged => "default_user_changed",
        WslAvailability::NeedsRefresh => "needs_refresh",
    }
}

fn parse_availability(value: &str) -> WslAvailability {
    match value {
        "manageable" => WslAvailability::Manageable,
        "infrastructure" => WslAvailability::Infrastructure,
        "unsupported_version" => WslAvailability::UnsupportedVersion,
        "ambiguous" => WslAvailability::Ambiguous,
        "removed" => WslAvailability::Removed,
        "unavailable" => WslAvailability::Unavailable,
        "default_user_changed" => WslAvailability::DefaultUserChanged,
        _ => WslAvailability::NeedsRefresh,
    }
}

fn wsl_availability_failure(availability: WslAvailability) -> WslFailure {
    match availability {
        WslAvailability::DefaultUserChanged => WslFailure::new(
            WslFailureCategory::DefaultUserChanged,
            "wsl.default_user_changed",
        ),
        WslAvailability::NeedsRefresh => WslFailure::new(
            WslFailureCategory::EnvironmentChanged,
            "wsl.refresh_required",
        ),
        WslAvailability::Unavailable => WslFailure::new(
            WslFailureCategory::ProbeFailed,
            "wsl.environment_unavailable",
        ),
        WslAvailability::Removed => WslFailure::new(
            WslFailureCategory::EnvironmentNotFound,
            "wsl.environment_removed",
        ),
        WslAvailability::Ambiguous => WslFailure::new(
            WslFailureCategory::InvalidEnvironment,
            "wsl.environment_ambiguous",
        ),
        WslAvailability::Infrastructure => WslFailure::new(
            WslFailureCategory::InvalidEnvironment,
            "wsl.infrastructure_distribution",
        ),
        WslAvailability::UnsupportedVersion => {
            WslFailure::new(WslFailureCategory::InvalidEnvironment, "wsl.wsl2_required")
        }
        WslAvailability::Manageable => state_unavailable(),
    }
}

fn revision_for_probe(probe: &WslProbe) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-wsl-revision-v1\0");
    for value in [
        probe.environment_id.as_str(),
        probe.display_name.as_str(),
        probe.command_name.as_deref().unwrap_or(""),
        probe
            .default_uid
            .map(|uid| uid.to_string())
            .as_deref()
            .unwrap_or(""),
        probe
            .wsl_version
            .map(|version| version.to_string())
            .as_deref()
            .unwrap_or(""),
        if probe.running { "running" } else { "stopped" },
        availability_name(probe.availability),
    ] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn same_environment_identity(left: &WslProbe, right: &WslProbe) -> bool {
    left.environment_id == right.environment_id
        && left.display_name == right.display_name
        && left.command_name == right.command_name
        && left.default_uid == right.default_uid
        && left.wsl_version == right.wsl_version
}

fn bundle_bytes(config: &[u8], credentials: &[u8]) -> Vec<u8> {
    let mut bundle =
        format!("{BUNDLE_MAGIC}\n{}\n{}\n", config.len(), credentials.len()).into_bytes();
    bundle.extend_from_slice(config);
    bundle.extend_from_slice(credentials);
    bundle
}

fn hash_optional(bytes: Option<&[u8]>) -> String {
    bytes
        .map(hash_bytes)
        .unwrap_or_else(|| "missing".to_owned())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn render_config(
    original: Option<&[u8]>,
    provider: &WslProvider,
    source_id: &str,
) -> Result<Vec<u8>, WslFailure> {
    let text = original
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| WslFailure::new(WslFailureCategory::InvalidEnvironment, "wsl.config_invalid"))?
        .unwrap_or("");
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let parsed = if text.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        text.parse::<toml_edit::DocumentMut>().map_err(|_| {
            WslFailure::new(WslFailureCategory::InvalidEnvironment, "wsl.config_invalid")
        })?
    };
    let mut document = parsed;
    let credential_relative = format!(
        ".gpteasy-shell/credentials/{source_id}/{}.token",
        provider.id
    );
    let auth_script = format!("cat -- \"${{CODEX_HOME:-$HOME/.codex}}/{credential_relative}\"");
    let block = [
        "# >>> GPTEasy managed provider >>>".to_owned(),
        "# GPTEasy schema-version: 1".to_owned(),
        format!("# GPTEasy provider-id: {}", provider.id),
        format!("# GPTEasy source-id: {source_id}"),
        format!("# GPTEasy credential-file: {credential_relative}"),
        format!(
            "model = {}",
            toml_edit::Value::from(provider.default_model.as_str())
        ),
        "model_provider = \"gpteasy\"".to_owned(),
        format!(
            "model_providers.gpteasy.name = {}",
            toml_edit::Value::from(provider.name.as_str())
        ),
        format!(
            "model_providers.gpteasy.base_url = {}",
            toml_edit::Value::from(provider.base_url.as_str())
        ),
        "model_providers.gpteasy.wire_api = \"responses\"".to_owned(),
        "model_providers.gpteasy.supports_websockets = false".to_owned(),
        "model_providers.gpteasy.auth.command = \"sh\"".to_owned(),
        format!(
            "model_providers.gpteasy.auth.args = [\"-c\", {}]",
            toml_edit::Value::from(auth_script)
        ),
        "# <<< GPTEasy managed provider <<<".to_owned(),
        String::new(),
    ]
    .join(newline);
    let source = document.to_string();
    let start = "# >>> GPTEasy managed provider >>>";
    let end = "# <<< GPTEasy managed provider <<<";
    let start_count = source.match_indices(start).count();
    let end_count = source.match_indices(end).count();
    let rendered = match (start_count, end_count) {
        (1, 1) => {
            let start_at = source.find(start).expect("counted start marker");
            let end_at = source.find(end).expect("counted end marker");
            if start_at >= end_at {
                return Err(WslFailure::new(
                    WslFailureCategory::InvalidEnvironment,
                    "wsl.managed_conflict",
                ));
            }
            let end_line = source[end_at..]
                .find('\n')
                .map(|offset| end_at + offset + 1)
                .unwrap_or(source.len());
            format!("{}{}{}", &source[..start_at], block, &source[end_line..])
        }
        (0, 0) => {
            document.remove("model");
            document.remove("model_provider");
            format!("{}{}{}", block, newline, document)
        }
        _ => {
            return Err(WslFailure::new(
                WslFailureCategory::InvalidEnvironment,
                "wsl.managed_conflict",
            ));
        }
    };
    rendered.parse::<toml_edit::DocumentMut>().map_err(|_| {
        WslFailure::new(WslFailureCategory::InvalidEnvironment, "wsl.config_invalid")
    })?;
    Ok(rendered.into_bytes())
}

fn render_credentials(api_key: &str) -> Result<Vec<u8>, WslFailure> {
    if api_key.is_empty() || api_key.contains(['\0', '\r', '\n']) {
        return Err(WslFailure::new(
            WslFailureCategory::InvalidEnvironment,
            "wsl.credentials_invalid",
        ));
    }
    Ok(api_key.as_bytes().to_vec())
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn state_unavailable() -> WslFailure {
    WslFailure::new(
        WslFailureCategory::StateUnavailable,
        "wsl.state_unavailable",
    )
}

#[derive(Debug)]
struct SystemWslRuntime {
    program: OsString,
    #[cfg(windows)]
    distribution_filter: Option<String>,
}

impl Default for SystemWslRuntime {
    fn default() -> Self {
        Self {
            program: OsString::from("wsl.exe"),
            #[cfg(windows)]
            distribution_filter: None,
        }
    }
}

impl WslRuntime for SystemWslRuntime {
    fn probe(&self) -> Result<Vec<WslProbe>, WslFailure> {
        #[cfg(not(windows))]
        {
            return Err(WslFailure::new(
                WslFailureCategory::UnsupportedPlatform,
                "wsl.unsupported_platform",
            ));
        }
        #[cfg(windows)]
        {
            let mut registry = read_registry_distributions()?;
            if let Some(filter) = self.distribution_filter.as_deref() {
                registry.retain(|item| item.name.eq_ignore_ascii_case(filter));
            }
            if registry.is_empty() {
                return Ok(Vec::new());
            }
            let _ = run_wsl_with(&self.program, &["--version"], None)?;
            let all =
                decode_wsl_output(&run_wsl_with(&self.program, &["--list", "--quiet"], None)?)?;
            let running = decode_wsl_output(&run_wsl_with(
                &self.program,
                &["--list", "--running", "--quiet"],
                None,
            )?)?;
            let running_names: HashSet<String> = running
                .lines()
                .map(|line| line.trim().to_ascii_lowercase())
                .filter(|line| !line.is_empty())
                .collect();
            let listed_names = all
                .lines()
                .map(|line| line.trim().to_ascii_lowercase())
                .filter(|line| !line.is_empty())
                .collect::<HashSet<_>>();
            Ok(probes_from_registry(
                registry,
                &listed_names,
                &running_names,
            ))
        }
    }

    fn start(&self, environment: &WslProbe) -> Result<(), WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        run_wsl_with(
            &self.program,
            &["--distribution", name, "--exec", "/bin/true"],
            None,
        )
        .map(|_| ())
    }

    fn wait_for_natural_stop(
        &self,
        environment: &WslProbe,
        timeout: Duration,
    ) -> Result<bool, WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        let deadline = Instant::now() + timeout;
        loop {
            let running = decode_wsl_output(&run_wsl_with(
                &self.program,
                &["--list", "--running", "--quiet"],
                None,
            )?)?;
            let is_running = running
                .lines()
                .map(|line| line.trim_matches(['\0', '\u{feff}', '\r']).trim())
                .any(|running_name| running_name.eq_ignore_ascii_case(name));
            if !is_running {
                return Ok(true);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            thread::sleep(NATURAL_STOP_POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
        }
    }

    fn acquire_lock(
        &self,
        environment: &WslProbe,
        token: &str,
        operation: &str,
    ) -> Result<(), WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        let output = run_wsl_raw_with(
            &self.program,
            &[
                "--distribution",
                name,
                "--exec",
                "/bin/sh",
                "-c",
                GUEST_LOCK,
                "gpteasy",
                "acquire",
                token,
                operation,
            ],
            None,
        )?;
        if output.status.success() {
            Ok(())
        } else if output.status.code() == Some(42) {
            Err(WslFailure::new(WslFailureCategory::Busy, "wsl.lock_busy"))
        } else {
            Err(WslFailure::new(
                WslFailureCategory::NeedsAttention,
                "wsl.lock_unsafe",
            ))
        }
    }

    fn release_lock(&self, environment: &WslProbe, token: &str) -> Result<(), WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        let output = run_wsl_raw_with(
            &self.program,
            &[
                "--distribution",
                name,
                "--exec",
                "/bin/sh",
                "-c",
                GUEST_LOCK,
                "gpteasy",
                "release",
                token,
            ],
            None,
        )?;
        if output.status.success() {
            Ok(())
        } else {
            Err(WslFailure::new(
                WslFailureCategory::RecoveryPending,
                "wsl.lock_recovery_required",
            ))
        }
    }

    fn check_codex_version(&self, environment: &WslProbe) -> Result<(), WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        let output = run_wsl_raw_with(&self.program, &codex_version_probe_args(name), None)?;
        let decoded = decode_wsl_output(&output)?;
        let message_id = match classify_codex_version_probe(&decoded) {
            CodexVersionProbe::Supported if output.status.success() => return Ok(()),
            CodexVersionProbe::NotFound => "wsl.codex_not_found",
            CodexVersionProbe::TooOld => "wsl.codex_version_too_old",
            CodexVersionProbe::Supported | CodexVersionProbe::Unrecognized => {
                "wsl.codex_version_required"
            }
        };
        Err(WslFailure::new(
            WslFailureCategory::GuestUnavailable,
            message_id,
        ))
    }

    fn read_artifacts(&self, environment: &WslProbe) -> Result<WslArtifacts, WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        let config = read_guest_file(&self.program, name, ".codex/config.toml")?;
        let credentials = config
            .as_deref()
            .and_then(credential_relative_from_config)
            .map(|relative| read_guest_private_file(&self.program, name, relative))
            .transpose()?
            .flatten();
        Ok(WslArtifacts {
            config,
            credentials,
        })
    }

    fn cleanup_credentials(
        &self,
        environment: &WslProbe,
        lock_token: &str,
    ) -> Result<(), WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        let output = run_wsl_raw_with(
            &self.program,
            &[
                "--distribution",
                name,
                "--exec",
                "/bin/sh",
                "-c",
                GUEST_CREDENTIAL_CLEANUP,
                "gpteasy",
                lock_token,
            ],
            None,
        )?;
        if output.status.success() {
            Ok(())
        } else {
            let (category, message_id) = match output.status.code() {
                Some(43) => (
                    WslFailureCategory::RecoveryPending,
                    "wsl.credential_cleanup_unsafe",
                ),
                Some(47) => (
                    WslFailureCategory::NeedsAttention,
                    "wsl.credential_cleanup_conflict",
                ),
                _ => (
                    WslFailureCategory::GuestWriteFailed,
                    "wsl.credential_cleanup_failed",
                ),
            };
            Err(WslFailure::new(category, message_id))
        }
    }

    fn ensure_helper(&self, environment: &WslProbe) -> Result<(), WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        let command = format!(
            "set -eu; umask 077; mkdir -p \"$HOME/.local/lib/gpteasy\"; \
             tmp=$(mktemp \"$HOME/.local/lib/gpteasy/.writer.XXXXXX\"); \
             trap 'rm -f \"$tmp\"' EXIT HUP INT TERM; cat > \"$tmp\"; chmod 700 \"$tmp\"; \
             mv \"$tmp\" \"{HELPER_PATH}\"; trap - EXIT HUP INT TERM; \
             sha256sum \"{HELPER_PATH}\" | awk '{{print $1}}'"
        );
        let output = run_wsl_with(
            &self.program,
            &["--distribution", name, "--exec", "/bin/sh", "-c", &command],
            Some(GUEST_WRITER),
        )?;
        let actual = decode_wsl_output(&output)?;
        if actual.trim() == hash_bytes(GUEST_WRITER) {
            Ok(())
        } else {
            Err(WslFailure::new(
                WslFailureCategory::GuestUnavailable,
                "wsl.helper_verification_failed",
            ))
        }
    }

    fn write_bundle(
        &self,
        environment: &WslProbe,
        lock_token: &str,
        old_config_hash: &str,
        bundle: &[u8],
    ) -> Result<String, WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        let command = format!("exec \"{HELPER_PATH}\" \"$@\"");
        let args = [
            "--distribution",
            name,
            "--exec",
            "/bin/sh",
            "-c",
            command.as_str(),
            "gpteasy",
            lock_token,
            old_config_hash,
        ];
        let output = run_wsl_raw_with(&self.program, &args, Some(bundle))?;
        if output.status.success() {
            decode_wsl_output(&output)
        } else {
            let (category, message_id) = match output.status.code() {
                Some(40) => (WslFailureCategory::InvalidEnvironment, "wsl.config_invalid"),
                Some(41) => (
                    WslFailureCategory::ConcurrentModification,
                    "wsl.concurrent_modification",
                ),
                Some(43) => (
                    WslFailureCategory::RecoveryPending,
                    "wsl.lock_recovery_required",
                ),
                Some(46) => (
                    WslFailureCategory::InvalidEnvironment,
                    "wsl.credential_conflict",
                ),
                _ => (
                    WslFailureCategory::GuestWriteFailed,
                    "wsl.guest_write_failed",
                ),
            };
            Err(WslFailure::new(category, message_id))
        }
    }
}

#[cfg(all(windows, test))]
fn run_wsl(args: &[&str], stdin: Option<&[u8]>) -> Result<Output, WslFailure> {
    run_wsl_with(OsStr::new("wsl.exe"), args, stdin)
}

#[cfg(windows)]
fn run_wsl_with(
    program: &OsStr,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<Output, WslFailure> {
    let output = run_wsl_raw_with(program, args, stdin)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(WslFailure::new(
            WslFailureCategory::ProbeFailed,
            "wsl.command_failed",
        ))
    }
}

#[cfg(windows)]
fn run_wsl_raw_with(
    program: &OsStr,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<Output, WslFailure> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().map_err(|_| {
        WslFailure::new(WslFailureCategory::ProbeFailed, "wsl.process_start_failed")
    })?;
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .ok_or_else(state_unavailable)?
            .write_all(bytes)
            .map_err(|_| {
                WslFailure::new(WslFailureCategory::GuestWriteFailed, "wsl.stdin_failed")
            })?;
    }
    let output = child
        .wait_with_output()
        .map_err(|_| WslFailure::new(WslFailureCategory::ProbeFailed, "wsl.process_failed"))?;
    Ok(output)
}

#[cfg(not(windows))]
fn run_wsl_with(
    _program: &OsStr,
    _args: &[&str],
    _stdin: Option<&[u8]>,
) -> Result<Output, WslFailure> {
    Err(WslFailure::new(
        WslFailureCategory::UnsupportedPlatform,
        "wsl.unsupported_platform",
    ))
}

#[cfg(not(windows))]
fn run_wsl_raw_with(
    _program: &OsStr,
    _args: &[&str],
    _stdin: Option<&[u8]>,
) -> Result<Output, WslFailure> {
    Err(WslFailure::new(
        WslFailureCategory::UnsupportedPlatform,
        "wsl.unsupported_platform",
    ))
}

#[cfg(windows)]
fn read_guest_file(
    program: &OsStr,
    name: &str,
    relative: &str,
) -> Result<Option<Vec<u8>>, WslFailure> {
    let command =
        format!("if [ -f \"$HOME/{relative}\" ]; then cat \"$HOME/{relative}\"; else exit 44; fi");
    let mut command_process = Command::new(program);
    use std::os::windows::process::CommandExt;
    command_process
        .creation_flags(0x08000000)
        .args(["--distribution", name, "--exec", "/bin/sh", "-c", &command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command_process
        .output()
        .map_err(|_| WslFailure::new(WslFailureCategory::GuestUnavailable, "wsl.read_failed"))?;
    if output.status.success() {
        Ok(Some(output.stdout))
    } else if output.status.code() == Some(44) {
        Ok(None)
    } else {
        Err(WslFailure::new(
            WslFailureCategory::GuestUnavailable,
            "wsl.read_failed",
        ))
    }
}

#[cfg(windows)]
fn read_guest_private_file(
    program: &OsStr,
    name: &str,
    relative: &str,
) -> Result<Option<Vec<u8>>, WslFailure> {
    let output = run_wsl_raw_with(
        program,
        &[
            "--distribution",
            name,
            "--exec",
            "/bin/sh",
            "-c",
            GUEST_PRIVATE_READER,
            "gpteasy",
            relative,
        ],
        None,
    )?;
    if output.status.success() {
        Ok(Some(output.stdout))
    } else if output.status.code() == Some(44) {
        Ok(None)
    } else if output.status.code() == Some(43) {
        Err(WslFailure::new(
            WslFailureCategory::NeedsAttention,
            "wsl.credentials_invalid",
        ))
    } else {
        Err(WslFailure::new(
            WslFailureCategory::GuestUnavailable,
            "wsl.read_failed",
        ))
    }
}

fn codex_version_probe_args(name: &str) -> [&str; 6] {
    [
        "--distribution",
        name,
        "--exec",
        "/bin/bash",
        "-lic",
        CODEX_VERSION_PROBE_SCRIPT,
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexVersionProbe {
    Supported,
    NotFound,
    TooOld,
    Unrecognized,
}

fn classify_codex_version_probe(output: &str) -> CodexVersionProbe {
    let version_at = output.rfind(CODEX_VERSION_PROBE_PREFIX);
    let not_found_at = output.rfind(CODEX_NOT_FOUND_PROBE_RESULT);
    if not_found_at.is_some_and(|index| version_at.is_none_or(|version| index > version)) {
        return CodexVersionProbe::NotFound;
    }
    let Some(version_at) = version_at else {
        return CodexVersionProbe::Unrecognized;
    };
    let version_output = output[version_at + CODEX_VERSION_PROBE_PREFIX.len()..]
        .lines()
        .next()
        .unwrap_or_default();
    let Some(version) = parse_codex_version(version_output) else {
        return CodexVersionProbe::Unrecognized;
    };
    if version >= (0, 147, 0) {
        CodexVersionProbe::Supported
    } else {
        CodexVersionProbe::TooOld
    }
}

fn parse_codex_version(output: &str) -> Option<(u64, u64, u64)> {
    output.split_whitespace().find_map(|token| {
        let token = token.strip_prefix('v').unwrap_or(token);
        let parts = token
            .split('.')
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>();
        match parts.as_deref() {
            Ok([major, minor, patch]) => Some((*major, *minor, *patch)),
            _ => None,
        }
    })
}

#[cfg(not(windows))]
fn read_guest_file(
    _program: &OsStr,
    _name: &str,
    _relative: &str,
) -> Result<Option<Vec<u8>>, WslFailure> {
    Err(WslFailure::new(
        WslFailureCategory::UnsupportedPlatform,
        "wsl.unsupported_platform",
    ))
}

#[cfg(not(windows))]
fn read_guest_private_file(
    _program: &OsStr,
    _name: &str,
    _relative: &str,
) -> Result<Option<Vec<u8>>, WslFailure> {
    Err(WslFailure::new(
        WslFailureCategory::UnsupportedPlatform,
        "wsl.unsupported_platform",
    ))
}

#[cfg(any(windows, test))]
#[derive(Debug)]
struct RegistryDistro {
    id: String,
    name: String,
    default_uid: Option<u32>,
    version: Option<u32>,
    base_path_available: bool,
}

#[cfg(any(windows, test))]
fn probes_from_registry(
    registry: Vec<RegistryDistro>,
    listed_names: &HashSet<String>,
    running_names: &HashSet<String>,
) -> Vec<WslProbe> {
    let counts = registry
        .iter()
        .filter(|item| item.base_path_available)
        .fold(
            std::collections::HashMap::<String, usize>::new(),
            |mut counts, item| {
                *counts.entry(item.name.to_ascii_lowercase()).or_default() += 1;
                counts
            },
        );
    registry
        .into_iter()
        .map(|item| {
            let normalized_name = item.name.to_ascii_lowercase();
            let infrastructure = matches!(
                normalized_name.as_str(),
                "docker-desktop" | "docker-desktop-data"
            );
            let ambiguous = counts.get(&normalized_name).copied().unwrap_or_default() != 1;
            let (availability, message_id, command_name) = if infrastructure {
                (
                    WslAvailability::Infrastructure,
                    Some("wsl.infrastructure_distribution"),
                    None,
                )
            } else if !item.base_path_available {
                (
                    WslAvailability::Unavailable,
                    Some("wsl.environment_unavailable"),
                    None,
                )
            } else if item.version != Some(2) {
                (
                    WslAvailability::UnsupportedVersion,
                    Some("wsl.wsl2_required"),
                    None,
                )
            } else if ambiguous || !listed_names.contains(&normalized_name) {
                (
                    WslAvailability::Ambiguous,
                    Some("wsl.environment_ambiguous"),
                    None,
                )
            } else {
                (WslAvailability::Manageable, None, Some(item.name.clone()))
            };
            WslProbe {
                environment_id: item.id,
                display_name: item.name,
                command_name,
                default_uid: item.default_uid,
                wsl_version: item.version,
                running: item.base_path_available && running_names.contains(&normalized_name),
                availability,
                message_id,
            }
        })
        .collect()
}

#[cfg(windows)]
fn read_registry_distributions() -> Result<Vec<RegistryDistro>, WslFailure> {
    use std::io::ErrorKind;
    use std::path::Path;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let lxss = match current_user.open_subkey_with_flags(WSL_REGISTRY_KEY, KEY_READ) {
        Ok(lxss) => lxss,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => {
            return Err(WslFailure::new(
                WslFailureCategory::ProbeFailed,
                "wsl.registry_unavailable",
            ));
        }
    };
    let mut result = Vec::new();
    for id in lxss.enum_keys() {
        let id = id.map_err(|_| {
            WslFailure::new(WslFailureCategory::ProbeFailed, "wsl.registry_unavailable")
        })?;
        let distribution = lxss.open_subkey_with_flags(&id, KEY_READ).map_err(|_| {
            WslFailure::new(WslFailureCategory::ProbeFailed, "wsl.registry_unavailable")
        })?;
        let name = distribution
            .get_value::<String, _>("DistributionName")
            .map_err(|_| {
                WslFailure::new(WslFailureCategory::ProbeFailed, "wsl.registry_unavailable")
            })?;
        let base_path_available = distribution
            .get_value::<String, _>("BasePath")
            .ok()
            .is_some_and(|base_path| Path::new(&base_path).is_dir());
        result.push(RegistryDistro {
            id,
            name,
            default_uid: distribution.get_value::<u32, _>("DefaultUid").ok(),
            version: distribution.get_value::<u32, _>("Version").ok(),
            base_path_available,
        });
    }
    Ok(result)
}

#[cfg(windows)]
fn decode_wsl_output(output: &Output) -> Result<String, WslFailure> {
    let bytes = &output.stdout;
    let utf16 = bytes.len() >= 2
        && ((bytes[0] == 0xff && bytes[1] == 0xfe)
            || bytes
                .iter()
                .skip(1)
                .step_by(2)
                .filter(|byte| **byte == 0)
                .count()
                > bytes.len() / 8);
    if utf16 {
        let start = if bytes.starts_with(&[0xff, 0xfe]) {
            2
        } else {
            0
        };
        let units = bytes[start..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        Ok(String::from_utf16_lossy(&units).replace('\0', ""))
    } else {
        Ok(String::from_utf8_lossy(bytes).replace('\0', ""))
    }
}

#[cfg(not(windows))]
fn decode_wsl_output(_output: &Output) -> Result<String, WslFailure> {
    Err(WslFailure::new(
        WslFailureCategory::UnsupportedPlatform,
        "wsl.unsupported_platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use tempfile::TempDir;

    #[test]
    fn codex_version_parser_ignores_login_noise_outside_the_probe_result() {
        let output = concat!(
            "shell startup: codex-cli 999.0.0\n",
            "__GPTEASY_CODEX_VERSION__:codex-cli 0.146.9\n",
        );

        assert_eq!(
            classify_codex_version_probe(output),
            CodexVersionProbe::TooOld,
        );
    }

    #[test]
    fn codex_version_probe_uses_login_bash_and_the_real_executable_path() {
        let args = codex_version_probe_args("Ubuntu");

        assert_eq!(
            &args[..5],
            ["--distribution", "Ubuntu", "--exec", "/bin/bash", "-lic"]
        );
        assert!(args[5].contains("type -P codex"));
        assert!(args[5].contains(CODEX_VERSION_PROBE_PREFIX));
    }

    #[test]
    fn codex_version_probe_distinguishes_supported_old_missing_and_unknown_results() {
        assert_eq!(
            classify_codex_version_probe(concat!(
                "shell startup output\n",
                "__GPTEASY_CODEX_VERSION__:codex-cli 0.147.0\n",
            )),
            CodexVersionProbe::Supported,
        );
        assert_eq!(
            classify_codex_version_probe("__GPTEASY_CODEX_VERSION__:codex-cli 0.146.9\n"),
            CodexVersionProbe::TooOld,
        );
        assert_eq!(
            classify_codex_version_probe("__GPTEASY_CODEX_NOT_FOUND__\n"),
            CodexVersionProbe::NotFound,
        );
        assert_eq!(
            classify_codex_version_probe("codex-cli version unknown\n"),
            CodexVersionProbe::Unrecognized,
        );
    }

    struct FakeRuntime {
        probes: Mutex<Vec<WslProbe>>,
        artifacts: Mutex<WslArtifacts>,
        read_failure: Mutex<Option<WslFailure>>,
        starts: AtomicUsize,
        terminations: AtomicUsize,
        writes: AtomicUsize,
        reads: AtomicUsize,
        lock_acquisitions: AtomicUsize,
        lock_releases: AtomicUsize,
        lock_busy: AtomicBool,
        fail_lock_release: AtomicBool,
        active_locks: Mutex<Vec<(String, String, String)>>,
        state_database: Mutex<Option<PathBuf>>,
        released_after_state_commit: AtomicBool,
        fail_probe_after_start: AtomicBool,
        fail_codex_version: AtomicBool,
        probe_calls: AtomicUsize,
        stop_on_probe_call: AtomicUsize,
        natural_stop_on_wait: AtomicBool,
        lifecycle_waits: AtomicUsize,
        waited_after_lock_release: AtomicBool,
        credential_cleanups: AtomicUsize,
    }

    impl FakeRuntime {
        fn new(probe: WslProbe, artifacts: WslArtifacts) -> Self {
            Self {
                probes: Mutex::new(vec![probe]),
                artifacts: Mutex::new(artifacts),
                read_failure: Mutex::new(None),
                starts: AtomicUsize::new(0),
                terminations: AtomicUsize::new(0),
                writes: AtomicUsize::new(0),
                reads: AtomicUsize::new(0),
                lock_acquisitions: AtomicUsize::new(0),
                lock_releases: AtomicUsize::new(0),
                lock_busy: AtomicBool::new(false),
                fail_lock_release: AtomicBool::new(false),
                active_locks: Mutex::new(Vec::new()),
                state_database: Mutex::new(None),
                released_after_state_commit: AtomicBool::new(false),
                fail_probe_after_start: AtomicBool::new(false),
                fail_codex_version: AtomicBool::new(false),
                probe_calls: AtomicUsize::new(0),
                stop_on_probe_call: AtomicUsize::new(usize::MAX),
                natural_stop_on_wait: AtomicBool::new(true),
                lifecycle_waits: AtomicUsize::new(0),
                waited_after_lock_release: AtomicBool::new(false),
                credential_cleanups: AtomicUsize::new(0),
            }
        }
    }

    impl WslRuntime for FakeRuntime {
        fn probe(&self) -> Result<Vec<WslProbe>, WslFailure> {
            let call = self.probe_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_probe_after_start.load(Ordering::SeqCst)
                && self.starts.load(Ordering::SeqCst) > 0
            {
                return Err(WslFailure::new(
                    WslFailureCategory::ProbeFailed,
                    "wsl.environment_unavailable",
                ));
            }
            let mut probes = self.probes.lock().expect("probes");
            if self.stop_on_probe_call.load(Ordering::SeqCst) == call {
                probes[0].running = false;
            }
            Ok(probes.clone())
        }

        fn start(&self, environment: &WslProbe) -> Result<(), WslFailure> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            if let Some(probe) = self
                .probes
                .lock()
                .expect("probes")
                .iter_mut()
                .find(|probe| probe.environment_id == environment.environment_id)
            {
                probe.running = true;
            }
            Ok(())
        }

        fn wait_for_natural_stop(
            &self,
            environment: &WslProbe,
            _timeout: Duration,
        ) -> Result<bool, WslFailure> {
            self.lifecycle_waits.fetch_add(1, Ordering::SeqCst);
            self.waited_after_lock_release.store(
                self.active_locks.lock().expect("active locks").is_empty(),
                Ordering::SeqCst,
            );
            let stopped = self.natural_stop_on_wait.load(Ordering::SeqCst);
            if stopped
                && let Some(probe) = self
                    .probes
                    .lock()
                    .expect("probes")
                    .iter_mut()
                    .find(|probe| probe.environment_id == environment.environment_id)
            {
                probe.running = false;
            }
            Ok(stopped)
        }

        fn acquire_lock(
            &self,
            environment: &WslProbe,
            token: &str,
            _operation: &str,
        ) -> Result<(), WslFailure> {
            self.lock_acquisitions.fetch_add(1, Ordering::SeqCst);
            if self.lock_busy.load(Ordering::SeqCst) {
                return Err(WslFailure::new(WslFailureCategory::Busy, "wsl.lock_busy"));
            }
            let mut active = self.active_locks.lock().expect("active locks");
            if active
                .iter()
                .any(|(environment_id, _, _)| environment_id == &environment.environment_id)
            {
                return Err(WslFailure::new(WslFailureCategory::Busy, "wsl.lock_busy"));
            }
            active.push((
                environment.environment_id.clone(),
                "desktop".to_owned(),
                token.to_owned(),
            ));
            Ok(())
        }

        fn release_lock(&self, environment: &WslProbe, token: &str) -> Result<(), WslFailure> {
            self.lock_releases.fetch_add(1, Ordering::SeqCst);
            if self.fail_lock_release.load(Ordering::SeqCst) {
                return Err(WslFailure::new(
                    WslFailureCategory::RecoveryPending,
                    "wsl.lock_recovery_required",
                ));
            }
            let mut active = self.active_locks.lock().expect("active locks");
            match active
                .iter()
                .position(|(environment_id, _, _)| environment_id == &environment.environment_id)
            {
                None => {}
                Some(index)
                    if active[index].1 == "desktop" && active[index].2.as_str() == token =>
                {
                    active.remove(index);
                }
                Some(_) => {
                    return Err(WslFailure::new(
                        WslFailureCategory::RecoveryPending,
                        "wsl.lock_recovery_required",
                    ));
                }
            }
            if let Some(path) = self.state_database.lock().expect("state database").as_ref() {
                let committed = Connection::open(path)
                    .and_then(|connection| {
                        connection.query_row(
                            "SELECT configuration_state FROM wsl_environments WHERE environment_id = ?1",
                            [environment.environment_id.as_str()],
                            |row| row.get::<_, String>(0),
                        )
                    })
                    .is_ok_and(|state| state == "current");
                self.released_after_state_commit
                    .store(committed, Ordering::SeqCst);
            }
            Ok(())
        }

        fn read_artifacts(&self, _environment: &WslProbe) -> Result<WslArtifacts, WslFailure> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if let Some(failure) = self.read_failure.lock().expect("read failure").clone() {
                return Err(failure);
            }
            Ok(self.artifacts.lock().expect("artifacts").clone())
        }

        fn check_codex_version(&self, _environment: &WslProbe) -> Result<(), WslFailure> {
            if self.fail_codex_version.load(Ordering::SeqCst) {
                Err(WslFailure::new(
                    WslFailureCategory::GuestUnavailable,
                    "wsl.codex_version_required",
                ))
            } else {
                Ok(())
            }
        }

        fn ensure_helper(&self, _environment: &WslProbe) -> Result<(), WslFailure> {
            Ok(())
        }

        fn cleanup_credentials(
            &self,
            _environment: &WslProbe,
            _lock_token: &str,
        ) -> Result<(), WslFailure> {
            self.credential_cleanups.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn write_bundle(
            &self,
            _environment: &WslProbe,
            _lock_token: &str,
            _old_config_hash: &str,
            bundle: &[u8],
        ) -> Result<String, WslFailure> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            let (config, credentials) = decode_test_bundle(bundle);
            *self.artifacts.lock().expect("artifacts") = WslArtifacts {
                config: Some(config),
                credentials: Some(credentials),
            };
            Ok(format!(
                "{{\"status\":\"written\",\"helper\":\"{HELPER_VERSION}\"}}"
            ))
        }
    }

    fn decode_test_bundle(bundle: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut newlines = bundle
            .iter()
            .enumerate()
            .filter_map(|(index, byte)| (*byte == b'\n').then_some(index));
        let first = newlines.next().expect("magic line");
        let second = newlines.next().expect("config length line");
        let third = newlines.next().expect("credentials length line");
        assert_eq!(&bundle[..first], BUNDLE_MAGIC.as_bytes());
        let config_length = std::str::from_utf8(&bundle[first + 1..second])
            .expect("config length utf8")
            .parse::<usize>()
            .expect("config length");
        let credentials_length = std::str::from_utf8(&bundle[second + 1..third])
            .expect("credentials length utf8")
            .parse::<usize>()
            .expect("credentials length");
        let config_end = third + 1 + config_length;
        let credentials_end = config_end + credentials_length;
        (
            bundle[third + 1..config_end].to_vec(),
            bundle[config_end..credentials_end].to_vec(),
        )
    }

    fn probe() -> WslProbe {
        WslProbe {
            environment_id: "{11111111-1111-1111-1111-111111111111}".into(),
            display_name: "Ubuntu".into(),
            command_name: Some("Ubuntu".into()),
            default_uid: Some(1000),
            wsl_version: Some(2),
            running: false,
            availability: WslAvailability::Manageable,
            message_id: None,
        }
    }

    fn application(runtime: Arc<FakeRuntime>) -> (TempDir, StateStore, WslApplication) {
        let temp = TempDir::new().expect("temp");
        let store = StateStore::new(crate::state::StatePaths::from_root(temp.path()));
        assert!(store.bootstrap().is_ready());
        let connection = Connection::open(store.paths().database()).expect("state database");
        connection
            .execute(
                "INSERT INTO providers(
                    id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint, sort_order
                 ) VALUES (?1, 'Example', 'https://provider.example/v1', 'secret',
                    'model-a', '1', 'fingerprint', 0)",
                ["22222222-2222-4222-8222-222222222222"],
            )
            .expect("provider");
        let application = WslApplication::with_runtime(store.clone(), runtime);
        (temp, store, application)
    }

    fn provider(id: &str) -> WslProvider {
        WslProvider {
            id: id.to_owned(),
            name: "Example".to_owned(),
            base_url: "https://provider.example/v1".to_owned(),
            api_key: "secret".to_owned(),
            default_model: "model-a".to_owned(),
            verified_at: 1,
            recommendation_id: None,
        }
    }

    fn schema_v1_config(provider_id: &str, source_id: &str) -> Vec<u8> {
        format!(
            "# >>> GPTEasy managed provider >>>\n\
# GPTEasy schema-version: 1\n\
# GPTEasy provider-id: {provider_id}\n\
# GPTEasy source-id: {source_id}\n\
# GPTEasy credential-file: .gpteasy-shell/credentials/{source_id}/{provider_id}.token\n\
model = \"model-a\"\n\
model_provider = \"gpteasy\"\n\
model_providers.gpteasy.name = \"Example\"\n\
model_providers.gpteasy.base_url = \"https://provider.example/v1\"\n\
model_providers.gpteasy.wire_api = \"responses\"\n\
model_providers.gpteasy.supports_websockets = false\n\
model_providers.gpteasy.auth.command = \"sh\"\n\
 model_providers.gpteasy.auth.args = [\"-c\", 'cat -- \"${{CODEX_HOME:-$HOME/.codex}}/.gpteasy-shell/credentials/{source_id}/{provider_id}.token\"']\n\
# <<< GPTEasy managed provider <<<\n"
        )
        .into_bytes()
    }

    #[test]
    fn running_refresh_uses_the_actual_schema_v1_provider_instead_of_sqlite_history() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, _store, application) = application(runtime);

        let refreshed = application.list().expect("refresh running WSL").remove(0);

        assert_eq!(
            refreshed
                .current_provider
                .as_ref()
                .map(|provider| provider.id.as_str()),
            Some(provider_id)
        );
    }

    #[test]
    fn desktop_apply_writes_the_schema_v1_command_credential_protocol() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(b"custom = true\n".to_vec()),
                credentials: None,
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        let environment = application.list().expect("list").remove(0);

        application
            .apply_provider(
                &environment.environment_id,
                provider_id,
                &environment.revision,
                true,
            )
            .expect("apply provider");

        let artifacts = runtime.artifacts.lock().expect("artifacts").clone();
        assert_eq!(artifacts.credentials.as_deref(), Some(b"secret".as_slice()));
        assert!(
            !String::from_utf8_lossy(artifacts.config.as_deref().expect("written config"))
                .contains("requires_openai_auth")
        );
        assert!(matches!(
            inspect_actual_managed_state(
                artifacts.config.as_deref(),
                artifacts.credentials.as_deref()
            ),
            ActualManagedState::Current { provider_id: actual, .. } if actual == provider_id
        ));
    }

    #[test]
    fn running_refresh_holds_the_desktop_lock_through_sqlite_commit() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, store, application) = application(runtime.clone());
        *runtime.state_database.lock().expect("state database") =
            Some(store.paths().database().to_path_buf());

        application.list().expect("refresh running WSL");

        assert_eq!(runtime.lock_acquisitions.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.lock_releases.load(Ordering::SeqCst), 1);
        assert!(runtime.released_after_state_commit.load(Ordering::SeqCst));
    }

    #[test]
    fn running_refresh_reports_lock_competition_as_a_redacted_busy_state() {
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: None,
                credentials: None,
            },
        ));
        runtime.lock_busy.store(true, Ordering::SeqCst);
        let (_temp, _store, application) = application(runtime.clone());

        let refreshed = application
            .list()
            .expect("busy is an environment state")
            .remove(0);

        assert_eq!(refreshed.configuration_state, WslConfigurationState::Busy);
        assert_eq!(refreshed.message_id.as_deref(), Some("wsl.lock_busy"));
        assert_eq!(runtime.lock_releases.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn running_refresh_recovers_its_persisted_desktop_lock_after_interruption() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, store, _) = application(runtime.clone());
        let interrupted = WslApplication::with_dependencies(
            store.clone(),
            runtime.clone(),
            Arc::new(InterruptAt(WslFailurePoint::AfterRefreshLockAcquired)),
        );

        interrupted
            .list()
            .expect("surface interrupted refresh state");

        assert!(matches!(
            runtime.active_locks.lock().expect("active locks").as_slice(),
            [(_, owner, token)] if owner == "desktop" && !token.is_empty()
        ));
        let connection = Connection::open(store.paths().database()).expect("state database");
        let persisted_token = connection
            .query_row(
                "SELECT refresh_lock_token FROM wsl_environments WHERE environment_id = ?1",
                [probe().environment_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .expect("refresh lock token");
        assert!(persisted_token.is_some());
        drop(connection);

        let recovered = WslApplication::with_runtime(store.clone(), runtime.clone());
        let environment = recovered.list().expect("recover refresh lock").remove(0);

        assert_eq!(
            environment.configuration_state,
            WslConfigurationState::Current
        );
        assert!(
            runtime
                .active_locks
                .lock()
                .expect("active locks")
                .is_empty()
        );
        let connection = Connection::open(store.paths().database()).expect("state database");
        let persisted_token = connection
            .query_row(
                "SELECT refresh_lock_token FROM wsl_environments WHERE environment_id = ?1",
                [environment.environment_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .expect("refresh lock token");
        assert!(persisted_token.is_none());
    }

    #[test]
    fn desktop_apply_holds_the_guest_lock_through_reread_and_sqlite_commit() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(b"custom = true\n".to_vec()),
                credentials: None,
            },
        ));
        let (_temp, store, application) = application(runtime.clone());
        *runtime.state_database.lock().expect("state database") =
            Some(store.paths().database().to_path_buf());
        let environment = application.list().expect("list").remove(0);

        application
            .apply_provider(
                &environment.environment_id,
                provider_id,
                &environment.revision,
                true,
            )
            .expect("apply provider");

        assert_eq!(runtime.lock_acquisitions.load(Ordering::SeqCst), 2);
        assert_eq!(runtime.lock_releases.load(Ordering::SeqCst), 2);
        assert_eq!(runtime.reads.load(Ordering::SeqCst), 3);
        assert!(runtime.released_after_state_commit.load(Ordering::SeqCst));
    }

    struct InterruptAt(WslFailurePoint);

    impl WslFaultInjector for InterruptAt {
        fn check(&self, point: WslFailurePoint) -> Result<(), WslFailure> {
            if point == self.0 {
                Err(WslFailure::new(
                    WslFailureCategory::Interrupted,
                    "wsl.test_interruption",
                ))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn every_desktop_saga_stage_recovers_state_and_its_own_guest_lock() {
        let points = [
            WslFailurePoint::AfterPendingRegistered,
            WslFailurePoint::AfterLockAcquired,
            WslFailurePoint::AfterPrepared,
            WslFailurePoint::AfterArtifactsReplaced,
            WslFailurePoint::AfterStateCommitted,
        ];
        for point in points {
            let provider_id = "22222222-2222-4222-8222-222222222222";
            let mut running_probe = probe();
            running_probe.running = true;
            let runtime = Arc::new(FakeRuntime::new(
                running_probe,
                WslArtifacts {
                    config: Some(schema_v1_config(provider_id, "shell-export")),
                    credentials: Some(b"secret".to_vec()),
                },
            ));
            let (temp, store, _) = application(runtime.clone());
            let application = WslApplication::with_dependencies(
                store.clone(),
                runtime.clone(),
                Arc::new(InterruptAt(point)),
            );
            let environment = application.list().expect("list").remove(0);

            application
                .apply_provider(
                    &environment.environment_id,
                    provider_id,
                    &environment.revision,
                    true,
                )
                .expect_err("simulate process interruption");

            let recovery = WslApplication::with_runtime(store.clone(), runtime.clone());
            recovery
                .recover_pending()
                .expect("recover pending WSL saga");
            assert!(
                runtime
                    .active_locks
                    .lock()
                    .expect("active locks")
                    .is_empty()
            );
            let connection = Connection::open(store.paths().database()).expect("state database");
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM wsl_pending_operation", [], |row| row
                        .get::<_, i64>(
                        0
                    ))
                    .expect("pending count"),
                0,
                "failure point: {point:?}"
            );
            drop(connection);
            drop(temp);
        }
    }

    #[test]
    fn desktop_recovery_never_releases_an_active_shell_owner_lock() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, store, application) = application(runtime.clone());
        let environment = application.list().expect("list").remove(0);
        let connection = Connection::open(store.paths().database()).expect("state database");
        connection
            .execute(
                "INSERT INTO wsl_pending_operation(
                    environment_id, operation_id, stage, target_provider_id,
                    originally_running, expected_default_uid, expected_revision,
                    started_at, lock_token
                 ) VALUES (?1, 'operation', 'locked', ?2, 1, 1000, ?3, '1', 'desktop-token')",
                params![
                    environment.environment_id,
                    provider_id,
                    environment.revision
                ],
            )
            .expect("pending operation");
        drop(connection);
        runtime.active_locks.lock().expect("active locks").push((
            environment.environment_id.clone(),
            "shell".to_owned(),
            "shell-token".to_owned(),
        ));

        let failure = application
            .recover_pending()
            .expect_err("shell owner lock must remain authoritative");

        assert_eq!(failure.message_id, "wsl.lock_recovery_required");
        assert_eq!(
            runtime
                .active_locks
                .lock()
                .expect("active locks")
                .as_slice(),
            &[(
                environment.environment_id.clone(),
                "shell".to_owned(),
                "shell-token".to_owned(),
            )]
        );
        let connection = Connection::open(store.paths().database()).expect("state database");
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM wsl_pending_operation", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .expect("pending count"),
            1
        );
    }

    #[test]
    fn shell_changes_refresh_updated_legacy_missing_and_conflict_states() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        let current = application.list().expect("current").remove(0);
        assert_eq!(current.configuration_state, WslConfigurationState::Current);
        assert!(!current.pending_restart);

        let updated_config = String::from_utf8(schema_v1_config(provider_id, "shell-export"))
            .expect("utf8")
            .replace("model-a", "model-from-old-snapshot")
            .into_bytes();
        *runtime.artifacts.lock().expect("artifacts") = WslArtifacts {
            config: Some(updated_config),
            credentials: Some(b"old-secret".to_vec()),
        };
        let updated = application.list().expect("updated").remove(0);
        assert_eq!(updated.configuration_state, WslConfigurationState::Updated);
        assert!(updated.pending_restart);

        *runtime.artifacts.lock().expect("artifacts") = WslArtifacts {
            config: Some(
                format!(
                    "# >>> GPTEasy managed provider >>>\n\
# GPTEasy provider-id: {provider_id}\n\
model = \"model-a\"\n\
model_provider = \"{provider_id}\"\n\
model_providers.{provider_id}.name = \"Example\"\n\
model_providers.{provider_id}.base_url = \"https://provider.example/v1\"\n\
model_providers.{provider_id}.wire_api = \"responses\"\n\
model_providers.{provider_id}.requires_openai_auth = true\n\
# <<< GPTEasy managed provider <<<\n"
                )
                .into_bytes(),
            ),
            credentials: None,
        };
        let legacy = application.list().expect("legacy").remove(0);
        assert_eq!(legacy.configuration_state, WslConfigurationState::Legacy);

        let missing_id = "33333333-3333-4333-8333-333333333333";
        *runtime.artifacts.lock().expect("artifacts") = WslArtifacts {
            config: Some(schema_v1_config(missing_id, "old-export")),
            credentials: Some(b"retired-secret".to_vec()),
        };
        let missing = application.list().expect("missing provider").remove(0);
        assert_eq!(
            missing.configuration_state,
            WslConfigurationState::ProviderMissing
        );
        assert_eq!(missing.actual_provider_id.as_deref(), Some(missing_id));
        assert!(missing.current_provider.is_none());

        let conflict_config = String::from_utf8(schema_v1_config(provider_id, "shell-export"))
            .expect("utf8")
            .replace("# GPTEasy schema-version: 1", "# GPTEasy schema-version: 9")
            .into_bytes();
        *runtime.artifacts.lock().expect("artifacts") = WslArtifacts {
            config: Some(conflict_config),
            credentials: Some(b"secret".to_vec()),
        };
        let conflict = application.list().expect("conflict").remove(0);
        assert_eq!(
            conflict.configuration_state,
            WslConfigurationState::Conflict
        );
        assert!(conflict.requires_attention);
    }

    #[test]
    fn invalid_schema_v1_credential_reference_is_a_management_conflict() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let other_id = "33333333-3333-4333-8333-333333333333";
        let config = String::from_utf8(schema_v1_config(provider_id, "shell-export"))
            .expect("utf8")
            .replace(
                &format!(
                    "# GPTEasy credential-file: .gpteasy-shell/credentials/shell-export/{provider_id}.token"
                ),
                &format!(
                    "# GPTEasy credential-file: .gpteasy-shell/credentials/shell-export/{other_id}.token"
                ),
            )
            .into_bytes();

        assert!(matches!(
            inspect_actual_managed_state(Some(&config), Some(b"secret")),
            ActualManagedState::Conflict
        ));
    }

    #[test]
    fn schema_v1_with_an_unknown_managed_line_is_a_management_conflict() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        for unknown_line in [
            "# GPTEasy unexpected-metadata: value",
            "model_providers.gpteasy.unexpected = true",
        ] {
            let config = String::from_utf8(schema_v1_config(provider_id, "shell-export"))
                .expect("utf8")
                .replace(
                    "# <<< GPTEasy managed provider <<<",
                    &format!("{unknown_line}\n# <<< GPTEasy managed provider <<<"),
                )
                .into_bytes();

            assert!(
                matches!(
                    inspect_actual_managed_state(Some(&config), Some(b"secret")),
                    ActualManagedState::Conflict
                ),
                "unknown schema v1 line must conflict: {unknown_line}",
            );
        }
    }

    #[test]
    fn unsafe_private_credential_refresh_is_a_management_conflict() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        *runtime.read_failure.lock().expect("read failure") = Some(WslFailure::new(
            WslFailureCategory::NeedsAttention,
            "wsl.credentials_invalid",
        ));
        let (_temp, _store, application) = application(runtime);

        let environment = application.list().expect("list conflict").remove(0);

        assert_eq!(
            environment.configuration_state,
            WslConfigurationState::Conflict
        );
        assert!(environment.requires_attention);
        assert_eq!(
            environment.message_id.as_deref(),
            Some("wsl.credentials_invalid")
        );
    }

    #[test]
    fn desktop_apply_checks_the_guest_codex_version_before_locking_or_writing() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, store, application) = application(runtime.clone());
        let environment = application.list().expect("list").remove(0);
        let lock_count_before = runtime.lock_acquisitions.load(Ordering::SeqCst);
        runtime.fail_codex_version.store(true, Ordering::SeqCst);

        let failure = application
            .apply_provider(
                &environment.environment_id,
                provider_id,
                &environment.revision,
                true,
            )
            .expect_err("unsupported Codex version must stop the write");

        assert_eq!(failure.message_id, "wsl.codex_version_required");
        assert_eq!(
            runtime.lock_acquisitions.load(Ordering::SeqCst),
            lock_count_before
        );
        assert_eq!(runtime.writes.load(Ordering::SeqCst), 0);
        let connection = Connection::open(store.paths().database()).expect("state database");
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM wsl_pending_operation", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .expect("pending count"),
            0
        );
    }

    #[test]
    fn desktop_apply_never_overwrites_an_unknown_managed_schema() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let mut running_probe = probe();
        running_probe.running = true;
        let unknown_schema = String::from_utf8(schema_v1_config(provider_id, "shell-export"))
            .expect("utf8")
            .replace("# GPTEasy schema-version: 1", "# GPTEasy schema-version: 9")
            .into_bytes();
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(unknown_schema.clone()),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        let environment = application.list().expect("list conflict").remove(0);

        let failure = application
            .apply_provider(
                &environment.environment_id,
                provider_id,
                &environment.revision,
                true,
            )
            .expect_err("unknown schema must not be overwritten");

        assert_eq!(failure.message_id, "wsl.managed_conflict");
        assert_eq!(runtime.writes.load(Ordering::SeqCst), 0);
        assert_eq!(
            runtime
                .artifacts
                .lock()
                .expect("artifacts")
                .config
                .as_deref(),
            Some(unknown_schema.as_slice())
        );
    }

    #[test]
    fn bundle_has_unambiguous_lengths_and_no_secret_in_header() {
        let bundle = bundle_bytes(b"config\n", br#"{"OPENAI_API_KEY":"secret"}"#);
        let header_end = bundle.iter().position(|byte| *byte == b'\n').unwrap() + 1;
        let header_end = bundle[header_end..]
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap()
            + header_end
            + 1;
        let header_end = bundle[header_end..]
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap()
            + header_end
            + 1;
        assert!(!String::from_utf8_lossy(&bundle[..header_end]).contains("secret"));
        assert!(String::from_utf8_lossy(&bundle).contains("GPTEASY_WSL_BUNDLE_V2"));
    }

    #[test]
    fn guest_writer_keeps_five_desktop_backups_and_accepts_crlf_markers() {
        let script = std::str::from_utf8(GUEST_WRITER).expect("guest writer is UTF-8");

        assert!(script.contains("desktop-backups"));
        assert!(script.contains("awk 'NR > 5 { print }'"));
        assert!(!script.contains("NR > 10"));
        assert!(
            script
                .matches("sed 's/\\r$//' \"$incoming_config\"")
                .count()
                >= 2
        );
    }

    #[test]
    fn guest_writer_uses_command_credentials_without_touching_auth_json() {
        let script = std::str::from_utf8(GUEST_WRITER).expect("guest writer is UTF-8");

        assert!(!script.contains("auth.json"));
        assert!(script.contains(".gpteasy-shell/credentials"));
        assert!(script.contains("desktop-backups"));
        assert!(script.contains("lock_value \"$OWNER_FILE\" owner"));
        assert!(script.contains("= desktop"));
    }

    #[test]
    fn guest_writer_removes_a_new_credential_until_config_replacement_succeeds() {
        let script = std::str::from_utf8(GUEST_WRITER).expect("guest writer is UTF-8");

        assert!(script.contains("config_replaced=false"));
        assert!(script.contains(
            "[ \"$credential_created\" = false ] || [ \"$config_replaced\" = true ] || rm -f \"$CREDENTIAL\""
        ));
        assert!(script.contains("config_replaced=true"));
    }

    #[test]
    fn revision_changes_when_default_user_changes() {
        let base = WslProbe {
            environment_id: "id".into(),
            display_name: "Ubuntu".into(),
            command_name: Some("Ubuntu".into()),
            default_uid: Some(1000),
            wsl_version: Some(2),
            running: false,
            availability: WslAvailability::Manageable,
            message_id: None,
        };
        let mut changed = base.clone();
        changed.default_uid = Some(1001);
        assert_ne!(revision_for_probe(&base), revision_for_probe(&changed));
    }

    #[test]
    fn non_wsl2_registration_is_rejected() {
        assert_eq!(
            wsl_availability_failure(WslAvailability::UnsupportedVersion).message_id,
            "wsl.wsl2_required"
        );
    }

    #[cfg(windows)]
    #[test]
    fn no_registered_distribution_does_not_require_a_working_wsl_command() {
        let runtime = SystemWslRuntime {
            program: OsString::from("gpteasy-missing-wsl-command.exe"),
            distribution_filter: Some("GPTEasy missing distribution".to_owned()),
        };

        assert!(
            runtime
                .probe()
                .expect("no WSL registration is an empty inventory")
                .is_empty()
        );
    }

    #[test]
    fn stale_duplicate_registration_does_not_block_the_valid_distribution() {
        let registry = vec![
            RegistryDistro {
                id: "{stale}".into(),
                name: "Ubuntu".into(),
                default_uid: Some(1000),
                version: Some(2),
                base_path_available: false,
            },
            RegistryDistro {
                id: "{valid}".into(),
                name: "Ubuntu".into(),
                default_uid: Some(1000),
                version: Some(2),
                base_path_available: true,
            },
        ];
        let listed_names = HashSet::from(["ubuntu".to_owned()]);
        let running_names = HashSet::from(["ubuntu".to_owned()]);

        let probes = probes_from_registry(registry, &listed_names, &running_names);
        let stale = probes
            .iter()
            .find(|probe| probe.environment_id == "{stale}")
            .unwrap();
        let valid = probes
            .iter()
            .find(|probe| probe.environment_id == "{valid}")
            .unwrap();

        assert_eq!(stale.availability, WslAvailability::Unavailable);
        assert!(stale.command_name.is_none());
        assert!(!stale.running);
        assert_eq!(valid.availability, WslAvailability::Manageable);
        assert_eq!(valid.command_name.as_deref(), Some("Ubuntu"));
        assert!(valid.running);
    }

    #[test]
    fn first_takeover_removes_legacy_selection_and_preserves_unrelated_fields() {
        let original = br#"custom_flag = true
model = "legacy-model"
model_provider = "legacy"

[model_providers.legacy]
name = "Legacy"
base_url = "https://legacy.example/v1"
"#;
        let rendered = render_config(Some(original), &provider("provider-id"), "desktop-test")
            .expect("render");
        let text = String::from_utf8(rendered).expect("utf8");
        let document = text.parse::<toml_edit::DocumentMut>().expect("valid toml");

        assert_eq!(document["custom_flag"].as_bool(), Some(true));
        assert_eq!(document["model"].as_str(), Some("model-a"));
        assert_eq!(document["model_provider"].as_str(), Some("gpteasy"));
        assert_eq!(
            document["model_providers"]["legacy"]["name"].as_str(),
            Some("Legacy")
        );
        assert_eq!(
            text.matches("# >>> GPTEasy managed provider >>>").count(),
            1
        );
    }

    #[test]
    fn damaged_marker_pair_is_rejected_before_credentials_are_rendered() {
        let original = b"# >>> GPTEasy managed provider >>>\nmodel = \"legacy\"\n";
        let failure = render_config(Some(original), &provider("provider-id"), "desktop-test")
            .expect_err("marker damage must stop");
        assert_eq!(failure.message_id, "wsl.managed_conflict");
    }

    #[test]
    fn stopped_distribution_is_started_written_without_forced_termination() {
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: Some(b"custom = true\n".to_vec()),
                credentials: None,
            },
        ));
        let (_temp, store, application) = application(runtime.clone());
        let environment = application.list().expect("list").remove(0);

        let result = application
            .apply_provider(
                &environment.environment_id,
                "22222222-2222-4222-8222-222222222222",
                &environment.revision,
                true,
            )
            .expect("apply");

        assert!(!result.pending_restart);
        assert_eq!(
            result.lifecycle_outcome,
            WslLifecycleOutcome::StoppedNaturally
        );
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.terminations.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.lifecycle_waits.load(Ordering::SeqCst), 1);
        assert!(runtime.waited_after_lock_release.load(Ordering::SeqCst));
        assert_eq!(runtime.writes.load(Ordering::SeqCst), 1);
        assert!(!runtime.probes.lock().expect("probes")[0].running);
        let connection = Connection::open(store.paths().database()).expect("state database");
        assert_eq!(
            connection
                .query_row(
                    "SELECT current_provider_id FROM wsl_environments WHERE environment_id = ?1",
                    [environment.environment_id],
                    |row| row.get::<_, String>(0),
                )
                .expect("current provider"),
            "22222222-2222-4222-8222-222222222222"
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM wsl_pending_operation", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .expect("pending count"),
            0
        );
    }

    #[test]
    fn stopped_distribution_refresh_requires_authorization_and_waits_after_unlocking() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        let observed = application.list().expect("side-effect free list").remove(0);

        assert!(!observed.running);
        assert_eq!(observed.configuration_state, WslConfigurationState::Unknown);
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.reads.load(Ordering::SeqCst), 0);
        let denied = application
            .refresh_environment(&observed.environment_id, &observed.revision, false)
            .expect_err("stopped refresh needs explicit start authorization");
        assert_eq!(denied.message_id, "wsl.start_authorization_required");
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);

        let refreshed = application
            .refresh_environment(&observed.environment_id, &observed.revision, true)
            .expect("authorized actual-state refresh");

        assert_eq!(
            refreshed.lifecycle_outcome,
            WslLifecycleOutcome::StoppedNaturally
        );
        assert_eq!(
            refreshed.environment.configuration_state,
            WslConfigurationState::Current
        );
        assert!(!refreshed.environment.running);
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.reads.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.lock_acquisitions.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.lock_releases.load(Ordering::SeqCst), 1);
        assert!(runtime.waited_after_lock_release.load(Ordering::SeqCst));
    }

    #[test]
    fn provider_deletion_audits_every_environment_and_requires_stopped_authorization() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "shell-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let mut running_probe = probe();
        running_probe.environment_id = "{33333333-3333-3333-3333-333333333333}".to_owned();
        running_probe.display_name = "Debian".to_owned();
        running_probe.command_name = Some("Debian".to_owned());
        running_probe.running = true;
        let mut stale_duplicate = probe();
        stale_duplicate.environment_id = "{44444444-4444-4444-8444-444444444444}".to_owned();
        stale_duplicate.command_name = None;
        stale_duplicate.availability = WslAvailability::Unavailable;
        stale_duplicate.message_id = Some("wsl.environment_unavailable");
        runtime
            .probes
            .lock()
            .expect("probes")
            .extend([running_probe, stale_duplicate]);
        let (_temp, _store, application) = application(runtime.clone());

        let denied = application
            .audit_provider_deletion("not-current-provider", false)
            .expect_err("stopped environment must be verified before deletion");
        assert_eq!(denied.message_id, "wsl.delete_start_authorization_required");
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);

        let audit = application
            .audit_provider_deletion("not-current-provider", true)
            .expect("all environments verified unused");

        assert_eq!(audit.lifecycle_results.len(), 2);
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.credential_cleanups.load(Ordering::SeqCst), 2);
        assert!(audit.lifecycle_results.iter().any(|result| {
            result.display_name == "Ubuntu"
                && result.outcome == WslLifecycleOutcome::StoppedNaturally
        }));
        assert!(audit.lifecycle_results.iter().any(|result| {
            result.display_name == "Debian"
                && result.outcome == WslLifecycleOutcome::UnchangedRunning
        }));
    }

    #[test]
    fn provider_deletion_action_runs_while_the_guest_lock_is_held() {
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: Some(schema_v1_config(
                    "22222222-2222-4222-8222-222222222222",
                    "shell-export",
                )),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        let action_observed_lock = Arc::new(AtomicBool::new(false));
        let observed = action_observed_lock.clone();
        let action_runtime = runtime.clone();

        application
            .audit_provider_deletion_then("not-current-provider", true, move || {
                observed.store(
                    action_runtime
                        .active_locks
                        .lock()
                        .expect("active locks")
                        .iter()
                        .any(|(_, owner, _)| owner == "desktop"),
                    Ordering::SeqCst,
                );
                Ok::<_, ()>(())
            })
            .expect("delete after audit");

        assert!(action_observed_lock.load(Ordering::SeqCst));
        assert!(
            runtime
                .active_locks
                .lock()
                .expect("active locks")
                .is_empty()
        );
        assert!(runtime.waited_after_lock_release.load(Ordering::SeqCst));
    }

    #[test]
    fn provider_deletion_does_not_wait_when_the_guest_lock_release_fails() {
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: None,
                credentials: None,
            },
        ));
        runtime.fail_lock_release.store(true, Ordering::SeqCst);
        let (_temp, _store, application) = application(runtime.clone());

        let failure = application
            .audit_provider_deletion("not-current-provider", true)
            .expect_err("failed guest unlock must fail the audit");

        assert_eq!(failure.message_id, "wsl.lock_recovery_required");
        assert_eq!(runtime.lifecycle_waits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn provider_deletion_failure_keeps_completed_lifecycle_results() {
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: None,
                credentials: None,
            },
        ));
        let (_temp, _store, application) = application(runtime);

        let failure = application
            .audit_provider_deletion_then("not-current-provider", true, || {
                Err::<(), _>("catalog failure")
            })
            .expect_err("catalog deletion fails after WSL audit");

        match failure {
            WslDeletionAuditError::Deletion {
                failure,
                lifecycle_results,
            } => {
                assert_eq!(failure, "catalog failure");
                assert_eq!(lifecycle_results.len(), 1);
                assert_eq!(
                    lifecycle_results[0].outcome,
                    WslLifecycleOutcome::StoppedNaturally
                );
            }
            WslDeletionAuditError::Verification(failure) => {
                panic!("unexpected verification failure: {}", failure.message_id)
            }
        }
    }

    #[test]
    fn provider_deletion_is_blocked_by_stopped_environment_actual_state() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: Some(schema_v1_config(provider_id, "old-export")),
                credentials: Some(b"secret".to_vec()),
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());

        let blocked = application
            .audit_provider_deletion(provider_id, true)
            .expect_err("actual stopped configuration still references provider");

        assert_eq!(blocked.message_id, "provider.wsl_current_delete_forbidden");
        assert_eq!(
            blocked.lifecycle_outcome,
            Some(WslLifecycleOutcome::StoppedNaturally)
        );
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.credential_cleanups.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stopped_distribution_is_not_forced_down_when_the_post_start_probe_fails() {
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: Some(b"custom = true\n".to_vec()),
                credentials: None,
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        let environment = application.list().expect("list").remove(0);
        runtime.fail_probe_after_start.store(true, Ordering::SeqCst);

        let failure = application
            .apply_provider(
                &environment.environment_id,
                "22222222-2222-4222-8222-222222222222",
                &environment.revision,
                true,
            )
            .expect_err("post-start probe fails");

        assert_eq!(failure.message_id, "wsl.environment_unavailable");
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.terminations.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.writes.load(Ordering::SeqCst), 0);
        assert!(!runtime.probes.lock().expect("probes")[0].running);
    }

    #[test]
    fn running_distribution_that_stops_before_writing_is_not_restarted() {
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(b"custom = true\n".to_vec()),
                credentials: None,
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        let environment = application.list().expect("list").remove(0);
        runtime.stop_on_probe_call.store(2, Ordering::SeqCst);

        let failure = application
            .apply_provider(
                &environment.environment_id,
                "22222222-2222-4222-8222-222222222222",
                &environment.revision,
                true,
            )
            .expect_err("stopped distribution must not be reopened by artifact reads");

        assert_eq!(failure.message_id, "wsl.environment_changed");
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.terminations.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.writes.load(Ordering::SeqCst), 0);
        assert!(!runtime.probes.lock().expect("probes")[0].running);
    }

    #[test]
    fn default_user_change_stays_visible_until_explicit_apply() {
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: None,
                credentials: None,
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        application.list().expect("initial list");
        runtime.probes.lock().expect("probes")[0].default_uid = Some(1001);

        let changed = application.list().expect("changed list").remove(0);
        assert_eq!(changed.availability, WslAvailability::DefaultUserChanged);
        let changed_again = application.list().expect("second changed list").remove(0);
        assert_eq!(
            changed_again.availability,
            WslAvailability::DefaultUserChanged
        );
    }

    #[test]
    fn removed_distribution_remains_as_a_read_only_record() {
        let runtime = Arc::new(FakeRuntime::new(
            probe(),
            WslArtifacts {
                config: None,
                credentials: None,
            },
        ));
        let (_temp, _store, application) = application(runtime.clone());
        application.list().expect("initial list");
        runtime.probes.lock().expect("probes").clear();

        let removed = application.list().expect("removed list").remove(0);
        assert_eq!(removed.availability, WslAvailability::Removed);
        assert!(removed.requires_attention);
        assert!(removed.command_name.is_none());
    }

    #[test]
    fn recovery_commits_new_hashes_without_forcing_down_a_temporarily_started_distribution() {
        let provider_id = "22222222-2222-4222-8222-222222222222";
        let target_config = schema_v1_config(provider_id, "desktop-recovery");
        let target_credentials = b"secret".to_vec();
        let mut running_probe = probe();
        running_probe.running = true;
        let runtime = Arc::new(FakeRuntime::new(
            running_probe,
            WslArtifacts {
                config: Some(target_config.clone()),
                credentials: Some(target_credentials.clone()),
            },
        ));
        let (_temp, store, application) = application(runtime.clone());
        let environment = application.list().expect("list").remove(0);
        let connection = Connection::open(store.paths().database()).expect("state database");
        connection
            .execute(
                "INSERT INTO wsl_pending_operation(
                    environment_id, operation_id, stage, target_provider_id,
                    old_config_fingerprint, new_config_fingerprint,
                    old_credentials_fingerprint, new_credentials_fingerprint,
                    originally_running, expected_default_uid, expected_revision, started_at,
                    lock_token
                 ) VALUES (?1, 'operation', 'artifacts_replaced', ?2, ?3, ?4, ?3, ?5, 0, 1000, ?6, '1', 'recovery-token')",
                params![
                    environment.environment_id,
                    provider_id,
                    "missing",
                    hash_bytes(&target_config),
                    hash_bytes(&target_credentials),
                    environment.revision,
                ],
            )
            .expect("pending operation");
        drop(connection);
        runtime.active_locks.lock().expect("active locks").push((
            environment.environment_id.clone(),
            "desktop".to_owned(),
            "recovery-token".to_owned(),
        ));
        runtime.natural_stop_on_wait.store(false, Ordering::SeqCst);

        application.recover_pending().expect("recover");

        assert_eq!(runtime.terminations.load(Ordering::SeqCst), 0);
        assert!(runtime.probes.lock().expect("probes")[0].running);
        let connection = Connection::open(store.paths().database()).expect("state database");
        assert_eq!(
            connection
                .query_row(
                    "SELECT current_provider_id FROM wsl_environments WHERE environment_id = ?1",
                    [environment.environment_id],
                    |row| row.get::<_, String>(0),
                )
                .expect("current provider"),
            "22222222-2222-4222-8222-222222222222"
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM wsl_pending_operation", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .expect("pending count"),
            0
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "reads the real Windows WSL2 registry and running set"]
    fn real_system_probe_does_not_start_or_stop_distributions() {
        let before = decode_wsl_output(
            &run_wsl(&["--list", "--running", "--quiet"], None).expect("running before"),
        )
        .expect("decode before");
        let probes = SystemWslRuntime::default().probe().expect("probe");
        let after = decode_wsl_output(
            &run_wsl(&["--list", "--running", "--quiet"], None).expect("running after"),
        )
        .expect("decode after");

        assert_eq!(before, after);
        assert!(!probes.is_empty());
    }
}
