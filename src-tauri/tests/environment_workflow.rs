use std::fs;
use std::sync::Arc;

use gpteasy_lib::environment::{
    ArtifactAction, ArtifactKind, EnvironmentApplication, EnvironmentFailureCategory,
    EnvironmentFailurePoint, EnvironmentFaultInjector, EnvironmentRecovery, EnvironmentState,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use rusqlite::{Connection, params};
use serde_json::Value;
use tempfile::TempDir;

const PROVIDER_ID: &str = "9f319739-f219-48ee-be35-22e08d5402d7";
const API_KEY: &str = "test-key-not-real";

fn fixture() -> (TempDir, StateStore, EnvironmentApplication) {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    let application = EnvironmentApplication::new(store.clone(), temp.path().join(".codex"));
    (temp, store, application)
}

fn insert_provider(store: &StateStore) {
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                PROVIDER_ID,
                "Fixture Provider",
                "https://fixture.example/v1",
                API_KEY,
                "fixture-model",
                "1775606400",
                "fixture-verification-fingerprint",
            ],
        )
        .expect("insert provider fixture");
}

#[test]
fn missing_codex_artifacts_are_previewed_without_being_created() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");

    let snapshot = application.inspect().expect("inspect missing environment");

    assert_eq!(snapshot.state, EnvironmentState::External);
    assert!(snapshot.requires_takeover_confirmation);
    assert_eq!(
        snapshot
            .impacts
            .iter()
            .map(|impact| (impact.artifact, impact.action))
            .collect::<Vec<_>>(),
        [
            (ArtifactKind::Config, ArtifactAction::Create),
            (ArtifactKind::Credentials, ArtifactAction::Create),
        ]
    );
    assert!(!codex_home.exists());

    let failure = application
        .apply_provider(PROVIDER_ID, false)
        .expect_err("external environment must require confirmation");
    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::TakeoverConfirmationRequired
    );
    assert!(!codex_home.exists());
}

#[test]
fn confirmed_takeover_preserves_external_fields_and_records_the_applied_provider() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let original_config = concat!(
        "# user heading\r\n",
        "model = \"old-model\"\r\n",
        "model_provider = \"legacy\"\r\n",
        "custom_flag = true\r\n",
        "\r\n",
        "[model_providers.legacy]\r\n",
        "name = \"Legacy\"\r\n",
        "base_url = \"https://legacy.example/v1\"\r\n",
        "wire_api = \"responses\"\r\n",
        "\r\n",
        "[projects.demo]\r\n",
        "trust_level = \"trusted\"\r\n",
    );
    let original_auth = concat!(
        "{\n",
        "  \"auth_mode\": \"chatgpt\",\n",
        "  \"tokens\": {\"access_token\": \"fixture-token\"},\n",
        "  \"custom_auth_field\": true\n",
        "}\n",
    );
    fs::write(codex_home.join("config.toml"), original_config).expect("write config fixture");
    fs::write(codex_home.join("auth.json"), original_auth).expect("write auth fixture");

    let applied = application
        .apply_provider(PROVIDER_ID, true)
        .expect("take over external environment");

    assert_eq!(applied.state, EnvironmentState::Managed);
    assert_eq!(
        applied
            .current_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(PROVIDER_ID)
    );
    let config = fs::read_to_string(codex_home.join("config.toml")).expect("read applied config");
    let document = config
        .parse::<toml_edit::DocumentMut>()
        .expect("applied config is TOML");
    assert_eq!(document["model"].as_str(), Some("fixture-model"));
    assert_eq!(document["model_provider"].as_str(), Some(PROVIDER_ID));
    assert_eq!(
        document["model_providers"][PROVIDER_ID]["base_url"].as_str(),
        Some("https://fixture.example/v1")
    );
    assert_eq!(
        document["model_providers"][PROVIDER_ID]["requires_openai_auth"].as_bool(),
        Some(true)
    );
    assert_eq!(document["custom_flag"].as_bool(), Some(true));
    assert_eq!(
        document["model_providers"]["legacy"]["name"].as_str(),
        Some("Legacy")
    );
    assert_eq!(
        document["projects"]["demo"]["trust_level"].as_str(),
        Some("trusted")
    );
    assert!(config.contains("# GPTEasy provider-id: 9f319739-f219-48ee-be35-22e08d5402d7"));
    assert!(!config.replace("\r\n", "").contains('\n'));

    let auth: Value = serde_json::from_slice(
        &fs::read(codex_home.join("auth.json")).expect("read applied credentials"),
    )
    .expect("applied credentials are JSON");
    assert_eq!(auth["auth_mode"], "apikey");
    assert_eq!(auth["OPENAI_API_KEY"], API_KEY);
    assert_eq!(auth["tokens"]["access_token"], "fixture-token");
    assert_eq!(auth["custom_auth_field"], true);

    let backup_root = codex_home.join(".gpteasy-backups");
    let operation_backup = fs::read_dir(&backup_root)
        .expect("read backups")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .expect("operation backup");
    assert_eq!(
        fs::read(operation_backup.join("config.toml")).expect("read config backup"),
        original_config.as_bytes()
    );
    assert_eq!(
        fs::read(operation_backup.join("auth.json")).expect("read auth backup"),
        original_auth.as_bytes()
    );
    assert!(operation_backup.join("manifest.json").is_file());

    let state = store.bootstrap();
    let contents = state.contents.expect("database contents");
    assert!(contents.has_last_applied_state);
    assert!(!contents.has_pending_config_operation);
}

