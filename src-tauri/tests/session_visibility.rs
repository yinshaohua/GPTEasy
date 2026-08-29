use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpteasy_lib::session_visibility::{
    SessionVisibilityApplication, VisibilityAppServerCapability, VisibilityConsumerState,
    VisibilityExecutionRequest, VisibilityExecutionRuntime, VisibilityFailurePoint,
    VisibilityFaultInjector, VisibilityScanContext, VisibilityTarget, VisibilityTargetMode,
    VisibilityThreadView,
};
use rusqlite::{Connection, params};
use serde_json::json;
use tempfile::TempDir;

const TARGET_PROVIDER: &str = "4c8f7402-669f-40cf-a2a9-cfc6f124de6d";

#[tokio::test]
async fn confirmed_repair_atomically_changes_only_the_runtime_provider_and_verifies_a_clean_view() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.rollout(
        "sessions/2026/08/29/candidate.jsonl",
        "11111111-1111-4111-8111-111111111111",
        "old-provider",
        "cli",
        true,
        true,
    );
    fixture.index(&rollout, "old-provider", "cli", true, false);
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-atomic"))
        .expect("scan visibility");
    let before = fs::read(&rollout).expect("read rollout before repair");
    let runtime = VisibilityRuntimeFixture::new(fixture.codex_home());

    let result = application
        .execute(
            VisibilityExecutionRequest {
                confirmation_id: preview.confirmation_id.clone(),
                target: preview.target.clone(),
            },
            &runtime,
        )
        .await
        .expect("execute visibility repair");

    assert_eq!(result.status, "complete", "{result:?}");
    assert_eq!(result.succeeded, 1);
    assert_eq!(result.retryable, 0);
    assert!(!result.block_codex_restart);
    assert_eq!(runtime.shutdowns(), 1);
    assert_eq!(
        runtime.starts(),
        2,
        "baseline and post-write verification use clean servers"
    );
    let after = fs::read(&rollout).expect("read rollout after repair");
    assert_only_model_provider_changed(&before, &after, TARGET_PROVIDER);
}

#[tokio::test]
async fn execution_rejects_changed_candidates_consumers_and_revision_without_writing() {
    for (consumer, expected_message) in [
        (
            VisibilityConsumerState::CliRunning,
            "session_visibility.cli_running",
        ),
        (
            VisibilityConsumerState::Unknown,
            "session_visibility.consumer_unknown",
        ),
    ] {
        let fixture = VisibilityFixture::new();
        let rollout = fixture.indexed_candidate("consumer.jsonl");
        let application =
            SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
        let preview = application
            .scan(executable_context("revision-gate"))
            .expect("scan visibility");
        let before = fs::read(&rollout).expect("read rollout before gate");
        let runtime = VisibilityRuntimeFixture::with_state(
            fixture.codex_home(),
            consumer,
            "revision-gate",
            None,
        );

        let failure = application
            .execute(execution_request(&preview), &runtime)
            .await
            .expect_err("consumer blocks repair");

        assert_eq!(failure.message_id, expected_message);
        assert_eq!(fs::read(&rollout).expect("read rollout after gate"), before);
        assert_eq!(runtime.shutdowns(), 0);
        assert_eq!(
            runtime.starts(),
            0,
            "consumer gate runs before App Server scans"
        );
    }

    let fixture = VisibilityFixture::new();
    let rollout = fixture.indexed_candidate("revision.jsonl");
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-before"))
        .expect("scan visibility");
    let before = fs::read(&rollout).expect("read rollout before revision race");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-before",
        Some("revision-after"),
    );

    let failure = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect_err("revision race blocks repair");

    assert_eq!(failure.message_id, "session_visibility.rescan_required");
    assert_eq!(
        fs::read(&rollout).expect("read rollout after revision race"),
        before
    );
}

#[tokio::test]
async fn execution_rejects_a_candidate_identity_change_before_stopping_the_app_server() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.indexed_candidate("identity.jsonl");
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-identity"))
        .expect("scan visibility");
    let changed = fs::read_to_string(&rollout)
        .expect("read rollout")
        .replacen("\"source\":\"cli\"", "\"source\":\"exec\"", 1);
    fs::write(&rollout, &changed).expect("change candidate identity");
    let runtime = VisibilityRuntimeFixture::new(fixture.codex_home());

    let failure = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect_err("changed identity requires another scan");

    assert_eq!(failure.message_id, "session_visibility.rescan_required");
    assert_eq!(runtime.shutdowns(), 0);
    assert!(changed.contains("\"model_provider\":\"old-provider\""));
}

