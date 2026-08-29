use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpteasy_lib::session_visibility::{
    SessionVisibilityApplication, VisibilityAppServerCapability, VisibilityConsumerState,
    VisibilityCoordinationStatus, VisibilityExecutionReadiness, VisibilityExecutionRequest,
    VisibilityExecutionRuntime, VisibilityFailurePoint, VisibilityFaultInjector,
    VisibilityScanContext, VisibilityTarget, VisibilityTargetMode, VisibilityThreadView,
};
use gpteasy_lib::state::{
    PendingSessionVisibilityStatus, PendingSessionVisibilityTargetMode, StatePaths, StateStore,
};
#[cfg(windows)]
use gpteasy_lib::{
    consumer::{ConsumerScanner, ConsumerStatus, WindowsConsumerScanner},
    environment::{AuthenticationMode, EnvironmentApplication, EnvironmentState},
    session::{SessionApplication, SessionAvailabilityStatus, SessionQuery},
};
use rusqlite::{Connection, params};
use serde_json::json;
use tempfile::TempDir;

const TARGET_PROVIDER: &str = "4c8f7402-669f-40cf-a2a9-cfc6f124de6d";

#[cfg(windows)]
#[tokio::test]
#[ignore = "opt-in read-only diagnosis of the current Windows user Codex environment"]
async fn real_current_user_codex_0_150_1_visibility_counts_are_stable_and_redacted() {
    let local_app_data = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA"));
    let user_profile = PathBuf::from(std::env::var_os("USERPROFILE").expect("USERPROFILE"));
    let state_root = local_app_data.join("com.gpteasy.desktop");
    let codex_home = user_profile.join(".codex");
    let state_store = StateStore::new(StatePaths::from_root(&state_root));
    let environment = EnvironmentApplication::new(state_store.clone(), &codex_home)
        .inspect_for_session_visibility()
        .expect("read current environment target");
    assert_eq!(environment.state, EnvironmentState::Managed);
    assert!(!environment.pending_operation);
    let (mode, model_provider) = match (environment.mode, environment.provider_id) {
        (Some(AuthenticationMode::Provider), Some(provider_id)) => {
            (VisibilityTargetMode::Provider, provider_id)
        }
        (Some(AuthenticationMode::OpenaiLogin), _) => {
            (VisibilityTargetMode::OpenaiLogin, "openai".to_owned())
        }
        _ => panic!("current visibility target is not executable"),
    };
    let consumer_scan = WindowsConsumerScanner::new().scan();
    let consumer_state = if consumer_scan.desktop == ConsumerStatus::Unknown
        || consumer_scan.cli == ConsumerStatus::Unknown
    {
        VisibilityConsumerState::Unknown
    } else if consumer_scan.cli == ConsumerStatus::Running {
        VisibilityConsumerState::CliRunning
    } else if consumer_scan.desktop == ConsumerStatus::Running {
        VisibilityConsumerState::DesktopRunning
    } else {
        VisibilityConsumerState::NoConsumers
    };
    let visibility = SessionVisibilityApplication::with_recovery_root(&codex_home, &state_root)
        .with_pending_state(state_store.clone());
    let scan_context = VisibilityScanContext {
        target: VisibilityTarget {
            mode,
            model_provider: model_provider.clone(),
            environment_revision: environment.revision,
        },
        codex_version: Some("codex-cli 0.150.1".to_owned()),
        app_server: VisibilityAppServerCapability::Available,
        consumer_state,
        execution_blockers: Vec::new(),
    };
    let before = visibility
        .scan(scan_context.clone())
        .expect("scan before App Server views");

    let isolated_state_root = TempDir::new().expect("isolated App Server state");
    let isolated_store = StateStore::new(StatePaths::from_root(isolated_state_root.path()));
    assert!(isolated_store.bootstrap().is_ready());
    let sessions = SessionApplication::new(isolated_store);
    let availability = sessions.enter("real-visibility-diagnostic").await;
    assert_eq!(availability.status, SessionAvailabilityStatus::Available);
    let all_providers = real_app_server_view(&sessions, None).await;
    let target_provider = real_app_server_view(&sessions, Some(model_provider)).await;
    sessions.shutdown_now().await;

    let after = visibility
        .scan(scan_context)
        .expect("scan after App Server views");
    assert_eq!(before.summary.candidates, after.summary.candidates);
    assert_eq!(before.summary.unchanged, after.summary.unchanged);
    assert_eq!(before.summary.skipped, after.summary.skipped);
    assert_eq!(after.summary.missing_index, 0);
    assert_eq!(target_provider, after.summary.unchanged as usize);
    assert_eq!(
        all_providers,
        target_provider + after.summary.candidates as usize
    );
    assert_optional_expected_count("GPTEASY_VISIBILITY_EXPECTED_ALL", all_providers);
    assert_optional_expected_count("GPTEASY_VISIBILITY_EXPECTED_TARGET", target_provider);
    assert_optional_expected_count(
        "GPTEASY_VISIBILITY_EXPECTED_CANDIDATES",
        after.summary.candidates as usize,
    );
    eprintln!(
        "safe_counts all={all_providers} target={target_provider} candidates={} unchanged={} skipped={} encrypted_risk={} consumer_state={}",
        after.summary.candidates,
        after.summary.unchanged,
        after.summary.skipped,
        after.summary.encrypted_content_risk,
        consumer_state.diagnostic_name(),
    );
    assert_eq!(before.schema.variant, "codex_0_150_1");
    assert_eq!(after.schema.variant, "codex_0_150_1");
    assert_eq!(before.consumer_state, consumer_state);
    let confirmation_stable = before.confirmation_id == after.confirmation_id;
    if consumer_state == VisibilityConsumerState::NoConsumers {
        assert!(confirmation_stable);
        assert_eq!(after.readiness, VisibilityExecutionReadiness::Ready);
    } else {
        assert!(!after.can_execute);
    }
    eprintln!("safe_hash_state stable={confirmation_stable}");
    let recovery = visibility
        .assess_recovery()
        .expect("assess recovery evidence");
    assert_eq!(recovery.status, "none");
    let pending = state_store
        .pending_session_visibility()
        .expect("read pending visibility state");
    if let Some(pending) = pending {
        assert!(matches!(
            pending.status,
            PendingSessionVisibilityStatus::Pending
                | PendingSessionVisibilityStatus::Running
                | PendingSessionVisibilityStatus::Partial
                | PendingSessionVisibilityStatus::Blocked
        ));
        assert!(!pending.diagnostic_stage.is_empty());
        assert!(!pending.error_code.is_empty());
        assert!(safe_diagnostic_token(&pending.diagnostic_stage));
        assert!(safe_diagnostic_token(&pending.error_code));
        eprintln!(
            "safe_pending status={:?} succeeded={} retryable={}",
            pending.status, pending.succeeded, pending.retryable,
        );
    } else {
        eprintln!("safe_pending status=none");
    }
}

