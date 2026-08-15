use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::provider::ProviderSummary;
use crate::state::StateStore;

const WSL_REGISTRY_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss";
const HELPER_VERSION: &str = "gpteasy-wsl-guest-writer-v1";
const HELPER_PATH: &str = "$HOME/.local/lib/gpteasy/guest-writer-v1";
const BUNDLE_MAGIC: &str = "GPTEASY_WSL_BUNDLE_V1";

const GUEST_WRITER: &[u8] = br#"#!/bin/sh
set -eu

EXPECTED_CONFIG=${1-}
EXPECTED_CREDENTIALS=${2-}
BUNDLE_MAGIC='GPTEASY_WSL_BUNDLE_V1'
TARGET_DIR="$HOME/.codex"
CONFIG="$TARGET_DIR/config.toml"
CREDENTIALS="$TARGET_DIR/auth.json"
BACKUP_DIR="$TARGET_DIR/backups"
umask 077

read -r magic
[ "$magic" = "$BUNDLE_MAGIC" ] || { printf '%s\n' '{"status":"candidate_rejected","reason":"bundle_magic"}'; exit 40; }
read -r config_length
read -r credentials_length
case "$config_length:$credentials_length" in
  *[!0-9:]*|:*) printf '%s\n' '{"status":"candidate_rejected","reason":"bundle_length"}'; exit 40;;
esac

mkdir -p "$TARGET_DIR" "$BACKUP_DIR"
config_tmp=$(mktemp "$TARGET_DIR/.config.gpteasy.XXXXXX")
credentials_tmp=$(mktemp "$TARGET_DIR/.auth.gpteasy.XXXXXX")
config_old=$(mktemp "$TARGET_DIR/.config-old.gpteasy.XXXXXX")
credentials_old=$(mktemp "$TARGET_DIR/.auth-old.gpteasy.XXXXXX")
cleanup() { rm -f "$config_tmp" "$credentials_tmp" "$config_old" "$credentials_old"; }
rollback() {
  if [ "$config_existed" = true ]; then mv "$config_old" "$CONFIG"; else rm -f "$CONFIG"; fi
  if [ "$credentials_existed" = true ]; then mv "$credentials_old" "$CREDENTIALS"; else rm -f "$CREDENTIALS"; fi
}
trap cleanup EXIT HUP INT TERM

dd bs=1 count="$config_length" of="$config_tmp" 2>/dev/null
dd bs=1 count="$credentials_length" of="$credentials_tmp" 2>/dev/null
start_count=$(sed 's/\r$//' "$config_tmp" | grep -c '^# >>> GPTEasy managed provider >>>$' || true)
end_count=$(sed 's/\r$//' "$config_tmp" | grep -c '^# <<< GPTEasy managed provider <<<$' || true)
if [ "$start_count" -ne 1 ] || [ "$end_count" -ne 1 ]; then
  printf '%s\n' '{"status":"candidate_rejected","reason":"managed_marker_count"}'
  exit 40
fi

hash_file() { sha256sum "$1" | awk '{print $1}'; }
hash_missing() { printf '' | sha256sum | awk '{print $1}'; }
current_config=$(if [ -f "$CONFIG" ]; then hash_file "$CONFIG"; else hash_missing; fi)
current_credentials=$(if [ -f "$CREDENTIALS" ]; then hash_file "$CREDENTIALS"; else hash_missing; fi)
[ "$current_config" = "$EXPECTED_CONFIG" ] || { printf '%s\n' '{"status":"concurrent_change","phase":"initial_hash"}'; exit 41; }
[ "$current_credentials" = "$EXPECTED_CREDENTIALS" ] || { printf '%s\n' '{"status":"concurrent_change","phase":"initial_hash"}'; exit 41; }