#[tokio::test]
async fn rollout_failures_are_isolated_and_the_latest_manifest_is_compact_and_redacted() {
    let fixture = VisibilityFixture::new();
    let first =
        fixture.indexed_rollout("first.jsonl", "11111111-1111-4111-8111-111111111111", false);
    let second =
        fixture.indexed_rollout("second.jsonl", "22222222-2222-4222-8222-222222222222", true);
    let faults =
        FailingVisibilityWrites::before_replace_for("22222222-2222-4222-8222-222222222222");
    let application = SessionVisibilityApplication::with_fault_injector(
        fixture.codex_home(),
        fixture.root(),
        faults,
    );
    let preview = application
        .scan(executable_context("revision-partial"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-partial",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("execute partial repair");

    assert_eq!(result.status, "partial");
    assert_eq!((result.succeeded, result.retryable), (1, 1));
    assert_eq!(result.diagnostic_stage, "rollout_replace");
    assert_eq!(result.error_code, "session_visibility.write_failed");
    assert_eq!(provider_in_rollout(&first), TARGET_PROVIDER);
    assert_eq!(provider_in_rollout(&second), "old-provider");
    let manifest_path = fixture.root().join("session-visibility-recovery.json");
    let manifest = fs::read_to_string(&manifest_path).expect("read recovery manifest");
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest).expect("parse manifest");
    assert_eq!(manifest_json["items"].as_array().map(Vec::len), Some(2));
    for sensitive in [
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
        "private-title",
        "C:\\\\private\\\\workspace",
        "encrypted-secret",
    ] {
        assert!(!manifest.contains(sensitive), "manifest leaked {sensitive}");
    }
    assert_eq!(
        fs::read_dir(fixture.root())
            .expect("list recovery root")
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("session-visibility-recovery"))
            .count(),
        1,
    );
}

#[tokio::test]
async fn missing_index_sessions_remain_preview_only_for_the_follow_up_index_feature() {
    let fixture = VisibilityFixture::new();
    fixture.indexed_candidate("indexed.jsonl");
    fixture.rollout(
        "sessions/2026/08/29/missing-index.jsonl",
        "33333333-3333-4333-8333-333333333333",
        TARGET_PROVIDER,
        "cli",
        true,
        false,
    );
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-missing-index"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-missing-index",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("repair indexed sessions");

    assert_eq!(preview.summary.missing_index, 1);
    assert_eq!(result.status, "complete");
    assert_eq!((result.succeeded, result.retryable), (1, 0));
}

#[tokio::test]
async fn repair_rechecks_the_manifest_hash_before_each_atomic_replace() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.indexed_candidate("concurrent-change.jsonl");
    let untouched = fixture.indexed_rollout(
        "not-attempted.jsonl",
        "22222222-2222-4222-8222-222222222222",
        false,
    );
    let application = SessionVisibilityApplication::with_fault_injector(
        fixture.codex_home(),
        fixture.root(),
        ChangingVisibilityRollout::new(&rollout),
    );
    let preview = application
        .scan(executable_context("revision-concurrent-change"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-concurrent-change",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("concurrent rollout change remains retryable");

    assert_eq!(result.status, "failed");
    assert_eq!((result.succeeded, result.retryable), (0, 2));
    assert_eq!(result.message_id, "session_visibility.rescan_required");
    assert_eq!(result.diagnostic_stage, "candidate_hash_recheck");
    assert_eq!(provider_in_rollout(&rollout), "external-provider");
    assert_eq!(provider_in_rollout(&untouched), "old-provider");
    let recovery = application
        .assess_recovery()
        .expect("external change is a determinate retryable state");
    assert_eq!(recovery.status, "retryable");
    assert!(!recovery.block_codex_restart);
}

#[tokio::test]
async fn an_invalid_provider_field_is_skipped_without_aborting_other_sessions() {
    let fixture = VisibilityFixture::new();
    let valid = fixture.indexed_candidate("valid.jsonl");
    let invalid = fixture.indexed_rollout(
        "invalid-provider.jsonl",
        "22222222-2222-4222-8222-222222222222",
        false,
    );
    let changed = fs::read_to_string(&invalid)
        .expect("read invalid provider fixture")
        .replacen("\"model_provider\":\"old-provider\",", "", 1);
    fs::write(&invalid, changed).expect("remove provider field");
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-invalid-provider"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-invalid-provider",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("repair valid session");

    assert_eq!(result.status, "complete");
    assert_eq!((result.succeeded, result.retryable), (1, 0));
    assert_eq!(provider_in_rollout(&valid), TARGET_PROVIDER);
    assert_reason(&preview.reasons, "model_provider_invalid", 1);
}

#[tokio::test]
async fn an_interrupted_replace_is_retryable_until_the_file_matches_neither_known_hash() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.indexed_candidate("interrupted.jsonl");
    let application = SessionVisibilityApplication::with_fault_injector(
        fixture.codex_home(),
        fixture.root(),
        FailingVisibilityWrites::after_replace_once(),
    );
    let preview = application
        .scan(executable_context("revision-interrupted"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-interrupted",
        None,
    );

    let failure = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect_err("fault simulates an interrupted process");

    assert_eq!(failure.message_id, "session_visibility.interrupted");
    assert_eq!(provider_in_rollout(&rollout), TARGET_PROVIDER);
    let assessment = application
        .assess_recovery()
        .expect("assess known post-write state");
    assert_eq!(assessment.status, "retryable");
    assert_eq!(assessment.retryable, 1);
    assert!(!assessment.block_codex_restart);

    fs::write(&rollout, b"externally changed after interruption\n").expect("tamper rollout");
    let assessment = application.assess_recovery().expect("assess unknown state");
    assert_eq!(assessment.status, "indeterminate");
    assert!(assessment.block_codex_restart);

    let next = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("public execution reports an indeterminate recovery state");
    assert_eq!(next.status, "indeterminate");
    assert!(next.block_codex_restart);
    assert_eq!(
        runtime.shutdowns(),
        1,
        "unknown recovery stops before another shutdown"
    );
}

#[tokio::test]
async fn a_verified_manifest_does_not_claim_later_external_changes_are_an_interruption() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.indexed_candidate("verified.jsonl");
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-verified"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-verified",
        None,
    );
    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("complete repair");
    assert_eq!(result.status, "complete");

    fs::write(&rollout, b"later external change\n").expect("change a completed rollout");
    let assessment = application
        .assess_recovery()
        .expect("assess completed manifest");

    assert_eq!(assessment.status, "complete");
    assert!(!assessment.block_codex_restart);
}