#[cfg(windows)]
fn assert_optional_expected_count(variable: &str, actual: usize) {
    let Some(value) = std::env::var_os(variable) else {
        return;
    };
    let expected = value
        .to_string_lossy()
        .parse::<usize>()
        .expect("expected count must be an unsigned integer");
    assert_eq!(actual, expected, "{variable}");
}

#[cfg(windows)]
fn safe_diagnostic_token(value: &str) -> bool {
    value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
}

#[cfg(windows)]
async fn real_app_server_view(
    application: &SessionApplication,
    model_provider: Option<String>,
) -> usize {
    let mut count = 0;
    for archived in [false, true] {
        let mut cursor = None;
        loop {
            let page = application
                .list(SessionQuery {
                    request_id: None,
                    archived,
                    search_term: None,
                    project: None,
                    model_provider: model_provider.clone(),
                    cursor,
                    limit: 100,
                })
                .await
                .expect("read redacted App Server count");
            count += page.sessions.len();
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }
    }
    count
}

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
    let state_store = fixture.state_store();
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root())
            .with_pending_state(state_store.clone());
    let preview = application
        .scan(executable_context("revision-atomic"))
        .expect("scan visibility");
    let before = fs::read(&rollout).expect("read rollout before repair");
    let runtime = VisibilityRuntimeFixture::new(fixture.codex_home());
    application
        .record_pending(&preview.target)
        .expect("record pending visibility");

    let result = application
        .execute_pending(
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
    assert!(
        state_store
            .pending_session_visibility()
            .expect("read completed pending state")
            .is_none(),
        "complete repair clears the independently persisted pending state",
    );
    assert_eq!(
        runtime.starts(),
        2,
        "baseline and post-write verification use clean servers"
    );
    let after = fs::read(&rollout).expect("read rollout after repair");
    assert_only_model_provider_changed(&before, &after, TARGET_PROVIDER);
}

#[tokio::test]
async fn confirmed_repair_supports_the_codex_0_150_1_index_contract_end_to_end() {
    let fixture = VisibilityFixture::new();
    fixture.use_codex_0_150_1_schema();
    let rollout = fixture.rollout(
        "sessions/2026/08/29/codex-0-150-1-execution.jsonl",
        "11111111-1111-4111-8111-111111111111",
        "old-provider",
        "cli",
        true,
        true,
    );
    fixture.index(&rollout, "old-provider", "cli", false, false);
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-codex-0-150-1-execution"))
        .expect("scan Codex 0.150.1 execution candidate");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-codex-0-150-1-execution",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("execute Codex 0.150.1 visibility repair");

    assert_eq!(result.status, "complete", "{result:?}");
    assert_eq!((result.succeeded, result.retryable), (1, 0));
    assert_eq!(provider_in_rollout(&rollout), TARGET_PROVIDER);
    let indexed: (String, i64) = Connection::open(fixture.codex_home().join("state_5.sqlite"))
        .expect("open Codex 0.150.1 index")
        .query_row(
            "SELECT model_provider, has_user_event FROM threads WHERE id = ?1",
            ["11111111-1111-4111-8111-111111111111"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read repaired Codex 0.150.1 index");
    assert_eq!(
        indexed,
        ("old-provider".to_owned(), 0),
        "existing indexes remain App Server-owned; GPTEasy only rewrites rollout metadata"
    );
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
        (
            VisibilityConsumerState::DesktopRunning,
            "session_visibility.desktop_running",
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
async fn scan_reports_consumer_readiness_and_a_clean_rescan_can_execute() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.indexed_candidate("consumer-preview.jsonl");
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let before = fs::read(&rollout).expect("read rollout before previews");

    for (consumer_state, readiness, blocker) in [
        (
            VisibilityConsumerState::CliRunning,
            VisibilityExecutionReadiness::CliRunning,
            "cli_running",
        ),
        (
            VisibilityConsumerState::Unknown,
            VisibilityExecutionReadiness::UnknownConsumer,
            "consumer_unknown",
        ),
        (
            VisibilityConsumerState::DesktopRunning,
            VisibilityExecutionReadiness::DesktopRunning,
            "desktop_running",
        ),
    ] {
        let mut context = executable_context("revision-consumer-preview");
        context.consumer_state = consumer_state;
        let preview = application.scan(context).expect("scan consumer blocker");

        assert_eq!(preview.readiness, readiness);
        assert_eq!(preview.consumer_state, consumer_state);
        assert!(!preview.can_execute);
        assert!(preview.blockers.iter().any(|value| value == blocker));
        assert_eq!(fs::read(&rollout).expect("read after preview"), before);
        assert!(
            !fixture
                .root()
                .join("session-visibility-recovery.json")
                .exists()
        );
    }

    let preview = application
        .scan(executable_context("revision-consumer-preview"))
        .expect("rescan after consumer exit");
    assert_eq!(preview.readiness, VisibilityExecutionReadiness::Ready);
    assert!(preview.can_execute);
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-consumer-preview",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("execute after clean rescan");

    assert_eq!(result.status, "complete");
    assert_eq!(result.succeeded, 1);
}

#[test]
fn adding_a_late_execution_blocker_recomputes_preview_readiness() {
    let fixture = VisibilityFixture::new();
    fixture.indexed_candidate("late-blocker.jsonl");
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let mut preview = application
        .scan(executable_context("revision-late-blocker"))
        .expect("scan executable preview");

    SessionVisibilityApplication::add_execution_blocker(&mut preview, "app_server_unavailable");

    assert_eq!(
        preview.readiness,
        VisibilityExecutionReadiness::AppServerUnavailable
    );
    assert!(!preview.can_execute);

    SessionVisibilityApplication::add_execution_blocker(
        &mut preview,
        "environment_revision_changed",
    );
    assert_eq!(
        preview.readiness,
        VisibilityExecutionReadiness::AppServerUnavailable,
        "the highest-priority blocker remains visible"
    );

    let mut configuration_preview = application
        .scan(executable_context("revision-late-configuration-blocker"))
        .expect("scan second executable preview");
    SessionVisibilityApplication::add_execution_blocker(
        &mut configuration_preview,
        "environment_revision_changed",
    );
    assert_eq!(
        configuration_preview.readiness,
        VisibilityExecutionReadiness::ConfigurationBlocked
    );
}

#[tokio::test]
async fn automatic_coordination_defers_cli_and_unknown_consumers_without_starting_or_writing() {
    for (consumer, expected_code) in [
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
        let rollout = fixture.indexed_candidate("automatic-consumer-gate.jsonl");
        let state_store = fixture.state_store();
        let application =
            SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root())
                .with_pending_state(state_store.clone());
        let context = executable_context("revision-automatic-consumer-gate");
        application
            .record_pending(&context.target)
            .expect("record pending target");
        let before = fs::read(&rollout).expect("read rollout before automatic gate");
        let runtime = VisibilityRuntimeFixture::with_state(
            fixture.codex_home(),
            consumer,
            "revision-automatic-consumer-gate",
            None,
        );

        let outcome = application
            .coordinate_pending(context, &runtime)
            .await
            .expect("consumer deferral is a normal coordination result");

        assert_eq!(outcome.status, VisibilityCoordinationStatus::Deferred);
        assert!(!outcome.block_codex_restart);
        assert_eq!(outcome.error_code, expected_code);
        assert_eq!(runtime.starts(), 0);
        assert_eq!(runtime.shutdowns(), 0);
        assert_eq!(fs::read(&rollout).expect("read rollout after gate"), before);
        let pending = state_store
            .pending_session_visibility()
            .expect("read deferred pending state")
            .expect("deferred state remains pending");
        assert_eq!(pending.status, PendingSessionVisibilityStatus::Pending);
        assert_eq!(pending.error_code, expected_code);
    }
}

#[tokio::test]
async fn automatic_coordination_repairs_an_openai_target_when_no_consumers_are_running() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.indexed_candidate("automatic-openai.jsonl");
    let state_store = fixture.state_store();
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root())
            .with_pending_state(state_store.clone());
    let context = openai_context("revision-automatic-openai");
    application
        .record_pending(&context.target)
        .expect("record OpenAI pending target");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-automatic-openai",
        None,
    )
    .with_target(VisibilityTargetMode::OpenaiLogin, "openai");

    let outcome = application
        .coordinate_pending(context, &runtime)
        .await
        .expect("coordinate OpenAI target");

    assert_eq!(outcome.status, VisibilityCoordinationStatus::Complete);
    assert_eq!(provider_in_rollout(&rollout), "openai");
    assert!(
        state_store
            .pending_session_visibility()
            .expect("read completed OpenAI state")
            .is_none()
    );
}

