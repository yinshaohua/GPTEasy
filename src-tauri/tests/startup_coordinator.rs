#![cfg(target_os = "windows")]

use std::fs;

use gpteasy_lib::codex::{
    CodexConfigStatus, CodexInspector, CredentialFileStatus, CredentialStore, LoginStatus,
    LoginStatusCommand,
};
use gpteasy_lib::environment::EnvironmentApplication;
use gpteasy_lib::startup::{ApplicationMode, StartupBlockReason, StartupCoordinator};
use gpteasy_lib::state::{StatePaths, StateStore};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn login_command(exit_code: i32) -> LoginStatusCommand {
    LoginStatusCommand::new("cmd.exe", ["/D", "/S", "/C", &format!("exit {exit_code}")])
}

#[test]
fn startup_reports_missing_codex_config_without_creating_it() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let config_path = codex_home.path().join("config.toml");
    let coordinator = StartupCoordinator::new(
        StateStore::new(StatePaths::from_root(app_data.path())),
        CodexInspector::new(codex_home.path(), login_command(0)),
    );

    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Ready);
    assert_eq!(snapshot.codex.config_status, CodexConfigStatus::Missing);
    assert_eq!(snapshot.codex.login_status, LoginStatus::LoggedIn);
    assert_eq!(snapshot.block_reason, None);
    assert!(!config_path.exists());
}

#[test]
fn startup_reports_invalid_codex_toml_without_modifying_it() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let config_path = codex_home.path().join("config.toml");
    fs::write(&config_path, b"model = [not-valid-toml\n").expect("write invalid config");
    let original = fs::read(&config_path).expect("read invalid config");
    let coordinator = StartupCoordinator::new(
        StateStore::new(StatePaths::from_root(app_data.path())),
        CodexInspector::new(codex_home.path(), login_command(7)),
    );

    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Blocked);
    assert_eq!(
        snapshot.block_reason,
        Some(StartupBlockReason::CodexConfigInvalid)
    );
    assert_eq!(snapshot.codex.config_status, CodexConfigStatus::Invalid);
    assert_eq!(snapshot.codex.login_status, LoginStatus::NotLoggedIn);
    assert_eq!(
        fs::read(config_path).expect("read unchanged config"),
        original
    );
}

#[test]
fn startup_reports_credential_carrier_without_exposing_auth_contents() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    fs::write(
        codex_home.path().join("config.toml"),
        "cli_auth_credentials_store = 'file'\n",
    )
    .expect("write valid config");
    fs::write(codex_home.path().join("auth.json"), b"not inspected").expect("write auth marker");
    let coordinator = StartupCoordinator::new(
        StateStore::new(StatePaths::from_root(app_data.path())),
        CodexInspector::new(codex_home.path(), login_command(7)),
    );

    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.codex.credential_store, CredentialStore::File);
    assert_eq!(
        snapshot.codex.credential_file_status,
        CredentialFileStatus::Present
    );
    assert_eq!(snapshot.codex.login_status, LoginStatus::NotLoggedIn);
}

#[test]
fn unsupported_credential_carrier_blocks_startup() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    fs::write(
        codex_home.path().join("config.toml"),
        "cli_auth_credentials_store = { kind = 'file' }\n",
    )
    .expect("write unsupported config");
    let coordinator = StartupCoordinator::new(
        StateStore::new(StatePaths::from_root(app_data.path())),
        CodexInspector::new(codex_home.path(), login_command(7)),
    );

    let snapshot = coordinator.inspect();

    assert_eq!(
        snapshot.codex.credential_store,
        CredentialStore::Unsupported
    );
    assert_eq!(snapshot.mode, ApplicationMode::Blocked);
    assert_eq!(
        snapshot.block_reason,
        Some(StartupBlockReason::UnsupportedCredentialStore)
    );
}

#[test]
fn blocked_database_prevents_normal_startup_but_codex_inspection_remains_read_only() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    fs::remove_file(store.paths().database()).expect("remove database");
    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(7)),
    );

    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Blocked);
    assert_eq!(
        snapshot.block_reason,
        Some(StartupBlockReason::DatabaseUnavailable)
    );
    assert!(!codex_home.path().join("config.toml").exists());
}