#[tokio::test]
async fn app_server_filter_and_global_invariant_failures_remain_retryable_and_diagnostics_are_redacted()
 {
    for verification in [
        VerificationFixture::MissingTarget,
        VerificationFixture::ArchiveChanged,
        VerificationFixture::AppServerUnavailable,
    ] {
        let fixture = VisibilityFixture::new();
        let rollout = fixture.indexed_candidate("verification.jsonl");
        let application =
            SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
        let preview = application
            .scan(executable_context("private-revision"))
            .expect("scan visibility");
        let runtime = VisibilityRuntimeFixture::with_state(
            fixture.codex_home(),
            VisibilityConsumerState::NoConsumers,
            "private-revision",
            None,
        )
        .with_verification(verification);

        let result = application
            .execute(execution_request(&preview), &runtime)
            .await
            .expect("verification produces a determinate retryable result");

        assert_eq!(result.status, "failed");
        assert_eq!((result.succeeded, result.retryable), (0, 1));
        assert!(!result.block_codex_restart);
        assert_eq!(provider_in_rollout(&rollout), TARGET_PROVIDER);
        let diagnostics = result.diagnostic_details();
        assert!(diagnostics.contains("stage=app_server_verify"));
        assert!(
            diagnostics.contains("session_visibility.app_server_verification_failed")
                || diagnostics.contains("session_visibility.verification_invariant_failed")
        );
        for sensitive in [
            "11111111-1111-4111-8111-111111111111",
            TARGET_PROVIDER,
            "private-revision",
            "private-title",
            "encrypted-secret",
            r"C:\private\workspace",
        ] {
            assert!(
                !diagnostics.contains(sensitive),
                "diagnostics leaked {sensitive}"
            );
        }
    }
}

#[tokio::test]
async fn verification_preserves_the_existing_target_provider_filter_view() {
    let fixture = VisibilityFixture::new();
    fixture.indexed_candidate("candidate.jsonl");
    fixture.indexed_rollout_with_provider(
        "existing-target.jsonl",
        "00000000-0000-4000-8000-000000000000",
        TARGET_PROVIDER,
    );
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-filter-baseline"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-filter-baseline",
        None,
    )
    .with_verification(VerificationFixture::MissingExistingTarget);

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("filter invariant failure is determinate");

    assert_eq!(result.status, "failed");
    assert_eq!((result.succeeded, result.retryable), (0, 1));
}

fn execution_request(
    preview: &gpteasy_lib::session_visibility::SessionVisibilityPreview,
) -> VisibilityExecutionRequest {
    VisibilityExecutionRequest {
        confirmation_id: preview.confirmation_id.clone(),
        target: preview.target.clone(),
    }
}