#[tokio::test]
async fn automatic_coordination_repairs_a_provider_target_when_no_consumers_are_running() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.indexed_candidate("automatic-provider.jsonl");
    let state_store = fixture.state_store();
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root())
            .with_pending_state(state_store.clone());
    let context = executable_context("revision-automatic-provider");
    application
        .record_pending(&context.target)
        .expect("record provider pending target");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-automatic-provider",
        None,
    );

    let outcome = application
        .coordinate_pending(context, &runtime)
        .await
        .expect("coordinate provider target");

    assert_eq!(outcome.status, VisibilityCoordinationStatus::Complete);
    assert_eq!(provider_in_rollout(&rollout), TARGET_PROVIDER);
    assert!(
        state_store
            .pending_session_visibility()
            .expect("read completed provider state")
            .is_none()
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
    let state_store = fixture.state_store();
    let application = SessionVisibilityApplication::with_fault_injector(
        fixture.codex_home(),
        fixture.root(),
        faults,
    )
    .with_pending_state(state_store.clone());
    let preview = application
        .scan(executable_context("revision-partial"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-partial",
        None,
    );
    application
        .record_pending(&preview.target)
        .expect("record pending visibility");

    let result = application
        .execute_pending(execution_request(&preview), &runtime)
        .await
        .expect("execute partial repair");

    assert_eq!(result.status, "partial");
    assert_eq!((result.succeeded, result.retryable), (1, 1));
    assert_eq!(result.diagnostic_stage, "rollout_replace");
    assert_eq!(result.error_code, "session_visibility.write_failed");
    let pending = state_store
        .pending_session_visibility()
        .expect("read partial pending state")
        .expect("partial repair remains pending");
    assert_eq!(pending.status, PendingSessionVisibilityStatus::Partial);
    assert_eq!(
        pending.target_mode,
        PendingSessionVisibilityTargetMode::Provider
    );
    assert_eq!((pending.succeeded, pending.retryable), (1, 1));
    assert_eq!(pending.diagnostic_stage, "rollout_replace");
    assert_eq!(pending.error_code, "session_visibility.write_failed");
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
async fn a_valid_missing_index_is_repaired_with_other_indexed_sessions() {
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
    assert_eq!((result.succeeded, result.retryable), (2, 0));
    assert_eq!(
        (
            result.breakdown.app_server_coordinated,
            result.breakdown.sqlite_fallback,
        ),
        (0, 1)
    );
    assert_eq!(
        (
            result.breakdown.schema_skipped,
            result.breakdown.verification_failed,
        ),
        (0, 0),
    );
}

#[tokio::test]
async fn a_supported_index_schema_uses_a_transactional_fallback_for_a_valid_missing_session() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.rollout(
        "sessions/2026/08/29/missing-supported.jsonl",
        "33333333-3333-4333-8333-333333333333",
        "old-provider",
        "cli",
        true,
        false,
    );
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-index-fallback"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-index-fallback",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("repair missing index with the supported fallback");

    assert_eq!(result.status, "complete");
    assert_eq!((result.succeeded, result.retryable), (1, 0));
    assert_eq!(provider_in_rollout(&rollout), TARGET_PROVIDER);
    let indexed: (String, String, i64) =
        Connection::open(fixture.codex_home().join("state_5.sqlite"))
            .expect("open supported index")
            .query_row(
                "SELECT model_provider, rollout_path, archived FROM threads WHERE id = ?1",
                ["33333333-3333-4333-8333-333333333333"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("fallback inserted the missing thread");
    assert_eq!(indexed.0, "old-provider");
    assert_eq!(PathBuf::from(indexed.1), rollout);
    assert_eq!(indexed.2, 0);
}

#[tokio::test]
async fn duplicate_rollout_ids_are_all_skipped_even_when_only_one_has_fallback_metadata() {
    let fixture = VisibilityFixture::new();
    let duplicate_id = "33333333-3333-4333-8333-333333333333";
    let complete = fixture.rollout(
        "sessions/2026/08/29/duplicate-complete.jsonl",
        duplicate_id,
        "old-provider",
        "cli",
        true,
        false,
    );
    let incomplete = fixture.rollout(
        "archived_sessions/duplicate-incomplete.jsonl",
        duplicate_id,
        "old-provider",
        "cli",
        true,
        false,
    );
    let changed = fs::read_to_string(&incomplete)
        .expect("read duplicate rollout")
        .replacen("\"approval_policy\":\"on-request\",", "", 1);
    fs::write(&incomplete, changed).expect("remove fallback metadata");
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-duplicate-id"))
        .expect("scan duplicate rollout IDs");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-duplicate-id",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("skip ambiguous duplicate IDs");

    assert_eq!(preview.summary.candidates, 0);
    assert_eq!(preview.summary.missing_index, 0);
    assert_eq!(preview.summary.skipped, 2);
    assert_reason(&preview.reasons, "identity_ambiguous", 2);
    assert_eq!((result.succeeded, result.retryable), (0, 0));
    assert_eq!(result.breakdown.sqlite_fallback, 0);
    assert_eq!(provider_in_rollout(&complete), "old-provider");
    assert_eq!(provider_in_rollout(&incomplete), "old-provider");
    let index_count: i64 = Connection::open(fixture.codex_home().join("state_5.sqlite"))
        .expect("open duplicate-ID index")
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .expect("count duplicate-ID index rows");
    assert_eq!(index_count, 0);
}

#[tokio::test]
async fn app_server_coordination_precedes_sqlite_fallback_for_missing_indexes() {
    let fixture = VisibilityFixture::new();
    let coordinated = fixture.rollout(
        "sessions/2026/08/29/app-server-coordinated.jsonl",
        "33333333-3333-4333-8333-333333333333",
        "old-provider",
        "cli",
        true,
        false,
    );
    let fallback = fixture.rollout(
        "sessions/2026/08/29/sqlite-fallback.jsonl",
        "44444444-4444-4444-8444-444444444444",
        "old-provider",
        "cli",
        true,
        false,
    );
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-coordination-first"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-coordination-first",
        None,
    )
    .with_app_server_coordination(&coordinated);

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("coordinate before fallback");

    assert_eq!(result.status, "complete");
    assert_eq!((result.succeeded, result.retryable), (2, 0));
    assert_eq!(
        (
            result.breakdown.app_server_coordinated,
            result.breakdown.sqlite_fallback,
        ),
        (1, 1)
    );
    assert_eq!(
        (
            result.breakdown.schema_skipped,
            result.breakdown.verification_failed,
        ),
        (0, 0),
    );
    assert_eq!(provider_in_rollout(&coordinated), TARGET_PROVIDER);
    assert_eq!(provider_in_rollout(&fallback), TARGET_PROVIDER);
    assert_eq!(
        runtime.starts(),
        2,
        "one baseline and one clean verification"
    );
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.root().join("session-visibility-recovery.json"))
            .expect("read index recovery manifest"),
    )
    .expect("parse index recovery manifest");
    let index_stages = manifest["items"]
        .as_array()
        .expect("manifest items")
        .iter()
        .filter_map(|item| item["indexStage"].as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        index_stages,
        BTreeSet::from(["app_server_coordinated", "sqlite_fallback_committed"]),
    );
    assert_ne!(
        manifest["indexDatabaseBeforeHash"],
        manifest["indexDatabaseAfterHash"],
    );
}

