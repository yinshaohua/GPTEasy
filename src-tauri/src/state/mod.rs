use std::{
    fs,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::StateSnapshot;

pub mod coordination;
pub mod migrations;
pub mod repositories;

use coordination::{CoordinationError, StateCoordinator};

pub const APPLICATION_ID: i64 = 0x4750_5445;
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

const DATABASE_FILENAME: &str = "state.sqlite3";
const INITIAL_MIGRATION_NAME: &str = "0001_initial";
const INITIAL_MIGRATION_SQL: &str = include_str!("migrations/0001_initial.sql");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettingsRecord {
    pub locale: String,
    pub theme: String,
    pub launch_at_login_desired: bool,
    pub close_to_tray_notice_seen: bool,
    pub onboarding_completed: bool,
    pub last_update_check_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("failed to prepare the local state directory")]
    PrepareDirectory(#[source] std::io::Error),
    #[error("failed to access the local state database")]
    Database(#[source] rusqlite::Error),
    #[error("the local state database does not match this application")]
    ContractMismatch,
    #[error("the local state database failed its integrity check")]
    IntegrityCheckFailed,
    #[error("the local state service is unavailable")]
    Unavailable,
    #[error("the local state is busy in another process")]
    StateBusy,
    #[error("the local state coordinator is unavailable")]
    Coordination(#[source] CoordinationError),
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

pub struct StateStore {
    connection: Mutex<Connection>,
    _coordinator: StateCoordinator,
}

impl StateStore {
    pub fn open(state_root: &Path) -> Result<Self, StoreError> {
        Self::open_with_owner_run_id(state_root, None)
    }

    pub(crate) fn open_with_run_id(state_root: &Path, run_id: &str) -> Result<Self, StoreError> {
        Self::open_with_owner_run_id(state_root, Some(run_id))
    }

    fn open_with_owner_run_id(state_root: &Path, run_id: Option<&str>) -> Result<Self, StoreError> {
        fs::create_dir_all(state_root).map_err(StoreError::PrepareDirectory)?;
        let coordinator = StateCoordinator::acquire(state_root, run_id).map_err(|error| {
            if matches!(error, CoordinationError::Busy) {
                StoreError::StateBusy
            } else {
                StoreError::Coordination(error)
            }
        })?;
        let database_path = state_root.join(DATABASE_FILENAME);
        let is_new = !database_path.exists();

        if !is_new {
            inspect_existing_database(&database_path)?;
        }

        let mut connection = Connection::open(&database_path)?;
        configure_ready_connection(&connection)?;
        if is_new {
            initialize_database(&mut connection)?;
        }

        Ok(Self {
            connection: Mutex::new(connection),
            _coordinator: coordinator,
        })
    }

    pub fn settings(&self) -> Result<AppSettingsRecord, StoreError> {
        let connection = self.connection()?;
        read_settings(&connection)
    }

    pub fn update_theme(&self, theme: &str) -> Result<AppSettingsRecord, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "UPDATE app_settings
             SET theme = ?1, updated_at = ?2
             WHERE singleton_id = 1",
        )?;
        let changed = statement.execute(params![theme, utc_now()])?;
        if changed != 1 {
            return Err(StoreError::ContractMismatch);
        }
        drop(statement);

        read_settings(&connection)
    }

    pub fn replace_snapshot(&self, snapshot: &StateSnapshot) -> Result<StateSnapshot, StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        repositories::replace_snapshot(&transaction, snapshot)?;
        ensure_integrity(&transaction)?;
        transaction.commit()?;
        repositories::read_snapshot(&connection)
    }

    pub fn snapshot(&self) -> Result<StateSnapshot, StoreError> {
        let connection = self.connection()?;
        repositories::read_snapshot(&connection)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.connection.lock().map_err(|_| StoreError::Unavailable)
    }
}

fn utc_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
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

fn initial_migration_checksum() -> String {
    sha256_hex(INITIAL_MIGRATION_SQL.as_bytes())
}

fn current_schema_fingerprint(migration_checksum: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-schema-fingerprint-v1\0");
    hasher.update(APPLICATION_ID.to_be_bytes());
    hasher.update(CURRENT_SCHEMA_VERSION.to_be_bytes());
    hasher.update(migration_checksum.as_bytes());
    lowercase_hex(&hasher.finalize())
}

fn inspect_existing_database(database_path: &Path) -> Result<(), StoreError> {
    let connection = Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let application_id: i64 =
        connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let user_version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if application_id != APPLICATION_ID || user_version != CURRENT_SCHEMA_VERSION {
        return Err(StoreError::ContractMismatch);
    }

    let checksum = initial_migration_checksum();
    let expected_fingerprint = current_schema_fingerprint(&checksum);
    let (database_uuid, observed_fingerprint): (String, String) = connection
        .query_row(
            "SELECT database_uuid, schema_fingerprint
             FROM state_metadata
             WHERE singleton_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| StoreError::ContractMismatch)?;
    if Uuid::parse_str(&database_uuid).is_err() || observed_fingerprint != expected_fingerprint {
        return Err(StoreError::ContractMismatch);
    }

    let ledger_matches: i64 = connection
        .query_row(
            "SELECT COUNT(*)
             FROM schema_migrations
             WHERE version = ?1 AND name = ?2 AND checksum = ?3",
            params![CURRENT_SCHEMA_VERSION, INITIAL_MIGRATION_NAME, checksum],
            |row| row.get(0),
        )
        .map_err(|_| StoreError::ContractMismatch)?;
    if ledger_matches != 1 {
        return Err(StoreError::ContractMismatch);
    }

    ensure_integrity(&connection)
}

fn configure_ready_connection(connection: &Connection) -> Result<(), StoreError> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    let journal_mode: String =
        connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::ContractMismatch);
    }
    Ok(())
}