fn executable_context(revision: &str) -> VisibilityScanContext {
    VisibilityScanContext {
        target: VisibilityTarget {
            mode: VisibilityTargetMode::Provider,
            model_provider: TARGET_PROVIDER.to_owned(),
            environment_revision: revision.to_owned(),
        },
        codex_version: Some("codex-cli fixture".to_owned()),
        app_server: VisibilityAppServerCapability::Available,
        execution_blockers: Vec::new(),
    }
}

struct VisibilityRuntimeFixture {
    codex_home: PathBuf,
    shutdowns: AtomicUsize,
    starts: AtomicUsize,
    consumer: VisibilityConsumerState,
    revision: Mutex<String>,
    revision_after_shutdown: Option<String>,
    verification: VerificationFixture,
}

#[derive(Clone, Copy)]
enum VerificationFixture {
    Normal,
    MissingTarget,
    MissingExistingTarget,
    ArchiveChanged,
    AppServerUnavailable,
}

impl VisibilityRuntimeFixture {
    fn new(codex_home: &Path) -> Self {
        Self::with_state(
            codex_home,
            VisibilityConsumerState::NoConsumers,
            "revision-atomic",
            None,
        )
    }

    fn with_state(
        codex_home: &Path,
        consumer: VisibilityConsumerState,
        revision: &str,
        revision_after_shutdown: Option<&str>,
    ) -> Self {
        Self {
            codex_home: codex_home.to_path_buf(),
            shutdowns: AtomicUsize::new(0),
            starts: AtomicUsize::new(0),
            consumer,
            revision: Mutex::new(revision.to_owned()),
            revision_after_shutdown: revision_after_shutdown.map(str::to_owned),
            verification: VerificationFixture::Normal,
        }
    }

    fn with_verification(mut self, verification: VerificationFixture) -> Self {
        self.verification = verification;
        self
    }

    fn shutdowns(&self) -> usize {
        self.shutdowns.load(Ordering::SeqCst)
    }

    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    fn all_threads(&self) -> Vec<VisibilityThreadView> {
        let connection = Connection::open(self.codex_home.join("state_5.sqlite"))
            .expect("open visibility index");
        let mut statement = connection
            .prepare("SELECT id, archived FROM threads ORDER BY id")
            .expect("prepare visibility view");
        statement
            .query_map([], |row| {
                Ok(VisibilityThreadView {
                    id: row.get(0)?,
                    archived: row.get(1)?,
                })
            })
            .expect("query visibility view")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect visibility view")
    }

    fn target_threads(&self, target_provider: &str) -> Vec<VisibilityThreadView> {
        let connection = Connection::open(self.codex_home.join("state_5.sqlite"))
            .expect("open visibility index");
        self.all_threads()
            .into_iter()
            .filter(|thread| {
                let path: String = connection
                    .query_row(
                        "SELECT rollout_path FROM threads WHERE id = ?1",
                        [&thread.id],
                        |row| row.get(0),
                    )
                    .expect("read rollout path");
                let contents =
                    fs::read_to_string(path).expect("read rollout for App Server fixture");
                let line = contents.lines().next().expect("session meta");
                serde_json::from_str::<serde_json::Value>(line)
                    .expect("parse session meta")
                    .pointer("/payload/model_provider")
                    .and_then(serde_json::Value::as_str)
                    == Some(target_provider)
            })
            .collect()
    }
}

impl VisibilityExecutionRuntime for VisibilityRuntimeFixture {
    fn current_target(
        &self,
    ) -> Result<VisibilityTarget, gpteasy_lib::session_visibility::VisibilityFailure> {
        Ok(VisibilityTarget {
            mode: VisibilityTargetMode::Provider,
            model_provider: TARGET_PROVIDER.to_owned(),
            environment_revision: self.revision.lock().expect("revision lock").clone(),
        })
    }

