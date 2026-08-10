use std::fs;
use std::sync::Arc;

use gpteasy_lib::state::{StateFailurePoint, StateFaultInjector, StatePaths, StateStore};
use rusqlite::Connection;

pub fn create_version_zero_database(store: &StateStore) {
    fs::create_dir_all(store.paths().root()).expect("create state root");
    let connection = Connection::open(store.paths().database()).expect("create v0 database");
    connection
        .pragma_update(None, "application_id", 1_196_446_789_i64)
        .expect("mark v0 database as GPTEasy state");
    connection
        .pragma_update(None, "user_version", 0_i64)
        .expect("create v0 schema");
}

pub fn with_migration_failure(paths: StatePaths) -> StateStore {
    StateStore::with_fault_injector(paths, Arc::new(FailMigration))
}

struct FailMigration;

impl StateFaultInjector for FailMigration {
    fn fails_at(&self, point: StateFailurePoint) -> bool {
        point == StateFailurePoint::BeforeMigration
    }
}
