use std::env;
use std::time::Duration;

use gpteasy_lib::session::{
    SessionApplication, SessionAvailabilityStatus, SessionMutationResultStatus, SessionQuery,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use tempfile::TempDir;

/// Runs against the installed Codex executable without reading Codex private
/// storage. It is opt-in because the result depends on the local installation
/// and authentication state of the Windows UAT machine.
#[tokio::test]
#[ignore = "requires GPTEASY_RUN_REAL_CODEX_SESSION_CONTRACT=1 and an installed Codex CLI"]
async fn installed_codex_app_server_supports_the_session_contract() {
    if env::var("GPTEASY_RUN_REAL_CODEX_SESSION_CONTRACT").as_deref() != Ok("1") {
        return;
    }

    let state_root = TempDir::new().expect("state root");
    let store = StateStore::new(StatePaths::from_root(state_root.path()));
    assert!(store.bootstrap().is_ready());
    let application = SessionApplication::new(store);

    let availability = application.enter("real-session-contract").await;
    assert_eq!(availability.status, SessionAvailabilityStatus::Available);
    let page = application
        .list(SessionQuery {
            request_id: None,
            archived: false,
            search_term: None,
            project: None,
            model_provider: None,
            cursor: None,
            limit: 2,
        })
        .await
        .expect("real App Server thread/list");

    if let Some(session) = page.sessions.first() {
        assert!(!session.id.is_empty());
        assert!(!session.title.is_empty());
        let detail = application
            .read(&session.id)
            .await
            .expect("real App Server thread/read");
        assert_eq!(detail.summary.id, session.id);

        let filtered = application
            .list(SessionQuery {
                request_id: None,
                archived: false,
                search_term: Some(session.title.clone()),
                project: (!session.project.is_empty()).then(|| session.project.clone()),
                model_provider: (!session.model_provider.is_empty())
                    .then(|| session.model_provider.clone()),
                cursor: None,
                limit: 2,
            })
            .await
            .expect("real App Server metadata filters");
        assert!(
            filtered
                .sessions
                .iter()
                .all(|candidate| candidate.source != "exec" && candidate.source != "subAgent")
        );

        if let Some(cursor) = page.next_cursor {
            application
                .list(SessionQuery {
                    request_id: None,
                    archived: false,
                    search_term: None,
                    project: None,
                    model_provider: None,
                    cursor: Some(cursor),
                    limit: 2,
                })
                .await
                .expect("real App Server cursor");
        }

        if env::var("GPTEASY_RUN_REAL_CODEX_SESSION_MUTATIONS").as_deref() == Ok("1") {
            let mutation_id = env::var("GPTEASY_REAL_CODEX_SESSION_ID")
                .expect("GPTEASY_REAL_CODEX_SESSION_ID is required for mutations");
            let archived = application.archive(vec![mutation_id.clone()]).await;
            assert_eq!(archived[0].status, SessionMutationResultStatus::Succeeded);
            let restored = application.unarchive(vec![mutation_id.clone()]).await;
            assert_eq!(restored[0].status, SessionMutationResultStatus::Succeeded);
            if env::var("GPTEASY_ALLOW_REAL_CODEX_DELETE").as_deref() == Ok("1") {
                let deleted = application.delete(&mutation_id).await;
                assert_eq!(deleted.status, SessionMutationResultStatus::Succeeded);
            }
        }
    }
    application.leave("real-session-contract").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    application.shutdown_now().await;
}