    fn baseline_views<'a>(
        &'a self,
        target_provider: &'a str,
    ) -> gpteasy_lib::session_visibility::VisibilityRuntimeFuture<
        'a,
        gpteasy_lib::session_visibility::VisibilityVerificationViews,
    > {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(
                gpteasy_lib::session_visibility::VisibilityVerificationViews {
                    all_providers: self.all_threads(),
                    target_provider: self.target_threads(target_provider),
                },
            )
        })
    }

    fn shutdown_owned_app_server(
        &self,
    ) -> gpteasy_lib::session_visibility::VisibilityRuntimeFuture<'_, ()> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        if let Some(revision) = &self.revision_after_shutdown {
            *self.revision.lock().expect("revision lock") = revision.clone();
        }
        Box::pin(async { Ok(()) })
    }

    fn consumers(&self, _exclude_owned_app_server: bool) -> VisibilityConsumerState {
        self.consumer
    }

    fn verification_views<'a>(
        &'a self,
        target_provider: &'a str,
    ) -> gpteasy_lib::session_visibility::VisibilityRuntimeFuture<
        'a,
        gpteasy_lib::session_visibility::VisibilityVerificationViews,
    > {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if matches!(self.verification, VerificationFixture::AppServerUnavailable) {
                return Err(gpteasy_lib::session_visibility::VisibilityFailure {
                    message_id: "session_visibility.app_server_verification_failed",
                    stage: "fixture_verification",
                });
            }
            let mut all_providers = self.all_threads();
            let mut target_provider = self.target_threads(target_provider);
            match self.verification {
                VerificationFixture::Normal => {}
                VerificationFixture::MissingTarget => target_provider.clear(),
                VerificationFixture::MissingExistingTarget => {
                    target_provider
                        .retain(|thread| thread.id != "00000000-0000-4000-8000-000000000000");
                }
                VerificationFixture::ArchiveChanged => {
                    if let Some(thread) = all_providers.first_mut() {
                        thread.archived = !thread.archived;
                    }
                }
                VerificationFixture::AppServerUnavailable => unreachable!(),
            }
            Ok(
                gpteasy_lib::session_visibility::VisibilityVerificationViews {
                    all_providers,
                    target_provider,
                },
            )
        })
    }
}

fn assert_only_model_provider_changed(before: &[u8], after: &[u8], target_provider: &str) {
    let before = String::from_utf8(before.to_vec()).expect("rollout utf8");
    let expected = before.replacen(
        "\"model_provider\":\"old-provider\"",
        &format!("\"model_provider\":\"{target_provider}\""),
        1,
    );
    assert_ne!(
        expected, before,
        "fixture contains the original provider field"
    );
    assert_eq!(
        String::from_utf8(after.to_vec()).expect("rollout utf8"),
        expected
    );
}

fn provider_in_rollout(path: &Path) -> String {
    let contents = fs::read_to_string(path).expect("read rollout provider");
    serde_json::from_str::<serde_json::Value>(contents.lines().next().expect("session meta"))
        .expect("parse session meta")
        .pointer("/payload/model_provider")
        .and_then(serde_json::Value::as_str)
        .expect("model provider")
        .to_owned()
}

struct FailingVisibilityWrites {
    point: VisibilityFailurePoint,
    session_reference: Option<String>,
    remaining: AtomicUsize,
}

struct ChangingVisibilityRollout {
    path: PathBuf,
    remaining: AtomicUsize,
}

impl ChangingVisibilityRollout {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            remaining: AtomicUsize::new(1),
        }
    }
}

impl VisibilityFaultInjector for ChangingVisibilityRollout {
    fn fails_at(&self, point: VisibilityFailurePoint, _session_reference: &str) -> bool {
        if point == VisibilityFailurePoint::BeforeRolloutReplace
            && self
                .remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
        {
            let changed = fs::read_to_string(&self.path)
                .expect("read concurrent rollout")
                .replacen(
                    "\"model_provider\":\"old-provider\"",
                    "\"model_provider\":\"external-provider\"",
                    1,
                );
            fs::write(&self.path, changed).expect("write concurrent rollout");
        }
        false
    }
}

impl FailingVisibilityWrites {
    fn before_replace_for(id: &str) -> Self {
        Self {
            point: VisibilityFailurePoint::BeforeRolloutReplace,
            session_reference: Some(SessionVisibilityApplication::session_reference(id)),
            remaining: AtomicUsize::new(1),
        }
    }

    fn after_replace_once() -> Self {
        Self {
            point: VisibilityFailurePoint::AfterRolloutReplace,
            session_reference: None,
            remaining: AtomicUsize::new(1),
        }
    }
}

impl VisibilityFaultInjector for FailingVisibilityWrites {
    fn fails_at(&self, point: VisibilityFailurePoint, session_reference: &str) -> bool {
        self.point == point
            && self
                .session_reference
                .as_deref()
                .is_none_or(|expected| expected == session_reference)
            && self
                .remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
    }
}