backup_stamp=$(date -u +%Y%m%dT%H%M%S%N)
backup_config=''
backup_credentials=''
config_existed=false
credentials_existed=false
if [ -f "$CONFIG" ]; then
  config_existed=true
  backup_config="$BACKUP_DIR/config-$backup_stamp-$$.toml"
  cp -p "$CONFIG" "$backup_config"
  chmod 600 "$backup_config"
  cp -p "$CONFIG" "$config_old"
  chmod --reference="$CONFIG" "$config_tmp"
else
  chmod 600 "$config_tmp"
fi
if [ -f "$CREDENTIALS" ]; then
  credentials_existed=true
  backup_credentials="$BACKUP_DIR/auth-$backup_stamp-$$.json"
  cp -p "$CREDENTIALS" "$backup_credentials"
  chmod 600 "$backup_credentials"
  cp -p "$CREDENTIALS" "$credentials_old"
  chmod --reference="$CREDENTIALS" "$credentials_tmp"
else
  chmod 600 "$credentials_tmp"
fi
sync -f "$config_tmp"
sync -f "$credentials_tmp"

current_config=$(if [ -f "$CONFIG" ]; then hash_file "$CONFIG"; else hash_missing; fi)
current_credentials=$(if [ -f "$CREDENTIALS" ]; then hash_file "$CREDENTIALS"; else hash_missing; fi)
if [ "$current_config" != "$EXPECTED_CONFIG" ] || [ "$current_credentials" != "$EXPECTED_CREDENTIALS" ]; then
  printf '%s\n' '{"status":"concurrent_change","phase":"pre_replace"}'
  exit 43
fi

if ! mv "$config_tmp" "$CONFIG"; then
  printf '%s\n' '{"status":"write_failed","phase":"config_replace"}'
  exit 44
fi
if ! mv "$credentials_tmp" "$CREDENTIALS"; then
  rollback
  printf '%s\n' '{"status":"rollback","phase":"credentials_replace"}'
  exit 45
fi
sync -f "$TARGET_DIR"

for pattern in 'config-*.toml' 'auth-*.json'; do
  find "$BACKUP_DIR" -maxdepth 1 -type f -name "$pattern" -printf '%f\n' |
    sort -r | awk 'NR > 5 { print }' | while IFS= read -r stale; do rm -f "$BACKUP_DIR/$stale"; done
done
backup_count=$(find "$BACKUP_DIR" -maxdepth 1 -type f \( -name 'config-*.toml' -o -name 'auth-*.json' \) | wc -l | awk '{print $1}')
printf '{"status":"written","backup_count":%s,"helper":"%s"}\n' "$backup_count" 'gpteasy-wsl-guest-writer-v1'
"#;

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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslFailure {
    pub category: WslFailureCategory,
    pub message_id: &'static str,
}

impl WslFailure {
    pub(crate) fn new(category: WslFailureCategory, message_id: &'static str) -> Self {
        Self {
            category,
            message_id,
        }
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
    fn terminate(&self, environment: &WslProbe) -> Result<(), WslFailure>;
    fn read_artifacts(&self, environment: &WslProbe) -> Result<WslArtifacts, WslFailure>;
    fn ensure_helper(&self, environment: &WslProbe) -> Result<(), WslFailure>;
    fn write_bundle(
        &self,
        environment: &WslProbe,
        old_config_hash: &str,
        old_credentials_hash: &str,
        bundle: &[u8],
    ) -> Result<String, WslFailure>;
}

#[derive(Clone)]
pub struct WslApplication {
    state_store: StateStore,
    operation_lock: Arc<Mutex<()>>,
    runtime: Arc<dyn WslRuntime>,
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
        Self::with_runtime(state_store, Arc::new(SystemWslRuntime))
    }

