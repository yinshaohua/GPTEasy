use std::fmt;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::backup::Backup;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction};
use serde::Serialize;

pub const CURRENT_SCHEMA_VERSION: i64 = 4;
const APPLICATION_ID: i64 = 0x4750_5445;
const BACKUP_LIMIT: usize = 3;
const INSTALLATION_MARKER_CONTENT: &[u8] = b"gpteasy-state-v1\n";

const SCHEMA_V1: &str = r#"
CREATE TABLE providers (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL COLLATE NOCASE UNIQUE CHECK (length(trim(name)) > 0),
    base_url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    default_model TEXT NOT NULL,
    verified_at TEXT NOT NULL,
    verification_fingerprint TEXT NOT NULL
) STRICT;

CREATE TABLE last_applied_state (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    mode TEXT NOT NULL CHECK (mode IN ('provider', 'openai_login')),
    provider_id TEXT REFERENCES providers(id),
    config_fingerprint TEXT,
    credentials_fingerprint TEXT,
    applied_at TEXT NOT NULL,
    CHECK (
        (mode = 'provider' AND provider_id IS NOT NULL) OR
        (mode = 'openai_login' AND provider_id IS NULL)
    )
) STRICT;

CREATE TABLE pending_config_operation (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    operation_id TEXT NOT NULL UNIQUE,
    operation_kind TEXT NOT NULL,
    stage TEXT NOT NULL,
    target_provider_id TEXT REFERENCES providers(id),
    old_config_fingerprint TEXT,
    new_config_fingerprint TEXT,
    old_credentials_fingerprint TEXT,
    new_credentials_fingerprint TEXT,
    backup_reference TEXT NOT NULL,
    target_snapshot_json TEXT NOT NULL,
    started_at TEXT NOT NULL
) STRICT;

CREATE TABLE app_state (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    first_close_notice_seen INTEGER NOT NULL DEFAULT 0 CHECK (first_close_notice_seen IN (0, 1)),
    pending_restart INTEGER NOT NULL DEFAULT 0 CHECK (pending_restart IN (0, 1))
) STRICT;

INSERT INTO app_state (singleton) VALUES (1);
"#;

const SCHEMA_V2: &str = r#"
ALTER TABLE app_state ADD COLUMN pending_restart_context TEXT;
ALTER TABLE pending_config_operation ADD COLUMN restart_context TEXT;
"#;

const SCHEMA_V3: &str = r#"
ALTER TABLE providers ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
UPDATE providers
SET sort_order = (
    SELECT COUNT(*) FROM providers earlier WHERE earlier.rowid < providers.rowid
);
"#;

const SCHEMA_V4: &str = r#"
ALTER TABLE providers ADD COLUMN recommendation_id TEXT
    CHECK (recommendation_id IS NULL OR recommendation_id = 'dayway');
ALTER TABLE providers ADD COLUMN recommendation_template_base_url TEXT
    CHECK (recommendation_template_base_url IS NULL OR recommendation_id = 'dayway');
CREATE UNIQUE INDEX providers_recommendation_id_unique
    ON providers(recommendation_id) WHERE recommendation_id IS NOT NULL;
"#;

const MIGRATIONS: &[(i64, &str)] = &[
    (1, SCHEMA_V1),
    (2, SCHEMA_V2),
    (3, SCHEMA_V3),
    (4, SCHEMA_V4),
];

#[derive(Debug, Clone)]
pub struct StatePaths {
    root: PathBuf,
    database: PathBuf,
    installation_marker: PathBuf,
    backups: PathBuf,
}