#[test]
fn read_only_scan_classifies_active_archived_missing_internal_and_encrypted_rollouts() {
    let fixture = VisibilityFixture::new();
    let candidate = fixture.rollout(
        "sessions/2026/08/29/candidate.jsonl",
        "11111111-1111-4111-8111-111111111111",
        "old-provider",
        "cli",
        true,
        true,
    );
    fixture.index(&candidate, "old-provider", "cli", true, false);
    let unchanged = fixture.rollout(
        "archived_sessions/unchanged.jsonl",
        "22222222-2222-4222-8222-222222222222",
        TARGET_PROVIDER,
        "vscode",
        true,
        false,
    );
    fixture.index(&unchanged, TARGET_PROVIDER, "vscode", true, true);
    fixture.rollout(
        "sessions/2026/08/29/missing-index.jsonl",
        "33333333-3333-4333-8333-333333333333",
        TARGET_PROVIDER,
        "cli",
        true,
        false,
    );
    let exec = fixture.rollout(
        "sessions/2026/08/29/exec.jsonl",
        "44444444-4444-4444-8444-444444444444",
        "old-provider",
        "exec",
        true,
        false,
    );
    fixture.index(&exec, "old-provider", "exec", true, false);
    fixture.rollout_without_identity("sessions/2026/08/29/ambiguous.jsonl", "private-title");
    let before = fixture.snapshot_bytes();

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(VisibilityScanContext {
            target: VisibilityTarget {
                mode: VisibilityTargetMode::Provider,
                model_provider: TARGET_PROVIDER.to_owned(),
                environment_revision: "revision-42".to_owned(),
            },
            codex_version: Some("codex-cli 0.150.1".to_owned()),
            app_server: VisibilityAppServerCapability::Available,
            execution_blockers: Vec::new(),
        })
        .expect("scan visibility");

    assert_eq!(preview.target.model_provider, TARGET_PROVIDER);
    assert_eq!(preview.target.environment_revision, "revision-42");
    assert_eq!(preview.summary.candidates, 1);
    assert_eq!(preview.summary.unchanged, 1);
    assert_eq!(preview.summary.missing_index, 1);
    assert_eq!(preview.summary.skipped, 2);
    assert_eq!(preview.summary.blocked, 0);
    assert_eq!(preview.summary.encrypted_content_risk, 1);
    assert_eq!(preview.summary.active, 4);
    assert_eq!(preview.summary.archived, 1);
    assert!(preview.can_execute);
    assert_eq!(preview.schema.status, "supported");
    assert_reason(&preview.reasons, "provider_mismatch", 1);
    assert_reason(&preview.reasons, "index_missing", 1);
    assert_reason(&preview.reasons, "excluded_exec", 1);
    assert_reason(&preview.reasons, "identity_ambiguous", 1);
    assert_reason(&preview.reasons, "encrypted_content", 1);

    let serialized = serde_json::to_string(&preview).expect("serialize preview");
    for sensitive in [
        "private-title",
        "opaque-encrypted-body",
        "C:\\private\\workspace",
        "11111111-1111-4111-8111-111111111111",
    ] {
        assert!(
            !serialized.contains(sensitive),
            "preview leaked {sensitive}"
        );
    }
    assert_eq!(
        fixture.snapshot_bytes(),
        before,
        "scan must not write Codex data"
    );
}

#[test]
fn scan_remains_available_but_marks_environment_schema_and_app_server_blockers() {
    let fixture = VisibilityFixture::new();
    let connection = Connection::open(fixture.codex_home.join("state_5.sqlite"))
        .expect("open Codex state fixture");
    connection
        .execute_batch("DROP TABLE threads; CREATE TABLE threads (id TEXT PRIMARY KEY);")
        .expect("replace with unknown schema");
    fixture.rollout(
        "archived_sessions/blocked.jsonl",
        "55555555-5555-4555-8555-555555555555",
        "old-provider",
        "cli",
        true,
        false,
    );

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(VisibilityScanContext {
            target: VisibilityTarget {
                mode: VisibilityTargetMode::OpenaiLogin,
                model_provider: "openai".to_owned(),
                environment_revision: "revision-openai".to_owned(),
            },
            codex_version: Some("codex-cli 0.150.1".to_owned()),
            app_server: VisibilityAppServerCapability::Unavailable,
            execution_blockers: vec![
                "external_configuration".to_owned(),
                "pending_config_operation".to_owned(),
            ],
        })
        .expect("scan blocked visibility");

    assert!(!preview.can_execute);
    assert_eq!(preview.schema.status, "unknown");
    assert_eq!(preview.summary.candidates, 0);
    assert_eq!(preview.summary.missing_index, 0);
    assert_eq!(preview.summary.blocked, 1);
    assert_eq!(
        preview.blockers,
        [
            "app_server_unavailable",
            "external_configuration",
            "pending_config_operation",
            "unsupported_index_schema",
        ],
    );

    let details = preview.diagnostic_details();
    for required in [
        "stage=scan",
        "target_mode=openai_login",
        "codex_version=codex-cli 0.150.1",
        "schema=unknown",
        "candidates=0",
        "missing_index=0",
        "blocked=1",
        "error_codes=app_server_unavailable,external_configuration,pending_config_operation,unsupported_index_schema",
    ] {
        assert!(details.contains(required), "missing {required}: {details}");
    }
    for sensitive in [
        "old-provider",
        "55555555-5555-4555-8555-555555555555",
        "C:\\private\\workspace",
        "private-title",
    ] {
        assert!(
            !details.contains(sensitive),
            "diagnostic leaked {sensitive}"
        );
    }
}

