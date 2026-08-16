#[path = "support/state.rs"]
mod state_support;

use std::fs;
use std::path::PathBuf;

use gpteasy_lib::state::{
    CURRENT_SCHEMA_VERSION, DatabaseBlockReason, DatabaseStatus, StatePaths, StateStore,
};
use rusqlite::Connection;
use tempfile::TempDir;

use state_support::create_version_zero_database;

fn store_in(temp: &TempDir) -> StateStore {
    StateStore::new(StatePaths::from_root(temp.path()))
}

#[test]
fn fresh_install_initializes_only_the_minimum_schema() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);

    let snapshot = store.bootstrap();

    assert_eq!(
        snapshot.status,
        DatabaseStatus::Initialized,
        "unexpected bootstrap snapshot: {snapshot:?}"
    );
    assert_eq!(snapshot.schema_version, Some(CURRENT_SCHEMA_VERSION));
    let contents = snapshot.contents.expect("initialized database contents");
    assert_eq!(contents.provider_count, 0);
    assert!(!contents.has_last_applied_state);
    assert!(!contents.has_pending_config_operation);
    assert!(!contents.pending_restart);
    assert!(store.paths().database().is_file());
    assert!(store.paths().installation_marker().is_file());

    let connection = Connection::open(store.paths().database()).expect("open initialized database");
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_schema \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .expect("prepare schema query");
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query schema")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect schema");

    assert_eq!(
        tables,
        vec![
            "app_state",
            "last_applied_state",
            "pending_config_operation",
            "providers",
            "session_capability",
            "session_process_ownership",
            "wsl_environments",
            "wsl_pending_operation",
        ]
    );
}

#[test]
fn first_close_notice_is_recorded_only_after_it_is_shown() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    assert!(store.bootstrap().is_ready());

    assert!(store.should_show_first_close_notice());
    assert!(store.mark_first_close_notice_seen());
    assert!(!store.should_show_first_close_notice());
}

#[test]
fn first_close_notice_check_does_not_create_a_missing_database() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);

    assert!(!store.should_show_first_close_notice());
    assert!(!store.mark_first_close_notice_seen());
    assert!(!store.paths().database().exists());
}

#[test]
fn migration_uses_a_consistent_backup_and_keeps_only_three() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    create_version_zero_database(&store);

    let migrated = store.bootstrap();

    assert_eq!(migrated.status, DatabaseStatus::Ready);
    assert_eq!(migrated.schema_version, Some(CURRENT_SCHEMA_VERSION));
    let first_backup = fs::read_dir(store.paths().backups())
        .expect("read backup directory")
        .map(|entry| entry.expect("backup entry").path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "sqlite3")
        })
        .expect("migration backup");
    Connection::open(&first_backup)
        .expect("open backup")
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .expect("backup integrity check");

    for suffix in ["9001", "9002", "9003", "9004"] {
        fs::copy(
            &first_backup,
            store
                .paths()
                .backups()
                .join(format!("state.sqlite3.backup.{suffix}.sqlite3")),
        )
        .expect("duplicate valid backup");
    }

    let ready = store.bootstrap();
    assert_eq!(ready.status, DatabaseStatus::Ready);
    let retained = fs::read_dir(store.paths().backups())
        .expect("read pruned backups")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "sqlite3")
        })
        .count();
    assert_eq!(retained, 3);
}