    #[doc(hidden)]
    pub(crate) fn with_runtime(state_store: StateStore, runtime: Arc<dyn WslRuntime>) -> Self {
        Self {
            state_store,
            operation_lock: Arc::new(Mutex::new(())),
            runtime,
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
        load_summaries(&connection, &probes)
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
                    return Err(self.restore_after_failed_start(&connection, &probe, failure));
                }
            };
            if refreshed.availability != WslAvailability::Manageable {
                let failure = wsl_availability_failure(refreshed.availability);
                return Err(self.restore_after_failed_start(&connection, &refreshed, failure));
            }
            if !same_environment_identity(&refreshed, &pre_start) || !refreshed.running {
                return Err(self.restore_after_failed_start(
                    &connection,
                    &refreshed,
                    WslFailure::new(
                        WslFailureCategory::EnvironmentChanged,
                        "wsl.environment_changed",
                    ),
                ));
            }
            refreshed
        };

        let result = self.apply_started(
            &mut connection,
            &active_probe,
            &provider,
            originally_running,
        );
        if !originally_running {
            let terminate_result = self.runtime.terminate(&active_probe);
            if terminate_result.is_err() {
                let _ = mark_wsl_attention(
                    &connection,
                    &active_probe.environment_id,
                    "wsl.lifecycle_restore_failed",
                );
                return Err(WslFailure::new(
                    WslFailureCategory::GuestWriteFailed,
                    "wsl.lifecycle_restore_failed",
                ));
            }
        }
        result
    }

    fn restore_after_failed_start(
        &self,
        connection: &Connection,
        probe: &WslProbe,
        failure: WslFailure,
    ) -> WslFailure {
        if self.runtime.terminate(probe).is_ok() {
            failure
        } else {
            let _ = mark_wsl_attention(
                connection,
                &probe.environment_id,
                "wsl.lifecycle_restore_failed",
            );
            WslFailure::new(
                WslFailureCategory::GuestWriteFailed,
                "wsl.lifecycle_restore_failed",
            )
        }
    }

    fn apply_started(
        &self,
        connection: &mut Connection,
        probe: &WslProbe,
        provider: &WslProvider,
        originally_running: bool,
    ) -> Result<WslApplyResult, WslFailure> {
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
        if let Some(pending) = load_pending_operation(connection, &current_probe.environment_id)? {
            self.reconcile_pending_for_probe(connection, &pending, &current_probe, false)?;
        }
        let original = self.runtime.read_artifacts(&current_probe)?;
        let config = render_config(original.config.as_deref(), provider)?;
        let credentials = render_credentials(original.credentials.as_deref(), &provider.api_key)?;
        let old_config_hash = hash_optional(original.config.as_deref());
        let old_credentials_hash = hash_optional(original.credentials.as_deref());
        let new_config_hash = hash_bytes(&config);
        let new_credentials_hash = hash_bytes(&credentials);
        let expected_revision = revision_for_probe(&current_probe);
        self.runtime.ensure_helper(&current_probe)?;

        let operation_id = Uuid::new_v4().to_string();
        let old_provider_id = connection
            .query_row(
                "SELECT current_provider_id FROM wsl_environments WHERE environment_id = ?1",
                [current_probe.environment_id.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|_| state_unavailable())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| state_unavailable())?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO wsl_pending_operation(
                    environment_id, operation_id, stage, old_provider_id, target_provider_id, old_config_fingerprint,
                    new_config_fingerprint, old_credentials_fingerprint,
                    new_credentials_fingerprint, originally_running, expected_default_uid,
                    expected_revision, started_at
                 ) VALUES (?1, ?2, 'prepared', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    current_probe.environment_id,
                    operation_id,
                    old_provider_id,
                    provider.id,
                    old_config_hash,
                    new_config_hash,
                    old_credentials_hash,
                    new_credentials_hash,
                    originally_running,
                    current_probe.default_uid,
                    expected_revision,
                    epoch_seconds().to_string(),
                ],
            )
            .map_err(|_| state_unavailable())?;
        transaction.commit().map_err(|_| state_unavailable())?;

        let bundle = bundle_bytes(&config, &credentials);
        let writer_output = match self.runtime.write_bundle(
            &current_probe,
            &old_config_hash,
            &old_credentials_hash,
            &bundle,
        ) {
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
        connection
            .execute(
                "UPDATE wsl_pending_operation SET stage = 'artifacts_replaced' WHERE environment_id = ?1",
                [current_probe.environment_id.as_str()],
            )
            .map_err(|_| state_unavailable())?;
        connection
            .execute(
                "UPDATE wsl_environments
                 SET current_provider_id = ?2, config_fingerprint = ?3,
                     credentials_fingerprint = ?4, pending_restart = ?5,
                     requires_attention = 0, last_error = NULL,
                     availability = 'manageable', updated_at = ?6
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
        connection
            .execute(
                "UPDATE wsl_pending_operation SET stage = 'state_committed' WHERE environment_id = ?1",
                [current_probe.environment_id.as_str()],
            )
            .map_err(|_| state_unavailable())?;
        connection
            .execute(
                "DELETE FROM wsl_pending_operation WHERE environment_id = ?1",
                [current_probe.environment_id.as_str()],
            )
            .map_err(|_| state_unavailable())?;
        let summary = load_summary(connection, &current_probe)?;
        Ok(WslApplyResult {
            environment: summary,
            pending_restart: originally_running,
        })
    }

    fn reconcile_pending_for_probe(
        &self,
        connection: &Connection,
        pending: &PendingWslOperation,
        probe: &WslProbe,
        restore_lifecycle: bool,
    ) -> Result<(), WslFailure> {
        let result = (|| {
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
            let artifacts = self.runtime.read_artifacts(probe)?;
            let config_hash = hash_optional(artifacts.config.as_deref());
            let credentials_hash = hash_optional(artifacts.credentials.as_deref());
            let matches_old = config_hash == pending.old_config_hash
                && credentials_hash == pending.old_credentials_hash;
            let matches_new = config_hash == pending.new_config_hash
                && credentials_hash == pending.new_credentials_hash;
            if !matches_old && !matches_new {
                mark_pending_attention(connection, &probe.environment_id, "wsl.recovery_conflict")?;
                return Err(WslFailure::new(
                    WslFailureCategory::NeedsAttention,
                    "wsl.recovery_conflict",
                ));
            }
            let (provider_id, config_fingerprint, credentials_fingerprint, pending_restart) =
                if matches_new {
                    (
                        Some(pending.target_provider_id.as_str()),
                        pending.new_config_hash.as_str(),
                        pending.new_credentials_hash.as_str(),
                        pending.originally_running,
                    )
                } else {
                    (
                        pending.old_provider_id.as_deref(),
                        pending.old_config_hash.as_str(),
                        pending.old_credentials_hash.as_str(),
                        false,
                    )
                };
            let transaction = connection
                .unchecked_transaction()
                .map_err(|_| state_unavailable())?;
            transaction
                .execute(
                    "UPDATE wsl_environments SET current_provider_id = ?2,
                        config_fingerprint = ?3, credentials_fingerprint = ?4,
                        pending_restart = ?5, requires_attention = 0,
                        availability = 'manageable', last_error = NULL, updated_at = ?6
                     WHERE environment_id = ?1",
                    params![
                        probe.environment_id,
                        provider_id,
                        config_fingerprint,
                        credentials_fingerprint,
                        pending_restart,
                        epoch_seconds().to_string(),
                    ],
                )
                .map_err(|_| state_unavailable())?;
            transaction
                .execute(
                    "DELETE FROM wsl_pending_operation WHERE environment_id = ?1",
                    [probe.environment_id.as_str()],
                )
                .map_err(|_| state_unavailable())?;
            transaction.commit().map_err(|_| state_unavailable())
        })();

        let lifecycle = if restore_lifecycle && !pending.originally_running && probe.running {
            self.runtime.terminate(probe)
        } else {
            Ok(())
        };
        result?;
        lifecycle
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
    old_provider_id: Option<String>,
    target_provider_id: String,
    old_config_hash: String,
    new_config_hash: String,
    old_credentials_hash: String,
    new_credentials_hash: String,
    originally_running: bool,
    expected_default_uid: Option<u32>,
}

fn load_pending_operations(
    connection: &Connection,
) -> Result<Vec<PendingWslOperation>, WslFailure> {
    let mut statement = connection
        .prepare(
            "SELECT environment_id, old_provider_id, target_provider_id,
                    old_config_fingerprint, new_config_fingerprint,
                    old_credentials_fingerprint, new_credentials_fingerprint,
                    originally_running, expected_default_uid
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
            "SELECT environment_id, old_provider_id, target_provider_id,
                    old_config_fingerprint, new_config_fingerprint,
                    old_credentials_fingerprint, new_credentials_fingerprint,
                    originally_running, expected_default_uid
             FROM wsl_pending_operation WHERE environment_id = ?1",
            [environment_id],
            pending_from_row,
        )
        .optional()
        .map_err(|_| state_unavailable())
}

fn pending_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingWslOperation> {
    Ok(PendingWslOperation {
        environment_id: row.get(0)?,
        old_provider_id: row.get(1)?,
        target_provider_id: row.get(2)?,
        old_config_hash: row.get(3)?,
        new_config_hash: row.get(4)?,
        old_credentials_hash: row.get(5)?,
        new_credentials_hash: row.get(6)?,
        originally_running: row.get(7)?,
        expected_default_uid: row.get(8)?,
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
            "SELECT current_provider_id, requires_attention, pending_restart, last_error, availability
             FROM wsl_environments WHERE environment_id = ?1",
            [probe.environment_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|_| state_unavailable())?
        .unwrap_or((None, false, false, None, availability_name(probe.availability).to_owned()));
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
        requires_attention: row.1 || parse_availability(&row.4) != WslAvailability::Manageable,
        pending_restart: row.2,
        revision: revision_for_probe(probe),
        message_id: probe.message_id.or(row.3.as_deref()).map(str::to_owned),
    })
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
    bytes.map(hash_bytes).unwrap_or_else(|| hash_bytes(b""))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn render_config(original: Option<&[u8]>, provider: &WslProvider) -> Result<Vec<u8>, WslFailure> {
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
    if let Some(store) = document.get("cli_auth_credentials_store") {
        if store.as_str() != Some("file") {
            return Err(WslFailure::new(
                WslFailureCategory::InvalidEnvironment,
                "wsl.file_credentials_required",
            ));
        }
    }
    let block = [
        "# >>> GPTEasy managed provider >>>".to_owned(),
        format!("# GPTEasy provider-id: {}", provider.id),
        format!(
            "model = {}",
            toml_edit::Value::from(provider.default_model.as_str())
        ),
        format!(
            "model_provider = {}",
            toml_edit::Value::from(provider.id.as_str())
        ),
        format!(
            "model_providers.{}.name = {}",
            provider.id,
            toml_edit::Value::from(provider.name.as_str())
        ),
        format!(
            "model_providers.{}.base_url = {}",
            provider.id,
            toml_edit::Value::from(provider.base_url.as_str())
        ),
        format!("model_providers.{}.wire_api = \"responses\"", provider.id),
        format!(
            "model_providers.{}.requires_openai_auth = true",
            provider.id
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

fn render_credentials(original: Option<&[u8]>, api_key: &str) -> Result<Vec<u8>, WslFailure> {
    let mut object = match original {
        Some(bytes) => serde_json::from_slice::<serde_json::Value>(bytes)
            .map_err(|_| {
                WslFailure::new(
                    WslFailureCategory::InvalidEnvironment,
                    "wsl.credentials_invalid",
                )
            })?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                WslFailure::new(
                    WslFailureCategory::InvalidEnvironment,
                    "wsl.credentials_invalid",
                )
            })?,
        None => serde_json::Map::new(),
    };
    object.insert(
        "auth_mode".to_owned(),
        serde_json::Value::String("apikey".to_owned()),
    );
    object.insert(
        "OPENAI_API_KEY".to_owned(),
        serde_json::Value::String(api_key.to_owned()),
    );
    let mut result =
        serde_json::to_vec_pretty(&serde_json::Value::Object(object)).map_err(|_| {
            WslFailure::new(
                WslFailureCategory::InvalidEnvironment,
                "wsl.credentials_invalid",
            )
        })?;
    result.push(b'\n');
    Ok(result)
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
struct SystemWslRuntime;

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
            let _ = run_wsl(&["--version"], None)?;
            let all = decode_wsl_output(&run_wsl(&["--list", "--quiet"], None)?)?;
            let running = decode_wsl_output(&run_wsl(&["--list", "--running", "--quiet"], None)?)?;
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
            let registry = read_registry_distributions();
            if registry.is_empty() {
                return Ok(all
                    .lines()
                    .filter_map(|name| {
                        let name = name.trim();
                        (!name.is_empty()).then(|| WslProbe {
                            environment_id: format!("name:{}", hash_bytes(name.as_bytes())),
                            display_name: name.to_owned(),
                            command_name: Some(name.to_owned()),
                            default_uid: None,
                            wsl_version: None,
                            running: running_names.contains(&name.to_ascii_lowercase()),
                            availability: WslAvailability::Ambiguous,
                            message_id: Some("wsl.registry_unavailable"),
                        })
                    })
                    .collect());
            }
            let counts =
                registry
                    .iter()
                    .fold(HashMap::<String, usize>::new(), |mut counts, item| {
                        *counts.entry(item.name.to_ascii_lowercase()).or_default() += 1;
                        counts
                    });
            Ok(registry
                .into_iter()
                .map(|item| {
                    let infrastructure = matches!(
                        item.name.to_ascii_lowercase().as_str(),
                        "docker-desktop" | "docker-desktop-data"
                    );
                    let normalized_name = item.name.to_ascii_lowercase();
                    let ambiguous = counts.get(&normalized_name).copied().unwrap_or_default() != 1;
                    let (availability, message_id, command_name) = if infrastructure {
                        (
                            WslAvailability::Infrastructure,
                            Some("wsl.infrastructure_distribution"),
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
                        display_name: item.name.clone(),
                        command_name,
                        default_uid: item.default_uid,
                        wsl_version: item.version,
                        running: running_names.contains(&normalized_name),
                        availability,
                        message_id,
                    }
                })
                .collect())
        }
    }

    fn start(&self, environment: &WslProbe) -> Result<(), WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        run_wsl(&["--distribution", name, "--exec", "/bin/true"], None).map(|_| ())
    }

    fn terminate(&self, environment: &WslProbe) -> Result<(), WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        run_wsl(&["--terminate", name], None).map(|_| ())
    }

    fn read_artifacts(&self, environment: &WslProbe) -> Result<WslArtifacts, WslFailure> {
        let name = environment
            .command_name
            .as_deref()
            .ok_or_else(|| wsl_availability_failure(environment.availability))?;
        Ok(WslArtifacts {
            config: read_guest_file(name, ".codex/config.toml")?,
            credentials: read_guest_file(name, ".codex/auth.json")?,
        })
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
        let output = run_wsl(
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
        old_config_hash: &str,
        old_credentials_hash: &str,
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
            old_config_hash,
            old_credentials_hash,
        ];
        let output = run_wsl(&args, Some(bundle))?;
        decode_wsl_output(&output)
    }
}