#[test]
fn diagnostics_include_stable_scan_reason_codes() {
    let fixture = VisibilityFixture::new();
    let candidate = fixture.rollout(
        "sessions/candidate.jsonl",
        "77777777-7777-4777-8777-777777777777",
        "old-provider",
        "cli",
        true,
        true,
    );
    fixture.index(&candidate, "old-provider", "cli", true, false);
    fixture.rollout_without_identity("sessions/ambiguous.jsonl", "private-title");

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(VisibilityScanContext {
            target: VisibilityTarget {
                mode: VisibilityTargetMode::Provider,
                model_provider: TARGET_PROVIDER.to_owned(),
                environment_revision: "revision-diagnostics".to_owned(),
            },
            codex_version: None,
            app_server: VisibilityAppServerCapability::Available,
            execution_blockers: Vec::new(),
        })
        .expect("scan diagnostic reasons");

    let details = preview.diagnostic_details();
    assert!(details.contains("error_codes=encrypted_content,identity_ambiguous,provider_mismatch"));
    assert!(!details.contains("private-title"));
    assert!(!details.contains("77777777-7777-4777-8777-777777777777"));
}

#[test]
fn scan_excludes_every_known_non_interactive_source_kind() {
    let fixture = VisibilityFixture::new();
    let excluded = [
        ("exec", "excluded_exec"),
        ("automation", "excluded_internal"),
        ("internal", "excluded_internal"),
        ("remoteHost", "excluded_remote"),
        ("subAgent", "excluded_subagent"),
        ("subAgentReview", "excluded_subagent"),
        ("subAgentCompact", "excluded_subagent"),
        ("subAgentThreadSpawn", "excluded_subagent"),
        ("subAgentOther", "excluded_subagent"),
    ];
    for (index, (source, _)) in excluded.iter().enumerate() {
        fixture.rollout(
            &format!("sessions/excluded-{index}.jsonl"),
            &format!("{index:08x}-1111-4111-8111-111111111111"),
            "old-provider",
            source,
            true,
            false,
        );
    }

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(VisibilityScanContext {
            target: VisibilityTarget {
                mode: VisibilityTargetMode::Provider,
                model_provider: TARGET_PROVIDER.to_owned(),
                environment_revision: "revision-sources".to_owned(),
            },
            codex_version: None,
            app_server: VisibilityAppServerCapability::Available,
            execution_blockers: Vec::new(),
        })
        .expect("scan excluded sources");

    assert_eq!(preview.summary.candidates, 0);
    assert_eq!(preview.summary.skipped, excluded.len() as u32);
    assert_reason(&preview.reasons, "excluded_exec", 1);
    assert_reason(&preview.reasons, "excluded_internal", 2);
    assert_reason(&preview.reasons, "excluded_remote", 1);
    assert_reason(&preview.reasons, "excluded_subagent", 5);
}

#[test]
fn scan_skips_an_index_row_that_points_at_another_rollout() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.rollout(
        "sessions/path-mismatch.jsonl",
        "66666666-6666-4666-8666-666666666666",
        "old-provider",
        "cli",
        true,
        false,
    );
    fixture.index_with_rollout_path(
        &rollout,
        &fixture.codex_home.join("sessions/another-rollout.jsonl"),
        "old-provider",
        "cli",
        true,
        false,
    );

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(VisibilityScanContext {
            target: VisibilityTarget {
                mode: VisibilityTargetMode::Provider,
                model_provider: TARGET_PROVIDER.to_owned(),
                environment_revision: "revision-path".to_owned(),
            },
            codex_version: None,
            app_server: VisibilityAppServerCapability::Available,
            execution_blockers: Vec::new(),
        })
        .expect("scan mismatched index path");

    assert_eq!(preview.summary.candidates, 0);
    assert_eq!(preview.summary.skipped, 1);
    assert_reason(&preview.reasons, "identity_ambiguous", 1);
}

fn assert_reason(
    reasons: &[gpteasy_lib::session_visibility::VisibilityReason],
    code: &str,
    count: u32,
) {
    assert!(
        reasons
            .iter()
            .any(|reason| reason.code == code && reason.count == count),
        "missing reason {code}={count}: {reasons:?}",
    );
}

struct VisibilityFixture {
    _temp: TempDir,
    codex_home: PathBuf,
}

