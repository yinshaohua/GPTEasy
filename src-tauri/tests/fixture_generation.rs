use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

const APPLICATION_ID: i64 = 0x4750_5445;
const FIXED_TIMESTAMP: &str = "2026-08-01T00:00:00.000Z";
const FIXED_DATABASE_UUID: &str = "00000000-0000-4000-8000-000000000001";
const FIXED_PROVIDER_IDS: [&str; 2] = [
    "11111111-1111-4111-8111-111111111111",
    "22222222-2222-4222-8222-222222222222",
];
const FIXED_ENVIRONMENT_IDS: [&str; 2] = [
    "33333333-3333-4333-8333-333333333333",
    "44444444-4444-4444-8444-444444444444",
];
const FIXED_FAKE_KEYS: [&str; 2] = [
    "fixture-only-fake-key-alpha-v001",
    "fixture-only-fake-key-beta-v001",
];

fn generator() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_generate_v001_fixture"))
}

fn run_generator(root: &Path) -> Output {
    Command::new(generator())
        .arg(root)
        .output()
        .expect("run v001 fixture generator")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "generator failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture_path(root: &Path) -> PathBuf {
    root.join("v001").join("state.sqlite3")
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join("manifest.json")
}

fn read_manifest(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(manifest_path(root)).expect("read fixture manifest"))
        .expect("parse fixture manifest")
}

fn fixture_entry(manifest: &Value) -> &Value {
    let fixtures = manifest["fixtures"]
        .as_array()
        .expect("manifest fixtures array");
    assert_eq!(fixtures.len(), 1);
    &fixtures[0]
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn assert_fixture_identity(root: &Path, manifest: &Value) {
    let entry = fixture_entry(manifest);
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(entry["fixture_id"], "v001");
    assert_eq!(entry["database_path"], "v001/state.sqlite3");
    assert_eq!(entry["application_id"], APPLICATION_ID);
    assert_eq!(entry["application_id_hex"], "0x47505445");
    assert_eq!(entry["user_version"], 1);
    assert_eq!(entry["migration"]["version"], 1);
    assert_eq!(entry["migration"]["name"], "0001_initial");
    assert_eq!(entry["fake_data"]["declaration"], "synthetic_fixture_only");
    assert_eq!(entry["fake_data"]["contains_real_credentials"], false);
    assert_eq!(entry["fake_data"]["fixed_utc"], FIXED_TIMESTAMP);

    for digest_name in [
        "file_sha256",
        "schema_digest_sha256",
        "data_digest_sha256",
        "logical_digest_sha256",
        "schema_fingerprint",
    ] {
        let digest = entry[digest_name].as_str().expect("manifest digest string");
        assert_eq!(digest.len(), 64, "{digest_name}");
        assert!(
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "{digest_name} must be lowercase SHA-256"
        );
    }

    let database_bytes = fs::read(fixture_path(root)).expect("read generated fixture bytes");
    assert_eq!(entry["file_sha256"], sha256_hex(&database_bytes));

    let manifest_text = fs::read_to_string(manifest_path(root)).expect("read manifest text");
    for fake_key in FIXED_FAKE_KEYS {
        assert!(
            !manifest_text.contains(fake_key),
            "manifest must declare fake data without exposing credential bytes"
        );
    }
}

fn assert_fixed_database(root: &Path, manifest: &Value) {
    let connection = Connection::open_with_flags(
        fixture_path(root),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open fixture read-only");
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .expect("read application ID");
    let user_version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read user version");
    let quick_check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .expect("run fixture quick check");
    assert_eq!(application_id, APPLICATION_ID);
    assert_eq!(user_version, 1);
    assert_eq!(quick_check, "ok");

    let (database_uuid, schema_fingerprint, created_at): (String, String, String) = connection
        .query_row(
            "SELECT database_uuid, schema_fingerprint, created_at
             FROM state_metadata WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read fixed state metadata");
    assert_eq!(database_uuid, FIXED_DATABASE_UUID);
    assert_eq!(schema_fingerprint, fixture_entry(manifest)["schema_fingerprint"]);
    assert_eq!(created_at, FIXED_TIMESTAMP);

    let provider_rows = connection
        .prepare("SELECT id, api_key, created_at, updated_at FROM providers ORDER BY id")
        .expect("prepare provider fixture query")
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .expect("query fixture providers")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect fixture providers");
    assert_eq!(provider_rows.len(), FIXED_PROVIDER_IDS.len());
    for (index, (id, api_key, created_at, updated_at)) in provider_rows.iter().enumerate() {
        assert_eq!(id, FIXED_PROVIDER_IDS[index]);
        assert_eq!(api_key, FIXED_FAKE_KEYS[index]);
        assert_eq!(created_at, FIXED_TIMESTAMP);
        assert_eq!(updated_at, FIXED_TIMESTAMP);
    }

    let environment_ids = connection
        .prepare("SELECT id FROM managed_environments ORDER BY id")
        .expect("prepare environment fixture query")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query fixture environments")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect fixture environments");
    assert_eq!(environment_ids, FIXED_ENVIRONMENT_IDS);

    let timestamps_outside_fixture = connection
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM schema_migrations WHERE applied_at <> ?1) +
                 (SELECT COUNT(*) FROM provider_verifications WHERE verified_at <> ?1) +
                 (SELECT COUNT(*) FROM managed_environments
                    WHERE first_seen_at <> ?1 OR last_seen_at <> ?1) +
                 (SELECT COUNT(*) FROM app_settings
                    WHERE updated_at <> ?1 OR last_update_check_at <> ?1)",
            [FIXED_TIMESTAMP],
            |row| row.get::<_, i64>(0),
        )
        .expect("check all fixture timestamps");
    assert_eq!(timestamps_outside_fixture, 0);
}

