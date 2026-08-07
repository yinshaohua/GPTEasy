use std::{
    env,
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use gpteasy_lib::state::migrations::{validate_registry, Migration, APPLICATION_ID, MIGRATIONS};
use rusqlite::{params, types::ValueRef, Connection, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};

const FIXTURE_ID: &str = "v001";
const DATABASE_RELATIVE_PATH: &str = "v001/state.sqlite3";
const FIXED_TIMESTAMP: &str = "2026-08-01T00:00:00.000Z";
const FIXED_DATABASE_UUID: &str = "00000000-0000-4000-8000-000000000001";
const ALPHA_PROVIDER_ID: &str = "11111111-1111-4111-8111-111111111111";
const BETA_PROVIDER_ID: &str = "22222222-2222-4222-8222-222222222222";
const NATIVE_ENVIRONMENT_ID: &str = "33333333-3333-4333-8333-333333333333";
const WSL_ENVIRONMENT_ID: &str = "44444444-4444-4444-8444-444444444444";
const ALPHA_FAKE_KEY: &str = "fixture-only-fake-key-alpha-v001";
const BETA_FAKE_KEY: &str = "fixture-only-fake-key-beta-v001";

#[derive(Serialize)]
struct Manifest<'a> {
    format_version: u32,
    fixtures: [FixtureManifest<'a>; 1],
}

#[derive(Serialize)]
struct FixtureManifest<'a> {
    fixture_id: &'a str,
    database_path: &'a str,
    file_sha256: String,
    schema_digest_sha256: String,
    data_digest_sha256: String,
    logical_digest_sha256: String,
    application_id: i64,
    application_id_hex: &'a str,
    user_version: u32,
    schema_fingerprint: &'a str,
    migration: MigrationManifest<'a>,
    fake_data: FakeDataManifest<'a>,
}

#[derive(Serialize)]
struct MigrationManifest<'a> {
    version: u32,
    name: &'a str,
    checksum: &'a str,
}

#[derive(Serialize)]
struct FakeDataManifest<'a> {
    declaration: &'a str,
    contains_real_credentials: bool,
    fixed_utc: &'a str,
    database_uuid: &'a str,
    provider_ids: [&'a str; 2],
    environment_ids: [&'a str; 2],
}

struct GenerationLock {
    path: PathBuf,
    _file: File,
}

impl GenerationLock {
    fn acquire(root: &Path) -> Result<Self, Box<dyn Error>> {
        let path = root.join(".generate-v001-fixture.lock");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "fixture generation is already running or requires manual cleanup")?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for GenerationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("v001 fixture generation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let root = parse_output_root()?;
    generate(&root)
}

fn parse_output_root() -> Result<PathBuf, Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let root = args
        .next()
        .ok_or("usage: generate_v001_fixture <database-fixture-root>")?;
    if args.next().is_some() {
        return Err("usage: generate_v001_fixture <database-fixture-root>".into());
    }
    Ok(PathBuf::from(root))
}

fn generate(root: &Path) -> Result<(), Box<dyn Error>> {
    validate_registry()?;
    let migration = MIGRATIONS
        .iter()
        .find(|migration| migration.version == 1)
        .ok_or("v001 migration is missing from the production registry")?;

    fs::create_dir_all(root)?;
    let _lock = GenerationLock::acquire(root)?;
    let database_path = root.join(DATABASE_RELATIVE_PATH);
    let manifest_path = root.join("manifest.json");
    if database_path.exists() || manifest_path.exists() {
        return Err("refusing to overwrite an existing fixture database or manifest".into());
    }

    let version_dir = database_path
        .parent()
        .ok_or("fixture database path has no parent")?;
    fs::create_dir_all(version_dir)?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&database_path)?;

    let result = generate_outputs(&database_path, &manifest_path, migration);
    if result.is_err() {
        let _ = fs::remove_file(&manifest_path);
        let _ = fs::remove_file(&database_path);
    }
    result
}