#[cfg(windows)]
fn run_wsl(args: &[&str], stdin: Option<&[u8]>) -> Result<Output, WslFailure> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut command = Command::new("wsl.exe");
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
            .ok_or_else(|| state_unavailable())?
            .write_all(bytes)
            .map_err(|_| {
                WslFailure::new(WslFailureCategory::GuestWriteFailed, "wsl.stdin_failed")
            })?;
    }
    let output = child
        .wait_with_output()
        .map_err(|_| WslFailure::new(WslFailureCategory::ProbeFailed, "wsl.process_failed"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(WslFailure::new(
            WslFailureCategory::ProbeFailed,
            "wsl.command_failed",
        ))
    }
}

#[cfg(not(windows))]
fn run_wsl(_args: &[&str], _stdin: Option<&[u8]>) -> Result<Output, WslFailure> {
    Err(WslFailure::new(
        WslFailureCategory::UnsupportedPlatform,
        "wsl.unsupported_platform",
    ))
}

#[cfg(windows)]
fn read_guest_file(name: &str, relative: &str) -> Result<Option<Vec<u8>>, WslFailure> {
    let command =
        format!("if [ -f \"$HOME/{relative}\" ]; then cat \"$HOME/{relative}\"; else exit 44; fi");
    let mut command_process = Command::new("wsl.exe");
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

#[cfg(not(windows))]
fn read_guest_file(_name: &str, _relative: &str) -> Result<Option<Vec<u8>>, WslFailure> {
    Err(WslFailure::new(
        WslFailureCategory::UnsupportedPlatform,
        "wsl.unsupported_platform",
    ))
}

#[cfg(windows)]
#[derive(Debug)]
struct RegistryDistro {
    id: String,
    name: String,
    default_uid: Option<u32>,
    version: Option<u32>,
}

#[cfg(windows)]
fn read_registry_distributions() -> Vec<RegistryDistro> {
    let output = Command::new("reg")
        .args(["query", WSL_REGISTRY_KEY])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut result = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(id) = trimmed
            .strip_prefix("HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Lxss\\")
        {
            let name_output = Command::new("reg")
                .args(["query", &format!("{WSL_REGISTRY_KEY}\\{id}")])
                .output();
            let Ok(name_output) = name_output else {
                continue;
            };
            let values = String::from_utf8_lossy(&name_output.stdout);
            let mut name = None;
            let mut default_uid = None;
            let mut version = None;
            for value_line in values.lines() {
                let parts = value_line.split_whitespace().collect::<Vec<_>>();
                if parts.len() < 3 {
                    continue;
                }
                match parts[0] {
                    "DistributionName" => name = Some(parts[2..].join(" ")),
                    "DefaultUid" => default_uid = parse_registry_u32(parts[2]),
                    "Version" => version = parse_registry_u32(parts[2]),
                    _ => {}
                }
            }
            if let Some(name) = name {
                result.push(RegistryDistro {
                    id: id.to_owned(),
                    name,
                    default_uid,
                    version,
                });
            }
        }
    }
    result
}