impl VisibilityFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("fixture root");
        let codex_home = temp.path().join(".codex");
        fs::create_dir_all(&codex_home).expect("Codex home");
        let connection = Connection::open(codex_home.join("state_5.sqlite"))
            .expect("create Codex state fixture");
        connection
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    source TEXT NOT NULL,
                    model_provider TEXT NOT NULL,
                    has_user_event INTEGER NOT NULL,
                    archived INTEGER NOT NULL,
                    thread_source TEXT,
                    agent_path TEXT
                );",
            )
            .expect("create real thread-index shape");
        Self {
            _temp: temp,
            codex_home,
        }
    }

    fn codex_home(&self) -> &Path {
        &self.codex_home
    }

    fn root(&self) -> &Path {
        self._temp.path()
    }

    fn rollout(
        &self,
        relative: &str,
        id: &str,
        provider: &str,
        source: &str,
        has_user_event: bool,
        encrypted: bool,
    ) -> PathBuf {
        let path = self.codex_home.join(relative);
        fs::create_dir_all(path.parent().expect("rollout parent")).expect("create rollout parent");
        let mut lines = vec![json!({
            "timestamp": "2026-08-29T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": id,
                "timestamp": "2026-08-29T00:00:00Z",
                "cwd": "C:\\private\\workspace",
                "originator": "codex_cli_rs",
                "cli_version": "0.150.1",
                "source": source,
                "thread_source": "user",
                "model_provider": provider
            }
        })];
        if has_user_event {
            lines.push(json!({
                "timestamp": "2026-08-29T00:00:01Z",
                "type": "event_msg",
                "payload": { "type": "user_message", "message": "private-title" }
            }));
        }
        if encrypted {
            lines.push(json!({
                "timestamp": "2026-08-29T00:00:02Z",
                "type": "response_item",
                "payload": {
                    "type": "reasoning",
                    "encrypted_content": "opaque-encrypted-body"
                }
            }));
        }
        let contents = lines
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{contents}\n")).expect("write rollout");
        path
    }

    fn indexed_candidate(&self, name: &str) -> PathBuf {
        self.indexed_rollout(name, "11111111-1111-4111-8111-111111111111", false)
    }

    fn indexed_rollout(&self, name: &str, id: &str, encrypted: bool) -> PathBuf {
        let rollout = self.rollout(
            &format!("sessions/2026/08/29/{name}"),
            id,
            "old-provider",
            "cli",
            true,
            encrypted,
        );
        self.index(&rollout, "old-provider", "cli", true, false);
        rollout
    }

    fn indexed_rollout_with_provider(&self, name: &str, id: &str, provider: &str) -> PathBuf {
        let rollout = self.rollout(
            &format!("sessions/2026/08/29/{name}"),
            id,
            provider,
            "cli",
            true,
            false,
        );
        self.index(&rollout, provider, "cli", true, false);
        rollout
    }

    fn rollout_without_identity(&self, relative: &str, private_title: &str) {
        let path = self.codex_home.join(relative);
        fs::create_dir_all(path.parent().expect("rollout parent")).expect("create rollout parent");
        fs::write(
            path,
            format!(
                "{}\n",
                json!({
                    "timestamp": "2026-08-29T00:00:00Z",
                    "type": "event_msg",
                    "payload": { "type": "user_message", "message": private_title }
                })
            ),
        )
        .expect("write ambiguous rollout");
    }

    fn index(
        &self,
        rollout: &Path,
        provider: &str,
        source: &str,
        has_user_event: bool,
        archived: bool,
    ) {
        self.index_with_rollout_path(rollout, rollout, provider, source, has_user_event, archived);
    }

    fn index_with_rollout_path(
        &self,
        rollout: &Path,
        indexed_rollout_path: &Path,
        provider: &str,
        source: &str,
        has_user_event: bool,
        archived: bool,
    ) {
        let first_line = fs::read_to_string(rollout)
            .expect("read rollout")
            .lines()
            .next()
            .expect("session meta")
            .to_owned();
        let id = serde_json::from_str::<serde_json::Value>(&first_line)
            .expect("parse session meta")["payload"]["id"]
            .as_str()
            .expect("thread id")
            .to_owned();
        Connection::open(self.codex_home.join("state_5.sqlite"))
            .expect("open state fixture")
            .execute(
                "INSERT INTO threads (
                    id, rollout_path, source, model_provider, has_user_event, archived,
                    thread_source, agent_path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'user', NULL)",
                params![
                    id,
                    indexed_rollout_path.to_string_lossy(),
                    source,
                    provider,
                    has_user_event,
                    archived,
                ],
            )
            .expect("index rollout");
    }

    fn snapshot_bytes(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        collect_files(&self.codex_home, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }
}

fn collect_files(root: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
    for entry in fs::read_dir(root).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push((path, fs::read(entry.path()).expect("read fixture file")));
        }
    }
}