fn generate_outputs(
    database_path: &Path,
    manifest_path: &Path,
    migration: &Migration,
) -> Result<(), Box<dyn Error>> {
    let mut connection = Connection::open(database_path)?;
    configure_deterministic_database(&connection)?;
    initialize_v001(&mut connection, migration)?;
    let schema_digest = query_set_digest(
        &connection,
        b"gpteasy-fixture-schema-v1\0",
        &[(
            "sqlite_schema",
            "SELECT type, name, tbl_name, sql
             FROM sqlite_schema
             ORDER BY type, name, tbl_name",
        )],
    )?;
    let data_digest = query_set_digest(
        &connection,
        b"gpteasy-fixture-data-v1\0",
        &[
            (
                "state_metadata",
                "SELECT singleton_id, database_uuid, schema_fingerprint, created_at
                 FROM state_metadata ORDER BY singleton_id",
            ),
            (
                "schema_migrations",
                "SELECT version, name, checksum, applied_at
                 FROM schema_migrations ORDER BY version",
            ),
            (
                "providers",
                "SELECT id, provider_kind, built_in_key, display_name, base_url,
                        api_key, default_model, created_at, updated_at
                 FROM providers ORDER BY id",
            ),
            (
                "provider_verifications",
                "SELECT provider_id, combination_fingerprint, verified_at, contract_version
                 FROM provider_verifications ORDER BY provider_id",
            ),
            (
                "managed_environments",
                "SELECT id, environment_kind, platform_identity, display_name,
                        current_provider_id, first_seen_at, last_seen_at
                 FROM managed_environments ORDER BY id",
            ),
            (
                "app_settings",
                "SELECT singleton_id, locale, theme, launch_at_login_desired,
                        close_to_tray_notice_seen, onboarding_completed,
                        last_update_check_at, updated_at
                 FROM app_settings ORDER BY singleton_id",
            ),
        ],
    )?;
    let logical_digest = logical_digest(migration, &schema_digest, &data_digest);
    drop(connection);

    let database_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(database_path)?;
    database_file.sync_all()?;
    drop(database_file);
    let file_sha256 = sha256_hex(&fs::read(database_path)?);

    let manifest = Manifest {
        format_version: 1,
        fixtures: [FixtureManifest {
            fixture_id: FIXTURE_ID,
            database_path: DATABASE_RELATIVE_PATH,
            file_sha256,
            schema_digest_sha256: schema_digest,
            data_digest_sha256: data_digest,
            logical_digest_sha256: logical_digest,
            application_id: APPLICATION_ID,
            application_id_hex: "0x47505445",
            user_version: migration.version,
            schema_fingerprint: migration.schema_fingerprint,
            migration: MigrationManifest {
                version: migration.version,
                name: migration.name,
                checksum: migration.checksum,
            },
            fake_data: FakeDataManifest {
                declaration: "synthetic_fixture_only",
                contains_real_credentials: false,
                fixed_utc: FIXED_TIMESTAMP,
                database_uuid: FIXED_DATABASE_UUID,
                provider_ids: [ALPHA_PROVIDER_ID, BETA_PROVIDER_ID],
                environment_ids: [NATIVE_ENVIRONMENT_ID, WSL_ENVIRONMENT_ID],
            },
        }],
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    let mut manifest_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(manifest_path)?;
    manifest_file.write_all(&manifest_bytes)?;
    manifest_file.sync_all()?;
    Ok(())
}

fn configure_deterministic_database(connection: &Connection) -> rusqlite::Result<()> {
    connection.pragma_update(None, "page_size", 4096)?;
    connection.pragma_update(None, "auto_vacuum", "NONE")?;
    connection.pragma_update(None, "encoding", "UTF-8")?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let journal_mode: String =
        connection.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("delete") {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn initialize_v001(connection: &mut Connection, migration: &Migration) -> rusqlite::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(migration.sql)?;
    transaction.execute(
        "INSERT INTO state_metadata (
             singleton_id, database_uuid, schema_fingerprint, created_at
         ) VALUES (1, ?1, ?2, ?3)",
        params![
            FIXED_DATABASE_UUID,
            migration.schema_fingerprint,
            FIXED_TIMESTAMP
        ],
    )?;
    transaction.execute(
        "INSERT INTO schema_migrations (version, name, checksum, applied_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            migration.version,
            migration.name,
            migration.checksum,
            FIXED_TIMESTAMP
        ],
    )?;
    transaction.execute(
        "INSERT INTO app_settings (
             singleton_id, locale, theme, launch_at_login_desired,
             close_to_tray_notice_seen, onboarding_completed,
             last_update_check_at, updated_at
         ) VALUES (1, 'zh-CN', 'dark', 1, 1, 1, ?1, ?1)",
        [FIXED_TIMESTAMP],
    )?;
    insert_fixed_state(&transaction)?;
    transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
    transaction.pragma_update(None, "user_version", migration.version)?;
    ensure_integrity(&transaction)?;
    transaction.commit()
}

