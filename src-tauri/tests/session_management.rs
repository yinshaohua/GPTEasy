use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use gpteasy_lib::session::{
    SessionApplication, SessionAvailabilityStatus, SessionEntryKind, SessionFailureCategory,
    SessionMutationAvailabilityStatus, SessionMutationResultStatus, SessionQuery,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use rusqlite::Connection;
use tempfile::TempDir;

#[tokio::test]
async fn public_session_interface_uses_the_app_server_contract_for_read_only_history() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec!["--fixture-log".into(), harness.log().as_os_str().to_owned()],
        Duration::from_millis(25),
    );

    let availability = application.enter("page-lease").await;
    assert_eq!(availability.status, SessionAvailabilityStatus::Available);

    let page = application
        .list(SessionQuery {
            request_id: None,
            archived: false,
            search_term: Some("登录".to_owned()),
            project: Some(r"C:\src\demo".to_owned()),
            model_provider: Some("history-provider".to_owned()),
            cursor: None,
            limit: 40,
        })
        .await
        .expect("list sessions");
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].title, "登录修复");
    assert_eq!(page.sessions[0].model_provider, "history-provider");
    assert_eq!(page.sessions[0].updated_at, 1_786_800_300);
    assert_eq!(page.next_cursor.as_deref(), Some("cursor-2"));

    application
        .list(SessionQuery {
            request_id: None,
            archived: false,
            search_term: None,
            project: None,
            model_provider: None,
            cursor: Some("cursor-2".to_owned()),
            limit: 40,
        })
        .await
        .expect("list the next cursor page");

    let detail = application.read("thread-1").await.expect("read session");
    assert_eq!(detail.entries.len(), 3);
    assert_eq!(detail.entries[0].kind, SessionEntryKind::User);
    assert_eq!(detail.entries[1].kind, SessionEntryKind::Tool);
    assert_eq!(detail.entries[2].kind, SessionEntryKind::Assistant);
    assert_eq!(detail.entries[1].output.as_deref(), Some("all passed"));

    let markdown = detail.to_markdown();
    assert!(markdown.contains("# 登录修复"));
    assert!(markdown.contains("## 用户\n\n请修复登录"));
    assert!(markdown.contains("```text\nnpm test\n```"));
    assert!(markdown.contains("## 助手\n\n登录流程已修复。"));

    let destination = state_root.path().join("session.md");
    application
        .export_markdown(&detail, &destination)
        .await
        .expect("export session");
    assert_eq!(
        fs::read_to_string(destination).expect("read export"),
        markdown,
    );

    application.leave("page-lease").await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert!(log.contains(r#""method":"initialize""#));
    assert!(log.contains(r#""method":"initialized""#));
    assert!(log.contains(r#""method":"thread/list""#));
    assert!(log.contains(r#""sortKey":"recency_at""#));
    assert!(log.contains(r#""sortDirection":"desc""#));
    assert!(log.contains(r#""sourceKinds":["cli","vscode","appServer"]"#));
    assert!(log.contains(r#""searchTerm":"登录""#));
    assert!(log.contains(r#""modelProviders":["history-provider"]"#));
    assert!(log.contains(r#""cursor":"cursor-2""#));
    assert!(log.contains(r#""method":"thread/read""#));
    assert!(
        log.contains(r#""includeTurns":true"#),
        "fixture log:\n{log}"
    );
    assert!(log.contains("EOF"));
    let connection = Connection::open(state_root.path().join("state.sqlite3")).expect("open state");
    let ownership_count = connection
        .query_row(
            "SELECT count(*) FROM session_process_ownership",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("read ownership state");
    assert_eq!(ownership_count, 0);
    let capability_count = connection
        .query_row("SELECT count(*) FROM session_capability", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("read capability state");
    assert_eq!(capability_count, 1);
}

#[tokio::test]
async fn unfiltered_session_list_explicitly_requests_all_model_providers() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--require-explicit-all-model-providers".into(),
        ],
        Duration::from_millis(25),
    );

    assert_eq!(
        application.enter("all-provider-lease").await.status,
        SessionAvailabilityStatus::Available,
    );
    let page = application
        .list(default_query())
        .await
        .expect("list sessions from all model providers");

    assert_eq!(page.sessions.len(), 1);
    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert!(log.lines().any(|line| {
        line.contains(r#""method":"thread/list""#)
            && line.contains(r#""limit":40"#)
            && line.contains(r#""modelProviders":[]"#)
    }));
    application.shutdown_now().await;
}

#[tokio::test]
async fn app_server_liveness_check_neither_starts_nor_queries_the_server() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec!["--fixture-log".into(), harness.log().as_os_str().to_owned()],
        Duration::from_millis(25),
    );

    assert!(application.active_app_server_version().await.is_none());
    assert!(!harness.log().exists());
    assert_eq!(
        application.enter("liveness-lease").await.status,
        SessionAvailabilityStatus::Available,
    );
    let before = fs::read_to_string(harness.log()).expect("read protocol log");

    assert_eq!(
        application.active_app_server_version().await.as_deref(),
        Some("codex-cli 0.147.0-fixture"),
    );
    assert_eq!(
        fs::read_to_string(harness.log()).expect("read unchanged protocol log"),
        before,
    );

    application.shutdown_now().await;
    let after_shutdown = fs::read_to_string(harness.log()).expect("read shutdown log");
    assert!(application.active_app_server_version().await.is_none());
    assert_eq!(
        fs::read_to_string(harness.log()).expect("read final protocol log"),
        after_shutdown,
    );
}

#[tokio::test]
async fn app_server_liveness_check_does_not_clean_up_an_exited_process() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store.clone(),
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--exit-after-capability".into(),
        ],
        Duration::from_millis(25),
    );

    assert_eq!(
        application.enter("exited-liveness-lease").await.status,
        SessionAvailabilityStatus::Available,
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before_log = fs::read_to_string(harness.log()).expect("read protocol log");
    let ownership = store
        .session_process_ownership()
        .expect("preserve exited process ownership until lifecycle cleanup");

    assert!(application.active_app_server_version().await.is_none());
    assert_eq!(
        store.session_process_ownership(),
        Some(ownership),
        "read-only liveness must not modify GPTEasy state",
    );
    assert_eq!(
        fs::read_to_string(harness.log()).expect("read unchanged protocol log"),
        before_log,
    );

    application.shutdown_now().await;
}

#[tokio::test]
async fn list_keeps_internal_sources_out_even_when_the_server_ignores_source_kinds() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--mixed-sources".into(),
        ],
        Duration::from_millis(25),
    );

    assert_eq!(
        application.enter("source-boundary-lease").await.status,
        SessionAvailabilityStatus::Available
    );
    let page = application
        .list(default_query())
        .await
        .expect("list sessions");

    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].id, "thread-1");
    application.shutdown_now().await;
}

#[tokio::test]
async fn missing_optional_thread_metadata_degrades_to_empty_values() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--legacy-metadata".into(),
        ],
        Duration::from_millis(25),
    );

    assert_eq!(
        application.enter("legacy-metadata-lease").await.status,
        SessionAvailabilityStatus::Available
    );
    let page = application
        .list(default_query())
        .await
        .expect("missing optional metadata must not fail listing");

    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].id, "legacy-thread");
    assert_eq!(page.sessions[0].project, "");
    assert_eq!(page.sessions[0].created_at, 0);
    assert_eq!(page.sessions[0].updated_at, 0);
    application.shutdown_now().await;
}

#[tokio::test]
async fn stale_list_request_is_cancelled_through_the_public_session_interface() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = Arc::new(SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--slow-list".into(),
        ],
        Duration::from_millis(25),
    ));
    assert_eq!(
        application.enter("cancel-list-lease").await.status,
        SessionAvailabilityStatus::Available,
    );

    let request_application = Arc::clone(&application);
    let request = tokio::spawn(async move {
        request_application
            .list(SessionQuery {
                request_id: Some("stale-list".to_owned()),
                archived: false,
                search_term: Some("pending-cancellation".to_owned()),
                project: None,
                model_provider: None,
                cursor: None,
                limit: 40,
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if fs::read_to_string(harness.log())
                .unwrap_or_default()
                .contains("pending-cancellation")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("list request reaches the App Server");

    assert!(application.cancel_list_request("stale-list").await);
    let failure = tokio::time::timeout(Duration::from_millis(500), request)
        .await
        .expect("cancelled list returns promptly")
        .expect("join list request")
        .expect_err("cancelled list request must fail");
    assert_eq!(failure.category, SessionFailureCategory::Cancelled);
    assert_eq!(failure.message_id, "session.request_cancelled");
    application.shutdown_now().await;
}

#[tokio::test]
async fn read_only_request_restarts_once_after_an_unexpected_exit() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let restart_marker = harness.temp_path().join("restart-marker");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--exit-first-list".into(),
            restart_marker.as_os_str().to_owned(),
        ],
        Duration::from_millis(25),
    );

    assert_eq!(
        application.enter("recovery-lease").await.status,
        SessionAvailabilityStatus::Available,
    );
    let page = application
        .list(SessionQuery {
            request_id: None,
            archived: false,
            search_term: None,
            project: None,
            model_provider: None,
            cursor: None,
            limit: 40,
        })
        .await
        .expect("recover the read once");
    assert_eq!(page.sessions.len(), 1);
    application.shutdown_now().await;

    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(log.matches(r#""method":"initialize""#).count(), 2);
    assert_eq!(log.matches(r#""method":"thread/list""#).count(), 4);
}

#[tokio::test]
async fn missing_core_method_disables_session_management_before_listing_history() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--missing-thread-read".into(),
        ],
        Duration::from_millis(25),
    );

    let availability = application.enter("incompatible-lease").await;
    assert_eq!(availability.status, SessionAvailabilityStatus::Incompatible);

    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(log.matches(r#""method":"initialize""#).count(), 1);
    assert_eq!(log.matches(r#""method":"thread/list""#).count(), 1);
    assert_eq!(log.matches(r#""method":"thread/read""#).count(), 1);
    assert!(log.contains("EOF"));
}

#[tokio::test]
async fn missing_mutation_method_disables_session_management_before_user_actions() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--missing-thread-delete".into(),
        ],
        Duration::from_millis(20),
    );

    let availability = application.enter("missing-mutation-lease").await;

    assert_eq!(availability.status, SessionAvailabilityStatus::Incompatible);
    assert_eq!(availability.message_id, "session.incompatible");
}

#[tokio::test]
async fn failed_version_probe_never_starts_an_unidentified_app_server() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store.clone(),
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--version-fails".into(),
        ],
        Duration::from_millis(25),
    );

    let availability = application.enter("version-lease").await;
    assert_eq!(
        availability.status,
        SessionAvailabilityStatus::InitializationFailed
    );
    assert!(store.session_process_ownership().is_none());
    assert!(!harness.log().exists());
}

#[tokio::test]
async fn repeated_unexpected_exit_is_fused_until_an_explicit_recheck() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--always-exit-list".into(),
        ],
        Duration::from_millis(25),
    );

    assert_eq!(
        application.enter("fused-recovery-lease").await.status,
        SessionAvailabilityStatus::Available,
    );
    let failure = application
        .list(default_query())
        .await
        .expect_err("the single automatic retry also exits");
    assert_eq!(failure.category, SessionFailureCategory::RecoveryFailed);
    let second_failure = application
        .list(default_query())
        .await
        .expect_err("a new request must not create a restart loop");
    assert_eq!(
        second_failure.category,
        SessionFailureCategory::RecoveryFailed
    );
    application.shutdown_now().await;

    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(log.matches(r#""method":"initialize""#).count(), 2);
    assert_eq!(
        log.lines()
            .filter(
                |line| line.contains(r#""method":"thread/list""#) && line.contains(r#""limit":40"#)
            )
            .count(),
        2,
    );
}

#[tokio::test]
async fn returning_during_idle_grace_reuses_the_owned_app_server() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec!["--fixture-log".into(), harness.log().as_os_str().to_owned()],
        Duration::from_millis(80),
    );

    assert_eq!(
        application.enter("idle-lease").await.status,
        SessionAvailabilityStatus::Available,
    );
    application.suspend().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    application.resume().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    application
        .list(default_query())
        .await
        .expect("the original process remains usable");
    application.shutdown_now().await;

    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(log.matches(r#""method":"initialize""#).count(), 1);
}

#[tokio::test]
async fn official_session_mutations_do_not_require_consumer_preconditions() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec!["--fixture-log".into(), harness.log().as_os_str().to_owned()],
        Duration::from_millis(20),
    );

    let availability = application.enter("mutation-lease").await;
    assert_eq!(
        availability.mutation.status,
        SessionMutationAvailabilityStatus::Allowed,
    );
    let archived = application.archive(vec!["thread-1".to_owned()]).await;
    let deleted = application.delete("thread-2").await;

    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].status, SessionMutationResultStatus::Succeeded);
    assert_eq!(archived[0].message_id, "session.archived");
    assert_eq!(deleted.status, SessionMutationResultStatus::Succeeded);
    assert_eq!(deleted.message_id, "session.deleted");
    application.shutdown_now().await;
    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert!(log.lines().any(|line| {
        line.contains(r#""method":"thread/archive""#) && line.contains(r#""threadId":"thread-1""#)
    }));
    assert!(log.lines().any(|line| {
        line.contains(r#""method":"thread/delete""#) && line.contains(r#""threadId":"thread-2""#)
    }));
}

#[tokio::test]
async fn batch_archive_returns_each_official_result_without_rolling_back_successes() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--fail-archive".into(),
            "thread-2".into(),
        ],
        Duration::from_millis(20),
    );
    assert_eq!(
        application.enter("batch-lease").await.status,
        SessionAvailabilityStatus::Available,
    );

    let results = application
        .archive(vec!["thread-1".to_owned(), "thread-2".to_owned()])
        .await;

    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].status,
        SessionMutationResultStatus::Succeeded,
        "{results:?}",
    );
    assert_eq!(results[0].message_id, "session.archived");
    assert_eq!(results[1].status, SessionMutationResultStatus::Failed);
    assert_eq!(results[1].message_id, "session.request_failed");
    application.shutdown_now().await;
    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(
        log.lines()
            .filter(|line| {
                line.contains(r#""method":"thread/archive""#)
                    && (line.contains(r#""threadId":"thread-1""#)
                        || line.contains(r#""threadId":"thread-2""#))
            })
            .count(),
        2,
    );
}

#[tokio::test]
async fn lost_archive_response_reconnects_and_refreshes_state_without_retrying_the_mutation() {
    let harness = AppServerHarness::new();
    let archived_marker = harness.temp_path().join("archived.marker");
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--lose-archive-response".into(),
            archived_marker.as_os_str().to_owned(),
        ],
        Duration::from_millis(20),
    );
    assert_eq!(
        application.enter("lost-response-lease").await.status,
        SessionAvailabilityStatus::Available,
    );

    let results = application.archive(vec!["thread-1".to_owned()]).await;

    assert_eq!(
        results[0].status,
        SessionMutationResultStatus::Succeeded,
        "{results:?}",
    );
    assert_eq!(
        results[0].actual_state,
        gpteasy_lib::session::SessionActualState::Archived
    );
    assert_eq!(results[0].message_id, "session.archive_reconciled");
    application.shutdown_now().await;
    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(
        log.lines()
            .filter(|line| {
                line.contains(r#""method":"thread/archive""#)
                    && line.contains(r#""threadId":"thread-1""#)
            })
            .count(),
        1,
    );
    assert_eq!(log.matches(r#""method":"initialize""#).count(), 2);
}

#[tokio::test]
async fn lost_unarchive_response_reconnects_and_refreshes_state_without_retrying_the_mutation() {
    let harness = AppServerHarness::new();
    let active_marker = harness.temp_path().join("active.marker");
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--lose-unarchive-response".into(),
            active_marker.as_os_str().to_owned(),
        ],
        Duration::from_millis(20),
    );
    assert_eq!(
        application.enter("lost-unarchive-lease").await.status,
        SessionAvailabilityStatus::Available,
    );

    let results = application.unarchive(vec!["thread-1".to_owned()]).await;

    assert_eq!(results[0].status, SessionMutationResultStatus::Succeeded);
    assert_eq!(
        results[0].actual_state,
        gpteasy_lib::session::SessionActualState::Active,
    );
    assert_eq!(results[0].message_id, "session.unarchive_reconciled");
    application.shutdown_now().await;
    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(
        log.lines()
            .filter(|line| {
                line.contains(r#""method":"thread/unarchive""#)
                    && line.contains(r#""threadId":"thread-1""#)
            })
            .count(),
        1,
    );
    assert_eq!(log.matches(r#""method":"initialize""#).count(), 2);
}

#[tokio::test]
async fn lost_delete_response_reconnects_and_refreshes_state_without_retrying_the_mutation() {
    let harness = AppServerHarness::new();
    let deleted_marker = harness.temp_path().join("deleted.marker");
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec![
            "--fixture-log".into(),
            harness.log().as_os_str().to_owned(),
            "--lose-delete-response".into(),
            deleted_marker.as_os_str().to_owned(),
        ],
        Duration::from_millis(20),
    );
    assert_eq!(
        application.enter("lost-delete-lease").await.status,
        SessionAvailabilityStatus::Available,
    );

    let result = application.delete("thread-1").await;

    assert_eq!(result.status, SessionMutationResultStatus::Succeeded);
    assert_eq!(
        result.actual_state,
        gpteasy_lib::session::SessionActualState::Deleted,
    );
    assert_eq!(result.message_id, "session.delete_reconciled");
    application.shutdown_now().await;
    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(
        log.lines()
            .filter(|line| {
                line.contains(r#""method":"thread/delete""#)
                    && line.contains(r#""threadId":"thread-1""#)
            })
            .count(),
        1,
    );
    assert_eq!(log.matches(r#""method":"initialize""#).count(), 2);
}

#[tokio::test]
async fn unarchive_and_permanent_delete_use_their_distinct_official_methods() {
    let harness = AppServerHarness::new();
    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::with_program_for_harness(
        store,
        harness.program(),
        vec!["--fixture-log".into(), harness.log().as_os_str().to_owned()],
        Duration::from_millis(20),
    );
    assert_eq!(
        application.enter("distinct-methods-lease").await.status,
        SessionAvailabilityStatus::Available,
    );

    let unarchived = application.unarchive(vec!["thread-1".to_owned()]).await;
    let deleted = application.delete("thread-2").await;

    assert_eq!(unarchived[0].status, SessionMutationResultStatus::Succeeded);
    assert_eq!(
        unarchived[0].actual_state,
        gpteasy_lib::session::SessionActualState::Active,
    );
    assert_eq!(deleted.status, SessionMutationResultStatus::Succeeded);
    assert_eq!(
        deleted.actual_state,
        gpteasy_lib::session::SessionActualState::Deleted,
    );
    application.shutdown_now().await;
    let log = fs::read_to_string(harness.log()).expect("read fixture log");
    assert_eq!(
        log.lines()
            .filter(|line| {
                line.contains(r#""method":"thread/unarchive""#)
                    && line.contains(r#""threadId":"thread-1""#)
            })
            .count(),
        1,
    );
    assert_eq!(
        log.lines()
            .filter(|line| {
                line.contains(r#""method":"thread/delete""#)
                    && line.contains(r#""threadId":"thread-2""#)
            })
            .count(),
        1,
    );
}

fn default_query() -> SessionQuery {
    SessionQuery {
        request_id: None,
        archived: false,
        search_term: None,
        project: None,
        model_provider: None,
        cursor: None,
        limit: 40,
    }
}

struct AppServerHarness {
    _temp: TempDir,
    program: PathBuf,
    log: PathBuf,
}

impl AppServerHarness {
    fn new() -> Self {
        let temp = TempDir::new().expect("fixture temp");
        let program = temp.path().join(if cfg!(windows) {
            "app-server-fixture.exe"
        } else {
            "app-server-fixture"
        });
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/app_server_fixture.rs");
        let compiled = Command::new("rustc")
            .args(["--edition=2024"])
            .arg(source)
            .arg("-o")
            .arg(&program)
            .output()
            .expect("compile app-server fixture");
        assert!(
            compiled.status.success(),
            "compile app-server fixture: {}",
            String::from_utf8_lossy(&compiled.stderr),
        );
        let log = temp.path().join("app-server.jsonl");
        Self {
            _temp: temp,
            program,
            log,
        }
    }

    fn program(&self) -> PathBuf {
        self.program.clone()
    }

    fn log(&self) -> &Path {
        &self.log
    }

    fn temp_path(&self) -> &Path {
        self._temp.path()
    }
}