#[test]
fn confirmed_switch_creates_the_file_credential_carrier() {
    let (temp, _, application) = fixture();

    let applied = application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply provider to missing environment");

    assert_eq!(applied.state, EnvironmentState::Managed);
    let auth: Value = serde_json::from_slice(
        &fs::read(temp.path().join(".codex/auth.json")).expect("read created credentials"),
    )
    .expect("created credentials are JSON");
    assert_eq!(auth["auth_mode"], "apikey");
    assert_eq!(auth["OPENAI_API_KEY"], API_KEY);
}

#[test]
fn credential_commit_failure_restores_every_old_artifact_and_database_fact() {
    let (temp, store, _) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let original_config = b"model = \"old\"\ncustom_flag = true\n";
    let original_auth = b"{\"auth_mode\":\"chatgpt\",\"custom\":true}\n";
    fs::write(codex_home.join("config.toml"), original_config).expect("write config fixture");
    fs::write(codex_home.join("auth.json"), original_auth).expect("write auth fixture");
    let application = EnvironmentApplication::with_fault_injector(
        store.clone(),
        &codex_home,
        Arc::new(FailBeforeCredentials),
    );

    let failure = application
        .apply_provider(PROVIDER_ID, true)
        .expect_err("credential failure must abort the switch");

    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ArtifactWriteFailed
    );
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read rolled back config"),
        original_config
    );
    assert_eq!(
        fs::read(codex_home.join("auth.json")).expect("read preserved credentials"),
        original_auth
    );
    let snapshot = application
        .inspect()
        .expect("inspect rolled back environment");
    assert_eq!(snapshot.state, EnvironmentState::External);
    assert!(snapshot.current_provider.is_none());
    let contents = store.bootstrap().contents.expect("database contents");
    assert!(!contents.has_last_applied_state);
    assert!(!contents.has_pending_config_operation);
}