impl StatePaths {
    pub fn from_root(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            database: root.join("state.sqlite3"),
            installation_marker: root.join(".initialized"),
            backups: root.join("backups"),
            root,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn database(&self) -> &Path {
        &self.database
    }

    pub fn installation_marker(&self) -> &Path {
        &self.installation_marker
    }

    pub fn backups(&self) -> &Path {
        &self.backups
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum StateFailurePoint {
    BeforeMigration,
}

#[doc(hidden)]
pub trait StateFaultInjector: Send + Sync {
    fn fails_at(&self, point: StateFailurePoint) -> bool;
}

#[derive(Debug)]
struct NoStateFaults;

impl StateFaultInjector for NoStateFaults {
    fn fails_at(&self, _point: StateFailurePoint) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct StateStore {
    paths: StatePaths,
    faults: Arc<dyn StateFaultInjector>,
}

impl fmt::Debug for StateStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateStore")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

impl StateStore {
    pub fn new(paths: StatePaths) -> Self {
        Self {
            paths,
            faults: Arc::new(NoStateFaults),
        }
    }

    #[doc(hidden)]
    pub fn with_fault_injector(paths: StatePaths, faults: Arc<dyn StateFaultInjector>) -> Self {
        Self { paths, faults }
    }

    pub fn paths(&self) -> &StatePaths {
        &self.paths
    }

    pub fn should_show_first_close_notice(&self) -> bool {
        let Some(connection) = self.open_existing_database() else {
            return false;
        };
        connection
            .query_row(
                "SELECT first_close_notice_seen FROM app_state WHERE singleton = 1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .is_ok_and(|seen| !seen)
    }

    pub fn mark_first_close_notice_seen(&self) -> bool {
        let Some(connection) = self.open_existing_database() else {
            return false;
        };
        connection
            .execute(
                "UPDATE app_state SET first_close_notice_seen = 1
                 WHERE singleton = 1 AND first_close_notice_seen = 0",
                [],
            )
            .is_ok_and(|changed| changed == 1)
    }

    fn open_existing_database(&self) -> Option<Connection> {
        let Ok(connection) = Connection::open_with_flags(
            &self.paths.database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            return None;
        };
        if configure_connection(&connection).is_err() {
            return None;
        }
        Some(connection)
    }

    pub fn bootstrap(&self) -> DatabaseSnapshot {
        match self.bootstrap_inner() {
            Ok(snapshot) => snapshot,
            Err(failure) => self.recover_or_block(failure.reason),
        }
    }

    fn bootstrap_inner(&self) -> Result<DatabaseSnapshot, StateFailure> {
        let database_exists = self.paths.database.is_file();
        let installation_exists = self.paths.installation_marker.is_file();
        let backups_exist = !self
            .backup_paths()
            .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))?
            .is_empty();

        if !database_exists && !installation_exists && !backups_exist {
            return self.initialize_fresh();
        }
        if !database_exists {
            return Err(StateFailure::new(DatabaseBlockReason::MissingDatabase));
        }

        self.open_existing()
    }

    fn initialize_fresh(&self) -> Result<DatabaseSnapshot, StateFailure> {
        self.ensure_directories()?;
        let temporary = self.unique_path("state.sqlite3.initializing", "sqlite3");
        let result = (|| {
            let mut connection = Connection::open(&temporary)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))?;
            configure_connection(&connection)?;
            set_application_id(&connection)?;
            apply_migrations(&mut connection, 0)?;
            validate_current_database(&connection)?;
            let contents = inspect_database_contents(&connection)?;
            drop(connection);
            fs::rename(&temporary, &self.paths.database)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))?;
            self.ensure_installation_marker()?;
            self.prune_backups()?;
            Ok(DatabaseSnapshot::ready(
                DatabaseStatus::Initialized,
                contents,
            ))
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn open_existing(&self) -> Result<DatabaseSnapshot, StateFailure> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connection = Connection::open_with_flags(&self.paths.database, flags)
            .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
        configure_connection(&connection)?;
        integrity_check(&connection)?;
        validate_application_id(&connection)?;
        let version = schema_version(&connection)?;
        if version < 0 {
            return Err(StateFailure::new(DatabaseBlockReason::CorruptDatabase));
        }
        if version > CURRENT_SCHEMA_VERSION {
            return Err(StateFailure::new(DatabaseBlockReason::FutureSchema));
        }
        validate_schema_contract(&connection, version)?;

        if version < CURRENT_SCHEMA_VERSION {
            self.create_consistent_backup(&connection)?;
            if self.faults.fails_at(StateFailurePoint::BeforeMigration) {
                return Err(StateFailure::new(DatabaseBlockReason::MigrationFailed));
            }
            apply_migrations(&mut connection, version)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::MigrationFailed))?;
        }
        validate_current_database(&connection)?;
        let contents = inspect_database_contents(&connection)?;
        drop(connection);
        self.ensure_installation_marker()?;
        self.prune_backups()?;
        Ok(DatabaseSnapshot::ready(DatabaseStatus::Ready, contents))
    }

    fn recover_or_block(&self, original_reason: DatabaseBlockReason) -> DatabaseSnapshot {
        match self.restore_latest_valid_backup() {
            Ok(Some(contents)) => DatabaseSnapshot::ready(DatabaseStatus::Recovered, contents),
            Ok(None) => DatabaseSnapshot::blocked(original_reason),
            Err(_) => DatabaseSnapshot::blocked(DatabaseBlockReason::RecoveryFailed),
        }
    }

    fn restore_latest_valid_backup(
        &self,
    ) -> Result<Option<DatabaseContentsSnapshot>, StateFailure> {
        for candidate in self.backup_paths()? {
            let temporary = self.unique_path("state.sqlite3.recovering", "sqlite3");
            if fs::copy(&candidate, &temporary).is_err() {
                continue;
            }

            let contents = match self.prepare_recovery_copy(&temporary) {
                Ok(contents) => contents,
                Err(_) => {
                    let _ = fs::remove_file(&temporary);
                    continue;
                }
            };

            if self.install_recovery_copy(&temporary).is_err() {
                let _ = fs::remove_file(&temporary);
                return Err(StateFailure::new(DatabaseBlockReason::RecoveryFailed));
            }
            self.ensure_installation_marker()?;
            self.prune_backups()?;
            return Ok(Some(contents));
        }
        Ok(None)
    }

    fn prepare_recovery_copy(&self, path: &Path) -> Result<DatabaseContentsSnapshot, StateFailure> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connection = Connection::open_with_flags(path, flags)
            .map_err(|_| StateFailure::new(DatabaseBlockReason::RecoveryFailed))?;
        configure_connection(&connection)?;
        integrity_check(&connection)?;
        validate_application_id(&connection)?;
        let version = schema_version(&connection)?;
        if version < 0 {
            return Err(StateFailure::new(DatabaseBlockReason::CorruptDatabase));
        }
        if version > CURRENT_SCHEMA_VERSION {
            return Err(StateFailure::new(DatabaseBlockReason::FutureSchema));
        }
        validate_schema_contract(&connection, version)?;
        if version < CURRENT_SCHEMA_VERSION {
            apply_migrations(&mut connection, version)?;
        }
        validate_current_database(&connection)?;
        inspect_database_contents(&connection)
    }

    fn install_recovery_copy(&self, temporary: &Path) -> Result<(), StateFailure> {
        let preserved = self.unique_path("state.sqlite3.failed", "sqlite3");
        let had_primary = self.paths.database.exists();
        if had_primary {
            fs::rename(&self.paths.database, &preserved)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::RecoveryFailed))?;
        }

        if let Err(error) = fs::rename(temporary, &self.paths.database) {
            if had_primary {
                let _ = fs::rename(&preserved, &self.paths.database);
            }
            return Err(StateFailure::with_io(
                DatabaseBlockReason::RecoveryFailed,
                error,
            ));
        }
        Ok(())
    }

    fn create_consistent_backup(&self, source: &Connection) -> Result<PathBuf, StateFailure> {
        fs::create_dir_all(&self.paths.backups)
            .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
        let destination_path = self.unique_backup_path();
        let result = (|| {
            let mut destination = Connection::open(&destination_path)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
            let backup = Backup::new(source, &mut destination)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
            backup
                .run_to_completion(16, Duration::from_millis(5), None)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
            drop(backup);
            drop(destination);
            validate_backup(&destination_path)?;
            Ok(destination_path.clone())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&destination_path);
        }
        result
    }

    fn ensure_directories(&self) -> Result<(), StateFailure> {
        fs::create_dir_all(&self.paths.root)
            .and_then(|_| fs::create_dir_all(&self.paths.backups))
            .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))
    }

    fn ensure_installation_marker(&self) -> Result<(), StateFailure> {
        if self.paths.installation_marker.is_file() {
            return Ok(());
        }
        fs::create_dir_all(&self.paths.root)
            .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))?;
        let temporary = self.unique_path(".initialized", "tmp");
        let result = (|| {
            fs::write(&temporary, INSTALLATION_MARKER_CONTENT)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))?;
            File::options()
                .write(true)
                .open(&temporary)
                .and_then(|file| file.sync_all())
                .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))?;
            fs::rename(&temporary, &self.paths.installation_marker)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    fn backup_paths(&self) -> Result<Vec<PathBuf>, StateFailure> {
        if !self.paths.backups.is_dir() {
            return Ok(Vec::new());
        }
        let mut paths = fs::read_dir(&self.paths.backups)
            .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_backup_path(path))
            .collect::<Vec<_>>();
        paths.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
        Ok(paths)
    }

    fn prune_backups(&self) -> Result<(), StateFailure> {
        for candidate in self.backup_paths()? {
            if validate_backup(&candidate).is_err() {
                fs::remove_file(candidate)
                    .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
                continue;
            }
        }
        for obsolete in self.backup_paths()?.into_iter().skip(BACKUP_LIMIT) {
            fs::remove_file(obsolete)
                .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
        }
        Ok(())
    }

    fn unique_backup_path(&self) -> PathBuf {
        loop {
            let path = self.paths.backups.join(format!(
                "state.sqlite3.backup.{}.sqlite3",
                timestamp_nonce()
            ));
            if !path.exists() {
                return path;
            }
            thread::yield_now();
        }
    }

    fn unique_path(&self, prefix: &str, extension: &str) -> PathBuf {
        loop {
            let path = self
                .paths
                .root
                .join(format!("{prefix}.{}.{extension}", timestamp_nonce()));
            if !path.exists() {
                return path;
            }
            thread::yield_now();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseStatus {
    Initialized,
    Ready,
    Recovered,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseBlockReason {
    MissingDatabase,
    CorruptDatabase,
    FutureSchema,
    MigrationFailed,
    BackupFailed,
    RecoveryFailed,
    IoFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSnapshot {
    pub status: DatabaseStatus,
    pub schema_version: Option<i64>,
    pub reason: Option<DatabaseBlockReason>,
    pub contents: Option<DatabaseContentsSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseContentsSnapshot {
    pub provider_count: i64,
    pub has_last_applied_state: bool,
    pub has_pending_config_operation: bool,
    pub pending_restart: bool,
    pub pending_config_operation: Option<PendingConfigOperationSnapshot>,
    #[serde(skip)]
    pub(crate) last_applied_config_fingerprint: Option<Option<String>>,
    #[serde(skip)]
    pub(crate) last_applied_credentials_fingerprint: Option<Option<String>>,
    #[serde(skip)]
    pub(crate) last_applied_mode: Option<AppliedMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingConfigOperationSnapshot {
    pub stage: String,
    pub old_config_fingerprint: Option<String>,
    pub new_config_fingerprint: Option<String>,
    pub old_credentials_fingerprint: Option<String>,
    pub new_credentials_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedMode {
    Provider,
    OpenaiLogin,
}

impl DatabaseSnapshot {
    fn ready(status: DatabaseStatus, contents: DatabaseContentsSnapshot) -> Self {
        Self {
            status,
            schema_version: Some(CURRENT_SCHEMA_VERSION),
            reason: None,
            contents: Some(contents),
        }
    }

    fn blocked(reason: DatabaseBlockReason) -> Self {
        Self {
            status: DatabaseStatus::Blocked,
            schema_version: None,
            reason: Some(reason),
            contents: None,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.status != DatabaseStatus::Blocked
    }
}

#[derive(Debug)]
struct StateFailure {
    reason: DatabaseBlockReason,
}

impl StateFailure {
    fn new(reason: DatabaseBlockReason) -> Self {
        Self { reason }
    }

    fn with_io(reason: DatabaseBlockReason, _error: std::io::Error) -> Self {
        Self { reason }
    }
}

fn configure_connection(connection: &Connection) -> Result<(), StateFailure> {
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))
}

fn set_application_id(connection: &Connection) -> Result<(), StateFailure> {
    connection
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(|_| StateFailure::new(DatabaseBlockReason::IoFailure))
}

fn validate_application_id(connection: &Connection) -> Result<(), StateFailure> {
    let application_id = connection
        .query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    if application_id == APPLICATION_ID {
        Ok(())
    } else {
        Err(StateFailure::new(DatabaseBlockReason::CorruptDatabase))
    }
}

fn schema_version(connection: &Connection) -> Result<i64, StateFailure> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))
}

fn integrity_check(connection: &Connection) -> Result<(), StateFailure> {
    let result = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(StateFailure::new(DatabaseBlockReason::CorruptDatabase))
    }
}

fn validate_current_database(connection: &Connection) -> Result<(), StateFailure> {
    integrity_check(connection)?;
    validate_application_id(connection)?;
    if schema_version(connection)? != CURRENT_SCHEMA_VERSION {
        return Err(StateFailure::new(DatabaseBlockReason::MigrationFailed));
    }
    validate_schema_contract(connection, CURRENT_SCHEMA_VERSION)?;
    foreign_key_check(connection)?;
    let app_state_rows = connection
        .query_row(
            "SELECT count(*) FROM app_state WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    if app_state_rows == 1 {
        Ok(())
    } else {
        Err(StateFailure::new(DatabaseBlockReason::CorruptDatabase))
    }
}

fn validate_schema_contract(connection: &Connection, version: i64) -> Result<(), StateFailure> {
    let actual = schema_contract(connection)?;
    let expected = expected_schema_contract(version)?;
    if actual == expected {
        Ok(())
    } else {
        Err(StateFailure::new(DatabaseBlockReason::CorruptDatabase))
    }
}

fn expected_schema_contract(version: i64) -> Result<Vec<(String, String)>, StateFailure> {
    let mut reference = Connection::open_in_memory()
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    apply_migrations_through(&mut reference, 0, version)?;
    schema_contract(&reference)
}

fn schema_contract(connection: &Connection) -> Result<Vec<(String, String)>, StateFailure> {
    let mut statement = connection
        .prepare(
            "SELECT name, sql FROM sqlite_schema \
             WHERE type IN ('table', 'index') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    statement
        .query_map([], |row| {
            let name = row.get::<_, String>(0)?;
            let sql = row.get::<_, String>(1)?;
            Ok((name, normalize_sql(&sql)))
        })
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))
}

fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn foreign_key_check(connection: &Connection) -> Result<(), StateFailure> {
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    let mut rows = statement
        .query([])
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    if rows
        .next()
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?
        .is_none()
    {
        Ok(())
    } else {
        Err(StateFailure::new(DatabaseBlockReason::CorruptDatabase))
    }
}

fn inspect_database_contents(
    connection: &Connection,
) -> Result<DatabaseContentsSnapshot, StateFailure> {
    let provider_count = connection
        .query_row("SELECT count(*) FROM providers", [], |row| row.get(0))
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    let last_applied = connection
        .query_row(
            "SELECT mode, config_fingerprint, credentials_fingerprint \
             FROM last_applied_state WHERE singleton = 1",
            [],
            |row| {
                let mode = match row.get::<_, String>(0)?.as_str() {
                    "provider" => AppliedMode::Provider,
                    "openai_login" => AppliedMode::OpenaiLogin,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok((
                    mode,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    let pending_config_operation = connection
        .query_row(
            "SELECT stage, old_config_fingerprint, new_config_fingerprint, \
                    old_credentials_fingerprint, new_credentials_fingerprint \
             FROM pending_config_operation WHERE singleton = 1",
            [],
            |row| {
                Ok(PendingConfigOperationSnapshot {
                    stage: row.get(0)?,
                    old_config_fingerprint: row.get(1)?,
                    new_config_fingerprint: row.get(2)?,
                    old_credentials_fingerprint: row.get(3)?,
                    new_credentials_fingerprint: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;
    let pending_restart = connection
        .query_row(
            "SELECT pending_restart FROM app_state WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| StateFailure::new(DatabaseBlockReason::CorruptDatabase))?;

    Ok(DatabaseContentsSnapshot {
        provider_count,
        has_last_applied_state: last_applied.is_some(),
        has_pending_config_operation: pending_config_operation.is_some(),
        pending_restart: pending_restart == 1,
        pending_config_operation,
        last_applied_config_fingerprint: last_applied
            .as_ref()
            .map(|(_, config_fingerprint, _)| config_fingerprint.clone()),
        last_applied_credentials_fingerprint: last_applied
            .as_ref()
            .map(|(_, _, credentials_fingerprint)| credentials_fingerprint.clone()),
        last_applied_mode: last_applied.map(|(mode, _, _)| mode),
    })
}

fn validate_backup(path: &Path) -> Result<(), StateFailure> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(path, flags)
        .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
    integrity_check(&connection)
        .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
    validate_application_id(&connection)
        .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
    let version = schema_version(&connection)
        .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
    if version < 0 {
        return Err(StateFailure::new(DatabaseBlockReason::BackupFailed));
    }
    if !(0..=CURRENT_SCHEMA_VERSION).contains(&version) {
        return Err(StateFailure::new(DatabaseBlockReason::BackupFailed));
    }
    validate_schema_contract(&connection, version)
        .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
    if version == CURRENT_SCHEMA_VERSION {
        validate_current_database(&connection)
            .map_err(|_| StateFailure::new(DatabaseBlockReason::BackupFailed))?;
    }
    Ok(())
}

fn apply_migrations(connection: &mut Connection, from_version: i64) -> Result<(), StateFailure> {
    apply_migrations_through(connection, from_version, CURRENT_SCHEMA_VERSION)
}

fn apply_migrations_through(
    connection: &mut Connection,
    from_version: i64,
    through_version: i64,
) -> Result<(), StateFailure> {
    let transaction = connection
        .transaction()
        .map_err(|_| StateFailure::new(DatabaseBlockReason::MigrationFailed))?;
    for &(version, sql) in MIGRATIONS {
        if version > from_version && version <= through_version {
            apply_migration(&transaction, version, sql)?;
        }
    }
    transaction
        .commit()
        .map_err(|_| StateFailure::new(DatabaseBlockReason::MigrationFailed))
}

fn apply_migration(
    transaction: &Transaction<'_>,
    version: i64,
    sql: &str,
) -> Result<(), StateFailure> {
    transaction
        .execute_batch(sql)
        .and_then(|_| transaction.pragma_update(None, "user_version", version))
        .map_err(|_| StateFailure::new(DatabaseBlockReason::MigrationFailed))
}

fn is_backup_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("state.sqlite3.backup.") && name.ends_with(".sqlite3")
}

fn timestamp_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migration_failure_rolls_back_after_a_consistent_backup() {
        let temp = TempDir::new().expect("temp dir");
        let store = StateStore::new(StatePaths::from_root(temp.path()));
        fs::create_dir_all(store.paths().root()).expect("create state root");
        let connection = Connection::open(store.paths().database()).expect("create v0 database");
        set_application_id(&connection).expect("mark v0 database");
        drop(connection);

        let original = fs::read(store.paths().database()).expect("read original database");
        let source = Connection::open(store.paths().database()).expect("open source database");
        store
            .create_consistent_backup(&source)
            .expect("create migration backup");
        drop(source);

        let mut read_only = Connection::open_with_flags(
            store.paths().database(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("open read-only source");
        let failure = apply_migrations(&mut read_only, 0);

        assert_eq!(
            failure.expect_err("read-only migration must fail").reason,
            DatabaseBlockReason::MigrationFailed
        );
        drop(read_only);
        assert_eq!(
            fs::read(store.paths().database()).expect("read preserved database"),
            original
        );
        assert!(
            store
                .backup_paths()
                .expect("read backups")
                .iter()
                .all(|path| validate_backup(path).is_ok())
        );
    }
}