#[test]
fn recommendation_migration_never_claims_or_overwrites_an_existing_dayway_name() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute("DROP TABLE wsl_pending_operation", [])
        .expect("remove v5 pending operation table");
    connection
        .execute("DROP TABLE wsl_environments", [])
        .expect("remove v5 environment table");
    connection
        .execute("DROP TABLE session_process_ownership", [])
        .expect("remove v8 process ownership table");
    connection
        .execute("DROP TABLE session_capability", [])
        .expect("remove v8 capability table");
    connection
        .execute("DROP INDEX providers_recommendation_id_unique", [])
        .expect("remove v4 index");
    connection
        .execute(
            "ALTER TABLE providers DROP COLUMN recommendation_template_base_url",
            [],
        )
        .expect("remove v4 template snapshot");
    connection
        .execute("ALTER TABLE providers DROP COLUMN recommendation_id", [])
        .expect("restore v3 provider schema");
    connection
        .execute(
            "INSERT INTO providers (id, name, base_url, api_key, default_model, verified_at, verification_fingerprint, sort_order)
             VALUES ('existing-id', 'DayWay', 'https://saved.example/v1', 'saved-key', 'saved-model', '123', 'saved-fingerprint', 0)",
            [],
        )
        .expect("insert existing same-name provider");
    connection
        .pragma_update(None, "user_version", 3_i64)
        .expect("mark v3 database");
    drop(connection);

    assert_eq!(store.bootstrap().status, DatabaseStatus::Ready);
    let connection = Connection::open(store.paths().database()).expect("open upgraded database");
    let preserved = connection
        .query_row(
            "SELECT name, base_url, api_key, default_model, verification_fingerprint, recommendation_id
             FROM providers WHERE id = 'existing-id'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<String>>(5)?)),
        )
        .expect("read preserved provider");
    assert_eq!(
        preserved,
        (
            "DayWay".to_owned(),
            "https://saved.example/v1".to_owned(),
            "saved-key".to_owned(),
            "saved-model".to_owned(),
            "saved-fingerprint".to_owned(),
            None,
        )
    );
}

#[test]
fn migration_failure_recovers_the_consistent_backup_through_the_public_gate() {
    let temp = TempDir::new().expect("temp dir");
    let paths = StatePaths::from_root(temp.path());
    let setup = StateStore::new(paths.clone());
    create_version_zero_database(&setup);
    let store = state_support::with_migration_failure(paths);

    let recovered = store.bootstrap();

    assert_eq!(recovered.status, DatabaseStatus::Recovered);
    assert_eq!(recovered.schema_version, Some(CURRENT_SCHEMA_VERSION));
    assert!(
        fs::read_dir(store.paths().root())
            .expect("read state root")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("state.sqlite3.failed.")),
        "the failed primary database must remain available for investigation"
    );
}

#[test]
fn existing_install_with_missing_database_fails_closed() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Initialized);
    fs::remove_file(store.paths().database()).expect("simulate missing database");

    let blocked = store.bootstrap();

    assert_eq!(blocked.status, DatabaseStatus::Blocked);
    assert_eq!(blocked.reason, Some(DatabaseBlockReason::MissingDatabase));
    assert!(!store.paths().database().exists());
}

#[test]
fn missing_database_restores_the_latest_valid_backup() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    create_version_zero_database(&store);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Ready);
    fs::remove_file(store.paths().database()).expect("simulate missing database");

    let recovered = store.bootstrap();

    assert_eq!(recovered.status, DatabaseStatus::Recovered);
    assert_eq!(recovered.schema_version, Some(CURRENT_SCHEMA_VERSION));
    assert!(store.paths().database().is_file());
}

#[test]
fn corrupt_database_is_preserved_and_latest_valid_backup_is_restored() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    create_version_zero_database(&store);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Ready);
    fs::write(store.paths().database(), b"not a sqlite database").expect("corrupt database");

    let recovered = store.bootstrap();

    assert_eq!(recovered.status, DatabaseStatus::Recovered);
    assert_eq!(recovered.schema_version, Some(CURRENT_SCHEMA_VERSION));
    let preserved = fs::read_dir(store.paths().root())
        .expect("read state root")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("state.sqlite3.failed.")
        });
    assert!(
        preserved,
        "the corrupt primary database must remain available"
    );
}

#[test]
fn recovery_skips_a_newer_invalid_backup() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    create_version_zero_database(&store);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Ready);
    fs::write(
        store
            .paths()
            .backups()
            .join("state.sqlite3.backup.99999999999999999999.sqlite3"),
        b"not a sqlite backup",
    )
    .expect("write newer invalid backup");
    fs::write(store.paths().database(), b"not a sqlite database").expect("corrupt database");

    let recovered = store.bootstrap();

    assert_eq!(recovered.status, DatabaseStatus::Recovered);
    assert_eq!(recovered.schema_version, Some(CURRENT_SCHEMA_VERSION));
}

#[test]
fn zero_byte_existing_database_is_not_migrated_into_an_empty_state() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Initialized);
    fs::write(store.paths().database(), []).expect("replace database with zero bytes");

    let blocked = store.bootstrap();

    assert_eq!(blocked.status, DatabaseStatus::Blocked);
    assert_eq!(blocked.reason, Some(DatabaseBlockReason::CorruptDatabase));
    assert_eq!(fs::metadata(store.paths().database()).unwrap().len(), 0);
}