#[test]
fn recovery_completes_the_database_commit_when_both_new_artifacts_are_present() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    let old = application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply old provider");
    let old_fingerprints = last_applied_fingerprints(&store);
    let next_id = "924b6c4b-889c-44af-9bd3-e4892e42dac1";
    insert_provider_values(
        &store,
        next_id,
        "Next Provider",
        "https://next.example/v1",
        "next-key-not-real",
        "next-model",
        "next-verification-fingerprint",
    );
    let next = application
        .apply_provider(next_id, false)
        .expect("write both new artifacts");
    let new_fingerprints = last_applied_fingerprints(&store);
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "UPDATE last_applied_state SET provider_id = ?1,
                config_fingerprint = ?2, credentials_fingerprint = ?3
             WHERE singleton = 1",
            params![PROVIDER_ID, old_fingerprints.0, old_fingerprints.1],
        )
        .expect("restore pre-crash database state");
    let target_snapshot = serde_json::json!({
        "id": next_id,
        "name": "Next Provider",
        "baseUrl": "https://next.example/v1",
        "apiKey": "next-key-not-real",
        "defaultModel": "next-model",
        "verifiedAtEpochSeconds": 1_775_606_400_u64,
        "verificationFingerprint": "next-verification-fingerprint",
    });
    connection
        .execute(
            "INSERT INTO pending_config_operation (
                singleton, operation_id, operation_kind, stage, target_provider_id,
                old_config_fingerprint, new_config_fingerprint,
                old_credentials_fingerprint, new_credentials_fingerprint,
                backup_reference, target_snapshot_json, started_at
             ) VALUES (1, 'crash-fixture', 'switch_provider', 'prepared', ?1, ?2, ?3, ?4, ?5,
                'unused-for-forward-recovery', ?6, '1')",
            params![
                next_id,
                old_fingerprints.0,
                new_fingerprints.0,
                old_fingerprints.1,
                new_fingerprints.1,
                target_snapshot.to_string(),
            ],
        )
        .expect("record crash fixture");
    drop(connection);

    let recovery = application
        .recover_pending()
        .expect("recover pending switch");

    assert_eq!(recovery, EnvironmentRecovery::CompletedNewState);
    let recovered = application
        .inspect()
        .expect("inspect recovered environment");
    assert_eq!(recovered.state, EnvironmentState::Managed);
    assert_eq!(
        recovered
            .current_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(next_id)
    );
    assert_eq!(recovered.current_provider, next.current_provider);
    assert_ne!(recovered.current_provider, old.current_provider);
    assert!(codex_home.join("config.toml").is_file());
    assert!(
        !store
            .bootstrap()
            .contents
            .expect("database contents")
            .has_pending_config_operation
    );
}

#[test]
fn configuration_backups_keep_the_latest_five_operations() {
    let (temp, _, application) = fixture();
    for _ in 0..7 {
        application
            .apply_provider(PROVIDER_ID, true)
            .expect("repeat provider application");
    }
    let backup_count = fs::read_dir(temp.path().join(".codex/.gpteasy-backups"))
        .expect("read backup root")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count();
    assert_eq!(backup_count, 5);
}

#[test]
fn malformed_managed_markers_stop_before_backup_or_write() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let original = b"# >>> GPTEasy managed provider >>>\nmodel = \"broken\"\n";
    fs::write(codex_home.join("config.toml"), original).expect("write malformed config");

    let snapshot = application.inspect().expect("inspect malformed config");
    assert_eq!(snapshot.state, EnvironmentState::Conflict);
    let failure = application
        .apply_provider(PROVIDER_ID, true)
        .expect_err("malformed managed block must be rejected");
    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ManagedConflict
    );
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read preserved malformed config"),
        original
    );
    assert!(!codex_home.join(".gpteasy-backups").exists());
}

fn last_applied_fingerprints(store: &StateStore) -> (String, String) {
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .query_row(
            "SELECT config_fingerprint, credentials_fingerprint
             FROM last_applied_state WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("last applied fingerprints")
}

struct FailBeforeCredentials;

impl EnvironmentFaultInjector for FailBeforeCredentials {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool {
        point == EnvironmentFailurePoint::BeforeCredentialsReplace
    }
}

fn insert_provider_values(
    store: &StateStore,
    id: &str,
    name: &str,
    base_url: &str,
    api_key: &str,
    default_model: &str,
    fingerprint: &str,
) {
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, ?2, ?3, ?4, ?5, '1775606400', ?6)",
            params![id, name, base_url, api_key, default_model, fingerprint],
        )
        .expect("insert provider fixture");
}