#[tokio::test]
async fn index_transaction_rollback_does_not_undo_other_verified_session_repairs() {
    let fixture = VisibilityFixture::new();
    let indexed = fixture.indexed_candidate("indexed-success.jsonl");
    let first_missing = fixture.rollout(
        "sessions/2026/08/29/index-rollback-first.jsonl",
        "33333333-3333-4333-8333-333333333333",
        "old-provider",
        "cli",
        true,
        false,
    );
    let second_missing = fixture.rollout(
        "sessions/2026/08/29/index-rollback-second.jsonl",
        "44444444-4444-4444-8444-444444444444",
        "old-provider",
        "cli",
        true,
        false,
    );
    let application = SessionVisibilityApplication::with_fault_injector(
        fixture.codex_home(),
        fixture.root(),
        FailingVisibilityWrites::before_index_insert_for("44444444-4444-4444-8444-444444444444"),
    );
    let preview = application
        .scan(executable_context("revision-index-rollback"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-index-rollback",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("index rollback remains a partial result");

    assert_eq!(result.status, "partial");
    assert_eq!((result.succeeded, result.retryable), (1, 2));
    assert_eq!(
        (
            result.breakdown.app_server_coordinated,
            result.breakdown.sqlite_fallback,
        ),
        (0, 0)
    );
    assert_eq!(
        (
            result.breakdown.schema_skipped,
            result.breakdown.verification_failed,
        ),
        (0, 0),
    );
    assert_eq!(result.diagnostic_stage, "index_transaction");
    assert_eq!(result.error_code, "session_visibility.index_write_failed");
    assert_eq!(provider_in_rollout(&indexed), TARGET_PROVIDER);
    assert_eq!(provider_in_rollout(&first_missing), "old-provider");
    assert_eq!(provider_in_rollout(&second_missing), "old-provider");
    let index_count: i64 = Connection::open(fixture.codex_home().join("state_5.sqlite"))
        .expect("open rolled back index")
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .expect("count index rows");
    assert_eq!(
        index_count, 1,
        "the whole missing-index transaction rolled back"
    );
    let manifest = fs::read_to_string(fixture.root().join("session-visibility-recovery.json"))
        .expect("read compact recovery manifest");
    assert!(manifest.contains("index_write_failed"));
    for sensitive in [
        "33333333-3333-4333-8333-333333333333",
        "44444444-4444-4444-8444-444444444444",
        "index-rollback-first.jsonl",
        "index-rollback-second.jsonl",
        "private-title",
    ] {
        assert!(!manifest.contains(sensitive), "manifest leaked {sensitive}");
    }
}

#[tokio::test]
async fn incomplete_fallback_metadata_does_not_block_app_server_index_coordination() {
    let fixture = VisibilityFixture::new();
    let indexed = fixture.indexed_candidate("indexed-with-legacy-meta.jsonl");
    let missing = fixture.rollout(
        "sessions/2026/08/29/missing-incomplete-meta.jsonl",
        "33333333-3333-4333-8333-333333333333",
        "old-provider",
        "cli",
        true,
        false,
    );
    for rollout in [&indexed, &missing] {
        let changed = fs::read_to_string(rollout)
            .expect("read metadata fixture")
            .replacen("\"approval_policy\":\"on-request\",", "", 1);
        fs::write(rollout, changed).expect("remove index-only metadata");
    }
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-index-metadata"))
        .expect("scan incomplete index metadata");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-index-metadata",
        None,
    )
    .with_app_server_coordination(&missing);

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("coordinate the missing index before considering fallback metadata");

    assert_eq!(preview.summary.candidates, 2);
    assert_eq!(preview.summary.missing_index, 1);
    assert_eq!(preview.summary.skipped, 0);
    assert_eq!(preview.index_plan.app_server_coordination, 1);
    assert_eq!(preview.index_plan.sqlite_fallback_eligible, 0);
    assert_reason(&preview.reasons, "index_fallback_metadata_incomplete", 1);
    assert_eq!((result.succeeded, result.retryable), (2, 0));
    assert_eq!(result.breakdown.app_server_coordinated, 1);
    assert_eq!(result.breakdown.sqlite_fallback, 0);
    assert_eq!(provider_in_rollout(&indexed), TARGET_PROVIDER);
    assert_eq!(provider_in_rollout(&missing), TARGET_PROVIDER);
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
    let state_store = fixture.state_store();
    let application = SessionVisibilityApplication::with_fault_injector(
        fixture.codex_home(),
        fixture.root(),
        FailingVisibilityWrites::after_replace_once(),
    )
    .with_pending_state(state_store.clone());
    let preview = application
        .scan(executable_context("revision-interrupted"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-interrupted",
        None,
    );
    application
        .record_pending(&preview.target)
        .expect("record pending visibility");

    let failure = application
        .execute_pending(execution_request(&preview), &runtime)
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
        .execute_pending(execution_request(&preview), &runtime)
        .await
        .expect("public execution reports an indeterminate recovery state");
    assert_eq!(next.status, "indeterminate");
    assert!(next.block_codex_restart);
    assert_eq!(
        state_store
            .pending_session_visibility()
            .expect("read blocked pending state")
            .expect("indeterminate state remains pending")
            .status,
        PendingSessionVisibilityStatus::Blocked,
    );
    assert!(next.block_codex_restart);
    assert_eq!(
        runtime.shutdowns(),
        1,
        "unknown recovery stops before another shutdown"
    );
}

#[tokio::test]
async fn an_index_commit_without_a_recorded_after_hash_blocks_restart_as_indeterminate() {
    let fixture = VisibilityFixture::new();
    fixture.rollout(
        "sessions/2026/08/29/index-commit-interrupted.jsonl",
        "33333333-3333-4333-8333-333333333333",
        "old-provider",
        "cli",
        true,
        false,
    );
    let application = SessionVisibilityApplication::with_fault_injector(
        fixture.codex_home(),
        fixture.root(),
        FailingVisibilityWrites::after_index_commit_once(),
    );
    let preview = application
        .scan(executable_context("revision-index-commit"))
        .expect("scan missing index");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-index-commit",
        None,
    );

    let failure = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect_err("fault interrupts after the index transaction commits");

    assert_eq!(failure.message_id, "session_visibility.interrupted");
    let index_count: i64 = Connection::open(fixture.codex_home().join("state_5.sqlite"))
        .expect("open index after interruption")
        .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
        .expect("count committed index rows");
    assert_eq!(index_count, 1);
    let assessment = application
        .assess_recovery()
        .expect("assess interrupted index transaction");
    assert_eq!(assessment.status, "indeterminate");
    assert!(assessment.block_codex_restart);

    let next = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("public execution reports the indeterminate index state");
    assert_eq!(next.status, "indeterminate");
    assert!(next.block_codex_restart);
    assert_eq!(runtime.shutdowns(), 1);
}

#[tokio::test]
async fn database_activity_after_the_index_result_is_recorded_does_not_block_restart() {
    let fixture = VisibilityFixture::new();
    fixture.indexed_candidate("retryable-after-database-activity.jsonl");
    let application = SessionVisibilityApplication::with_fault_injector(
        fixture.codex_home(),
        fixture.root(),
        FailingVisibilityWrites::before_replace_for("11111111-1111-4111-8111-111111111111"),
    );
    let preview = application
        .scan(executable_context("revision-database-activity"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-database-activity",
        None,
    );

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("record a retryable repair result");
    assert_eq!(result.status, "failed");
    Connection::open(fixture.codex_home().join("state_5.sqlite"))
        .expect("open index for normal activity")
        .execute(
            "UPDATE threads SET title = 'updated after repair' WHERE id = ?1",
            ["11111111-1111-4111-8111-111111111111"],
        )
        .expect("simulate normal index activity");

    let assessment = application
        .assess_recovery()
        .expect("assess retryable repair after normal database activity");
    assert_eq!(assessment.status, "retryable");
    assert!(!assessment.block_codex_restart);
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
        assert_eq!(result.breakdown.verification_failed, 1);
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
async fn verification_keeps_each_success_when_another_session_is_missing_from_the_target_view() {
    let fixture = VisibilityFixture::new();
    fixture.indexed_rollout(
        "verified.jsonl",
        "11111111-1111-4111-8111-111111111111",
        false,
    );
    fixture.indexed_rollout(
        "verification-failed.jsonl",
        "22222222-2222-4222-8222-222222222222",
        false,
    );
    let application =
        SessionVisibilityApplication::with_recovery_root(fixture.codex_home(), fixture.root());
    let preview = application
        .scan(executable_context("revision-partial-verification"))
        .expect("scan visibility");
    let runtime = VisibilityRuntimeFixture::with_state(
        fixture.codex_home(),
        VisibilityConsumerState::NoConsumers,
        "revision-partial-verification",
        None,
    )
    .with_verification(VerificationFixture::MissingSecondTarget);

    let result = application
        .execute(execution_request(&preview), &runtime)
        .await
        .expect("verify sessions independently");

    assert_eq!(result.status, "partial");
    assert_eq!((result.succeeded, result.retryable), (1, 1));
    assert_eq!(result.breakdown.verification_failed, 1);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.root().join("session-visibility-recovery.json"))
            .expect("read recovery manifest"),
    )
    .expect("parse recovery manifest");
    let stages = manifest["items"]
        .as_array()
        .expect("manifest items")
        .iter()
        .filter_map(|item| item["stage"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        stages.iter().filter(|stage| **stage == "verified").count(),
        1
    );
    assert_eq!(
        stages
            .iter()
            .filter(|stage| **stage == "verification_failed")
            .count(),
        1
    );
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

    assert_eq!(result.status, "partial");
    assert_eq!((result.succeeded, result.retryable), (1, 1));
    assert_eq!(result.breakdown.verification_failed, 1);
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
        consumer_state: VisibilityConsumerState::NoConsumers,
        execution_blockers: Vec::new(),
    }
}

fn openai_context(revision: &str) -> VisibilityScanContext {
    VisibilityScanContext {
        target: VisibilityTarget {
            mode: VisibilityTargetMode::OpenaiLogin,
            model_provider: "openai".to_owned(),
            environment_revision: revision.to_owned(),
        },
        codex_version: Some("codex-cli fixture".to_owned()),
        app_server: VisibilityAppServerCapability::Available,
        consumer_state: VisibilityConsumerState::NoConsumers,
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
    coordinated_rollout: Option<PathBuf>,
    target_mode: VisibilityTargetMode,
    target_provider: String,
}

#[derive(Clone, Copy)]
enum VerificationFixture {
    Normal,
    MissingTarget,
    MissingSecondTarget,
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
            coordinated_rollout: None,
            target_mode: VisibilityTargetMode::Provider,
            target_provider: TARGET_PROVIDER.to_owned(),
        }
    }

    fn with_target(mut self, mode: VisibilityTargetMode, model_provider: &str) -> Self {
        self.target_mode = mode;
        self.target_provider = model_provider.to_owned();
        self
    }

    fn with_verification(mut self, verification: VerificationFixture) -> Self {
        self.verification = verification;
        self
    }

    fn with_app_server_coordination(mut self, rollout: &Path) -> Self {
        self.coordinated_rollout = Some(rollout.to_path_buf());
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
            mode: self.target_mode,
            model_provider: self.target_provider.clone(),
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
            if let Some(rollout) = &self.coordinated_rollout {
                insert_fixture_thread(
                    &self.codex_home,
                    rollout,
                    rollout,
                    "old-provider",
                    "cli",
                    true,
                    false,
                );
            }
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
                VerificationFixture::MissingSecondTarget => {
                    target_provider
                        .retain(|thread| thread.id != "22222222-2222-4222-8222-222222222222");
                }
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

    fn after_index_commit_once() -> Self {
        Self {
            point: VisibilityFailurePoint::AfterIndexCommit,
            session_reference: None,
            remaining: AtomicUsize::new(1),
        }
    }

    fn before_index_insert_for(id: &str) -> Self {
        Self {
            point: VisibilityFailurePoint::BeforeIndexInsert,
            session_reference: Some(SessionVisibilityApplication::session_reference(id)),
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
            consumer_state: VisibilityConsumerState::NoConsumers,
            execution_blockers: Vec::new(),
        })
        .expect("scan visibility");

    assert_eq!(preview.target.model_provider, TARGET_PROVIDER);
    assert_eq!(preview.target.environment_revision, "revision-42");
    assert_eq!(preview.summary.candidates, 2);
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
fn codex_0_150_1_schema_keeps_index_user_event_default_out_of_identity_matching() {
    let fixture = VisibilityFixture::new();
    fixture.use_codex_0_150_1_schema();
    let candidate = fixture.rollout(
        "sessions/2026/08/29/codex-0-150-1-candidate.jsonl",
        "11111111-1111-4111-8111-111111111111",
        "old-provider",
        "cli",
        true,
        true,
    );
    fixture.index(&candidate, "old-provider", "cli", false, false);
    let unchanged = fixture.rollout(
        "sessions/2026/08/29/codex-0-150-1-unchanged.jsonl",
        "22222222-2222-4222-8222-222222222222",
        TARGET_PROVIDER,
        "vscode",
        true,
        false,
    );
    fixture.index(&unchanged, TARGET_PROVIDER, "vscode", false, false);

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(executable_context("revision-codex-0-150-1"))
        .expect("scan Codex 0.150.1 visibility");

    assert_eq!(preview.schema.status, "supported");
    assert_eq!(preview.summary.candidates, 1);
    assert_eq!(preview.summary.unchanged, 1);
    assert_eq!(preview.summary.skipped, 0);
    assert_eq!(preview.summary.blocked, 0);
    assert_eq!(preview.summary.encrypted_content_risk, 1);
    assert!(preview.can_execute);
    assert_reason(&preview.reasons, "provider_mismatch", 1);
    assert_reason(&preview.reasons, "encrypted_content", 1);
    let details = preview.diagnostic_details();
    assert!(details.contains("schema=supported"));
    assert!(details.contains("schema_variant=codex_0_150_1"));
    for sensitive in [
        "old-provider",
        TARGET_PROVIDER,
        "11111111-1111-4111-8111-111111111111",
        "C:\\private\\workspace",
        "private-title",
        "opaque-encrypted-body",
    ] {
        assert!(
            !details.contains(sensitive),
            "diagnostic leaked {sensitive}"
        );
    }
}

#[test]
fn structured_subagent_source_is_excluded_without_becoming_identity_ambiguous() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.rollout(
        "sessions/2026/08/29/structured-subagent.jsonl",
        "33333333-3333-4333-8333-333333333333",
        "old-provider",
        "subAgent",
        true,
        true,
    );
    fixture.rewrite_source_as_subagent_object(&rollout);

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(executable_context("revision-structured-subagent"))
        .expect("scan structured subagent source");

    assert_eq!(preview.summary.candidates, 0);
    assert_eq!(preview.summary.skipped, 1);
    assert_eq!(preview.summary.blocked, 0);
    assert_eq!(preview.summary.encrypted_content_risk, 0);
    assert_reason(&preview.reasons, "excluded_internal", 1);
    assert!(
        preview
            .reasons
            .iter()
            .all(|reason| reason.code != "identity_ambiguous")
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
            consumer_state: VisibilityConsumerState::NoConsumers,
            execution_blockers: vec![
                "external_configuration".to_owned(),
                "pending_config_operation".to_owned(),
            ],
        })
        .expect("scan blocked visibility");

    assert!(!preview.can_execute);
    assert_eq!(preview.schema.status, "unknown");
    assert_eq!(preview.index_plan.schema_skipped, 1);
    assert_eq!(preview.index_plan.sqlite_fallback_eligible, 0);
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
fn a_future_index_schema_is_skipped_without_inserting_or_cleaning_rows() {
    let fixture = VisibilityFixture::new();
    let indexed = fixture.indexed_candidate("existing-index.jsonl");
    fixture.rollout(
        "sessions/2026/08/29/future-schema-missing.jsonl",
        "55555555-5555-4555-8555-555555555555",
        "old-provider",
        "cli",
        true,
        false,
    );
    Connection::open(fixture.codex_home().join("state_5.sqlite"))
        .expect("open future schema fixture")
        .execute(
            "ALTER TABLE threads ADD COLUMN future_required TEXT NOT NULL DEFAULT 'future'",
            [],
        )
        .expect("add a future schema field");
    let before = fixture.snapshot_bytes();

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(executable_context("revision-future-schema"))
        .expect("scan future schema");

    assert_eq!(preview.schema.status, "unknown");
    assert!(!preview.can_execute);
    assert_eq!(preview.index_plan.schema_skipped, 2);
    assert_eq!(preview.index_plan.sqlite_fallback_eligible, 0);
    assert_eq!(preview.summary.candidates, 0);
    assert_eq!(preview.summary.blocked, 2);
    assert_reason(&preview.reasons, "unsupported_index_schema", 1);
    assert_eq!(fixture.snapshot_bytes(), before);
    assert_eq!(provider_in_rollout(&indexed), "old-provider");
}

#[test]
fn codex_0_150_1_schema_with_an_extra_index_is_not_written() {
    let fixture = VisibilityFixture::new();
    fixture.use_codex_0_150_1_schema();
    fixture.rollout(
        "sessions/2026/08/29/codex-0-150-1-extra-index.jsonl",
        "55555555-5555-4555-8555-555555555555",
        "old-provider",
        "cli",
        true,
        false,
    );
    Connection::open(fixture.codex_home().join("state_5.sqlite"))
        .expect("open Codex 0.150.1 extra-index fixture")
        .execute(
            "CREATE INDEX idx_threads_unrecognized ON threads(title)",
            [],
        )
        .expect("add an unrecognized thread index");
    let before = fixture.snapshot_bytes();

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(executable_context("revision-codex-0-150-1-extra-index"))
        .expect("scan Codex 0.150.1 with an extra index");

    assert_eq!(preview.schema.status, "unknown");
    assert_eq!(preview.schema.variant, "unknown");
    assert!(!preview.can_execute);
    assert_eq!(preview.summary.candidates, 0);
    assert_eq!(preview.summary.blocked, 1);
    assert_reason(&preview.reasons, "unsupported_index_schema", 1);
    assert_eq!(fixture.snapshot_bytes(), before);
}

#[test]
fn a_same_named_but_constraint_incompatible_schema_is_skipped() {
    let fixture = VisibilityFixture::new();
    fixture.rollout(
        "sessions/2026/08/29/constraint-mismatch.jsonl",
        "55555555-5555-4555-8555-555555555555",
        "old-provider",
        "cli",
        true,
        false,
    );
    let connection = Connection::open(fixture.codex_home().join("state_5.sqlite"))
        .expect("open constraint mismatch fixture");
    let schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'threads'",
            [],
            |row| row.get(0),
        )
        .expect("read supported schema");
    let incompatible = schema.replacen("rollout_path TEXT NOT NULL", "rollout_path TEXT", 1);
    assert_ne!(incompatible, schema, "fixture must change a constraint");
    connection
        .execute_batch(&format!("DROP TABLE threads; {incompatible};"))
        .expect("install same-named incompatible schema");

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(executable_context("revision-constraint-mismatch"))
        .expect("scan constraint mismatch");

    assert_eq!(preview.schema.status, "unknown");
    assert!(!preview.can_execute);
    assert_eq!(preview.summary.candidates, 0);
    assert_eq!(preview.summary.blocked, 1);
    assert_eq!(preview.index_plan.sqlite_fallback_eligible, 0);
    assert_eq!(preview.index_plan.schema_skipped, 1);
}