#[test]
fn pending_config_operation_blocks_startup_coordination() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let config = b"model = 'gpt-5'\n";
    fs::write(codex_home.path().join("config.toml"), config).expect("write config");
    let old_fingerprint = Sha256::digest(config)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let old_credentials_fingerprint = Sha256::digest(b"file:missing")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO providers (id, name, base_url, api_key, default_model, verified_at, verification_fingerprint) \
             VALUES ('provider-1', 'Provider', 'https://provider.example/v1', 'test-key', 'model', '2026-08-07T00:00:00Z', 'verification')",
            [],
        )
        .expect("insert provider evidence");
    connection
        .execute(
            "INSERT INTO last_applied_state (singleton, mode, provider_id, config_fingerprint, credentials_fingerprint, applied_at) \
             VALUES (1, 'provider', 'provider-1', ?1, ?2, '2026-08-07T00:00:00Z')",
            [&old_fingerprint, &old_credentials_fingerprint],
        )
        .expect("insert last applied provider evidence");
    connection
        .execute(
            "INSERT INTO pending_config_operation (singleton, operation_id, operation_kind, stage, old_config_fingerprint, old_credentials_fingerprint, backup_reference, target_snapshot_json, started_at) \
             VALUES (1, 'op-1', 'switch_provider', 'registered', ?1, ?2, 'backup', '{}', '2026-08-07T00:00:00Z')",
            [&old_fingerprint, &old_credentials_fingerprint],
        )
        .expect("insert pending operation");

    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(7)),
    );
    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Blocked);
    assert_eq!(
        snapshot.block_reason,
        Some(StartupBlockReason::PendingConfigOperation)
    );
    assert_eq!(
        snapshot.pending_operation_resolution,
        Some(gpteasy_lib::startup::PendingOperationResolution::MatchesOldState)
    );
}

#[test]
fn recovered_unknown_artifacts_are_explained_as_a_management_conflict() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    fs::write(
        codex_home.path().join("config.toml"),
        b"model = 'externally-changed'\n",
    )
    .expect("write externally changed config");
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO pending_config_operation (
                singleton, operation_id, operation_kind, stage,
                old_config_fingerprint, new_config_fingerprint,
                backup_reference, target_snapshot_json, started_at
             ) VALUES (1, 'op-conflict', 'switch_provider', 'conflict',
                'old-fingerprint', 'new-fingerprint', 'backup', '{}', '1')",
            [],
        )
        .expect("insert conflict recovery fixture");

    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(7)),
    );
    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Blocked);
    assert_eq!(
        snapshot.block_reason,
        Some(StartupBlockReason::ManagedConfigConflict)
    );
    assert_eq!(
        snapshot.pending_operation_resolution,
        Some(gpteasy_lib::startup::PendingOperationResolution::Conflict)
    );
}

#[test]
fn pending_config_only_operation_can_match_its_recorded_fingerprint() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let config = b"model = 'gpt-5'\n";
    fs::write(codex_home.path().join("config.toml"), config).expect("write config");
    let config_fingerprint = Sha256::digest(config)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO pending_config_operation (singleton, operation_id, operation_kind, stage, old_config_fingerprint, backup_reference, target_snapshot_json, started_at) \
             VALUES (1, 'op-config-only', 'switch_provider', 'registered', ?1, 'backup', '{}', '2026-08-07T00:00:00Z')",
            [&config_fingerprint],
        )
        .expect("insert config-only pending operation");

    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(7)),
    );
    let snapshot = coordinator.inspect();

    assert_eq!(
        snapshot.pending_operation_resolution,
        Some(gpteasy_lib::startup::PendingOperationResolution::MatchesOldState)
    );
}

#[test]
fn pending_provider_credentials_are_read_only_hashed_for_recovery() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let config = b"model = 'gpt-5'\n";
    let auth = b"pending-auth-content";
    fs::write(codex_home.path().join("config.toml"), config).expect("write config");
    fs::write(codex_home.path().join("auth.json"), auth).expect("write auth");
    let config_fingerprint = Sha256::digest(config)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let credentials_fingerprint = Sha256::digest([b"file:present:".as_slice(), auth].concat())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO pending_config_operation (singleton, operation_id, operation_kind, stage, old_config_fingerprint, old_credentials_fingerprint, backup_reference, target_snapshot_json, started_at) \
             VALUES (1, 'op-pending-provider', 'switch_provider', 'registered', ?1, ?2, 'backup', '{}', '2026-08-07T00:00:00Z')",
            [&config_fingerprint, &credentials_fingerprint],
        )
        .expect("insert pending provider operation");

    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(7)),
    );
    let snapshot = coordinator.inspect();

    assert_eq!(
        snapshot.pending_operation_resolution,
        Some(gpteasy_lib::startup::PendingOperationResolution::MatchesOldState)
    );
}

#[test]
fn managed_config_fingerprint_conflict_blocks_startup_coordination() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    fs::write(codex_home.path().join("config.toml"), "model = 'gpt-5'\n")
        .expect("write valid config");
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO last_applied_state (singleton, mode, provider_id, config_fingerprint, applied_at) \
             VALUES (1, 'openai_login', NULL, 'fingerprint-from-before', '2026-08-07T00:00:00Z')",
            [],
        )
        .expect("insert last applied evidence");

    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(7)),
    );
    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Blocked);
    assert_eq!(
        snapshot.block_reason,
        Some(StartupBlockReason::ManagedConfigConflict)
    );
}