fn insert_fixed_state(transaction: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    let alpha_fingerprint = provider_fingerprint(
        "https://fixture.invalid/v1",
        "fixture-model-alpha-v001",
        ALPHA_FAKE_KEY,
    );
    let beta_fingerprint = provider_fingerprint(
        "http://127.0.0.1:4010/v1",
        "fixture-model-beta-v001",
        BETA_FAKE_KEY,
    );
    transaction.execute(
        "INSERT INTO providers (
             id, provider_kind, built_in_key, display_name, base_url,
             api_key, default_model, created_at, updated_at
         ) VALUES (?1, 'built_in_recommended', 'fixture-dayway', 'Fixture DayWay',
                   'https://fixture.invalid/v1', ?2, 'fixture-model-alpha-v001', ?3, ?3)",
        params![ALPHA_PROVIDER_ID, ALPHA_FAKE_KEY, FIXED_TIMESTAMP],
    )?;
    transaction.execute(
        "INSERT INTO providers (
             id, provider_kind, built_in_key, display_name, base_url,
             api_key, default_model, created_at, updated_at
         ) VALUES (?1, 'custom', NULL, 'Fixture Loopback',
                   'http://127.0.0.1:4010/v1', ?2, 'fixture-model-beta-v001', ?3, ?3)",
        params![BETA_PROVIDER_ID, BETA_FAKE_KEY, FIXED_TIMESTAMP],
    )?;
    transaction.execute(
        "INSERT INTO provider_verifications (
             provider_id, combination_fingerprint, verified_at, contract_version
         ) VALUES (?1, ?2, ?3, 'gpteasy.provider-validation.v1')",
        params![ALPHA_PROVIDER_ID, alpha_fingerprint, FIXED_TIMESTAMP],
    )?;
    transaction.execute(
        "INSERT INTO provider_verifications (
             provider_id, combination_fingerprint, verified_at, contract_version
         ) VALUES (?1, ?2, ?3, 'gpteasy.provider-validation.v1')",
        params![BETA_PROVIDER_ID, beta_fingerprint, FIXED_TIMESTAMP],
    )?;
    transaction.execute(
        "INSERT INTO managed_environments (
             id, environment_kind, platform_identity, display_name,
             current_provider_id, first_seen_at, last_seen_at
         ) VALUES (?1, 'native_codex', 'fixture-native-current-user', 'Fixture Native',
                   ?2, ?3, ?3)",
        params![NATIVE_ENVIRONMENT_ID, ALPHA_PROVIDER_ID, FIXED_TIMESTAMP],
    )?;
    transaction.execute(
        "INSERT INTO managed_environments (
             id, environment_kind, platform_identity, display_name,
             current_provider_id, first_seen_at, last_seen_at
         ) VALUES (?1, 'wsl2', 'fixture-wsl-registration-v001', 'Fixture WSL2',
                   ?2, ?3, ?3)",
        params![WSL_ENVIRONMENT_ID, BETA_PROVIDER_ID, FIXED_TIMESTAMP],
    )?;
    Ok(())
}

fn provider_fingerprint(base_url: &str, default_model: &str, api_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-provider-combination-v1\0");
    hasher.update(base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(default_model.as_bytes());
    hasher.update(b"\0");
    hasher.update(api_key.as_bytes());
    lowercase_hex(&hasher.finalize())
}

fn ensure_integrity(connection: &Connection) -> rusqlite::Result<()> {
    let violation = {
        let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
        let mut rows = statement.query([])?;
        rows.next()?.is_some()
    };
    let quick_check: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if violation || quick_check != "ok" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
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

fn logical_digest(migration: &Migration, schema_digest: &str, data_digest: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-fixture-logical-v1\0");
    hasher.update(APPLICATION_ID.to_be_bytes());
    hasher.update(migration.version.to_be_bytes());
    hash_bytes(&mut hasher, migration.schema_fingerprint.as_bytes());
    hash_bytes(&mut hasher, schema_digest.as_bytes());
    hash_bytes(&mut hasher, data_digest.as_bytes());
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