#[test]
fn two_fresh_generations_are_byte_and_logically_deterministic() {
    let first = tempdir().expect("create first fixture tempdir");
    let second = tempdir().expect("create second fixture tempdir");
    assert_success(&run_generator(first.path()));
    assert_success(&run_generator(second.path()));

    let first_manifest = read_manifest(first.path());
    let second_manifest = read_manifest(second.path());
    assert_fixture_identity(first.path(), &first_manifest);
    assert_fixture_identity(second.path(), &second_manifest);
    assert_fixed_database(first.path(), &first_manifest);
    assert_fixed_database(second.path(), &second_manifest);

    let first_entry = fixture_entry(&first_manifest);
    let second_entry = fixture_entry(&second_manifest);
    for digest_name in [
        "file_sha256",
        "schema_digest_sha256",
        "data_digest_sha256",
        "logical_digest_sha256",
    ] {
        assert_eq!(first_entry[digest_name], second_entry[digest_name]);
    }
}

#[test]
fn generator_refuses_every_existing_output_without_modification() {
    let complete = tempdir().expect("create complete fixture tempdir");
    assert_success(&run_generator(complete.path()));
    let original_database = fs::read(fixture_path(complete.path())).expect("read original DB");
    let original_manifest = fs::read(manifest_path(complete.path())).expect("read original manifest");
    let repeated = run_generator(complete.path());
    assert!(!repeated.status.success());
    assert_eq!(fs::read(fixture_path(complete.path())).unwrap(), original_database);
    assert_eq!(fs::read(manifest_path(complete.path())).unwrap(), original_manifest);

    let manifest_only = tempdir().expect("create manifest-only tempdir");
    fs::write(manifest_path(manifest_only.path()), b"manifest-sentinel")
        .expect("write manifest sentinel");
    let rejected_manifest = run_generator(manifest_only.path());
    assert!(!rejected_manifest.status.success());
    assert_eq!(
        fs::read(manifest_path(manifest_only.path())).unwrap(),
        b"manifest-sentinel"
    );
    assert!(!fixture_path(manifest_only.path()).exists());

    let database_only = tempdir().expect("create database-only tempdir");
    fs::create_dir(database_only.path().join("v001")).expect("create v001 directory");
    fs::write(fixture_path(database_only.path()), b"database-sentinel")
        .expect("write database sentinel");
    let rejected_database = run_generator(database_only.path());
    assert!(!rejected_database.status.success());
    assert_eq!(
        fs::read(fixture_path(database_only.path())).unwrap(),
        b"database-sentinel"
    );
    assert!(!manifest_path(database_only.path()).exists());
}

#[test]
fn committed_v001_is_the_create_once_generator_output() {
    let committed_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("tests/fixtures/databases");
    let committed_manifest = read_manifest(&committed_root);
    assert_fixture_identity(&committed_root, &committed_manifest);
    assert_fixed_database(&committed_root, &committed_manifest);

    let regenerated = tempdir().expect("create regeneration tempdir");
    assert_success(&run_generator(regenerated.path()));
    let regenerated_manifest = read_manifest(regenerated.path());
    assert_eq!(committed_manifest, regenerated_manifest);
    assert_eq!(
        fs::read(fixture_path(&committed_root)).expect("read committed fixture"),
        fs::read(fixture_path(regenerated.path())).expect("read regenerated fixture")
    );
}
