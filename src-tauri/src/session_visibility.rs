use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const STATE_DATABASE: &str = "state_5.sqlite";
const REQUIRED_THREAD_COLUMNS: [&str; 6] = [
    "id",
    "rollout_path",
    "source",
    "model_provider",
    "has_user_event",
    "archived",
];

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
             schema={}; candidates={}; unchanged={}; missing_index={}; skipped={}; \
             blocked={}; encrypted_content_risk={}; active={}; archived={}; \
             error_codes={error_codes}",
            self.schema.status,
            self.summary.candidates,
            self.summary.unchanged,
            self.summary.missing_index,
            self.summary.skipped,
            self.summary.blocked,
            self.summary.encrypted_content_risk,
            self.summary.active,
            self.summary.archived,
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
    pub block_codex_restart: bool,
    pub message_id: &'static str,
    pub diagnostic_stage: &'static str,
    pub error_code: &'static str,
}

impl VisibilityExecutionResult {
    pub fn diagnostic_details(&self) -> String {
        format!(
            "stage={}; status={}; succeeded={}; retryable={}; \
             encrypted_content_risk={}; block_codex_restart={}; error_codes={}",
            self.diagnostic_stage,
            self.status,
            self.succeeded,
            self.retryable,
            self.encrypted_content_risk,
            self.block_codex_restart,
            self.error_code,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityFailurePoint {
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
}

impl SessionVisibilityApplication {
    pub fn new(codex_home: impl AsRef<Path>) -> Self {
        let codex_home = codex_home.as_ref().to_path_buf();
        Self {
            recovery_root: codex_home.join(".gpteasy"),
            codex_home,
            faults: Arc::new(NoVisibilityFaults),
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
        }
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
            &mut summary,
            &mut reasons,
        )?;
        self.scan_directory(
            &self.codex_home.join("archived_sessions"),
            true,
            &context.target.model_provider,
            &index,
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
        Ok(SessionVisibilityPreview {
            confirmation_id,
            target: context.target,
            codex_version: context.codex_version,
            app_server: context.app_server,
            schema: VisibilitySchemaCapability {
                status: index.schema_status,
                database: STATE_DATABASE.to_owned(),
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
        let candidates = self.execution_candidates(&request.target.model_provider, &index)?;
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
        if candidates
            .iter()
            .any(|candidate| !baseline.contains(&candidate.thread_view()))
        {
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
        if self.confirmation_id(&request.target, &refreshed_index)? != request.confirmation_id {
            return Err(visibility_failure_at(
                "session_visibility.rescan_required",
                "index_recheck",
            ));
        }

        let mut manifest = VisibilityRecoveryManifest::new(
            &request.target,
            &candidates,
            self.index_database_hash(),
        )?;
        self.write_manifest(&manifest)?;
        let mut written = Vec::new();
        let mut retryable = 0;
        let mut write_failed = false;
        let mut rescan_required = false;
        let encrypted_content_risk = candidates
            .iter()
            .filter(|candidate| candidate.has_encrypted_content)
            .count() as u32;
        for (index, candidate) in candidates.into_iter().enumerate() {
            let reference = &manifest.items[index].session_reference;
            if self
                .faults
                .fails_at(VisibilityFailurePoint::BeforeRolloutReplace, reference)
            {
                manifest.items[index].stage = "write_failed".to_owned();
                self.write_manifest(&manifest)?;
                retryable += 1;
                write_failed = true;
                continue;
            }
            match replace_rollout_provider(
                &candidate.path,
                &request.target.model_provider,
                &manifest.items[index].before_hash,
            ) {
                Ok(()) => {
                    if self
                        .faults
                        .fails_at(VisibilityFailurePoint::AfterRolloutReplace, reference)
                    {
                        return Err(visibility_failure("session_visibility.interrupted"));
                    }
                    manifest.items[index].stage = "written".to_owned();
                    self.write_manifest(&manifest)?;
                    written.push((index, candidate));
                }
                Err(failure) if failure.message_id == "session_visibility.rescan_required" => {
                    manifest.items[index].stage = "external_change".to_owned();
                    for item in manifest.items.iter_mut().skip(index + 1) {
                        item.stage = "not_attempted".to_owned();
                    }
                    self.write_manifest(&manifest)?;
                    retryable += (manifest.items.len() - index) as u32;
                    rescan_required = true;
                    break;
                }
                Err(_) => {
                    manifest.items[index].stage = "write_failed".to_owned();
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
                for (index, _) in &written {
                    manifest.items[*index].stage = "verification_failed".to_owned();
                }
                manifest.stage = "partial".to_owned();
                manifest.index_database_after_hash = self.index_database_hash();
                self.write_manifest(&manifest)?;
                return Ok(VisibilityExecutionResult {
                    status: "failed",
                    succeeded: 0,
                    retryable,
                    encrypted_content_risk,
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
        let expected_target = baseline_target
            .union(&written_target)
            .cloned()
            .collect::<BTreeSet<_>>();
        let invariants_hold = all_after_count == baseline_count
            && all_after.len() == all_after_count
            && target_after.len() == target_after_count
            && target_after_count == expected_target.len()
            && all_after == baseline
            && target_after == expected_target;
        let succeeded = written
            .iter()
            .filter(|(_, candidate)| {
                invariants_hold && target_after.contains(&candidate.thread_view())
            })
            .count() as u32;
        retryable += written.len() as u32 - succeeded;
        for (index, candidate) in &written {
            manifest.items[*index].stage =
                if invariants_hold && target_after.contains(&candidate.thread_view()) {
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
            } else if write_failed {
                "session_visibility.write_failed"
            } else {
                "session_visibility.repair_failed"
            },
        })
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
        let observed = self.observed_rollout_hashes()?;
        let mut retryable = 0;
        let mut indeterminate = false;
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
                let Some(indexed) = index.rows.get(&rollout.id) else {
                    continue;
                };
                if !same_rollout_path(&indexed.rollout_path, &path)
                    || indexed.source != source
                    || indexed.archived != archived
                    || !indexed.has_user_event
                {
                    continue;
                }
                if rollout.model_provider.as_deref() != Some(target_provider)
                    || indexed.model_provider != target_provider
                {
                    candidates.push(ExecutionCandidate {
                        id: rollout.id,
                        path,
                        archived,
                        before_provider,
                        has_encrypted_content: rollout.has_encrypted_content,
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
                if rollout.has_encrypted_content {
                    summary.encrypted_content_risk += 1;
                    increment(reasons, "encrypted_content");
                }
                continue;
            };
            if !same_rollout_path(&indexed.rollout_path, &path)
                || indexed.source != source
                || indexed.archived != archived
                || !indexed.has_user_event
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
    before_provider: String,
    has_encrypted_content: bool,
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
                    stage: "prepared".to_owned(),
                })
            })
            .collect::<Result<Vec<_>, VisibilityFailure>>()?;
        Ok(Self {
            schema_version: 1,
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
    stage: String,
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

struct IndexedThread {
    rollout_path: PathBuf,
    source: String,
    model_provider: String,
    has_user_event: bool,
    archived: bool,
}

struct IndexSnapshot {
    schema_status: String,
    rows: HashMap<String, IndexedThread>,
}

fn read_index(codex_home: &Path) -> IndexSnapshot {
    let database = codex_home.join(STATE_DATABASE);
    let Ok(connection) = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return empty_index("missing");
    };
    let Ok(columns) = thread_columns(&connection) else {
        return empty_index("unknown");
    };
    if !REQUIRED_THREAD_COLUMNS
        .iter()
        .all(|required| columns.iter().any(|column| column == required))
    {
        return empty_index("unknown");
    }
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
        rows,
    }
}

fn empty_index(schema_status: &str) -> IndexSnapshot {
    IndexSnapshot {
        schema_status: schema_status.to_owned(),
        rows: HashMap::new(),
    }
}

fn thread_columns(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare("PRAGMA table_info(threads)")?;
    statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect()
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
        .and_then(Value::as_str)
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
    };
    for line in lines {
        let line = line.map_err(|_| "rollout_unreadable")?;
        let value = serde_json::from_str::<Value>(&line).map_err(|_| "rollout_damaged")?;
        observation.has_user_event |= is_user_event(&value);
        observation.has_encrypted_content |= contains_encrypted_content(&value);
    }
    Ok(observation)
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
        block_codex_restart: true,
        message_id: "session_visibility.recovery_indeterminate",
        diagnostic_stage: "recovery",
        error_code: "session_visibility.recovery_indeterminate",
    }
}
