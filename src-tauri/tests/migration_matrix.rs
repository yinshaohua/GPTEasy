use std::{fs, path::Path};

use gpteasy_lib::state::migrations::{
    migration_checksum, schema_fingerprint, APPLICATION_ID, CURRENT_SCHEMA_VERSION, MIGRATIONS,
};
use rusqlite::{params, types::ValueRef, Connection, OpenFlags, TransactionBehavior};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

const FIXED_MIGRATION_TIME: &str = "2026-08-01T00:00:01.000Z";
const TEST_V2_SQL: &str = "CREATE TABLE migration_matrix_v2 (\n    id INTEGER PRIMARY KEY,\n    label TEXT NOT NULL\n) STRICT;\nINSERT INTO migration_matrix_v2 (id, label) VALUES (1, 'v2-applied');\n";
const TEST_V3_SQL: &str = "ALTER TABLE migration_matrix_v2 ADD COLUMN sequence_marker INTEGER NOT NULL DEFAULT 0;\nUPDATE migration_matrix_v2 SET sequence_marker = 3 WHERE id = 1;\n";

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    format_version: u32,
    fixtures: Vec<FixtureEntry>,
}

#[derive(Debug, Deserialize)]
struct FixtureEntry {
    fixture_id: String,
    database_path: String,
    file_sha256: String,
    schema_digest_sha256: String,
    data_digest_sha256: String,
    logical_digest_sha256: String,
    application_id: i64,
    user_version: u32,
    schema_fingerprint: String,
    migration: FixtureMigration,
}

#[derive(Debug, Deserialize)]
struct FixtureMigration {
    version: u32,
    name: String,
    checksum: String,
}

#[derive(Clone, Debug)]
struct MatrixMigration<'a> {
    version: u32,
    name: &'a str,
    sql: &'a str,
    checksum: String,
    fingerprint: String,
}

#[test]
fn manifest_drives_every_historical_fixture_through_the_upgrade_matrix() {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("tests/fixtures/databases");
    let manifest: FixtureManifest = serde_json::from_slice(
        &fs::read(fixture_root.join("manifest.json")).expect("read fixture manifest"),
    )
    .expect("parse fixture manifest");

    assert_eq!(manifest.format_version, 1);
    assert!(
        !manifest.fixtures.is_empty(),
        "fixture manifest must not be empty"
    );

    for fixture in &manifest.fixtures {
        let source = fixture_root.join(&fixture.database_path);
        let before_bytes = fs::read(&source).expect("read committed fixture");
        let before_modified = fs::metadata(&source)
            .expect("read committed fixture metadata")
            .modified()
            .expect("read committed fixture mtime");

        assert_eq!(sha256_hex(&before_bytes), fixture.file_sha256);
        assert_fixture_identity(&source, fixture);
        upgrade_fixture_copy(&source, fixture);

        assert_eq!(
            fs::read(&source).expect("reread committed fixture"),
            before_bytes
        );
        assert_eq!(
            fs::metadata(&source)
                .expect("reread committed fixture metadata")
                .modified()
                .expect("reread committed fixture mtime"),
            before_modified,
            "{} modified the committed fixture",
            fixture.fixture_id
        );
    }
}

