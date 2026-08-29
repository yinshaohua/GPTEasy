use std::fs;
use std::path::{Path, PathBuf};

use gpteasy_lib::session_visibility::{
    SessionVisibilityApplication, VisibilityAppServerCapability, VisibilityScanContext,
    VisibilityTarget, VisibilityTargetMode,
};
use rusqlite::{Connection, params};
use serde_json::json;
use tempfile::TempDir;

const TARGET_PROVIDER: &str = "4c8f7402-669f-40cf-a2a9-cfc6f124de6d";

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
    assert_eq!(preview.summary.candidates, 1);
    assert_eq!(preview.summary.missing_index, 1);
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
        "candidates=1",
        "missing_index=1",
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
