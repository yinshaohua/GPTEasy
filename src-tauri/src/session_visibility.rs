use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::state::{
    PendingSessionVisibilitySnapshot, PendingSessionVisibilityStatus,
    PendingSessionVisibilityTargetMode, StateStore,
};

const STATE_DATABASE: &str = "state_5.sqlite";
const SUPPORTED_THREADS_SCHEMA: &str = "CREATE TABLE threads (
    id TEXT PRIMARY KEY,
    rollout_path TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    source TEXT NOT NULL,
    model_provider TEXT NOT NULL,
    cwd TEXT NOT NULL,
    title TEXT NOT NULL,
    sandbox_policy TEXT NOT NULL,
    approval_mode TEXT NOT NULL,
    tokens_used INTEGER NOT NULL DEFAULT 0,
    has_user_event INTEGER NOT NULL,
    archived INTEGER NOT NULL,
    archived_at INTEGER,
    git_sha TEXT,
    git_branch TEXT,
    git_origin_url TEXT,
    cli_version TEXT NOT NULL DEFAULT '',
    first_user_message TEXT NOT NULL DEFAULT '',
    agent_nickname TEXT,
    agent_role TEXT,
    memory_mode TEXT NOT NULL DEFAULT 'enabled',
    model TEXT,
    reasoning_effort TEXT,
    agent_path TEXT,
    created_at_ms INTEGER,
    updated_at_ms INTEGER,
    thread_source TEXT,
    preview TEXT NOT NULL DEFAULT '',
    recency_at INTEGER NOT NULL DEFAULT 0,
    recency_at_ms INTEGER NOT NULL DEFAULT 0,
    history_mode TEXT NOT NULL DEFAULT 'legacy',
    name TEXT,
    is_pinned INTEGER NOT NULL DEFAULT 0,
    thread_section_id TEXT,
    section_position INTEGER,
    section_entered_at_ms INTEGER,
    project_id TEXT
)";
// Full sqlite_master contract: table plus all explicit indexes and triggers.
const CODEX_0_150_1_THREADS_SCHEMA_FINGERPRINT: &str =
    "cdc794aab210ed1108802047ac1dedecfd55a784de257f202fdb3290ecb1bd51";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityTargetMode {
    OpenaiLogin,
    Provider,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityTarget {
    pub mode: VisibilityTargetMode,
    pub model_provider: String,
    pub environment_revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityAppServerCapability {
    Available,
    Unavailable,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityScanContext {
    pub target: VisibilityTarget,
    pub codex_version: Option<String>,
    pub app_server: VisibilityAppServerCapability,
    pub execution_blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilitySummary {
    pub candidates: u32,
    pub unchanged: u32,
    pub missing_index: u32,
    pub skipped: u32,
    pub blocked: u32,
    pub encrypted_content_risk: u32,
    pub active: u32,
    pub archived: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilitySchemaCapability {
    pub status: String,
    pub database: String,
    #[serde(skip)]
    pub variant: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityIndexPlan {
    pub app_server_coordination: u32,
    pub sqlite_fallback_eligible: u32,
    pub schema_skipped: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityReason {
    pub code: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionVisibilityPreview {
    pub confirmation_id: String,
    pub target: VisibilityTarget,
    pub codex_version: Option<String>,
    pub app_server: VisibilityAppServerCapability,
    pub schema: VisibilitySchemaCapability,
    pub index_plan: VisibilityIndexPlan,
    pub summary: VisibilitySummary,
    pub can_execute: bool,
    pub blockers: Vec<String>,
    pub reasons: Vec<VisibilityReason>,
}

impl SessionVisibilityPreview {
    pub fn diagnostic_details(&self) -> String {
        let target_mode = match self.target.mode {
            VisibilityTargetMode::OpenaiLogin => "openai_login",
            VisibilityTargetMode::Provider => "provider",
            VisibilityTargetMode::Unknown => "unknown",
        };
        let codex_version = self
            .codex_version
            .as_deref()
            .map(safe_diagnostic_value)
            .unwrap_or_else(|| "unknown".to_owned());
        let error_codes = self
            .blockers
            .iter()
            .chain(self.reasons.iter().map(|reason| &reason.code))
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let error_codes = if error_codes.is_empty() {
            "none".to_owned()
        } else {
            error_codes.into_iter().collect::<Vec<_>>().join(",")
        };
        format!(
            "stage=scan; target_mode={target_mode}; codex_version={codex_version}; \
             schema={}; schema_variant={}; candidates={}; unchanged={}; missing_index={}; skipped={}; \
             blocked={}; encrypted_content_risk={}; active={}; archived={}; \
             index_app_server_coordination={}; index_sqlite_fallback_eligible={}; \
             index_schema_skipped={}; error_codes={error_codes}",
            self.schema.status,
            self.schema.variant,
            self.summary.candidates,
            self.summary.unchanged,
            self.summary.missing_index,
            self.summary.skipped,
            self.summary.blocked,
            self.summary.encrypted_content_risk,
            self.summary.active,
            self.summary.archived,
            self.index_plan.app_server_coordination,
            self.index_plan.sqlite_fallback_eligible,
            self.index_plan.schema_skipped,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityFailure {
    pub message_id: &'static str,
    pub stage: &'static str,
}

impl VisibilityFailure {
    pub fn at_stage(mut self, stage: &'static str) -> Self {
        self.stage = stage;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityExecutionRequest {
    pub confirmation_id: String,
    pub target: VisibilityTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityConsumerState {
    NoConsumers,
    DesktopRunning,
    CliRunning,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityThreadView {
    pub id: String,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityVerificationViews {
    pub all_providers: Vec<VisibilityThreadView>,
    pub target_provider: Vec<VisibilityThreadView>,
}

pub type VisibilityRuntimeFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, VisibilityFailure>> + Send + 'a>>;

pub trait VisibilityExecutionRuntime: Sync {
    fn current_target(&self) -> Result<VisibilityTarget, VisibilityFailure>;
    fn baseline_views<'a>(
        &'a self,
        target_provider: &'a str,
    ) -> VisibilityRuntimeFuture<'a, VisibilityVerificationViews>;
    fn shutdown_owned_app_server(&self) -> VisibilityRuntimeFuture<'_, ()>;
    fn consumers(&self, exclude_owned_app_server: bool) -> VisibilityConsumerState;
    fn verification_views<'a>(
        &'a self,
        target_provider: &'a str,
    ) -> VisibilityRuntimeFuture<'a, VisibilityVerificationViews>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityExecutionResult {
    pub status: &'static str,
    pub succeeded: u32,
    pub retryable: u32,
    pub encrypted_content_risk: u32,
    pub breakdown: VisibilityExecutionBreakdown,
    pub block_codex_restart: bool,
    pub message_id: &'static str,
    pub diagnostic_stage: &'static str,
    pub error_code: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityExecutionBreakdown {
    pub app_server_coordinated: u32,
    pub sqlite_fallback: u32,
    pub schema_skipped: u32,
    pub verification_failed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityCoordinationStatus {
    Idle,
    Deferred,
    Complete,
    Partial,
    Blocked,
}

impl VisibilityCoordinationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Deferred => "deferred",
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityCoordinationOutcome {
    pub status: VisibilityCoordinationStatus,
    pub block_codex_restart: bool,
    pub error_code: String,
    pub execution: Option<VisibilityExecutionResult>,
}

impl VisibilityExecutionResult {
    pub fn diagnostic_details(&self) -> String {
        format!(
            "stage={}; status={}; succeeded={}; retryable={}; \
             encrypted_content_risk={}; index_app_server_coordinated={}; \
             index_sqlite_fallback={}; index_schema_skipped={}; verification_failed={}; \
             block_codex_restart={}; error_codes={}",
            self.diagnostic_stage,
            self.status,
            self.succeeded,
            self.retryable,
            self.encrypted_content_risk,
            self.breakdown.app_server_coordinated,
            self.breakdown.sqlite_fallback,
            self.breakdown.schema_skipped,
            self.breakdown.verification_failed,
            self.block_codex_restart,
            self.error_code,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityFailurePoint {
    BeforeIndexInsert,
    AfterIndexCommit,
    BeforeRolloutReplace,
    AfterRolloutReplace,
}

pub trait VisibilityFaultInjector: Send + Sync {
    fn fails_at(&self, point: VisibilityFailurePoint, session_reference: &str) -> bool;
}

#[derive(Debug)]
struct NoVisibilityFaults;

impl VisibilityFaultInjector for NoVisibilityFaults {
    fn fails_at(&self, _point: VisibilityFailurePoint, _session_reference: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityRecoveryAssessment {
    pub status: &'static str,
    pub retryable: u32,
    pub block_codex_restart: bool,
}

#[derive(Clone)]
pub struct SessionVisibilityApplication {
    codex_home: PathBuf,
    recovery_root: PathBuf,
    faults: Arc<dyn VisibilityFaultInjector>,
    pending_state: Option<StateStore>,
}

impl SessionVisibilityApplication {
    pub fn new(codex_home: impl AsRef<Path>) -> Self {
        let codex_home = codex_home.as_ref().to_path_buf();
        Self {
            recovery_root: codex_home.join(".gpteasy"),
            codex_home,
            faults: Arc::new(NoVisibilityFaults),
            pending_state: None,
        }
    }

    pub fn with_recovery_root(
        codex_home: impl AsRef<Path>,
        recovery_root: impl AsRef<Path>,
    ) -> Self {
        Self {
            codex_home: codex_home.as_ref().to_path_buf(),
            recovery_root: recovery_root.as_ref().to_path_buf(),
            faults: Arc::new(NoVisibilityFaults),
            pending_state: None,
        }
    }

    pub fn with_fault_injector(
        codex_home: impl AsRef<Path>,
        recovery_root: impl AsRef<Path>,
        faults: impl VisibilityFaultInjector + 'static,
    ) -> Self {
        Self {
            codex_home: codex_home.as_ref().to_path_buf(),
            recovery_root: recovery_root.as_ref().to_path_buf(),
            faults: Arc::new(faults),
            pending_state: None,
        }
    }

    pub fn with_pending_state(mut self, state_store: StateStore) -> Self {
        self.pending_state = Some(state_store);
        self
    }

    pub fn record_pending(&self, target: &VisibilityTarget) -> Result<(), VisibilityFailure> {
        let Some(state_store) = &self.pending_state else {
            return Err(pending_state_failure());
        };
        let target_mode = match target.mode {
            VisibilityTargetMode::Provider => PendingSessionVisibilityTargetMode::Provider,
            VisibilityTargetMode::OpenaiLogin => PendingSessionVisibilityTargetMode::OpenaiLogin,
            VisibilityTargetMode::Unknown => return Err(pending_state_failure()),
        };
        state_store
            .record_pending_session_visibility(
                target_mode,
                &target.model_provider,
                &target.environment_revision,
            )
            .map_err(|_| pending_state_failure())
    }

    pub fn pending_status(
        &self,
    ) -> Result<Option<PendingSessionVisibilitySnapshot>, VisibilityFailure> {
        let Some(state_store) = &self.pending_state else {
            return Err(pending_state_failure());
        };
        state_store
            .pending_session_visibility()
            .map_err(|_| pending_state_failure())
    }

    pub fn mark_pending_running(&self, target: &VisibilityTarget) -> Result<(), VisibilityFailure> {
        self.update_pending_for_target(
            target,
            PendingSessionVisibilityStatus::Running,
            0,
            0,
            "consumer_recheck",
            "none",
        )
    }

    pub fn session_reference(id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"gpteasy-session-reference-v1\0");
        hasher.update(id.as_bytes());
        format!("{:x}", hasher.finalize())[..24].to_owned()
    }

    pub fn scan(
        &self,
        context: VisibilityScanContext,
    ) -> Result<SessionVisibilityPreview, VisibilityFailure> {
        let index = read_index(&self.codex_home);
        let ambiguous_rollout_ids = duplicate_rollout_ids(&self.codex_home)?;
        let mut summary = VisibilitySummary {
            candidates: 0,
            unchanged: 0,
            missing_index: 0,
            skipped: 0,
            blocked: 0,
            encrypted_content_risk: 0,
            active: 0,
            archived: 0,
        };
        let mut reasons = BTreeMap::<String, u32>::new();
        self.scan_directory(
            &self.codex_home.join("sessions"),
            false,
            &context.target.model_provider,
            &index,
            &ambiguous_rollout_ids,
            &mut summary,
            &mut reasons,
        )?;
        self.scan_directory(
            &self.codex_home.join("archived_sessions"),
            true,
            &context.target.model_provider,
            &index,
            &ambiguous_rollout_ids,
            &mut summary,
            &mut reasons,
        )?;

        let mut blockers = context.execution_blockers;
        if context.app_server != VisibilityAppServerCapability::Available {
            blockers.push(
                match context.app_server {
                    VisibilityAppServerCapability::Unavailable => "app_server_unavailable",
                    VisibilityAppServerCapability::Incompatible => "app_server_incompatible",
                    VisibilityAppServerCapability::Available => unreachable!(),
                }
                .to_owned(),
            );
        }
        if index.schema_status != "supported" {
            blockers.push("unsupported_index_schema".to_owned());
        }
        blockers.sort();
        blockers.dedup();
        let can_execute = blockers.is_empty();
        if !can_execute {
            summary.blocked = summary.blocked.max(summary.candidates);
            for blocker in &blockers {
                increment(&mut reasons, blocker);
            }
        }

        let confirmation_id = self.confirmation_id(&context.target, &index)?;
        let fallback_ineligible = reasons
            .get("index_fallback_metadata_incomplete")
            .copied()
            .unwrap_or(0);
        Ok(SessionVisibilityPreview {
            confirmation_id,
            target: context.target,
            codex_version: context.codex_version,
            app_server: context.app_server,
            schema: VisibilitySchemaCapability {
                status: index.schema_status.clone(),
                database: STATE_DATABASE.to_owned(),
                variant: index
                    .schema
                    .map(SupportedThreadSchema::diagnostic_name)
                    .unwrap_or(index.schema_status.as_str())
                    .to_owned(),
            },
            index_plan: VisibilityIndexPlan {
                app_server_coordination: summary.missing_index,
                sqlite_fallback_eligible: if index.schema_status == "supported" {
                    summary.missing_index.saturating_sub(fallback_ineligible)
                } else {
                    0
                },
                schema_skipped: if index.schema_status == "supported" {
                    0
                } else {
                    summary.blocked
                },
            },
            summary,
            can_execute,
            blockers,
            reasons: reasons
                .into_iter()
                .map(|(code, count)| VisibilityReason { code, count })
                .collect(),
        })
    }

    pub async fn execute<R: VisibilityExecutionRuntime>(
        &self,
        request: VisibilityExecutionRequest,
        runtime: &R,
    ) -> Result<VisibilityExecutionResult, VisibilityFailure> {
        match self.assess_recovery() {
            Ok(assessment) if assessment.block_codex_restart => {
                return Ok(indeterminate_execution_result());
            }
            Err(_) => return Ok(indeterminate_execution_result()),
            Ok(_) => {}
        }
        let index = read_index(&self.codex_home);
        let mut candidates = self.execution_candidates(&request.target.model_provider, &index)?;
        let execution_scan = self.scan(VisibilityScanContext {
            target: request.target.clone(),
            codex_version: None,
            app_server: VisibilityAppServerCapability::Available,
            execution_blockers: Vec::new(),
        })?;
        if !execution_scan.can_execute {
            return Err(visibility_failure_at(
                "session_visibility.rescan_required",
                "storage_capability_recheck",
            ));
        }
        if self.confirmation_id(&request.target, &index)? != request.confirmation_id {
            return Err(visibility_failure_at(
                "session_visibility.rescan_required",
                "confirmation_recheck",
            ));
        }
        if runtime
            .current_target()
            .map_err(|failure| failure.at_stage("target_recheck"))?
            != request.target
        {
            return Err(visibility_failure_at(
                "session_visibility.rescan_required",
                "target_recheck",
            ));
        }
        ensure_consumers_stopped(runtime.consumers(true), "consumer_preflight")?;
        let baseline_views = runtime
            .baseline_views(&request.target.model_provider)
            .await
            .map_err(|failure| failure.at_stage("app_server_baseline"))?;
        let baseline_count = baseline_views.all_providers.len();
        let baseline_target_count = baseline_views.target_provider.len();
        let baseline = baseline_views
            .all_providers
            .into_iter()
            .collect::<BTreeSet<_>>();
        let baseline_target = baseline_views
            .target_provider
            .into_iter()
            .collect::<BTreeSet<_>>();
        if baseline.len() != baseline_count
            || baseline_target.len() != baseline_target_count
            || !baseline_target.is_subset(&baseline)
        {
            return Err(visibility_failure_at(
                "session_visibility.app_server_verification_failed",
                "baseline_validation",
            ));
        }
        if candidates.iter().any(|candidate| {
            !candidate.missing_index && !baseline.contains(&candidate.thread_view())
        }) {
            return Err(visibility_failure_at(
                "session_visibility.rescan_required",
                "baseline_validation",
            ));
        }
        runtime
            .shutdown_owned_app_server()
            .await
            .map_err(|failure| failure.at_stage("app_server_shutdown"))?;
        if runtime
            .current_target()
            .map_err(|failure| failure.at_stage("post_shutdown_target_recheck"))?
            != request.target
        {
            return Err(visibility_failure_at(
                "session_visibility.rescan_required",
                "post_shutdown_target_recheck",
            ));
        }
        ensure_consumers_stopped(runtime.consumers(false), "post_shutdown_consumer_recheck")?;
        let refreshed_index = read_index(&self.codex_home);
        if candidates.iter().any(|candidate| {
            file_hash(&candidate.path).as_deref() != Ok(candidate.observed_hash.as_str())
        }) {
            return Err(visibility_failure_at(
                "session_visibility.rescan_required",
                "candidate_hash_recheck",
            ));
        }
        accept_app_server_index_coordination(&index, &refreshed_index, &baseline, &mut candidates)?;
        let app_server_coordinated = candidates
            .iter()
            .filter(|candidate| candidate.was_missing_index && !candidate.missing_index)
            .count() as u32;

        let mut manifest = VisibilityRecoveryManifest::new(
            &request.target,
            &candidates,
            self.index_database_hash(),
        )?;
        self.write_manifest(&manifest)?;
        if candidates.iter().any(|candidate| candidate.missing_index) {
            manifest.stage = "index_transaction".to_owned();
            self.write_manifest(&manifest)?;
        }
        let index_outcome =
            insert_missing_indexes(&self.codex_home, &candidates, self.faults.as_ref());
        let index_write_failed = index_outcome.failed;
        let index_metadata_incomplete = !index_outcome.ineligible.is_empty();
        let sqlite_fallback = index_outcome.inserted.len() as u32;
        if sqlite_fallback > 0
            && self
                .faults
                .fails_at(VisibilityFailurePoint::AfterIndexCommit, "index_database")
        {
            return Err(visibility_failure("session_visibility.interrupted"));
        }
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            if candidate.was_missing_index {
                manifest.items[candidate_index].index_stage = if !candidate.missing_index {
                    VisibilityIndexStage::AppServerCoordinated
                } else if candidate.index_metadata.is_none() {
                    VisibilityIndexStage::IndexFallbackMetadataIncomplete
                } else if index_write_failed {
                    VisibilityIndexStage::IndexWriteFailed
                } else {
                    VisibilityIndexStage::SqliteFallbackCommitted
                };
            }
        }
        manifest.stage = "rollout_replace".to_owned();
        manifest.index_database_after_hash = self.index_database_hash();
        self.write_manifest(&manifest)?;
        let mut written = Vec::new();
        let mut retryable = 0;
        let mut write_failed = false;
        let mut rescan_required = false;
        let encrypted_content_risk = candidates
            .iter()
            .filter(|candidate| candidate.has_encrypted_content)
            .count() as u32;
        for (candidate_index, candidate) in candidates.into_iter().enumerate() {
            if candidate.missing_index && (index_write_failed || candidate.index_metadata.is_none())
            {
                retryable += 1;
                continue;
            }
            let reference = &manifest.items[candidate_index].session_reference;
            if self
                .faults
                .fails_at(VisibilityFailurePoint::BeforeRolloutReplace, reference)
            {
                manifest.items[candidate_index].stage = "write_failed".to_owned();
                self.write_manifest(&manifest)?;
                retryable += 1;
                write_failed = true;
                continue;
            }
            match replace_rollout_provider(
                &candidate.path,
                &request.target.model_provider,
                &manifest.items[candidate_index].before_hash,
            ) {
                Ok(()) => {
                    if self
                        .faults
                        .fails_at(VisibilityFailurePoint::AfterRolloutReplace, reference)
                    {
                        return Err(visibility_failure("session_visibility.interrupted"));
                    }
                    manifest.items[candidate_index].stage = "written".to_owned();
                    self.write_manifest(&manifest)?;
                    written.push((candidate_index, candidate));
                }
                Err(failure) if failure.message_id == "session_visibility.rescan_required" => {
                    manifest.items[candidate_index].stage = "external_change".to_owned();
                    for item in manifest.items.iter_mut().skip(candidate_index + 1) {
                        item.stage = "not_attempted".to_owned();
                    }
                    self.write_manifest(&manifest)?;
                    retryable += (manifest.items.len() - candidate_index) as u32;
                    rescan_required = true;
                    break;
                }
                Err(_) => {
                    manifest.items[candidate_index].stage = "write_failed".to_owned();
                    self.write_manifest(&manifest)?;
                    retryable += 1;
                    write_failed = true;
                }
            }
        }
        let views = match runtime
            .verification_views(&request.target.model_provider)
            .await
        {
            Ok(views) => views,
            Err(_) => {
                retryable += written.len() as u32;
                for (candidate_index, _) in &written {
                    manifest.items[*candidate_index].stage = "verification_failed".to_owned();
                }
                manifest.stage = "partial".to_owned();
                manifest.index_database_after_hash = self.index_database_hash();
                self.write_manifest(&manifest)?;
                return Ok(VisibilityExecutionResult {
                    status: "failed",
                    succeeded: 0,
                    retryable,
                    encrypted_content_risk,
                    breakdown: VisibilityExecutionBreakdown {
                        app_server_coordinated,
                        sqlite_fallback,
                        schema_skipped: 0,
                        verification_failed: written.len() as u32,
                    },
                    block_codex_restart: false,
                    message_id: if rescan_required {
                        "session_visibility.rescan_required"
                    } else {
                        "session_visibility.repair_failed"
                    },
                    diagnostic_stage: "app_server_verify",
                    error_code: "session_visibility.app_server_verification_failed",
                });
            }
        };
        let all_after_count = views.all_providers.len();
        let target_after_count = views.target_provider.len();
        let all_after = views.all_providers.into_iter().collect::<BTreeSet<_>>();
        let target_after = views.target_provider.into_iter().collect::<BTreeSet<_>>();
        let written_target = written
            .iter()
            .map(|(_, candidate)| candidate.thread_view())
            .collect::<BTreeSet<_>>();
        let expected_all = baseline
            .union(&index_outcome.inserted)
            .cloned()
            .collect::<BTreeSet<_>>();
        let expected_target = baseline_target
            .union(&written_target)
            .cloned()
            .collect::<BTreeSet<_>>();
        let invariants_hold = all_after_count == expected_all.len()
            && all_after.len() == all_after_count
            && target_after.len() == target_after_count
            && target_after_count == expected_target.len()
            && all_after == expected_all
            && target_after == expected_target;
        let candidate_verified = |candidate: &ExecutionCandidate| {
            let thread = candidate.thread_view();
            all_after.contains(&thread) && target_after.contains(&thread)
        };
        let succeeded = written
            .iter()
            .filter(|(_, candidate)| candidate_verified(candidate))
            .count() as u32;
        let candidate_verification_failed = written.len() as u32 - succeeded;
        let global_verification_failed =
            u32::from(!invariants_hold && candidate_verification_failed == 0);
        let verification_failed = candidate_verification_failed + global_verification_failed;
        retryable += verification_failed;
        for (candidate_index, candidate) in &written {
            manifest.items[*candidate_index].stage = if candidate_verified(candidate) {
                "verified"
            } else {
                "verification_failed"
            }
            .to_owned();
        }
        manifest.stage = if retryable == 0 {
            "verified"
        } else {
            "partial"
        }
        .to_owned();
        manifest.index_database_after_hash = self.index_database_hash();
        self.write_manifest(&manifest)?;
        let status = if retryable == 0 {
            "complete"
        } else if succeeded > 0 {
            "partial"
        } else {
            "failed"
        };
        Ok(VisibilityExecutionResult {
            status,
            succeeded,
            retryable,
            encrypted_content_risk,
            breakdown: VisibilityExecutionBreakdown {
                app_server_coordinated,
                sqlite_fallback,
                schema_skipped: 0,
                verification_failed,
            },
            block_codex_restart: false,
            message_id: if rescan_required {
                "session_visibility.rescan_required"
            } else {
                match status {
                    "complete" => "session_visibility.repair_complete",
                    "partial" => "session_visibility.repair_partial",
                    _ => "session_visibility.repair_failed",
                }
            },
            diagnostic_stage: if retryable == 0 {
                "verify"
            } else if !invariants_hold {
                "app_server_verify"
            } else if rescan_required {
                "candidate_hash_recheck"
            } else if index_write_failed {
                "index_transaction"
            } else if index_metadata_incomplete {
                "index_fallback_plan"
            } else if write_failed {
                "rollout_replace"
            } else {
                "verify"
            },
            error_code: if retryable == 0 {
                "none"
            } else if !invariants_hold {
                "session_visibility.verification_invariant_failed"
            } else if rescan_required {
                "session_visibility.rescan_required"
            } else if index_write_failed {
                "session_visibility.index_write_failed"
            } else if index_metadata_incomplete {
                "session_visibility.index_fallback_metadata_incomplete"
            } else if write_failed {
                "session_visibility.write_failed"
            } else {
                "session_visibility.repair_failed"
            },
        })
    }

    pub async fn execute_pending<R: VisibilityExecutionRuntime>(
        &self,
        request: VisibilityExecutionRequest,
        runtime: &R,
    ) -> Result<VisibilityExecutionResult, VisibilityFailure> {
        self.update_pending_for_target(
            &request.target,
            PendingSessionVisibilityStatus::Running,
            0,
            0,
            "consumer_recheck",
            "none",
        )?;
        let result = self.execute(request.clone(), runtime).await;
        match &result {
            Ok(execution) => self.reconcile_pending_result(&request.target, execution)?,
            Err(failure) => {
                let blocked = match self.assess_recovery() {
                    Ok(assessment) => assessment.block_codex_restart,
                    Err(_) => true,
                };
                self.update_pending_for_target(
                    &request.target,
                    if blocked {
                        PendingSessionVisibilityStatus::Blocked
                    } else {
                        PendingSessionVisibilityStatus::Pending
                    },
                    0,
                    0,
                    failure.stage,
                    failure.message_id,
                )?;
            }
        }
        result
    }

    pub async fn coordinate_pending<R: VisibilityExecutionRuntime>(
        &self,
        context: VisibilityScanContext,
        runtime: &R,
    ) -> Result<VisibilityCoordinationOutcome, VisibilityFailure> {
        let Some(pending) = self.pending_status()? else {
            return Ok(coordination_outcome(
                VisibilityCoordinationStatus::Idle,
                false,
                "none",
                None,
            ));
        };
        let current_target = match runtime.current_target() {
            Ok(target) => target,
            Err(failure) => {
                self.update_pending_for_target(
                    &context.target,
                    PendingSessionVisibilityStatus::Pending,
                    pending.succeeded,
                    pending.retryable,
                    failure.stage,
                    failure.message_id,
                )?;
                return Ok(coordination_outcome(
                    VisibilityCoordinationStatus::Deferred,
                    false,
                    failure.message_id,
                    None,
                ));
            }
        };
        if !pending_snapshot_matches_target(&pending, &context.target)
            || current_target != context.target
        {
            self.update_pending_for_target(
                &context.target,
                PendingSessionVisibilityStatus::Pending,
                pending.succeeded,
                pending.retryable,
                "target_recheck",
                "session_visibility.rescan_required",
            )?;
            return Ok(coordination_outcome(
                VisibilityCoordinationStatus::Deferred,
                false,
                "session_visibility.rescan_required",
                None,
            ));
        }
        let consumer = runtime.consumers(true);
        if consumer != VisibilityConsumerState::NoConsumers {
            let error_code = consumer_error_code(consumer);
            self.update_pending_for_target(
                &context.target,
                PendingSessionVisibilityStatus::Pending,
                pending.succeeded,
                pending.retryable,
                "consumer_recheck",
                error_code,
            )?;
            return Ok(coordination_outcome(
                VisibilityCoordinationStatus::Deferred,
                false,
                error_code,
                None,
            ));
        }
        let preview = match self.scan(context.clone()) {
            Ok(preview) => preview,
            Err(failure) => {
                self.update_pending_for_target(
                    &context.target,
                    PendingSessionVisibilityStatus::Pending,
                    pending.succeeded,
                    pending.retryable,
                    failure.stage,
                    failure.message_id,
                )?;
                return Err(failure);
            }
        };
        if !preview.can_execute {
            let error_code = preview
                .blockers
                .first()
                .map(|blocker| format!("session_visibility.{blocker}"))
                .unwrap_or_else(|| "session_visibility.scan_blocked".to_owned());
            self.update_pending_for_target(
                &preview.target,
                PendingSessionVisibilityStatus::Pending,
                pending.succeeded,
                pending.retryable,
                "scan",
                &error_code,
            )?;
            return Ok(VisibilityCoordinationOutcome {
                status: VisibilityCoordinationStatus::Deferred,
                block_codex_restart: false,
                error_code,
                execution: None,
            });
        }
        let target = preview.target.clone();
        let result = self
            .execute_pending(
                VisibilityExecutionRequest {
                    confirmation_id: preview.confirmation_id,
                    target,
                },
                runtime,
            )
            .await;
        match result {
            Ok(execution) => {
                let status = if execution.block_codex_restart || execution.status == "indeterminate"
                {
                    VisibilityCoordinationStatus::Blocked
                } else if execution.status == "complete" && execution.retryable == 0 {
                    VisibilityCoordinationStatus::Complete
                } else if execution.status == "partial" || execution.succeeded > 0 {
                    VisibilityCoordinationStatus::Partial
                } else {
                    VisibilityCoordinationStatus::Deferred
                };
                Ok(coordination_outcome(
                    status,
                    execution.block_codex_restart,
                    execution.error_code,
                    Some(execution),
                ))
            }
            Err(failure) => {
                let blocked = self.pending_status()?.is_some_and(|pending| {
                    pending.status == PendingSessionVisibilityStatus::Blocked
                });
                Ok(coordination_outcome(
                    if blocked {
                        VisibilityCoordinationStatus::Blocked
                    } else {
                        VisibilityCoordinationStatus::Deferred
                    },
                    blocked,
                    failure.message_id,
                    None,
                ))
            }
        }
    }

    fn reconcile_pending_result(
        &self,
        target: &VisibilityTarget,
        result: &VisibilityExecutionResult,
    ) -> Result<(), VisibilityFailure> {
        let Some(state_store) = &self.pending_state else {
            return Ok(());
        };
        if !self.pending_matches_target(state_store, target)? {
            return Ok(());
        }
        if result.status == "complete" && result.retryable == 0 {
            state_store
                .clear_pending_session_visibility(&target.environment_revision)
                .map_err(|_| pending_state_failure())?;
            return Ok(());
        }
        let status = if result.block_codex_restart || result.status == "indeterminate" {
            PendingSessionVisibilityStatus::Blocked
        } else if result.status == "partial" || result.succeeded > 0 {
            PendingSessionVisibilityStatus::Partial
        } else {
            PendingSessionVisibilityStatus::Pending
        };
        self.update_pending_for_target(
            target,
            status,
            result.succeeded,
            result.retryable,
            result.diagnostic_stage,
            result.error_code,
        )
    }

    fn update_pending_for_target(
        &self,
        target: &VisibilityTarget,
        status: PendingSessionVisibilityStatus,
        succeeded: u32,
        retryable: u32,
        diagnostic_stage: &str,
        error_code: &str,
    ) -> Result<(), VisibilityFailure> {
        let Some(state_store) = &self.pending_state else {
            return Ok(());
        };
        if !self.pending_matches_target(state_store, target)? {
            return Ok(());
        }
        state_store
            .update_pending_session_visibility(
                &target.environment_revision,
                status,
                succeeded,
                retryable,
                diagnostic_stage,
                error_code,
            )
            .map_err(|_| pending_state_failure())?;
        Ok(())
    }

    fn pending_matches_target(
        &self,
        state_store: &StateStore,
        target: &VisibilityTarget,
    ) -> Result<bool, VisibilityFailure> {
        let pending = state_store
            .pending_session_visibility()
            .map_err(|_| pending_state_failure())?;
        Ok(pending.is_some_and(|pending| {
            pending.environment_revision == target.environment_revision
                && pending.model_provider == target.model_provider
                && matches!(
                    (pending.target_mode, target.mode),
                    (
                        PendingSessionVisibilityTargetMode::Provider,
                        VisibilityTargetMode::Provider
                    ) | (
                        PendingSessionVisibilityTargetMode::OpenaiLogin,
                        VisibilityTargetMode::OpenaiLogin
                    )
                )
        }))
    }

    pub fn assess_recovery(&self) -> Result<VisibilityRecoveryAssessment, VisibilityFailure> {
        let path = self.recovery_manifest_path();
        if !path.exists() {
            return Ok(VisibilityRecoveryAssessment {
                status: "none",
                retryable: 0,
                block_codex_restart: false,
            });
        }
        let manifest = serde_json::from_slice::<VisibilityRecoveryManifest>(
            &fs::read(&path)
                .map_err(|_| visibility_failure("session_visibility.recovery_unavailable"))?,
        )
        .map_err(|_| visibility_failure("session_visibility.recovery_unavailable"))?;
        if manifest.stage == "verified" {
            return Ok(VisibilityRecoveryAssessment {
                status: "complete",
                retryable: 0,
                block_codex_restart: false,
            });
        }
        let observed_index_hash = self.index_database_hash();
        let index_state_indeterminate = manifest.stage == "index_transaction"
            && manifest.index_database_after_hash.is_none()
            && observed_index_hash != manifest.index_database_before_hash;
        let observed = self.observed_rollout_hashes()?;
        let mut retryable = 0;
        let mut indeterminate = index_state_indeterminate;
        for item in manifest.items {
            let current = observed.get(&item.session_reference);
            if item.stage == "verified" && current == Some(&item.after_hash) {
                continue;
            }
            if item.stage == "external_change" {
                retryable += 1;
                continue;
            }
            if current == Some(&item.before_hash) || current == Some(&item.after_hash) {
                retryable += 1;
            } else {
                indeterminate = true;
            }
        }
        Ok(VisibilityRecoveryAssessment {
            status: if indeterminate {
                "indeterminate"
            } else if retryable > 0 {
                "retryable"
            } else {
                "complete"
            },
            retryable,
            block_codex_restart: indeterminate,
        })
    }

    fn write_manifest(
        &self,
        manifest: &VisibilityRecoveryManifest,
    ) -> Result<(), VisibilityFailure> {
        fs::create_dir_all(&self.recovery_root)
            .map_err(|_| visibility_failure("session_visibility.recovery_write_failed"))?;
        let bytes = serde_json::to_vec(manifest)
            .map_err(|_| visibility_failure("session_visibility.recovery_write_failed"))?;
        let path = self.recovery_manifest_path();
        if path.exists() {
            replace_file_atomically(&path, &bytes)
                .map_err(|_| visibility_failure("session_visibility.recovery_write_failed"))
        } else {
            write_new_file_atomically(&path, &bytes)
        }
    }

    fn recovery_manifest_path(&self) -> PathBuf {
        self.recovery_root.join("session-visibility-recovery.json")
    }

    fn observed_rollout_hashes(&self) -> Result<HashMap<String, String>, VisibilityFailure> {
        let mut observed = HashMap::new();
        for root in [
            self.codex_home.join("sessions"),
            self.codex_home.join("archived_sessions"),
        ] {
            if !root.exists() {
                continue;
            }
            let mut paths = Vec::new();
            collect_rollouts(&root, &mut paths)?;
            for path in paths {
                let Ok(rollout) = read_rollout(&path) else {
                    continue;
                };
                observed.insert(Self::session_reference(&rollout.id), file_hash(&path)?);
            }
        }
        Ok(observed)
    }

    fn execution_candidates(
        &self,
        target_provider: &str,
        index: &IndexSnapshot,
    ) -> Result<Vec<ExecutionCandidate>, VisibilityFailure> {
        let mut candidates = Vec::new();
        let ambiguous_rollout_ids = duplicate_rollout_ids(&self.codex_home)?;
        for (root, archived) in [
            (self.codex_home.join("sessions"), false),
            (self.codex_home.join("archived_sessions"), true),
        ] {
            if !root.exists() {
                continue;
            }
            let mut paths = Vec::new();
            collect_rollouts(&root, &mut paths)?;
            paths.sort();
            for path in paths {
                let Ok(rollout) = read_rollout(&path) else {
                    continue;
                };
                if ambiguous_rollout_ids.contains(&rollout.id) {
                    continue;
                }
                let Some(source) = rollout.source.as_deref() else {
                    continue;
                };
                if excluded_source_reason(source, rollout.thread_source.as_deref()).is_some()
                    || rollout.has_derived_identity
                    || !rollout.has_user_event
                {
                    continue;
                }
                let Some(before_provider) = rollout.model_provider.clone() else {
                    continue;
                };
                let missing_index = !index.rows.contains_key(&rollout.id);
                if let Some(indexed) = index.rows.get(&rollout.id) {
                    if !same_rollout_path(&indexed.rollout_path, &path)
                        || indexed.source != source
                        || indexed.archived != archived
                        || !index.accepts_index_user_event(indexed.has_user_event)
                    {
                        continue;
                    }
                }
                if missing_index
                    || rollout.model_provider.as_deref() != Some(target_provider)
                    || index
                        .rows
                        .get(&rollout.id)
                        .is_some_and(|indexed| indexed.model_provider != target_provider)
                {
                    let index_metadata = rollout.index_metadata.clone();
                    let observed_hash = file_hash(&path)?;
                    candidates.push(ExecutionCandidate {
                        id: rollout.id,
                        path,
                        archived,
                        source: source.to_owned(),
                        before_provider,
                        has_encrypted_content: rollout.has_encrypted_content,
                        missing_index,
                        was_missing_index: missing_index,
                        index_metadata,
                        observed_hash,
                    });
                }
            }
        }
        Ok(candidates)
    }

    fn confirmation_id(
        &self,
        target: &VisibilityTarget,
        index: &IndexSnapshot,
    ) -> Result<String, VisibilityFailure> {
        let candidates = self.execution_candidates(&target.model_provider, index)?;
        let mut hasher = Sha256::new();
        hasher.update(b"gpteasy-session-visibility-confirmation-v1\0");
        hasher.update(format!("{:?}", target.mode));
        hasher.update([0]);
        hasher.update(target.model_provider.as_bytes());
        hasher.update([0]);
        hasher.update(target.environment_revision.as_bytes());
        hasher.update(index.schema_status.as_bytes());
        hasher.update(format!("{:?}", index.schema).as_bytes());
        let mut index_rows = index.rows.iter().collect::<Vec<_>>();
        index_rows.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (id, row) in index_rows {
            hasher.update(id.as_bytes());
            hasher.update([0]);
            hasher.update(row.rollout_path.to_string_lossy().as_bytes());
            hasher.update([0]);
            hasher.update(row.source.as_bytes());
            hasher.update([0]);
            hasher.update(row.model_provider.as_bytes());
            hasher.update([row.has_user_event as u8, row.archived as u8]);
        }
        for root in [
            self.codex_home.join("sessions"),
            self.codex_home.join("archived_sessions"),
        ] {
            if !root.exists() {
                continue;
            }
            let mut paths = Vec::new();
            collect_rollouts(&root, &mut paths)?;
            paths.sort();
            for path in paths {
                hasher.update(path.to_string_lossy().as_bytes());
                hasher.update([0]);
                hasher.update(file_hash(&path)?.as_bytes());
            }
        }
        for candidate in candidates {
            hasher.update(candidate.id.as_bytes());
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn index_database_hash(&self) -> Option<String> {
        let mut hasher = Sha256::new();
        let mut found = false;
        for suffix in ["", "-wal", "-shm"] {
            let path = self.codex_home.join(format!("{STATE_DATABASE}{suffix}"));
            let Ok(bytes) = fs::read(path) else {
                continue;
            };
            found = true;
            hasher.update(suffix.as_bytes());
            hasher.update([0]);
            hasher.update(bytes);
        }
        found.then(|| format!("{:x}", hasher.finalize()))
    }

    pub fn add_execution_blocker(preview: &mut SessionVisibilityPreview, blocker: &str) {
        if preview.blockers.iter().any(|existing| existing == blocker) {
            return;
        }
        preview.blockers.push(blocker.to_owned());
        preview.blockers.sort();
        preview.can_execute = false;
        preview.summary.blocked = preview.summary.blocked.max(preview.summary.candidates);
        if let Some(reason) = preview
            .reasons
            .iter_mut()
            .find(|reason| reason.code == blocker)
        {
            reason.count += 1;
        } else {
            preview.reasons.push(VisibilityReason {
                code: blocker.to_owned(),
                count: 1,
            });
            preview
                .reasons
                .sort_by(|left, right| left.code.cmp(&right.code));
        }
    }

    fn scan_directory(
        &self,
        root: &Path,
        archived: bool,
        target_provider: &str,
        index: &IndexSnapshot,
        ambiguous_rollout_ids: &BTreeSet<String>,
        summary: &mut VisibilitySummary,
        reasons: &mut BTreeMap<String, u32>,
    ) -> Result<(), VisibilityFailure> {
        if !root.exists() {
            return Ok(());
        }
        let mut paths = Vec::new();
        collect_rollouts(root, &mut paths)?;
        paths.sort();
        for path in paths {
            if archived {
                summary.archived += 1;
            } else {
                summary.active += 1;
            }
            let rollout = match read_rollout(&path) {
                Ok(rollout) => rollout,
                Err(code) => {
                    summary.skipped += 1;
                    increment(reasons, code);
                    continue;
                }
            };
            if ambiguous_rollout_ids.contains(&rollout.id) {
                summary.skipped += 1;
                increment(reasons, "identity_ambiguous");
                continue;
            }
            let Some(source) = rollout.source.as_deref() else {
                summary.skipped += 1;
                increment(reasons, "identity_ambiguous");
                continue;
            };
            if let Some(code) = excluded_source_reason(source, rollout.thread_source.as_deref()) {
                summary.skipped += 1;
                increment(reasons, code);
                continue;
            }
            if rollout.has_derived_identity || !rollout.has_user_event {
                summary.skipped += 1;
                increment(
                    reasons,
                    if rollout.has_derived_identity {
                        "excluded_derived"
                    } else {
                        "no_user_event"
                    },
                );
                continue;
            }
            let Some(rollout_provider) = rollout.model_provider.as_deref() else {
                summary.skipped += 1;
                increment(reasons, "model_provider_invalid");
                continue;
            };
            if index.schema_status != "supported" {
                summary.blocked += 1;
                if rollout.has_encrypted_content {
                    summary.encrypted_content_risk += 1;
                    increment(reasons, "encrypted_content");
                }
                continue;
            }
            let Some(indexed) = index.rows.get(&rollout.id) else {
                summary.missing_index += 1;
                increment(reasons, "index_missing");
                if rollout.index_metadata.is_none() {
                    increment(reasons, "index_fallback_metadata_incomplete");
                }
                summary.candidates += 1;
                if rollout.has_encrypted_content {
                    summary.encrypted_content_risk += 1;
                    increment(reasons, "encrypted_content");
                }
                continue;
            };
            if !same_rollout_path(&indexed.rollout_path, &path)
                || indexed.source != source
                || indexed.archived != archived
                || !index.accepts_index_user_event(indexed.has_user_event)
            {
                summary.skipped += 1;
                increment(reasons, "identity_ambiguous");
                continue;
            }
            if rollout_provider == target_provider && indexed.model_provider == target_provider {
                summary.unchanged += 1;
                continue;
            }
            summary.candidates += 1;
            increment(reasons, "provider_mismatch");
            if rollout.has_encrypted_content {
                summary.encrypted_content_risk += 1;
                increment(reasons, "encrypted_content");
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ExecutionCandidate {
    id: String,
    path: PathBuf,
    archived: bool,
    source: String,
    before_provider: String,
    has_encrypted_content: bool,
    missing_index: bool,
    was_missing_index: bool,
    index_metadata: Option<IndexInsertMetadata>,
    observed_hash: String,
}

impl ExecutionCandidate {
    fn thread_view(&self) -> VisibilityThreadView {
        VisibilityThreadView {
            id: self.id.clone(),
            archived: self.archived,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibilityRecoveryManifest {
    schema_version: u32,
    target_mode: VisibilityTargetMode,
    target_provider: String,
    environment_revision: String,
    stage: String,
    index_database_before_hash: Option<String>,
    index_database_after_hash: Option<String>,
    items: Vec<VisibilityRecoveryItem>,
}

impl VisibilityRecoveryManifest {
    fn new(
        target: &VisibilityTarget,
        candidates: &[ExecutionCandidate],
        index_database_before_hash: Option<String>,
    ) -> Result<Self, VisibilityFailure> {
        let items = candidates
            .iter()
            .map(|candidate| {
                let before = fs::read(&candidate.path)
                    .map_err(|_| visibility_failure("session_visibility.scan_failed"))?;
                let after = render_rollout_provider(&before, &target.model_provider)?;
                Ok(VisibilityRecoveryItem {
                    session_reference: SessionVisibilityApplication::session_reference(
                        &candidate.id,
                    ),
                    before_provider: candidate.before_provider.clone(),
                    after_provider: target.model_provider.clone(),
                    before_hash: bytes_hash(&before),
                    after_hash: bytes_hash(&after),
                    index_identifier: index_reference(&candidate.id),
                    archived: candidate.archived,
                    index_stage: if candidate.was_missing_index {
                        VisibilityIndexStage::PendingCoordination
                    } else {
                        VisibilityIndexStage::Existing
                    },
                    stage: "prepared".to_owned(),
                })
            })
            .collect::<Result<Vec<_>, VisibilityFailure>>()?;
        Ok(Self {
            schema_version: 2,
            target_mode: target.mode,
            target_provider: target.model_provider.clone(),
            environment_revision: target.environment_revision.clone(),
            stage: "prepared".to_owned(),
            index_database_before_hash,
            index_database_after_hash: None,
            items,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibilityRecoveryItem {
    session_reference: String,
    before_provider: String,
    after_provider: String,
    before_hash: String,
    after_hash: String,
    index_identifier: String,
    archived: bool,
    #[serde(default)]
    index_stage: VisibilityIndexStage,
    stage: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum VisibilityIndexStage {
    #[default]
    Legacy,
    Existing,
    PendingCoordination,
    AppServerCoordinated,
    IndexFallbackMetadataIncomplete,
    IndexWriteFailed,
    SqliteFallbackCommitted,
}

impl PartialOrd for VisibilityThreadView {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VisibilityThreadView {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.id, self.archived).cmp(&(&other.id, other.archived))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedThread {
    rollout_path: PathBuf,
    source: String,
    model_provider: String,
    has_user_event: bool,
    archived: bool,
}

struct IndexSnapshot {
    schema_status: String,
    schema: Option<SupportedThreadSchema>,
    rows: HashMap<String, IndexedThread>,
}

impl IndexSnapshot {
    fn accepts_index_user_event(&self, has_user_event: bool) -> bool {
        !self
            .schema
            .is_some_and(SupportedThreadSchema::index_user_event_is_authoritative)
            || has_user_event
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportedThreadSchema {
    Legacy,
    Codex01501,
}

impl SupportedThreadSchema {
    fn index_user_event_is_authoritative(self) -> bool {
        matches!(self, Self::Legacy)
    }

    fn diagnostic_name(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Codex01501 => "codex_0_150_1",
        }
    }
}

fn accept_app_server_index_coordination(
    before: &IndexSnapshot,
    after: &IndexSnapshot,
    baseline: &BTreeSet<VisibilityThreadView>,
    candidates: &mut [ExecutionCandidate],
) -> Result<(), VisibilityFailure> {
    if before.schema_status != after.schema_status
        || before.schema != after.schema
        || before
            .rows
            .iter()
            .any(|(id, row)| after.rows.get(id) != Some(row))
    {
        return Err(visibility_failure_at(
            "session_visibility.rescan_required",
            "index_recheck",
        ));
    }
    for candidate in candidates
        .iter_mut()
        .filter(|candidate| candidate.missing_index)
    {
        let Some(indexed) = after.rows.get(&candidate.id) else {
            continue;
        };
        if !baseline.contains(&candidate.thread_view())
            || !same_rollout_path(&indexed.rollout_path, &candidate.path)
            || indexed.source != candidate.source
            || indexed.model_provider != candidate.before_provider
            || indexed.archived != candidate.archived
            || !before.accepts_index_user_event(indexed.has_user_event)
        {
            return Err(visibility_failure_at(
                "session_visibility.rescan_required",
                "index_recheck",
            ));
        }
        candidate.missing_index = false;
    }
    Ok(())
}

fn read_index(codex_home: &Path) -> IndexSnapshot {
    let database = codex_home.join(STATE_DATABASE);
    let Ok(connection) = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return empty_index("missing");
    };
    let Ok(Some(schema)) = supported_thread_schema(&connection) else {
        return empty_index("unknown");
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT id, rollout_path, source, model_provider, has_user_event, archived FROM threads",
    ) else {
        return empty_index("unknown");
    };
    let Ok(mapped) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            IndexedThread {
                rollout_path: PathBuf::from(row.get::<_, String>(1)?),
                source: row.get(2)?,
                model_provider: row.get(3)?,
                has_user_event: row.get(4)?,
                archived: row.get(5)?,
            },
        ))
    }) else {
        return empty_index("unknown");
    };
    let mut rows = HashMap::new();
    for row in mapped {
        let Ok((id, indexed)) = row else {
            return empty_index("unknown");
        };
        rows.insert(id, indexed);
    }
    IndexSnapshot {
        schema_status: "supported".to_owned(),
        schema: Some(schema),
        rows,
    }
}

fn empty_index(schema_status: &str) -> IndexSnapshot {
    IndexSnapshot {
        schema_status: schema_status.to_owned(),
        schema: None,
        rows: HashMap::new(),
    }
}

fn supported_thread_schema(
    connection: &Connection,
) -> rusqlite::Result<Option<SupportedThreadSchema>> {
    let schema_sql = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'threads'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    let extra_schema_objects = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE tbl_name = 'threads'
           AND type IN ('index', 'trigger')
           AND sql IS NOT NULL",
        [],
        |row| row.get::<_, u32>(0),
    )?;
    if canonical_schema_sql(&schema_sql) == canonical_schema_sql(SUPPORTED_THREADS_SCHEMA)
        && extra_schema_objects == 0
    {
        return Ok(Some(SupportedThreadSchema::Legacy));
    }
    if thread_schema_fingerprint(connection)? == CODEX_0_150_1_THREADS_SCHEMA_FINGERPRINT {
        return Ok(Some(SupportedThreadSchema::Codex01501));
    }
    Ok(None)
}

fn thread_schema_fingerprint(connection: &Connection) -> rusqlite::Result<String> {
    let mut statement = connection.prepare(
        "SELECT type, name, sql FROM sqlite_master
         WHERE tbl_name = 'threads'
           AND type IN ('table', 'index', 'trigger')
           AND sql IS NOT NULL
         ORDER BY type, name",
    )?;
    let objects = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-thread-schema-v1\0");
    for object in objects {
        let (kind, name, sql) = object?;
        hasher.update(kind.as_bytes());
        hasher.update([0]);
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(canonical_schema_sql(&sql).as_bytes());
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn canonical_schema_sql(sql: &str) -> String {
    let mut canonical = String::with_capacity(sql.len());
    let mut characters = sql.chars().peekable();
    let mut in_string = false;
    while let Some(character) = characters.next() {
        if character == '\'' {
            canonical.push(character);
            if in_string && characters.peek() == Some(&'\'') {
                canonical.push(characters.next().expect("peeked escaped quote"));
            } else {
                in_string = !in_string;
            }
        } else if in_string {
            canonical.push(character);
        } else if !character.is_ascii_whitespace() {
            canonical.push(character.to_ascii_lowercase());
        }
    }
    canonical
}

fn duplicate_rollout_ids(codex_home: &Path) -> Result<BTreeSet<String>, VisibilityFailure> {
    let mut counts = HashMap::<String, u32>::new();
    for root in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        if !root.exists() {
            continue;
        }
        let mut paths = Vec::new();
        collect_rollouts(&root, &mut paths)?;
        for path in paths {
            if let Ok(rollout) = read_rollout(&path) {
                *counts.entry(rollout.id).or_default() += 1;
            }
        }
    }
    Ok(counts
        .into_iter()
        .filter_map(|(id, count)| (count > 1).then_some(id))
        .collect())
}

fn same_rollout_path(indexed: &Path, observed: &Path) -> bool {
    match (fs::canonicalize(indexed), fs::canonicalize(observed)) {
        (Ok(indexed), Ok(observed)) => indexed == observed,
        _ => false,
    }
}

struct RolloutObservation {
    id: String,
    source: Option<String>,
    thread_source: Option<String>,
    model_provider: Option<String>,
    has_derived_identity: bool,
    has_user_event: bool,
    has_encrypted_content: bool,
    index_metadata: Option<IndexInsertMetadata>,
}

#[derive(Debug, Clone)]
struct IndexInsertMetadata {
    timestamp: String,
    cwd: String,
    cli_version: String,
    approval_mode: String,
    sandbox_policy: String,
    first_user_message: String,
}

fn read_rollout(path: &Path) -> Result<RolloutObservation, &'static str> {
    let file = fs::File::open(path).map_err(|_| "rollout_unreadable")?;
    let mut lines = BufReader::new(file).lines();
    let first = lines
        .next()
        .ok_or("rollout_damaged")?
        .map_err(|_| "rollout_unreadable")?;
    let first = serde_json::from_str::<Value>(&first).map_err(|_| "rollout_damaged")?;
    if first.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Err("identity_ambiguous");
    }
    let payload = first
        .get("payload")
        .and_then(Value::as_object)
        .ok_or("identity_ambiguous")?;
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .ok_or("identity_ambiguous")?
        .to_owned();
    let source = payload
        .get("source")
        .and_then(rollout_source_kind)
        .map(str::to_owned);
    let thread_source = payload
        .get("thread_source")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let model_provider = payload
        .get("model_provider")
        .and_then(Value::as_str)
        .filter(|provider| !provider.trim().is_empty())
        .map(str::to_owned);
    let timestamp = payload
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let cli_version = payload
        .get("cli_version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let approval_mode = payload
        .get("approval_policy")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let sandbox_policy = payload.get("sandbox_policy").map(Value::to_string);
    let has_derived_identity = [
        "forked_from_id",
        "parent_thread_id",
        "agent_nickname",
        "agent_role",
        "agent_path",
    ]
    .iter()
    .any(|field| payload.get(*field).is_some_and(|value| !value.is_null()));
    let mut observation = RolloutObservation {
        id,
        source,
        thread_source,
        model_provider,
        has_derived_identity,
        has_user_event: false,
        has_encrypted_content: contains_encrypted_content(&first),
        index_metadata: None,
    };
    let mut first_user_message = None;
    for line in lines {
        let line = line.map_err(|_| "rollout_unreadable")?;
        let value = serde_json::from_str::<Value>(&line).map_err(|_| "rollout_damaged")?;
        observation.has_user_event |= is_user_event(&value);
        if first_user_message.is_none() {
            first_user_message = user_message_text(&value);
        }
        observation.has_encrypted_content |= contains_encrypted_content(&value);
    }
    observation.index_metadata = match (
        timestamp,
        cwd,
        cli_version,
        approval_mode,
        sandbox_policy,
        first_user_message,
    ) {
        (
            Some(timestamp),
            Some(cwd),
            Some(cli_version),
            Some(approval_mode),
            Some(sandbox_policy),
            Some(first_user_message),
        ) => Some(IndexInsertMetadata {
            timestamp,
            cwd,
            cli_version,
            approval_mode,
            sandbox_policy,
            first_user_message,
        }),
        _ => None,
    };
    Ok(observation)
}

fn rollout_source_kind(value: &Value) -> Option<&str> {
    if let Some(source) = value.as_str() {
        return Some(source);
    }
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let source = object.keys().next()?.as_str();
    matches!(
        source,
        "subAgent"
            | "subagent"
            | "subAgentReview"
            | "subAgentCompact"
            | "subAgentThreadSpawn"
            | "subAgentOther"
    )
    .then_some(source)
}

fn user_message_text(value: &Value) -> Option<String> {
    let payload = value.get("payload")?;
    match (
        value.get("type").and_then(Value::as_str),
        payload.get("type").and_then(Value::as_str),
        payload.get("role").and_then(Value::as_str),
    ) {
        (Some("event_msg"), Some("user_message"), _) => payload
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        (Some("response_item"), Some("message"), Some("user")) => payload
            .get("content")
            .and_then(Value::as_array)
            .and_then(|content| {
                content
                    .iter()
                    .find_map(|item| item.get("text").and_then(Value::as_str).map(str::to_owned))
            }),
        _ => None,
    }
}

struct IndexFallbackOutcome {
    inserted: BTreeSet<VisibilityThreadView>,
    ineligible: BTreeSet<VisibilityThreadView>,
    failed: bool,
}

fn insert_missing_indexes(
    codex_home: &Path,
    candidates: &[ExecutionCandidate],
    faults: &dyn VisibilityFaultInjector,
) -> IndexFallbackOutcome {
    let missing = candidates
        .iter()
        .filter(|candidate| candidate.missing_index && candidate.index_metadata.is_some())
        .collect::<Vec<_>>();
    let ineligible = candidates
        .iter()
        .filter(|candidate| candidate.missing_index && candidate.index_metadata.is_none())
        .map(ExecutionCandidate::thread_view)
        .collect::<BTreeSet<_>>();
    if missing.is_empty() {
        return IndexFallbackOutcome {
            inserted: BTreeSet::new(),
            ineligible,
            failed: false,
        };
    }
    let failed = || IndexFallbackOutcome {
        inserted: BTreeSet::new(),
        ineligible: ineligible.clone(),
        failed: true,
    };
    let Ok(mut connection) = Connection::open(codex_home.join(STATE_DATABASE)) else {
        return failed();
    };
    let Ok(transaction) = connection.transaction() else {
        return failed();
    };
    for candidate in &missing {
        let reference = SessionVisibilityApplication::session_reference(&candidate.id);
        if faults.fails_at(VisibilityFailurePoint::BeforeIndexInsert, &reference) {
            return failed();
        }
        let Some(metadata) = candidate.index_metadata.as_ref() else {
            return failed();
        };
        if transaction
            .execute(
                "INSERT INTO threads (
                    id, rollout_path, created_at, updated_at, source, model_provider,
                    cwd, title, sandbox_policy, approval_mode, has_user_event, archived,
                    cli_version, first_user_message, thread_source, preview
                ) VALUES (
                    ?1, ?2, CAST(strftime('%s', ?3) AS INTEGER),
                    CAST(strftime('%s', ?3) AS INTEGER), ?4, ?5, ?6, ?7, ?8, ?9,
                    1, ?10, ?11, ?7, 'user', ?7
                )",
                params![
                    candidate.id,
                    candidate.path.to_string_lossy(),
                    metadata.timestamp,
                    candidate.source,
                    candidate.before_provider,
                    metadata.cwd,
                    metadata.first_user_message,
                    metadata.sandbox_policy,
                    metadata.approval_mode,
                    candidate.archived,
                    metadata.cli_version,
                ],
            )
            .is_err()
        {
            return failed();
        }
    }
    if transaction.commit().is_err() {
        return failed();
    }
    IndexFallbackOutcome {
        inserted: missing
            .into_iter()
            .map(|candidate| candidate.thread_view())
            .collect(),
        ineligible,
        failed: false,
    }
}

fn is_user_event(value: &Value) -> bool {
    let item_type = value.get("type").and_then(Value::as_str);
    let payload = value.get("payload");
    matches!(
        (
            item_type,
            payload
                .and_then(|payload| payload.get("type"))
                .and_then(Value::as_str),
            payload
                .and_then(|payload| payload.get("role"))
                .and_then(Value::as_str),
        ),
        (Some("event_msg"), Some("user_message"), _)
            | (Some("response_item"), Some("message"), Some("user"))
    )
}

fn contains_encrypted_content(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "encrypted_content" && !value.is_null()) || contains_encrypted_content(value)
        }),
        Value::Array(values) => values.iter().any(contains_encrypted_content),
        _ => false,
    }
}

fn excluded_source_reason(source: &str, thread_source: Option<&str>) -> Option<&'static str> {
    if thread_source.is_some_and(|source| source != "user") {
        return Some("excluded_internal");
    }
    match source {
        "cli" | "vscode" | "appServer" => None,
        "exec" => Some("excluded_exec"),
        "subAgent"
        | "subagent"
        | "subAgentReview"
        | "subAgentCompact"
        | "subAgentThreadSpawn"
        | "subAgentOther" => Some("excluded_subagent"),
        "mcp" | "automation" | "internal" => Some("excluded_internal"),
        "remote" | "remoteHost" => Some("excluded_remote"),
        _ => Some("identity_ambiguous"),
    }
}

fn collect_rollouts(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), VisibilityFailure> {
    let entries = fs::read_dir(root).map_err(|_| VisibilityFailure {
        message_id: "session_visibility.scan_failed",
        stage: "scan",
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| VisibilityFailure {
            message_id: "session_visibility.scan_failed",
            stage: "scan",
        })?;
        let file_type = entry.file_type().map_err(|_| VisibilityFailure {
            message_id: "session_visibility.scan_failed",
            stage: "scan",
        })?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_rollouts(&entry.path(), paths)?;
        } else if file_type.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn increment(reasons: &mut BTreeMap<String, u32>, code: &str) {
    *reasons.entry(code.to_owned()).or_default() += 1;
}

fn safe_diagnostic_value(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '.' | '-' | '_')
        })
        .take(64)
        .collect()
}

#[derive(Deserialize)]
struct RawSessionMeta<'a> {
    #[serde(borrow)]
    payload: &'a RawValue,
}

#[derive(Deserialize)]
struct RawSessionPayload<'a> {
    #[serde(borrow)]
    model_provider: &'a RawValue,
}

fn replace_rollout_provider(
    path: &Path,
    target_provider: &str,
    expected_hash: &str,
) -> Result<(), VisibilityFailure> {
    let bytes =
        fs::read(path).map_err(|_| visibility_failure("session_visibility.write_failed"))?;
    if bytes_hash(&bytes) != expected_hash {
        return Err(visibility_failure_at(
            "session_visibility.rescan_required",
            "candidate_hash_recheck",
        ));
    }
    let replacement = render_rollout_provider(&bytes, target_provider)?;
    replace_file_atomically(path, &replacement)
}

fn render_rollout_provider(
    bytes: &[u8],
    target_provider: &str,
) -> Result<Vec<u8>, VisibilityFailure> {
    let line_end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(bytes.len());
    let line = std::str::from_utf8(&bytes[..line_end])
        .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
    let meta = serde_json::from_str::<RawSessionMeta<'_>>(line)
        .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
    let payload = serde_json::from_str::<RawSessionPayload<'_>>(meta.payload.get())
        .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
    let _: String = serde_json::from_str(payload.model_provider.get())
        .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
    let value_start = payload.model_provider.get().as_ptr() as usize - line.as_ptr() as usize;
    let value_end = value_start + payload.model_provider.get().len();
    let encoded = serde_json::to_string(target_provider)
        .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
    let mut replacement = Vec::with_capacity(bytes.len() + encoded.len());
    replacement.extend_from_slice(&bytes[..value_start]);
    replacement.extend_from_slice(encoded.as_bytes());
    replacement.extend_from_slice(&bytes[value_end..]);
    Ok(replacement)
}

fn write_new_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), VisibilityFailure> {
    let parent = path
        .parent()
        .ok_or_else(|| visibility_failure("session_visibility.recovery_write_failed"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recovery");
    let temporary = parent.join(format!(".{name}.gpteasy-{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| visibility_failure("session_visibility.recovery_write_failed"))?;
    if file.write_all(bytes).and_then(|_| file.sync_all()).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(visibility_failure(
            "session_visibility.recovery_write_failed",
        ));
    }
    drop(file);
    fs::rename(&temporary, path).map_err(|_| {
        let _ = fs::remove_file(&temporary);
        visibility_failure("session_visibility.recovery_write_failed")
    })
}

fn replace_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), VisibilityFailure> {
    let parent = path
        .parent()
        .ok_or_else(|| visibility_failure("session_visibility.write_failed"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("rollout");
    let temporary = parent.join(format!(".{name}.gpteasy-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
        drop(file);
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temporary, metadata.permissions())
                .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
        }
        atomic_replace(path, &temporary)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn atomic_replace(target: &Path, replacement: &Path) -> Result<(), VisibilityFailure> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        Err(visibility_failure("session_visibility.write_failed"))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(target: &Path, replacement: &Path) -> Result<(), VisibilityFailure> {
    fs::rename(replacement, target)
        .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
    if let Some(parent) = target.parent() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| visibility_failure("session_visibility.write_failed"))?;
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<String, VisibilityFailure> {
    let bytes = fs::read(path).map_err(|_| visibility_failure("session_visibility.scan_failed"))?;
    Ok(bytes_hash(&bytes))
}

fn bytes_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn index_reference(id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-session-index-reference-v1\0");
    hasher.update(id.as_bytes());
    format!("{:x}", hasher.finalize())[..24].to_owned()
}

fn visibility_failure(message_id: &'static str) -> VisibilityFailure {
    let stage = match message_id {
        "session_visibility.scan_failed" => "scan",
        "session_visibility.recovery_unavailable"
        | "session_visibility.recovery_write_failed"
        | "session_visibility.recovery_indeterminate" => "recovery",
        "session_visibility.write_failed" => "rollout_replace",
        "session_visibility.app_server_verification_failed" => "app_server_verify",
        "session_visibility.interrupted" => "rollout_replace",
        _ => "preflight",
    };
    visibility_failure_at(message_id, stage)
}

fn visibility_failure_at(message_id: &'static str, stage: &'static str) -> VisibilityFailure {
    VisibilityFailure { message_id, stage }
}

fn pending_state_failure() -> VisibilityFailure {
    visibility_failure_at("session_visibility.state_unavailable", "pending_state")
}

fn pending_snapshot_matches_target(
    pending: &PendingSessionVisibilitySnapshot,
    target: &VisibilityTarget,
) -> bool {
    pending.environment_revision == target.environment_revision
        && pending.model_provider == target.model_provider
        && matches!(
            (pending.target_mode, target.mode),
            (
                PendingSessionVisibilityTargetMode::Provider,
                VisibilityTargetMode::Provider
            ) | (
                PendingSessionVisibilityTargetMode::OpenaiLogin,
                VisibilityTargetMode::OpenaiLogin
            )
        )
}

fn consumer_error_code(consumer: VisibilityConsumerState) -> &'static str {
    match consumer {
        VisibilityConsumerState::NoConsumers => "none",
        VisibilityConsumerState::DesktopRunning => "session_visibility.desktop_running",
        VisibilityConsumerState::CliRunning => "session_visibility.cli_running",
        VisibilityConsumerState::Unknown => "session_visibility.consumer_unknown",
    }
}

fn coordination_outcome(
    status: VisibilityCoordinationStatus,
    block_codex_restart: bool,
    error_code: &str,
    execution: Option<VisibilityExecutionResult>,
) -> VisibilityCoordinationOutcome {
    VisibilityCoordinationOutcome {
        status,
        block_codex_restart,
        error_code: error_code.to_owned(),
        execution,
    }
}

fn ensure_consumers_stopped(
    consumers: VisibilityConsumerState,
    stage: &'static str,
) -> Result<(), VisibilityFailure> {
    let message_id = match consumers {
        VisibilityConsumerState::NoConsumers => return Ok(()),
        VisibilityConsumerState::DesktopRunning => "session_visibility.desktop_running",
        VisibilityConsumerState::CliRunning => "session_visibility.cli_running",
        VisibilityConsumerState::Unknown => "session_visibility.consumer_unknown",
    };
    Err(visibility_failure_at(message_id, stage))
}

fn indeterminate_execution_result() -> VisibilityExecutionResult {
    VisibilityExecutionResult {
        status: "indeterminate",
        succeeded: 0,
        retryable: 0,
        encrypted_content_risk: 0,
        breakdown: VisibilityExecutionBreakdown {
            app_server_coordinated: 0,
            sqlite_fallback: 0,
            schema_skipped: 0,
            verification_failed: 0,
        },
        block_codex_restart: true,
        message_id: "session_visibility.recovery_indeterminate",
        diagnostic_stage: "recovery",
        error_code: "session_visibility.recovery_indeterminate",
    }
}