fn assert_fixture_identity(source: &Path, fixture: &FixtureEntry) {
    assert_eq!(fixture.application_id, APPLICATION_ID);
    assert_eq!(fixture.user_version, fixture.migration.version);
    assert!(fixture.user_version <= CURRENT_SCHEMA_VERSION);

    let registered = MIGRATIONS
        .iter()
        .find(|migration| migration.version == fixture.user_version)
        .expect("fixture version must exist in the production registry");
    assert_eq!(fixture.migration.name, registered.name);
    assert_eq!(fixture.migration.checksum, registered.checksum);
    assert_eq!(fixture.schema_fingerprint, registered.schema_fingerprint);

    let connection = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open committed fixture read-only");
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .expect("read fixture application ID");
    let user_version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read fixture user version");
    let observed_fingerprint: String = connection
        .query_row(
            "SELECT schema_fingerprint FROM state_metadata WHERE singleton_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read fixture schema fingerprint");
    assert_eq!(application_id, fixture.application_id);
    assert_eq!(user_version, fixture.user_version);
    assert_eq!(observed_fingerprint, fixture.schema_fingerprint);

    let observed_ledger: (u32, String, String) = connection
        .query_row(
            "SELECT version, name, checksum FROM schema_migrations WHERE version = ?1",
            [fixture.user_version],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read fixture migration ledger");
    assert_eq!(
        observed_ledger,
        (
            fixture.migration.version,
            fixture.migration.name.clone(),
            fixture.migration.checksum.clone()
        )
    );

    let schema_digest = schema_digest(&connection).expect("compute fixture schema digest");
    let data_digest = data_digest(&connection).expect("compute fixture data digest");
    assert_eq!(schema_digest, fixture.schema_digest_sha256);
    assert_eq!(data_digest, fixture.data_digest_sha256);
    assert_eq!(
        fixture_logical_digest(fixture, &schema_digest, &data_digest),
        fixture.logical_digest_sha256
    );
    assert_integrity(&connection);
}

fn upgrade_fixture_copy(source: &Path, fixture: &FixtureEntry) {
    let temp = tempdir().expect("create migration matrix tempdir");
    let copy = temp.path().join("state.sqlite3");
    fs::copy(source, &copy).expect("copy historical fixture");

    let mut connection = Connection::open(&copy).expect("open copied fixture");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("enable foreign keys");

    let test_v2_checksum = migration_checksum(TEST_V2_SQL);
    let test_v2_fingerprint = schema_fingerprint(CURRENT_SCHEMA_VERSION + 1, &test_v2_checksum);
    let test_v3_checksum = migration_checksum(TEST_V3_SQL);
    let test_v3_fingerprint = schema_fingerprint(CURRENT_SCHEMA_VERSION + 2, &test_v3_checksum);
    let test_migrations = [
        MatrixMigration {
            version: CURRENT_SCHEMA_VERSION + 1,
            name: "test_0002_matrix",
            sql: TEST_V2_SQL,
            checksum: test_v2_checksum,
            fingerprint: test_v2_fingerprint,
        },
        MatrixMigration {
            version: CURRENT_SCHEMA_VERSION + 2,
            name: "test_0003_matrix",
            sql: TEST_V3_SQL,
            checksum: test_v3_checksum,
            fingerprint: test_v3_fingerprint,
        },
    ];

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("begin matrix migration transaction");
    for migration in MIGRATIONS
        .iter()
        .map(|migration| MatrixMigration {
            version: migration.version,
            name: migration.name,
            sql: migration.sql,
            checksum: migration.checksum.to_owned(),
            fingerprint: migration.schema_fingerprint.to_owned(),
        })
        .chain(test_migrations.clone())
        .filter(|migration| migration.version > fixture.user_version)
    {
        transaction
            .execute_batch(migration.sql)
            .expect("apply matrix migration SQL");
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, name, checksum, applied_at)\n                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    migration.version,
                    migration.name,
                    migration.checksum,
                    FIXED_MIGRATION_TIME
                ],
            )
            .expect("record matrix migration");
        transaction
            .execute(
                "UPDATE state_metadata SET schema_fingerprint = ?1 WHERE singleton_id = 1",
                [migration.fingerprint],
            )
            .expect("update matrix schema fingerprint");
        transaction
            .pragma_update(None, "user_version", migration.version)
            .expect("update matrix user version");
    }
    transaction.commit().expect("commit matrix migrations");

    let expected_ledger = MIGRATIONS
        .iter()
        .map(|migration| {
            (
                migration.version,
                migration.name.to_owned(),
                migration.checksum.to_owned(),
            )
        })
        .chain(test_migrations.iter().map(|migration| {
            (
                migration.version,
                migration.name.to_owned(),
                migration.checksum.clone(),
            )
        }))
        .collect::<Vec<_>>();
    let observed_ledger = connection
        .prepare("SELECT version, name, checksum FROM schema_migrations ORDER BY version")
        .expect("prepare final migration ledger")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query final migration ledger")
        .collect::<rusqlite::Result<Vec<(u32, String, String)>>>()
        .expect("collect final migration ledger");
    assert_eq!(observed_ledger, expected_ledger);

    let final_version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read final user version");
    let final_fingerprint: String = connection
        .query_row(
            "SELECT schema_fingerprint FROM state_metadata WHERE singleton_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("read final schema fingerprint");
    assert_eq!(final_version, CURRENT_SCHEMA_VERSION + 2);
    assert_eq!(final_fingerprint, test_migrations[1].fingerprint);

    let v2_row: (String, i64) = connection
        .query_row(
            "SELECT label, sequence_marker FROM migration_matrix_v2 WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read test-only migration row");
    assert_eq!(v2_row, ("v2-applied".to_owned(), 3));
    assert_integrity(&connection);
}