fn parse_registry_u32(value: &str) -> Option<u32> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or_else(
            || value.parse().ok(),
            |hex| u32::from_str_radix(hex, 16).ok(),
        )
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use tempfile::TempDir;

    struct FakeRuntime {
        probes: Mutex<Vec<WslProbe>>,
        artifacts: Mutex<WslArtifacts>,
        starts: AtomicUsize,
        terminations: AtomicUsize,
        writes: AtomicUsize,
        fail_probe_after_start: AtomicBool,
        probe_calls: AtomicUsize,
        stop_on_probe_call: AtomicUsize,
    }

    impl FakeRuntime {
        fn new(probe: WslProbe, artifacts: WslArtifacts) -> Self {
            Self {
                probes: Mutex::new(vec![probe]),
                artifacts: Mutex::new(artifacts),
                starts: AtomicUsize::new(0),
                terminations: AtomicUsize::new(0),
                writes: AtomicUsize::new(0),
                fail_probe_after_start: AtomicBool::new(false),
                probe_calls: AtomicUsize::new(0),
                stop_on_probe_call: AtomicUsize::new(usize::MAX),
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

        fn terminate(&self, environment: &WslProbe) -> Result<(), WslFailure> {
            self.terminations.fetch_add(1, Ordering::SeqCst);
            if let Some(probe) = self
                .probes
                .lock()
                .expect("probes")
                .iter_mut()
                .find(|probe| probe.environment_id == environment.environment_id)
            {
                probe.running = false;
            }
            Ok(())
        }

        fn read_artifacts(&self, _environment: &WslProbe) -> Result<WslArtifacts, WslFailure> {
            Ok(self.artifacts.lock().expect("artifacts").clone())
        }

        fn ensure_helper(&self, _environment: &WslProbe) -> Result<(), WslFailure> {
            Ok(())
        }

        fn write_bundle(
            &self,
            _environment: &WslProbe,
            _old_config_hash: &str,
            _old_credentials_hash: &str,
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
        assert!(String::from_utf8_lossy(&bundle).contains("GPTEASY_WSL_BUNDLE_V1"));
    }

    #[test]
    fn guest_writer_keeps_five_backups_per_artifact_and_accepts_crlf_markers() {
        let script = std::str::from_utf8(GUEST_WRITER).expect("guest writer is UTF-8");

        assert!(script.contains("for pattern in 'config-*.toml' 'auth-*.json'"));
        assert!(script.contains("awk 'NR > 5 { print }'"));
        assert!(!script.contains("NR > 10"));
        assert_eq!(script.matches("sed 's/\\r$//' \"$config_tmp\"").count(), 2);
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
    fn registry_numbers_accept_hex_and_non_wsl2_is_rejected() {
        assert_eq!(parse_registry_u32("0x2"), Some(2));
        assert_eq!(parse_registry_u32("0X3e8"), Some(1000));
        assert_eq!(parse_registry_u32("2"), Some(2));
        assert_eq!(parse_registry_u32("invalid"), None);
        assert_eq!(
            wsl_availability_failure(WslAvailability::UnsupportedVersion).message_id,
            "wsl.wsl2_required"
        );
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
        let rendered = render_config(Some(original), &provider("provider-id")).expect("render");
        let text = String::from_utf8(rendered).expect("utf8");
        let document = text.parse::<toml_edit::DocumentMut>().expect("valid toml");

        assert_eq!(document["custom_flag"].as_bool(), Some(true));
        assert_eq!(document["model"].as_str(), Some("model-a"));
        assert_eq!(document["model_provider"].as_str(), Some("provider-id"));
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
        let failure = render_config(Some(original), &provider("provider-id"))
            .expect_err("marker damage must stop");
        assert_eq!(failure.message_id, "wsl.managed_conflict");
    }

    #[test]
    fn stopped_distribution_is_started_written_and_restored_to_stopped() {
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
        assert_eq!(runtime.starts.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.terminations.load(Ordering::SeqCst), 1);
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
    fn stopped_distribution_is_restored_when_the_post_start_probe_fails() {
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
        assert_eq!(runtime.terminations.load(Ordering::SeqCst), 1);
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
    fn recovery_commits_new_hashes_and_restores_a_temporarily_started_distribution() {
        let target_config = b"new config".to_vec();
        let target_credentials = b"new credentials".to_vec();
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
                    originally_running, expected_default_uid, expected_revision, started_at
                 ) VALUES (?1, 'operation', 'artifacts_replaced', ?2, ?3, ?4, ?3, ?5, 0, 1000, ?6, '1')",
                params![
                    environment.environment_id,
                    "22222222-2222-4222-8222-222222222222",
                    hash_bytes(b""),
                    hash_bytes(&target_config),
                    hash_bytes(&target_credentials),
                    environment.revision,
                ],
            )
            .expect("pending operation");
        drop(connection);

        application.recover_pending().expect("recover");

        assert_eq!(runtime.terminations.load(Ordering::SeqCst), 1);
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

    #[cfg(windows)]
    #[test]
    #[ignore = "reads the real Windows WSL2 registry and running set"]
    fn real_system_probe_does_not_start_or_stop_distributions() {
        let before = decode_wsl_output(
            &run_wsl(&["--list", "--running", "--quiet"], None).expect("running before"),
        )
        .expect("decode before");
        let probes = SystemWslRuntime.probe().expect("probe");
        let after = decode_wsl_output(
            &run_wsl(&["--list", "--running", "--quiet"], None).expect("running after"),
        )
        .expect("decode after");

        assert_eq!(before, after);
        assert!(!probes.is_empty());
    }
}