#[test]
fn external_openai_logout_stays_in_login_mode_with_a_warning() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let config = b"model = 'gpt-5'\n";
    fs::write(codex_home.path().join("config.toml"), config).expect("write config");
    let config_fingerprint = Sha256::digest(config)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO last_applied_state (singleton, mode, provider_id, config_fingerprint, applied_at) \
             VALUES (1, 'openai_login', NULL, ?1, '2026-08-07T00:00:00Z')",
            [&config_fingerprint],
        )
        .expect("insert login mode evidence");

    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(7)),
    );
    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Ready);
    assert_eq!(snapshot.block_reason, None);
    assert_eq!(snapshot.codex.login_status, LoginStatus::NotLoggedIn);
}

#[test]
fn matching_provider_config_and_credential_evidence_is_ready() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let config = b"cli_auth_credentials_store = 'file'\n";
    fs::write(codex_home.path().join("config.toml"), config).expect("write config");
    let auth = b"auth-content";
    fs::write(codex_home.path().join("auth.json"), auth).expect("write auth");
    let config_fingerprint = Sha256::digest(config)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let credential_fingerprint = Sha256::digest([b"file:present:".as_slice(), auth].concat())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO providers (id, name, base_url, api_key, default_model, verified_at, verification_fingerprint) \
             VALUES ('provider-1', 'Provider', 'https://provider.example/v1', 'test-key', 'model', '2026-08-07T00:00:00Z', 'verification')",
            [],
        )
        .expect("insert provider evidence");
    connection
        .execute(
            "INSERT INTO last_applied_state (singleton, mode, provider_id, config_fingerprint, credentials_fingerprint, applied_at) \
             VALUES (1, 'provider', 'provider-1', ?1, ?2, '2026-08-07T00:00:00Z')",
            [&config_fingerprint, &credential_fingerprint],
        )
        .expect("insert last applied evidence");

    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(0)),
    );
    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Ready);
    assert_eq!(snapshot.block_reason, None);
}

#[test]
fn provider_startup_accepts_outside_edits_but_blocks_managed_block_drift() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let provider_id = "9f319739-f219-48ee-be35-22e08d5402d7";
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO providers (id, name, base_url, api_key, default_model, verified_at, verification_fingerprint) \
             VALUES (?1, 'Provider', 'https://provider.example/v1', 'test-key', 'model', '1775606400', 'verification')",
            [provider_id],
        )
        .expect("insert provider evidence");
    EnvironmentApplication::new(store.clone(), codex_home.path())
        .apply_provider(provider_id, true)
        .expect("establish managed environment");
    let config_path = codex_home.path().join("config.toml");
    let mut outside_edit = fs::read_to_string(&config_path).expect("read managed config");
    outside_edit.push_str("\n[projects.external]\ntrust_level = 'trusted'\n");
    fs::write(&config_path, &outside_edit).expect("write outside edit");
    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(0)),
    );

    let compatible = coordinator.inspect();
    assert_eq!(compatible.mode, ApplicationMode::Ready);
    assert_eq!(compatible.block_reason, None);

    let drifted = outside_edit.replace("https://provider.example/v1", "https://drifted.example/v1");
    fs::write(config_path, drifted).expect("write managed block drift");
    let conflict = coordinator.inspect();
    assert_eq!(conflict.mode, ApplicationMode::Blocked);
    assert_eq!(
        conflict.block_reason,
        Some(StartupBlockReason::ManagedConfigConflict)
    );
}

#[test]
fn provider_mode_with_unverifiable_keyring_evidence_blocks_startup() {
    let app_data = TempDir::new().expect("app data");
    let codex_home = TempDir::new().expect("codex home");
    let config = b"cli_auth_credentials_store = 'keyring'\n";
    fs::write(codex_home.path().join("config.toml"), config).expect("write config");
    let config_fingerprint = Sha256::digest(config)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let store = StateStore::new(StatePaths::from_root(app_data.path()));
    assert!(store.bootstrap().is_ready());
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO providers (id, name, base_url, api_key, default_model, verified_at, verification_fingerprint) \
             VALUES ('provider-1', 'Provider', 'https://provider.example/v1', 'test-key', 'model', '2026-08-07T00:00:00Z', 'verification')",
            [],
        )
        .expect("insert provider evidence");
    connection
        .execute(
            "INSERT INTO last_applied_state (singleton, mode, provider_id, config_fingerprint, applied_at) \
             VALUES (1, 'provider', 'provider-1', ?1, '2026-08-07T00:00:00Z')",
            [&config_fingerprint],
        )
        .expect("insert last applied evidence");

    let coordinator = StartupCoordinator::new(
        store,
        CodexInspector::new(codex_home.path(), login_command(0)),
    );
    let snapshot = coordinator.inspect();

    assert_eq!(snapshot.mode, ApplicationMode::Blocked);
    assert_eq!(
        snapshot.block_reason,
        Some(StartupBlockReason::ManagedConfigConflict)
    );
}