fn assert_integrity(connection: &Connection) {
    let foreign_key_violation = {
        let mut statement = connection
            .prepare("PRAGMA foreign_key_check")
            .expect("prepare foreign key check");
        let mut rows = statement.query([]).expect("run foreign key check");
        rows.next().expect("read foreign key result").is_some()
    };
    let quick_check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .expect("run quick check");
    assert!(!foreign_key_violation, "foreign key check failed");
    assert_eq!(quick_check, "ok");
}

fn schema_digest(connection: &Connection) -> rusqlite::Result<String> {
    query_set_digest(
        connection,
        b"gpteasy-fixture-schema-v1\0",
        &[(
            "sqlite_schema",
            "SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY type, name, tbl_name",
        )],
    )
}

fn data_digest(connection: &Connection) -> rusqlite::Result<String> {
    query_set_digest(
        connection,
        b"gpteasy-fixture-data-v1\0",
        &[
            ("state_metadata", "SELECT singleton_id, database_uuid, schema_fingerprint, created_at FROM state_metadata ORDER BY singleton_id"),
            ("schema_migrations", "SELECT version, name, checksum, applied_at FROM schema_migrations ORDER BY version"),
            ("providers", "SELECT id, provider_kind, built_in_key, display_name, base_url, api_key, default_model, created_at, updated_at FROM providers ORDER BY id"),
            ("provider_verifications", "SELECT provider_id, combination_fingerprint, verified_at, contract_version FROM provider_verifications ORDER BY provider_id"),
            ("managed_environments", "SELECT id, environment_kind, platform_identity, display_name, current_provider_id, first_seen_at, last_seen_at FROM managed_environments ORDER BY id"),
            ("app_settings", "SELECT singleton_id, locale, theme, launch_at_login_desired, close_to_tray_notice_seen, onboarding_completed, last_update_check_at, updated_at FROM app_settings ORDER BY singleton_id"),
        ],
    )
}

fn query_set_digest(
    connection: &Connection,
    domain: &[u8],
    queries: &[(&str, &str)],
) -> rusqlite::Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for (label, sql) in queries {
        hash_bytes(&mut hasher, label.as_bytes());
        let mut statement = connection.prepare(sql)?;
        let column_count = statement.column_count();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            hasher.update([0xff]);
            hasher.update((column_count as u64).to_be_bytes());
            for index in 0..column_count {
                hash_value(&mut hasher, row.get_ref(index)?);
            }
        }
        hasher.update([0xfe]);
    }
    Ok(lowercase_hex(&hasher.finalize()))
}

fn hash_value(hasher: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hasher.update([0]),
        ValueRef::Integer(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        ValueRef::Real(value) => {
            hasher.update([2]);
            hasher.update(value.to_bits().to_be_bytes());
        }
        ValueRef::Text(value) => {
            hasher.update([3]);
            hash_bytes(hasher, value);
        }
        ValueRef::Blob(value) => {
            hasher.update([4]);
            hash_bytes(hasher, value);
        }
    }
}

fn fixture_logical_digest(fixture: &FixtureEntry, schema: &str, data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-fixture-logical-v1\0");
    hasher.update(fixture.application_id.to_be_bytes());
    hasher.update(fixture.user_version.to_be_bytes());
    hash_bytes(&mut hasher, fixture.schema_fingerprint.as_bytes());
    hash_bytes(&mut hasher, schema.as_bytes());
    hash_bytes(&mut hasher, data.as_bytes());
    lowercase_hex(&hasher.finalize())
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn sha256_hex(bytes: &[u8]) -> String {
    lowercase_hex(&Sha256::digest(bytes))
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