#[test]
fn a_corrupt_index_database_is_reported_unknown_without_any_repair_write() {
    let fixture = VisibilityFixture::new();
    let rollout = fixture.rollout(
        "sessions/2026/08/29/corrupt-database.jsonl",
        "55555555-5555-4555-8555-555555555555",
        "old-provider",
        "cli",
        true,
        false,
    );
    let database = fixture.codex_home().join("state_5.sqlite");
    fs::write(&database, b"not a sqlite database").expect("corrupt index fixture");
    let before_database = fs::read(&database).expect("read corrupt fixture");

    let preview = SessionVisibilityApplication::new(fixture.codex_home())
        .scan(executable_context("revision-corrupt-schema"))
        .expect("scan corrupt schema");

    assert_eq!(preview.schema.status, "unknown");
    assert!(!preview.can_execute);
    assert_eq!(preview.index_plan.schema_skipped, 1);
    assert_eq!(
        fs::read(database).expect("read unchanged corrupt fixture"),
        before_database
    );
    assert_eq!(provider_in_rollout(&rollout), "old-provider");
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
            consumer_state: VisibilityConsumerState::NoConsumers,
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
            consumer_state: VisibilityConsumerState::NoConsumers,
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
            consumer_state: VisibilityConsumerState::NoConsumers,
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

    fn use_codex_0_150_1_schema(&self) {
        Connection::open(self.codex_home.join("state_5.sqlite"))
            .expect("open Codex 0.150.1 state fixture")
            .execute_batch(CODEX_0_150_1_THREADS_SCHEMA)
            .expect("create Codex 0.150.1 thread-index shape");
    }

    fn rewrite_source_as_subagent_object(&self, rollout: &Path) {
        let contents = fs::read_to_string(rollout).expect("read rollout for source rewrite");
        let mut lines = contents.lines();
        let mut meta = serde_json::from_str::<serde_json::Value>(
            lines.next().expect("session meta for source rewrite"),
        )
        .expect("parse session meta for source rewrite");
        meta["payload"]["source"] = json!({
            "subAgent": {
                "thread_spawn": {
                    "parent_thread_id": "44444444-4444-4444-8444-444444444444"
                }
            }
        });
        meta["payload"]["thread_source"] = json!("subagent");
        meta["payload"]["agent_role"] = json!("worker");
        let rewritten = std::iter::once(meta.to_string())
            .chain(lines.map(str::to_owned))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(rollout, format!("{rewritten}\n")).expect("rewrite structured subagent source");
    }

    fn root(&self) -> &Path {
        self._temp.path()
    }

    fn state_store(&self) -> StateStore {
        let store = StateStore::new(StatePaths::from_root(self.root().join("gpteasy-state")));
        assert!(store.bootstrap().is_ready());
        store
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
                "model_provider": provider,
                "approval_policy": "on-request",
                "sandbox_policy": { "type": "workspace-write" }
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
        insert_fixture_thread(
            &self.codex_home,
            rollout,
            indexed_rollout_path,
            provider,
            source,
            has_user_event,
            archived,
        );
    }

    fn snapshot_bytes(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        collect_files(&self.codex_home, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }
}

const CODEX_0_150_1_THREADS_SCHEMA: &str = r#"
DROP TABLE threads;
CREATE TABLE projects (id TEXT PRIMARY KEY);
CREATE TABLE thread_sections (id TEXT PRIMARY KEY);
CREATE TABLE threads (
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
    has_user_event INTEGER NOT NULL DEFAULT 0,
    archived INTEGER NOT NULL DEFAULT 0,
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
    thread_section_id TEXT REFERENCES thread_sections(id) ON DELETE SET NULL,
    section_position INTEGER,
    section_entered_at_ms INTEGER,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL
);
CREATE INDEX idx_threads_archived ON threads(archived);
CREATE INDEX idx_threads_archived_cwd_created_at_ms ON threads(archived, cwd, created_at_ms DESC, id DESC);
CREATE INDEX idx_threads_archived_cwd_recency_at_ms ON threads(archived, cwd, recency_at_ms DESC, id DESC);
CREATE INDEX idx_threads_archived_cwd_updated_at_ms ON threads(archived, cwd, updated_at_ms DESC, id DESC);
CREATE INDEX idx_threads_created_at ON threads(created_at DESC, id DESC);
CREATE INDEX idx_threads_created_at_ms ON threads(created_at_ms DESC, id DESC);
CREATE INDEX idx_threads_pinned_recency_at_ms ON threads(archived, recency_at_ms DESC, id DESC) WHERE is_pinned = 1 AND preview <> '';
CREATE INDEX idx_threads_project_id ON threads(project_id, archived, created_at_ms DESC, id DESC) WHERE project_id IS NOT NULL;
CREATE INDEX idx_threads_provider ON threads(model_provider);
CREATE INDEX idx_threads_recency_at_ms ON threads(recency_at_ms DESC, id DESC);
CREATE INDEX idx_threads_section_position ON threads(archived, thread_section_id, section_position ASC, id ASC) WHERE thread_section_id IS NOT NULL;
CREATE INDEX idx_threads_section_recency_at_ms ON threads(archived, thread_section_id, recency_at_ms DESC, id DESC) WHERE thread_section_id IS NOT NULL;
CREATE INDEX idx_threads_source ON threads(source);
CREATE INDEX idx_threads_updated_at ON threads(updated_at DESC, id DESC);
CREATE INDEX idx_threads_updated_at_ms ON threads(updated_at_ms DESC, id DESC);
CREATE INDEX idx_threads_visible_created_at_ms ON threads(archived, created_at_ms DESC) WHERE preview <> '';
CREATE INDEX idx_threads_visible_recency_at_ms ON threads(archived, recency_at_ms DESC, id DESC) WHERE preview <> '';
CREATE INDEX idx_threads_visible_updated_at_ms ON threads(archived, updated_at_ms DESC) WHERE preview <> '';
CREATE TRIGGER threads_created_at_ms_after_insert
AFTER INSERT ON threads WHEN NEW.created_at_ms IS NULL
BEGIN
    UPDATE threads SET created_at_ms = NEW.created_at * 1000 WHERE id = NEW.id;
END;
CREATE TRIGGER threads_created_at_ms_after_update
AFTER UPDATE OF created_at ON threads
WHEN NEW.created_at != OLD.created_at AND NEW.created_at_ms IS OLD.created_at_ms
BEGIN
    UPDATE threads SET created_at_ms = NEW.created_at * 1000 WHERE id = NEW.id;
END;
CREATE TRIGGER threads_recency_at_after_insert
AFTER INSERT ON threads WHEN NEW.recency_at_ms = 0
BEGIN
    UPDATE threads
    SET recency_at = NEW.updated_at,
        recency_at_ms = COALESCE(NEW.updated_at_ms, NEW.updated_at * 1000)
    WHERE id = NEW.id;
END;
CREATE TRIGGER threads_updated_at_ms_after_insert
AFTER INSERT ON threads WHEN NEW.updated_at_ms IS NULL
BEGIN
    UPDATE threads SET updated_at_ms = NEW.updated_at * 1000 WHERE id = NEW.id;
END;
CREATE TRIGGER threads_updated_at_ms_after_update
AFTER UPDATE OF updated_at ON threads
WHEN NEW.updated_at != OLD.updated_at AND NEW.updated_at_ms IS OLD.updated_at_ms
BEGIN
    UPDATE threads SET updated_at_ms = NEW.updated_at * 1000 WHERE id = NEW.id;
END;
"#;

fn insert_fixture_thread(
    codex_home: &Path,
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
    Connection::open(codex_home.join("state_5.sqlite"))
        .expect("open state fixture")
        .execute(
            r#"INSERT INTO threads (
                id, rollout_path, created_at, updated_at, source, model_provider,
                cwd, title, sandbox_policy, approval_mode, has_user_event, archived,
                thread_source, agent_path, cli_version, first_user_message, preview
            ) VALUES (
                ?1, ?2, 1787932800, 1787932801, ?3, ?4,
                'C:\private\workspace', 'private-title', '{"type":"workspace-write"}',
                'on-request', ?5, ?6, 'user', NULL, '0.150.1',
                'private-title', 'private-title'
            )"#,
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