fn initialize_database(connection: &mut Connection) -> Result<(), StoreError> {
    let applied_at = utc_now();
    let checksum = initial_migration_checksum();
    let fingerprint = current_schema_fingerprint(&checksum);
    let database_uuid = Uuid::new_v4().to_string();
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

    transaction.execute_batch(INITIAL_MIGRATION_SQL)?;
    transaction.execute(
        "INSERT INTO state_metadata (
             singleton_id, database_uuid, schema_fingerprint, created_at
         ) VALUES (1, ?1, ?2, ?3)",
        params![database_uuid, fingerprint, applied_at],
    )?;
    transaction.execute(
        "INSERT INTO schema_migrations (version, name, checksum, applied_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            CURRENT_SCHEMA_VERSION,
            INITIAL_MIGRATION_NAME,
            checksum,
            applied_at
        ],
    )?;
    transaction.execute(
        "INSERT INTO app_settings (
             singleton_id, locale, theme, launch_at_login_desired,
             close_to_tray_notice_seen, onboarding_completed,
             last_update_check_at, updated_at
         ) VALUES (1, 'system', 'system', 0, 0, 0, NULL, ?1)",
        params![applied_at],
    )?;
    transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
    transaction.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
    ensure_integrity(&transaction)?;
    transaction.commit()?;
    Ok(())
}

fn ensure_integrity(connection: &Connection) -> Result<(), StoreError> {
    let foreign_key_violation: Option<i64> = {
        let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
        let mut rows = statement.query([])?;
        rows.next()?.map(|row| row.get(0)).transpose()?
    };
    if foreign_key_violation.is_some() {
        return Err(StoreError::IntegrityCheckFailed);
    }

    let quick_check: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        return Err(StoreError::IntegrityCheckFailed);
    }
    Ok(())
}

fn read_settings(connection: &Connection) -> Result<AppSettingsRecord, StoreError> {
    let mut statement = connection.prepare(
        "SELECT locale, theme, launch_at_login_desired,
                close_to_tray_notice_seen, onboarding_completed,
                last_update_check_at, updated_at
         FROM app_settings
         WHERE singleton_id = 1",
    )?;
    statement
        .query_row([], |row| {
            Ok(AppSettingsRecord {
                locale: row.get(0)?,
                theme: row.get(1)?,
                launch_at_login_desired: row.get::<_, i64>(2)? != 0,
                close_to_tray_notice_seen: row.get::<_, i64>(3)? != 0,
                onboarding_completed: row.get::<_, i64>(4)? != 0,
                last_update_check_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(Into::into)
}