#[test]
fn negative_schema_version_fails_closed() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Initialized);
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .pragma_update(None, "user_version", -1_i64)
        .expect("write negative schema version");
    drop(connection);

    let blocked = store.bootstrap();

    assert_eq!(blocked.status, DatabaseStatus::Blocked);
    assert_eq!(blocked.reason, Some(DatabaseBlockReason::CorruptDatabase));
}

#[test]
fn current_schema_with_wrong_columns_fails_closed() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Initialized);
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute_batch("DROP TABLE providers; CREATE TABLE providers (id TEXT PRIMARY KEY) STRICT;")
        .expect("tamper provider schema");
    drop(connection);

    let blocked = store.bootstrap();

    assert_eq!(blocked.status, DatabaseStatus::Blocked);
    assert_eq!(blocked.reason, Some(DatabaseBlockReason::CorruptDatabase));
}

#[test]
fn future_schema_is_never_opened_as_an_empty_current_database() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    fs::create_dir_all(store.paths().root()).expect("create state root");
    let connection = Connection::open(store.paths().database()).expect("create future database");
    connection
        .execute_batch(
            "PRAGMA application_id = 1196446789; \
             CREATE TABLE future_data (value TEXT); PRAGMA user_version = 99;",
        )
        .expect("create future schema");
    drop(connection);
    fs::write(store.paths().installation_marker(), b"gpteasy-state-v1\n")
        .expect("write install marker");
    let original = fs::read(store.paths().database()).expect("read original future database");

    let blocked = store.bootstrap();

    assert_eq!(blocked.status, DatabaseStatus::Blocked);
    assert_eq!(blocked.reason, Some(DatabaseBlockReason::FutureSchema));
    assert_eq!(
        fs::read(store.paths().database()).expect("read preserved future database"),
        original
    );
}

#[test]
fn future_schema_is_preserved_when_an_older_backup_is_recovered() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    create_version_zero_database(&store);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Ready);
    fs::remove_file(store.paths().database()).expect("remove current database");
    let connection = Connection::open(store.paths().database()).expect("create future database");
    connection
        .execute_batch(
            "PRAGMA application_id = 1196446789; \
             CREATE TABLE future_data (value TEXT); PRAGMA user_version = 99;",
        )
        .expect("create future schema");
    drop(connection);

    let recovered = store.bootstrap();

    assert_eq!(recovered.status, DatabaseStatus::Recovered);
    let preserved_future = fs::read_dir(store.paths().root())
        .expect("read state root")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("state.sqlite3.failed.")
        });
    assert!(
        preserved_future,
        "future schema must be preserved before recovery"
    );
}

#[test]
fn schema_enforces_a_single_pending_operation() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    assert_eq!(store.bootstrap().status, DatabaseStatus::Initialized);
    let connection = Connection::open(store.paths().database()).expect("open state database");
    let insert = "INSERT INTO pending_config_operation (\
        singleton, operation_id, operation_kind, stage, backup_reference, target_snapshot_json, started_at\
    ) VALUES (?1, ?2, 'switch_provider', 'registered', 'backup-ref', '{}', '2026-08-07T00:00:00Z')";
    connection
        .execute(insert, (1_i64, "operation-1"))
        .expect("insert first pending operation");

    let second = connection.execute(insert, (2_i64, "operation-2"));

    assert!(
        second.is_err(),
        "a second pending operation must be rejected"
    );
}

#[test]
fn formal_v001_fixture_is_empty_valid_and_upgradeable() {
    let temp = TempDir::new().expect("temp dir");
    let store = store_in(&temp);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("databases")
        .join("v001")
        .join("state.sqlite3");
    fs::create_dir_all(store.paths().root()).expect("create state root");
    fs::copy(&fixture, store.paths().database()).expect("copy v001 fixture");
    fs::write(store.paths().installation_marker(), b"gpteasy-state-v1\n")
        .expect("write installation marker");

    let snapshot = store.bootstrap();

    assert_eq!(snapshot.status, DatabaseStatus::Ready);
    assert_eq!(snapshot.schema_version, Some(CURRENT_SCHEMA_VERSION));
    let connection = Connection::open(store.paths().database()).expect("open fixture copy");
    let provider_count = connection
        .query_row("SELECT count(*) FROM providers", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("count fixture providers");
    assert_eq!(
        provider_count, 0,
        "historical fixtures must contain no credentials"
    );
}
