use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use gpteasy_lib::codex::{LoginInspection, LoginMethod, LoginStatus};
use gpteasy_lib::consumer::{
    ConsumerIdentity, ConsumerRole, ConsumerScan, ConsumerScanner, ConsumerStatus,
};
use gpteasy_lib::environment::{
    ArtifactAction, ArtifactKind, AuthenticationMode, EnvironmentApplication,
    EnvironmentFailureCategory, EnvironmentFailurePoint, EnvironmentFaultInjector,
    EnvironmentRecovery, EnvironmentState, OpenAiLoginProbe, RestoreAvailability,
};
use gpteasy_lib::session_visibility::{
    SessionVisibilityApplication, VisibilityConsumerState, VisibilityExecutionRequest,
    VisibilityExecutionRuntime, VisibilityFailure, VisibilityRuntimeFuture, VisibilityTarget,
    VisibilityTargetMode, VisibilityVerificationViews,
};
use gpteasy_lib::state::{
    PendingSessionVisibilityStatus, PendingSessionVisibilityTargetMode, StatePaths, StateStore,
};
use rusqlite::{Connection, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const PROVIDER_ID: &str = "9f319739-f219-48ee-be35-22e08d5402d7";
const API_KEY: &str = "test-key-not-real";

fn fixture() -> (TempDir, StateStore, EnvironmentApplication) {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    let application = EnvironmentApplication::with_runtime_probes(
        store.clone(),
        temp.path().join(".codex"),
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
        Arc::new(StoppedConsumers),
    );
    (temp, store, application)
}

struct StoppedConsumers;

impl ConsumerScanner for StoppedConsumers {
    fn scan(&self) -> ConsumerScan {
        ConsumerScan {
            desktop: ConsumerStatus::Stopped,
            cli: ConsumerStatus::Stopped,
            identities: Vec::new(),
            desktop_roots: Vec::new(),
        }
    }
}

struct TargetOnlyVisibilityRuntime(VisibilityTarget);

impl VisibilityExecutionRuntime for TargetOnlyVisibilityRuntime {
    fn current_target(&self) -> Result<VisibilityTarget, VisibilityFailure> {
        Ok(self.0.clone())
    }

    fn baseline_views<'a>(
        &'a self,
        _target_provider: &'a str,
    ) -> VisibilityRuntimeFuture<'a, VisibilityVerificationViews> {
        Box::pin(async {
            Ok(VisibilityVerificationViews {
                all_providers: Vec::new(),
                target_provider: Vec::new(),
            })
        })
    }

    fn shutdown_owned_app_server(&self) -> VisibilityRuntimeFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn consumers(&self, _exclude_owned_app_server: bool) -> VisibilityConsumerState {
        VisibilityConsumerState::NoConsumers
    }

    fn verification_views<'a>(
        &'a self,
        _target_provider: &'a str,
    ) -> VisibilityRuntimeFuture<'a, VisibilityVerificationViews> {
        Box::pin(async {
            Ok(VisibilityVerificationViews {
                all_providers: Vec::new(),
                target_provider: Vec::new(),
            })
        })
    }
}

struct MutableConsumers(Mutex<ConsumerScan>);

impl MutableConsumers {
    fn new(scan: ConsumerScan) -> Self {
        Self(Mutex::new(scan))
    }

    fn set(&self, scan: ConsumerScan) {
        *self.0.lock().expect("lock consumer fixture") = scan;
    }
}

impl ConsumerScanner for MutableConsumers {
    fn scan(&self) -> ConsumerScan {
        self.0.lock().expect("lock consumer fixture").clone()
    }
}

struct StartsConsumerDuringWrite {
    scanner: Arc<MutableConsumers>,
}

impl EnvironmentFaultInjector for StartsConsumerDuringWrite {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool {
        if point == EnvironmentFailurePoint::BeforeCredentialsReplace {
            let started_at_epoch_millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_millis() as u64;
            self.scanner.set(running_cli(42, started_at_epoch_millis));
        }
        false
    }
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

fn insert_second_provider(store: &StateStore, provider_id: &str) {
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, 'Second Provider', 'https://second.example/v1',
                       'second-key', 'second-model', '1775606500', 'second-fingerprint')",
            [provider_id],
        )
        .expect("insert second provider fixture");
}

fn running_cli(pid: u32, started_at_epoch_millis: u64) -> ConsumerScan {
    ConsumerScan {
        desktop: ConsumerStatus::Stopped,
        cli: ConsumerStatus::Running,
        identities: vec![ConsumerIdentity {
            role: ConsumerRole::Cli,
            pid,
            started_at_epoch_millis,
        }],
        desktop_roots: Vec::new(),
    }
}

fn stopped_consumers() -> ConsumerScan {
    ConsumerScan {
        desktop: ConsumerStatus::Stopped,
        cli: ConsumerStatus::Stopped,
        identities: Vec::new(),
        desktop_roots: Vec::new(),
    }
}

fn running_desktop(consumers: &[(u32, u64)]) -> ConsumerScan {
    let desktop_roots = consumers
        .iter()
        .map(|(pid, started_at_epoch_millis)| ConsumerIdentity {
            role: ConsumerRole::Desktop,
            pid: *pid,
            started_at_epoch_millis: *started_at_epoch_millis,
        })
        .collect::<Vec<_>>();
    ConsumerScan {
        desktop: ConsumerStatus::Running,
        cli: ConsumerStatus::Stopped,
        identities: desktop_roots.clone(),
        desktop_roots,
    }
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
fn session_visibility_context_resolves_provider_and_openai_targets() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    let visibility = SessionVisibilityApplication::with_recovery_root(
        &codex_home,
        temp.path().join("visibility-recovery"),
    )
    .with_pending_state(store.clone());
    let provider = application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish provider mode");

    let provider_context = application
        .inspect_for_session_visibility()
        .expect("inspect provider visibility target");

    assert_eq!(provider_context.state, EnvironmentState::Managed);
    assert_eq!(provider_context.mode, Some(AuthenticationMode::Provider));
    assert_eq!(provider_context.provider_id.as_deref(), Some(PROVIDER_ID));
    assert_eq!(provider_context.revision, provider.revision);
    assert!(!provider_context.pending_operation);
    visibility
        .record_pending(&VisibilityTarget {
            mode: VisibilityTargetMode::Provider,
            model_provider: PROVIDER_ID.to_owned(),
            environment_revision: provider_context.revision.clone(),
        })
        .expect("persist provider visibility target after mode switch");
    assert_eq!(
        store
            .pending_session_visibility()
            .expect("read provider visibility state")
            .expect("provider visibility state")
            .target_mode,
        PendingSessionVisibilityTargetMode::Provider,
    );

    let openai = application
        .switch_to_openai_login(true, &provider.revision)
        .expect("switch to OpenAI login mode");
    let openai_context = application
        .inspect_for_session_visibility()
        .expect("inspect OpenAI visibility target");

    assert_eq!(openai_context.state, EnvironmentState::Managed);
    assert_eq!(openai_context.mode, Some(AuthenticationMode::OpenaiLogin));
    assert!(openai_context.provider_id.is_none());
    assert_eq!(openai_context.revision, openai.revision);
    assert!(!openai_context.pending_operation);
    visibility
        .record_pending(&VisibilityTarget {
            mode: VisibilityTargetMode::OpenaiLogin,
            model_provider: "openai".to_owned(),
            environment_revision: openai_context.revision.clone(),
        })
        .expect("persist OpenAI visibility target after mode switch");
    let pending = store
        .pending_session_visibility()
        .expect("read OpenAI visibility state")
        .expect("OpenAI visibility state");
    assert_eq!(
        pending.target_mode,
        PendingSessionVisibilityTargetMode::OpenaiLogin,
    );
    assert_eq!(pending.environment_revision, openai.revision);
    assert!(codex_home.exists());
}

#[tokio::test]
async fn visibility_repair_failure_after_mode_switch_does_not_roll_back_the_environment() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    let switched = application
        .apply_provider(PROVIDER_ID, true)
        .expect("switch provider mode before visibility repair");
    let target = VisibilityTarget {
        mode: VisibilityTargetMode::Provider,
        model_provider: PROVIDER_ID.to_owned(),
        environment_revision: switched.revision.clone(),
    };
    let visibility = SessionVisibilityApplication::with_recovery_root(
        &codex_home,
        temp.path().join("visibility-recovery"),
    )
    .with_pending_state(store.clone());
    visibility
        .record_pending(&target)
        .expect("record visibility repair after successful switch");

    let failure = visibility
        .execute_pending(
            VisibilityExecutionRequest {
                confirmation_id: "stale-confirmation".to_owned(),
                target: target.clone(),
            },
            &TargetOnlyVisibilityRuntime(target),
        )
        .await
        .expect_err("stale confirmation is an ordinary determinate repair failure");

    assert_eq!(failure.message_id, "session_visibility.rescan_required");
    let after = application
        .inspect_for_session_visibility()
        .expect("inspect environment after failed visibility repair");
    assert_eq!(after.mode, Some(AuthenticationMode::Provider));
    assert_eq!(after.provider_id.as_deref(), Some(PROVIDER_ID));
    assert_eq!(after.revision, switched.revision);
    let pending = store
        .pending_session_visibility()
        .expect("read retryable visibility state")
        .expect("failed repair remains pending");
    assert_eq!(pending.status, PendingSessionVisibilityStatus::Pending);
    assert_eq!(pending.error_code, "session_visibility.rescan_required",);
}

#[test]
fn session_visibility_context_keeps_last_managed_target_during_conflict() {
    let (temp, _, application) = fixture();
    let config_path = temp.path().join(".codex/config.toml");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish provider mode");
    let managed = fs::read_to_string(&config_path).expect("read managed config");
    let unknown_provider = "11111111-1111-4111-8111-111111111111";
    fs::write(&config_path, managed.replace(PROVIDER_ID, unknown_provider))
        .expect("introduce managed provider conflict");

    let context = application
        .inspect_for_session_visibility()
        .expect("inspect conflicted visibility target");

    assert_eq!(context.state, EnvironmentState::Conflict);
    assert_eq!(context.mode, Some(AuthenticationMode::Provider));
    assert_eq!(context.provider_id.as_deref(), Some(PROVIDER_ID));
}

#[test]
fn confirmed_first_provider_application_initializes_a_never_started_codex_home() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");

    let applied = application
        .apply_provider(PROVIDER_ID, true)
        .expect("confirmed application initializes missing Codex artifacts");

    assert_eq!(applied.state, EnvironmentState::Managed);
    assert_eq!(applied.mode, Some(AuthenticationMode::Provider));
    assert!(codex_home.join("config.toml").is_file());
    assert!(codex_home.join("auth.json").is_file());
    let config =
        fs::read_to_string(codex_home.join("config.toml")).expect("read initialized config");
    assert!(config.contains("model_provider = \"9f319739-f219-48ee-be35-22e08d5402d7\""));
    assert!(config.contains("requires_openai_auth = true"));
    let credentials: Value = serde_json::from_slice(
        &fs::read(codex_home.join("auth.json")).expect("read initialized credentials"),
    )
    .expect("parse initialized credentials");
    assert_eq!(credentials["auth_mode"], "apikey");
    assert_eq!(credentials["OPENAI_API_KEY"], API_KEY);
}

#[test]
fn missing_codex_artifacts_can_be_recreated_by_standard_provider_application() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish provider mode");
    fs::remove_file(codex_home.join("config.toml")).expect("remove config fixture");
    fs::remove_file(codex_home.join("auth.json")).expect("remove credentials fixture");

    let missing = application
        .inspect()
        .expect("inspect missing Codex artifacts");
    assert_eq!(missing.state, EnvironmentState::Conflict);
    assert!(missing.takeover_available);

    let reapplied = application
        .apply_provider_at_revision(PROVIDER_ID, true, &missing.revision)
        .expect("standard provider application recreates missing artifacts");

    assert_eq!(reapplied.state, EnvironmentState::Managed);
    assert_eq!(reapplied.mode, Some(AuthenticationMode::Provider));
    assert!(codex_home.join("config.toml").is_file());
    assert!(codex_home.join("auth.json").is_file());
}

#[test]
fn changes_after_preview_abort_before_backup_or_write() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(
        codex_home.join("config.toml"),
        b"model = 'external-model'\ncustom_flag = true\n",
    )
    .expect("write previewed config");
    fs::write(
        codex_home.join("auth.json"),
        br#"{"auth_mode":"apikey","OPENAI_API_KEY":"external-key"}"#,
    )
    .expect("write previewed credentials");

    let preview = application.inspect().expect("preview external environment");
    let changed = b"model = 'externally-edited-model'\ncustom_flag = true\n";
    fs::write(codex_home.join("config.toml"), changed).expect("simulate external edit");

    let failure = application
        .apply_provider_at_revision(PROVIDER_ID, true, &preview.revision)
        .expect_err("stale preview must be rejected");

    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ConcurrentModification
    );
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read external edit"),
        changed
    );
    assert!(!codex_home.join(".gpteasy-backups").exists());
}

#[test]
fn credential_changes_after_preview_abort_before_backup_or_write() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(codex_home.join("config.toml"), b"custom_flag = true\n")
        .expect("write previewed config");
    fs::write(
        codex_home.join("auth.json"),
        br#"{"auth_mode":"apikey","OPENAI_API_KEY":"external-key"}"#,
    )
    .expect("write previewed credentials");

    let preview = application.inspect().expect("preview external environment");
    let changed = br#"{"auth_mode":"apikey","OPENAI_API_KEY":"externally-edited-key"}"#;
    fs::write(codex_home.join("auth.json"), changed).expect("simulate credential edit");

    let failure = application
        .apply_provider_at_revision(PROVIDER_ID, true, &preview.revision)
        .expect_err("stale credential preview must be rejected");

    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ConcurrentModification
    );
    assert_eq!(
        fs::read(codex_home.join("auth.json")).expect("read external credential edit"),
        changed
    );
    assert!(!codex_home.join(".gpteasy-backups").exists());
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
    assert_eq!(
        document["model_providers"][PROVIDER_ID]["supports_websockets"].as_bool(),
        Some(false)
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
    assert!(
        !operation_backup.join("auth.json").exists(),
        "OpenAI tokens must not be copied into configuration backups"
    );
    let manifest: Value = serde_json::from_slice(
        &fs::read(operation_backup.join("manifest.json")).expect("read backup manifest"),
    )
    .expect("parse backup manifest");
    assert_eq!(manifest["configExisted"], true);
    assert_eq!(manifest["credentialsExisted"], true);
    assert_eq!(manifest["credentialFields"]["authMode"], "chatgpt");
    assert_eq!(manifest["credentialFields"]["openaiApiKey"], Value::Null);
    assert!(!manifest.to_string().contains("fixture-token"));
    assert_eq!(
        manifest["oldConfigFingerprint"],
        format!("{:x}", Sha256::digest(original_config.as_bytes()))
    );
    let mut auth_hasher = Sha256::new();
    auth_hasher.update(b"file:present:");
    auth_hasher.update(original_auth.as_bytes());
    assert_eq!(
        manifest["oldCredentialsFingerprint"],
        format!("{:x}", auth_hasher.finalize())
    );

    let state = store.bootstrap();
    let contents = state.contents.expect("database contents");
    assert!(contents.has_last_applied_state);
    assert!(!contents.has_pending_config_operation);
}

#[test]
fn switching_provider_keeps_prior_managed_provider_id_as_current_provider_alias() {
    const PREVIOUS_PROVIDER_ID: &str = "63b3934d-36d2-44fa-ab24-b70f93b45418";
    let (temp, store, application) = fixture();
    insert_provider_values(
        &store,
        PREVIOUS_PROVIDER_ID,
        "Previous Provider",
        "https://previous.example/v1",
        "previous-key",
        "previous-model",
        "previous-fingerprint",
    );
    let codex_home = temp.path().join(".codex");

    application
        .apply_provider(PREVIOUS_PROVIDER_ID, true)
        .expect("apply previous provider");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("switch to current provider");

    let config = fs::read_to_string(codex_home.join("config.toml")).expect("read config");
    let document = config
        .parse::<toml_edit::DocumentMut>()
        .expect("applied config is TOML");
    let alias = &document["model_providers"][PREVIOUS_PROVIDER_ID];
    assert_eq!(alias["name"].as_str(), Some("Fixture Provider"));
    assert_eq!(
        alias["base_url"].as_str(),
        Some("https://fixture.example/v1")
    );
    assert_eq!(alias["wire_api"].as_str(), Some("responses"));
    assert_eq!(alias["requires_openai_auth"].as_bool(), Some(true));
    assert_eq!(document["model_provider"].as_str(), Some(PROVIDER_ID));
}

#[test]
fn managed_environment_accepts_an_equivalent_historical_provider_alias() {
    const HISTORICAL_PROVIDER_ID: &str = "63b3934d-36d2-44fa-ab24-b70f93b45418";
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");

    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let alias = format!(
        "model_providers.{HISTORICAL_PROVIDER_ID}.name = \"Fixture Provider\"\n\
model_providers.{HISTORICAL_PROVIDER_ID}.base_url = \"https://fixture.example/v1\"\n\
model_providers.{HISTORICAL_PROVIDER_ID}.wire_api = \"responses\"\n\
model_providers.{HISTORICAL_PROVIDER_ID}.requires_openai_auth = true\n"
    );
    let rewritten = original.replace(
        "# <<< GPTEasy managed provider <<<\n",
        &format!("{alias}# <<< GPTEasy managed provider <<<\n"),
    );
    fs::write(&config_path, rewritten).expect("add equivalent historical alias");

    let snapshot = application
        .inspect()
        .expect("inspect equivalent historical alias");

    assert_eq!(snapshot.state, EnvironmentState::Managed);
}

#[test]
fn managed_environment_rejects_a_historical_provider_alias_with_different_fields() {
    const HISTORICAL_PROVIDER_ID: &str = "63b3934d-36d2-44fa-ab24-b70f93b45418";
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");

    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let alias = format!(
        "model_providers.{HISTORICAL_PROVIDER_ID}.name = \"Fixture Provider\"\n\
model_providers.{HISTORICAL_PROVIDER_ID}.base_url = \"https://unexpected.example/v1\"\n\
model_providers.{HISTORICAL_PROVIDER_ID}.wire_api = \"responses\"\n\
model_providers.{HISTORICAL_PROVIDER_ID}.requires_openai_auth = true\n"
    );
    let rewritten = original.replace(
        "# <<< GPTEasy managed provider <<<\n",
        &format!("{alias}# <<< GPTEasy managed provider <<<\n"),
    );
    fs::write(&config_path, &rewritten).expect("add drifted historical alias");

    let snapshot = application
        .inspect()
        .expect("inspect drifted historical alias");
    assert_eq!(snapshot.state, EnvironmentState::Conflict);
}

#[test]
fn applying_provider_keeps_older_managed_provider_ids_from_completed_backups() {
    const HISTORICAL_PROVIDER_ID: &str = "63b3934d-36d2-44fa-ab24-b70f93b45418";
    const INTERMEDIATE_PROVIDER_ID: &str = "6cde0dd7-9725-462a-ac79-864f5cf63f76";
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    insert_provider_values(
        &store,
        HISTORICAL_PROVIDER_ID,
        "Historical Provider",
        "https://historical.example/v1",
        "historical-key",
        "historical-model",
        "historical-fingerprint",
    );
    insert_provider_values(
        &store,
        INTERMEDIATE_PROVIDER_ID,
        "Intermediate Provider",
        "https://intermediate.example/v1",
        "intermediate-key",
        "intermediate-model",
        "intermediate-fingerprint",
    );

    application
        .apply_provider(HISTORICAL_PROVIDER_ID, true)
        .expect("apply historical provider");
    application
        .apply_provider(INTERMEDIATE_PROVIDER_ID, true)
        .expect("create completed backup of historical provider");
    Connection::open(store.paths().database())
        .expect("open state database")
        .execute(
            "DELETE FROM providers WHERE id = ?1",
            [HISTORICAL_PROVIDER_ID],
        )
        .expect("delete historical provider from catalog");
    fs::write(codex_home.join("config.toml"), "custom_flag = true\n")
        .expect("replace current config with external fixture");

    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply current provider");

    let config = fs::read_to_string(codex_home.join("config.toml")).expect("read config");
    let document = config
        .parse::<toml_edit::DocumentMut>()
        .expect("applied config is TOML");
    let alias = &document["model_providers"][HISTORICAL_PROVIDER_ID];
    assert_eq!(alias["name"].as_str(), Some("Fixture Provider"));
    assert_eq!(
        alias["base_url"].as_str(),
        Some("https://fixture.example/v1")
    );
    assert_eq!(alias["wire_api"].as_str(), Some("responses"));
    assert_eq!(alias["requires_openai_auth"].as_bool(), Some(true));
}

#[test]
fn applying_provider_migrates_matching_legacy_custom_provider_to_openai_auth() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(
        codex_home.join("config.toml"),
        "model_provider = \"custom\"\n[model_providers.custom]\nname = \"custom\"\nbase_url = \"https://fixture.example/v1\"\nwire_api = \"responses\"\n",
    )
    .expect("write legacy custom config");

    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply current provider");

    let config = fs::read_to_string(codex_home.join("config.toml")).expect("read config");
    let document = config
        .parse::<toml_edit::DocumentMut>()
        .expect("applied config is TOML");
    assert_eq!(
        document["model_providers"]["custom"]["requires_openai_auth"].as_bool(),
        Some(true)
    );
}

#[test]
fn managed_environment_accepts_and_preserves_changes_outside_the_managed_block() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let mut externally_edited = fs::read_to_string(&config_path).expect("read managed config");
    externally_edited.push_str("\n[projects.external]\ntrust_level = \"trusted\"\n");
    fs::write(&config_path, &externally_edited).expect("write outside managed block");

    let snapshot = application.inspect().expect("inspect external edit");
    assert_eq!(snapshot.state, EnvironmentState::Managed);
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("reapply after compatible external edit");

    let reapplied = fs::read_to_string(config_path).expect("read reapplied config");
    assert!(reapplied.ends_with("\n[projects.external]\ntrust_level = \"trusted\"\n"));
}

#[test]
fn managed_environment_accepts_legacy_block_without_websocket_default() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let legacy = original.replace(
        &format!("model_providers.{PROVIDER_ID}.supports_websockets = false\n"),
        "",
    );
    assert_ne!(legacy, original);
    fs::write(&config_path, legacy).expect("write legacy managed config");

    let snapshot = application
        .inspect()
        .expect("inspect legacy managed config");

    assert_eq!(snapshot.state, EnvironmentState::Managed);
}

#[test]
fn incomplete_managed_provider_table_is_a_conflict_instead_of_a_panic() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let incomplete = original.replace(
        &format!("model_providers.{PROVIDER_ID}.base_url = \"https://fixture.example/v1\"\n"),
        "",
    );
    assert_ne!(incomplete, original);
    fs::write(&config_path, incomplete).expect("write incomplete managed config");

    let snapshot = application
        .inspect()
        .expect("inspect incomplete managed config");

    assert_eq!(snapshot.state, EnvironmentState::Conflict);
}

#[test]
fn managed_environment_recovers_the_end_marker_after_a_desktop_rewrite() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex home");
    fs::write(codex_home.join("config.toml"), "custom_flag = true\r\n").expect("write CRLF config");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let rewritten = original.replace("# <<< GPTEasy managed provider <<<\r\n", "")
        + "[desktop]\r\nconversationDetailMode = 'compact'\r\n";
    assert_ne!(rewritten, original);
    fs::write(&config_path, rewritten).expect("simulate desktop Codex rewrite");

    let snapshot = application.inspect().expect("inspect desktop rewrite");
    assert_eq!(snapshot.state, EnvironmentState::Managed);
    application
        .apply_provider_at_revision(PROVIDER_ID, true, &snapshot.revision)
        .expect("reapply after desktop rewrite");

    let reapplied = fs::read_to_string(config_path).expect("read reapplied config");
    assert_eq!(
        reapplied
            .matches("# <<< GPTEasy managed provider <<<")
            .count(),
        1
    );
    assert!(reapplied.contains("[desktop]\r\nconversationDetailMode = 'compact'\r\n"));
    assert!(reapplied.contains("custom_flag = true\r\n"));
}

#[test]
fn managed_environment_recovers_a_relocated_end_marker_after_a_desktop_rewrite() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let rewritten = original.replace(
        "# <<< GPTEasy managed provider <<<\n",
        "[desktop]\nconversationDetailMode = 'compact'\n# <<< GPTEasy managed provider <<<\n",
    );
    assert_ne!(rewritten, original);
    fs::write(&config_path, rewritten).expect("simulate desktop Codex marker relocation");

    let snapshot = application.inspect().expect("inspect desktop rewrite");
    assert_eq!(snapshot.state, EnvironmentState::Managed);
    application
        .apply_provider_at_revision(PROVIDER_ID, true, &snapshot.revision)
        .expect("reapply after desktop rewrite");

    let reapplied = fs::read_to_string(config_path).expect("read reapplied config");
    assert_eq!(
        reapplied
            .matches("# <<< GPTEasy managed provider <<<")
            .count(),
        1
    );
    assert!(reapplied.contains("[desktop]\nconversationDetailMode = 'compact'\n"));
    assert!(reapplied.find("# <<< GPTEasy managed provider <<<") < reapplied.find("[desktop]"));
}

#[test]
fn restore_of_an_unchanged_apply_preserves_a_compatible_desktop_rewrite() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("record an unchanged completed apply");
    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let rewritten = original.replace(
        "# <<< GPTEasy managed provider <<<\n",
        "[desktop]\nconversationDetailMode = 'compact'\n# <<< GPTEasy managed provider <<<\n",
    );
    fs::write(&config_path, &rewritten).expect("simulate desktop Codex marker relocation");

    let snapshot = application.inspect().expect("inspect desktop rewrite");
    assert_eq!(snapshot.state, EnvironmentState::Managed);
    assert_eq!(
        snapshot.restore_availability,
        RestoreAvailability::Available
    );
    let restored = application
        .restore_last_config(true, &snapshot.revision)
        .expect("restore unchanged apply without discarding desktop settings");

    assert_eq!(restored.state, EnvironmentState::Managed);
    assert_eq!(
        fs::read_to_string(config_path).expect("read restored config"),
        rewritten
    );
    let contents = store.bootstrap().contents.expect("database contents");
    assert!(contents.has_last_applied_state);
    assert!(!contents.has_pending_config_operation);
}

#[test]
fn restore_of_an_unchanged_apply_still_rejects_managed_drift() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("record an unchanged completed apply");
    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let drifted = original.replace("https://fixture.example/v1", "https://drifted.example/v1");
    assert_ne!(drifted, original);
    fs::write(&config_path, &drifted).expect("drift a managed field");

    let snapshot = application.inspect().expect("inspect managed drift");
    assert_eq!(snapshot.state, EnvironmentState::Conflict);
    assert_eq!(
        snapshot.restore_availability,
        RestoreAvailability::ArtifactsChanged
    );
    let failure = application
        .restore_last_config(true, &snapshot.revision)
        .expect_err("managed drift cannot use the unchanged-artifact exception");

    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ManagedConflict
    );
    assert_eq!(
        fs::read_to_string(config_path).expect("read preserved drift"),
        drifted
    );
    assert!(
        !store
            .bootstrap()
            .contents
            .expect("database contents")
            .has_pending_config_operation
    );
}

#[test]
fn missing_end_marker_without_last_applied_evidence_remains_a_conflict() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let rewritten = original.replace("# <<< GPTEasy managed provider <<<\n", "");
    fs::write(&config_path, rewritten).expect("remove end marker");
    Connection::open(store.paths().database())
        .expect("open state database")
        .execute("DELETE FROM last_applied_state", [])
        .expect("remove last applied evidence");

    let snapshot = application.inspect().expect("inspect missing evidence");

    assert_eq!(snapshot.state, EnvironmentState::Conflict);
}

#[test]
fn managed_block_with_an_unowned_field_is_a_conflict_and_cannot_be_repaired() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let original = fs::read_to_string(&config_path).expect("read managed config");
    let damaged = original.replace(
        &format!("model_provider = \"{PROVIDER_ID}\"\n"),
        &format!("model_provider = \"{PROVIDER_ID}\"\nunowned = true\n"),
    );
    assert_ne!(damaged, original);
    fs::write(&config_path, &damaged).expect("damage managed block");

    let preview = application.inspect().expect("inspect damaged block");
    assert_eq!(preview.state, EnvironmentState::Conflict);
    let failure = application
        .apply_provider_at_revision(PROVIDER_ID, true, &preview.revision)
        .expect_err("unowned managed fields must be rejected");

    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ManagedConflict
    );
    assert_eq!(
        fs::read_to_string(&config_path).expect("read preserved config"),
        damaged
    );
}

#[test]
fn managed_block_nested_under_a_toml_table_is_rejected_without_writing() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let managed = fs::read_to_string(&config_path).expect("read managed config");
    let nested = format!("[profile]\n{managed}");
    fs::write(&config_path, &nested).expect("nest managed block under a table");

    let preview = application.inspect().expect("inspect nested block");
    assert_eq!(preview.state, EnvironmentState::Conflict);
    let backup_root = codex_home.join(".gpteasy-backups");
    let backup_count_before = fs::read_dir(&backup_root)
        .expect("read existing backups")
        .filter_map(Result::ok)
        .count();
    let failure = application
        .apply_provider_at_revision(PROVIDER_ID, true, &preview.revision)
        .expect_err("nested managed block must be rejected");

    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ManagedConflict
    );
    assert_eq!(
        fs::read_to_string(&config_path).expect("read preserved config"),
        nested
    );
    assert_eq!(
        fs::read_dir(&backup_root)
            .expect("read preserved backups")
            .filter_map(Result::ok)
            .count(),
        backup_count_before
    );
}

#[test]
fn renaming_the_current_provider_does_not_create_a_management_conflict() {
    let (_temp, store, application) = fixture();
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "UPDATE providers SET name = 'Renamed Fixture' WHERE id = ?1",
            [PROVIDER_ID],
        )
        .expect("rename current provider");

    let snapshot = application.inspect().expect("inspect renamed provider");
    assert_eq!(snapshot.state, EnvironmentState::Managed);
    assert_eq!(
        snapshot
            .current_provider
            .as_ref()
            .map(|provider| provider.name.as_str()),
        Some("Renamed Fixture")
    );
}

#[test]
fn losing_the_managed_block_or_changing_its_id_is_a_conflict_after_takeover() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let without_block = fs::read_to_string(&config_path)
        .expect("read managed config")
        .split("# >>> GPTEasy managed provider >>>")
        .next()
        .unwrap_or_default()
        .to_owned();
    fs::write(&config_path, without_block).expect("remove managed block externally");
    assert_eq!(
        application.inspect().expect("inspect missing block").state,
        EnvironmentState::Conflict
    );

    application
        .apply_provider(PROVIDER_ID, true)
        .expect("explicitly retakeover missing block");
    let unknown_id = "924b6c4b-889c-44af-9bd3-e4892e42dac1";
    let changed_id = fs::read_to_string(&config_path)
        .expect("read retaken config")
        .replace(PROVIDER_ID, unknown_id);
    fs::write(&config_path, changed_id).expect("change managed provider id externally");
    assert_eq!(
        application.inspect().expect("inspect unknown id").state,
        EnvironmentState::Conflict
    );
}

#[test]
fn confirmed_retakeover_repairs_a_drifted_but_well_formed_managed_block() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let drifted = fs::read_to_string(&config_path)
        .expect("read managed config")
        .replace("https://fixture.example/v1", "https://drifted.example/v1");
    fs::write(&config_path, drifted).expect("write managed drift");

    let snapshot = application.inspect().expect("inspect managed drift");
    assert_eq!(snapshot.state, EnvironmentState::Conflict);
    assert!(snapshot.requires_takeover_confirmation);
    assert!(snapshot.takeover_available);
    let unconfirmed = application
        .apply_provider(PROVIDER_ID, false)
        .expect_err("retakeover requires confirmation");
    assert_eq!(
        unconfirmed.category,
        EnvironmentFailureCategory::TakeoverConfirmationRequired
    );

    let repaired = application
        .apply_provider(PROVIDER_ID, true)
        .expect("confirm safe retakeover");
    assert_eq!(repaired.state, EnvironmentState::Managed);
    assert!(
        fs::read_to_string(config_path)
            .expect("read repaired config")
            .contains("https://fixture.example/v1")
    );
}

#[test]
fn editing_only_managed_block_formatting_still_requires_confirmed_retakeover() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish managed environment");
    let config_path = codex_home.join("config.toml");
    let edited = fs::read_to_string(&config_path)
        .expect("read managed config")
        .replace(
            "# GPTEasy provider-id:",
            "# external formatting edit\n# GPTEasy provider-id:",
        );
    fs::write(&config_path, edited).expect("edit managed block formatting");

    let snapshot = application.inspect().expect("inspect formatting edit");
    assert_eq!(snapshot.state, EnvironmentState::Conflict);
    assert!(snapshot.requires_takeover_confirmation);
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
fn configuration_failure_categories_never_expose_api_keys() {
    let cases: [(
        Arc<dyn EnvironmentFaultInjector>,
        EnvironmentFailureCategory,
    ); 3] = [
        (
            Arc::new(FailBackupCreation),
            EnvironmentFailureCategory::BackupFailed,
        ),
        (
            Arc::new(FailBeforeCredentials),
            EnvironmentFailureCategory::ArtifactWriteFailed,
        ),
        (
            Arc::new(FailRollback),
            EnvironmentFailureCategory::RollbackFailed,
        ),
    ];

    for (fault, expected_category) in cases {
        let (temp, store, _) = fixture();
        let application =
            EnvironmentApplication::with_fault_injector(store, temp.path().join(".codex"), fault);

        let failure = application
            .apply_provider(PROVIDER_ID, true)
            .expect_err("injected configuration failure must be reported");

        assert_eq!(failure.category, expected_category);
        assert!(
            !format!("{failure:?}").contains(API_KEY),
            "{expected_category:?} must not expose the API key"
        );
    }
}

#[test]
fn logged_in_user_can_switch_to_openai_while_preserving_tokens_and_clearing_api_key() {
    const OPENAI_TOKEN_CANARY: &str = "openai-token-canary-must-stay-private";
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish provider mode");
    let login_credentials = format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"{OPENAI_TOKEN_CANARY}"}},"OPENAI_API_KEY":"obsolete-provider-key","last_refresh":"private"}}"#,
    );
    fs::write(codex_home.join("auth.json"), login_credentials.as_bytes())
        .expect("simulate Codex-managed OpenAI login");
    let application = EnvironmentApplication::with_login_probe(
        store.clone(),
        &codex_home,
        Arc::new(FixedLoginProbe(LoginStatus::LoggedIn)),
    );
    let before = application.inspect().expect("inspect before mode switch");

    let unconfirmed = application
        .switch_to_openai_login(false, &before.revision)
        .expect_err("mode switch requires explicit confirmation");
    assert_eq!(
        unconfirmed.category,
        EnvironmentFailureCategory::ModeSwitchConfirmationRequired
    );
    assert_eq!(
        fs::read(codex_home.join("auth.json")).expect("read unchanged login credentials"),
        login_credentials.as_bytes()
    );

    let switched = application
        .switch_to_openai_login(true, &before.revision)
        .expect("switch to OpenAI login mode");

    assert_eq!(switched.mode, Some(AuthenticationMode::OpenaiLogin));
    assert_eq!(switched.state, EnvironmentState::Managed);
    assert!(switched.current_provider.is_none());
    assert_eq!(switched.login_status, LoginStatus::LoggedIn);
    let config = fs::read_to_string(codex_home.join("config.toml")).expect("read config");
    assert!(!config.contains("GPTEasy managed provider"));
    let credentials: Value = serde_json::from_slice(
        &fs::read(codex_home.join("auth.json")).expect("read restored login credentials"),
    )
    .expect("restored login credentials remain valid JSON");
    assert_eq!(credentials["auth_mode"], "chatgpt");
    assert!(credentials.get("OPENAI_API_KEY").is_none());
    assert_eq!(credentials["tokens"]["access_token"], OPENAI_TOKEN_CANARY);
    assert_eq!(credentials["last_refresh"], "private");
    let latest_backup = latest_backup_path(&codex_home);
    let backup_contents = fs::read_dir(&latest_backup)
        .expect("read OpenAI switch backup")
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path()).ok())
        .flatten()
        .collect::<Vec<_>>();
    assert!(!String::from_utf8_lossy(&backup_contents).contains(OPENAI_TOKEN_CANARY));
    let connection = Connection::open(store.paths().database()).expect("open state database");
    let (mode, provider_id): (String, Option<String>) = connection
        .query_row(
            "SELECT mode, provider_id FROM last_applied_state WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read applied mode");
    assert_eq!(mode, "openai_login");
    assert_eq!(provider_id, None);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM providers", [], |row| row
                .get::<_, i64>(0))
            .expect("count providers"),
        1,
        "OpenAI login mode must not enter the provider catalog"
    );

    let logged_out = EnvironmentApplication::with_login_probe(
        store,
        &codex_home,
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
    )
    .inspect()
    .expect("inspect external logout");
    assert_eq!(logged_out.state, EnvironmentState::Managed);
    assert_eq!(logged_out.mode, Some(AuthenticationMode::OpenaiLogin));
    assert_eq!(logged_out.message_id, "environment.openai_login_missing");
}

#[test]
fn missing_or_unavailable_openai_login_still_exits_provider_mode() {
    for status in [LoginStatus::NotLoggedIn, LoginStatus::Unavailable] {
        let (temp, store, application) = fixture();
        let codex_home = temp.path().join(".codex");
        application
            .apply_provider(PROVIDER_ID, true)
            .expect("establish provider mode");
        assert!(!codex_home.join(".gpteasy-openai-login.json").exists());
        let application = EnvironmentApplication::with_login_probe(
            store.clone(),
            &codex_home,
            Arc::new(FixedLoginProbe(status)),
        );
        let before = application
            .inspect()
            .expect("inspect before unauthenticated switch");

        let switched = application
            .switch_to_openai_login(true, &before.revision)
            .expect("missing login must not trap the user in provider mode");

        assert_eq!(switched.state, EnvironmentState::Managed);
        assert_eq!(switched.mode, Some(AuthenticationMode::OpenaiLogin));
        assert_eq!(switched.login_status, status);
        assert!(switched.current_provider.is_none());
        let config = fs::read_to_string(codex_home.join("config.toml")).expect("read config");
        assert!(!config.contains("GPTEasy managed provider"));
        assert!(!codex_home.join("auth.json").exists());
        assert!(!codex_home.join(".gpteasy-openai-login.json").exists());
        let connection = Connection::open(store.paths().database()).expect("open state database");
        let (mode, provider_id): (String, Option<String>) = connection
            .query_row(
                "SELECT mode, provider_id FROM last_applied_state WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read applied mode");
        assert_eq!(mode, "openai_login");
        assert_eq!(provider_id, None);
    }
}

#[test]
fn returning_from_openai_to_a_provider_requires_confirmation_without_backing_up_tokens() {
    const OPENAI_TOKEN_CANARY: &str = "return-mode-openai-token-canary";
    let (temp, store, _) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(codex_home.join("config.toml"), b"custom_flag = true\n")
        .expect("write external config");
    let login_credentials = format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"{OPENAI_TOKEN_CANARY}"}},"last_refresh":"private"}}"#,
    );
    fs::write(codex_home.join("auth.json"), login_credentials.as_bytes())
        .expect("write Codex-managed login credentials");
    let application = EnvironmentApplication::with_login_probe(
        store,
        &codex_home,
        Arc::new(FixedLoginProbe(LoginStatus::LoggedIn)),
    );
    let external = application.inspect().expect("inspect external environment");
    let openai = application
        .switch_to_openai_login(true, &external.revision)
        .expect("establish OpenAI login mode");

    let failure = application
        .apply_provider_at_revision(PROVIDER_ID, false, &openai.revision)
        .expect_err("returning to provider mode requires confirmation");
    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ModeSwitchConfirmationRequired
    );
    assert_eq!(
        fs::read(codex_home.join("auth.json")).expect("read unchanged login credentials"),
        login_credentials.as_bytes()
    );

    let provider = application
        .apply_provider_at_revision(PROVIDER_ID, true, &openai.revision)
        .expect("confirm return to provider mode");

    assert_eq!(provider.mode, Some(AuthenticationMode::Provider));
    let credentials: Value = serde_json::from_slice(
        &fs::read(codex_home.join("auth.json")).expect("read provider credentials"),
    )
    .expect("provider credentials remain valid JSON");
    assert_eq!(credentials["auth_mode"], "apikey");
    assert_eq!(credentials["OPENAI_API_KEY"], API_KEY);
    assert_eq!(credentials["tokens"]["access_token"], OPENAI_TOKEN_CANARY);
    let latest_backup = latest_backup_path(&codex_home);
    let backup_contents = fs::read_dir(&latest_backup)
        .expect("read provider switch backup")
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path()).ok())
        .flatten()
        .collect::<Vec<_>>();
    assert!(!String::from_utf8_lossy(&backup_contents).contains(OPENAI_TOKEN_CANARY));
}

#[test]
fn returning_to_openai_reactivates_preserved_chatgpt_credentials() {
    const OPENAI_TOKEN_CANARY: &str = "reactivated-openai-token-canary";
    let (temp, store, _) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(codex_home.join("config.toml"), b"custom_flag = true\n")
        .expect("write external config");
    fs::write(
        codex_home.join("auth.json"),
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"{OPENAI_TOKEN_CANARY}"}},"last_refresh":"private"}}"#,
        ),
    )
    .expect("write Codex-managed login credentials");
    let application = EnvironmentApplication::with_login_probe(
        store.clone(),
        &codex_home,
        Arc::new(FixedLoginProbe(LoginStatus::LoggedIn)),
    );
    let external = application.inspect().expect("inspect external environment");
    let openai = application
        .switch_to_openai_login(true, &external.revision)
        .expect("establish OpenAI login mode");
    application
        .apply_provider_at_revision(PROVIDER_ID, true, &openai.revision)
        .expect("switch to provider mode");
    assert!(
        codex_home.join(".gpteasy-openai-login.json").exists(),
        "switching away from ChatGPT must retain one local recovery snapshot"
    );
    fs::write(
        codex_home.join("auth.json"),
        br#"{"auth_mode":"apikey","OPENAI_API_KEY":"provider-key-after-codex-rewrite"}"#,
    )
    .expect("simulate Codex rewriting provider credentials without ChatGPT tokens");
    let application = EnvironmentApplication::with_login_probe(
        store,
        &codex_home,
        Arc::new(MethodLoginProbe(LoginStatus::LoggedIn, LoginMethod::ApiKey)),
    );
    let before_return = application
        .inspect()
        .expect("inspect after Codex rewrites provider credentials");

    let returned = application
        .switch_to_openai_login(true, &before_return.revision)
        .expect("return to OpenAI login mode");

    assert_eq!(returned.mode, Some(AuthenticationMode::OpenaiLogin));
    let credentials: Value = serde_json::from_slice(
        &fs::read(codex_home.join("auth.json")).expect("read restored login credentials"),
    )
    .expect("restored login credentials remain valid JSON");
    assert_eq!(credentials["auth_mode"], "chatgpt");
    assert!(credentials.get("OPENAI_API_KEY").is_none());
    assert_eq!(credentials["tokens"]["access_token"], OPENAI_TOKEN_CANARY);
    assert_eq!(credentials["last_refresh"], "private");
    assert!(
        !codex_home.join(".gpteasy-openai-login.json").exists(),
        "the recovery snapshot is consumed after a successful return"
    );
}

#[test]
fn api_key_login_exits_to_openai_mode_without_impersonating_chatgpt_login() {
    let (temp, store, _) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(
        codex_home.join("config.toml"),
        b"cli_auth_credentials_store = 'keyring'\ncustom_flag = true\n",
    )
    .expect("write keyring config");
    let application = EnvironmentApplication::with_login_probe(
        store,
        &codex_home,
        Arc::new(MethodLoginProbe(LoginStatus::LoggedIn, LoginMethod::ApiKey)),
    );
    let before = application.inspect().expect("inspect API key login");
    assert_eq!(before.login_status, LoginStatus::NotLoggedIn);

    let switched = application
        .switch_to_openai_login(true, &before.revision)
        .expect("API key login must not trap the user outside OpenAI mode");

    assert_eq!(switched.state, EnvironmentState::Managed);
    assert_eq!(switched.mode, Some(AuthenticationMode::OpenaiLogin));
    assert_eq!(switched.login_status, LoginStatus::NotLoggedIn);
    assert!(switched.current_provider.is_none());
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read preserved config"),
        b"cli_auth_credentials_store = 'keyring'\ncustom_flag = true\n"
    );
    assert!(!codex_home.join("auth.json").exists());
    assert!(codex_home.join(".gpteasy-backups").exists());
}

#[test]
fn restoring_a_provider_switch_returns_to_openai_login_mode() {
    let (temp, store, _) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(codex_home.join("config.toml"), b"custom_flag = true\n")
        .expect("write external config");
    fs::write(
        codex_home.join("auth.json"),
        br#"{"auth_mode":"chatgpt","tokens":{"access_token":"private"}}"#,
    )
    .expect("write login credentials");
    let application = EnvironmentApplication::with_login_probe(
        store,
        &codex_home,
        Arc::new(FixedLoginProbe(LoginStatus::LoggedIn)),
    );
    let external = application.inspect().expect("inspect external environment");
    let openai = application
        .switch_to_openai_login(true, &external.revision)
        .expect("establish OpenAI login mode");
    let provider = application
        .apply_provider_at_revision(PROVIDER_ID, true, &openai.revision)
        .expect("switch to provider mode");
    let preview = provider.restore_preview.as_ref().expect("restore preview");
    assert_eq!(preview.target_mode, Some(AuthenticationMode::OpenaiLogin));

    let restored = application
        .restore_last_config(true, &provider.revision)
        .expect("restore OpenAI login mode");

    assert_eq!(restored.state, EnvironmentState::Managed);
    assert_eq!(restored.mode, Some(AuthenticationMode::OpenaiLogin));
    assert!(restored.current_provider.is_none());
}

#[test]
fn interrupted_openai_switch_recovers_without_exposing_or_rewriting_tokens() {
    const OPENAI_TOKEN_CANARY: &str = "recovery-openai-token-canary";
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish provider mode");
    let login_credentials = format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"{OPENAI_TOKEN_CANARY}"}}}}"#,
    );
    fs::write(codex_home.join("auth.json"), login_credentials.as_bytes())
        .expect("simulate Codex-managed login");
    let interrupted = EnvironmentApplication::with_dependencies(
        store.clone(),
        &codex_home,
        Arc::new(InterruptAt(
            EnvironmentFailurePoint::AfterAllArtifactsReplaced,
        )),
        Arc::new(FixedLoginProbe(LoginStatus::LoggedIn)),
    );
    let before = interrupted
        .inspect()
        .expect("inspect before interrupted switch");

    let failure = interrupted
        .switch_to_openai_login(true, &before.revision)
        .expect_err("inject interruption before database commit");
    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::OperationInterrupted
    );
    let restarted = EnvironmentApplication::with_login_probe(
        store,
        &codex_home,
        Arc::new(FixedLoginProbe(LoginStatus::LoggedIn)),
    );
    assert_eq!(
        restarted.recover_pending().expect("recover OpenAI switch"),
        EnvironmentRecovery::CompletedNewState
    );
    let recovered = restarted.inspect().expect("inspect recovered mode");
    assert_eq!(recovered.mode, Some(AuthenticationMode::OpenaiLogin));
    assert_eq!(
        fs::read(codex_home.join("auth.json")).expect("read untouched token carrier"),
        login_credentials.as_bytes()
    );
    let observable = format!("{failure:?} {recovered:?}");
    assert!(!observable.contains(OPENAI_TOKEN_CANARY));
    let backup_contents = fs::read_dir(latest_backup_path(&codex_home))
        .expect("read recovered backup")
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path()).ok())
        .flatten()
        .collect::<Vec<_>>();
    assert!(!String::from_utf8_lossy(&backup_contents).contains(OPENAI_TOKEN_CANARY));
}

#[test]
fn external_provider_configuration_replaces_openai_mode_without_becoming_a_conflict() {
    let (temp, store, _) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(codex_home.join("config.toml"), b"custom_flag = true\n")
        .expect("write initial config");
    let application = EnvironmentApplication::with_login_probe(
        store,
        &codex_home,
        Arc::new(FixedLoginProbe(LoginStatus::LoggedIn)),
    );
    let external = application.inspect().expect("inspect initial config");
    application
        .switch_to_openai_login(true, &external.revision)
        .expect("establish OpenAI login mode");
    fs::write(
        codex_home.join("config.toml"),
        b"model = 'external-model'\nmodel_provider = 'external-provider'\n\
          [model_providers.external-provider]\nbase_url = 'https://external.example/v1'\n\
          wire_api = 'responses'\n",
    )
    .expect("write external provider config");

    let snapshot = application
        .inspect()
        .expect("inspect external provider config");

    assert_eq!(snapshot.state, EnvironmentState::External);
    assert_eq!(snapshot.mode, None);
    assert_eq!(snapshot.message_id, "environment.external");

    let switched = application
        .switch_to_openai_login(true, &snapshot.revision)
        .expect("confirm OpenAI takeover of external provider selection");
    assert_eq!(switched.state, EnvironmentState::Managed);
    assert_eq!(switched.mode, Some(AuthenticationMode::OpenaiLogin));
    let config =
        fs::read_to_string(codex_home.join("config.toml")).expect("read OpenAI mode configuration");
    let document = config
        .parse::<toml_edit::DocumentMut>()
        .expect("OpenAI mode config remains valid TOML");
    assert!(document.get("model").is_none());
    assert!(document.get("model_provider").is_none());
    assert_eq!(
        document["model_providers"]["external-provider"]["base_url"].as_str(),
        Some("https://external.example/v1"),
        "inactive external definitions must be preserved"
    );
}

#[test]
fn externally_restored_chatgpt_credentials_override_stale_provider_history() {
    let (temp, _store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("establish provider history");
    fs::write(
        codex_home.join("config.toml"),
        b"model = 'gpt-5.4'\nmodel_reasoning_effort = 'high'\n",
    )
    .expect("simulate externally restored OpenAI config");
    fs::write(
        codex_home.join("auth.json"),
        br#"{"auth_mode":"chatgpt","tokens":{"access_token":"private"}}"#,
    )
    .expect("simulate externally restored ChatGPT credentials");

    let snapshot = application
        .inspect()
        .expect("inspect externally restored OpenAI login");

    assert_eq!(snapshot.state, EnvironmentState::External);
    assert_eq!(snapshot.mode, Some(AuthenticationMode::OpenaiLogin));
    assert_eq!(snapshot.message_id, "environment.external_openai_login");
    assert!(snapshot.current_provider.is_none());
    assert!(snapshot.takeover_available);
}

#[test]
fn recovery_completes_the_database_commit_when_both_new_artifacts_are_present() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    let old = application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply old provider");
    let old_fingerprints = artifact_fingerprints(&codex_home);
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
        .apply_provider(next_id, true)
        .expect("write both new artifacts");
    let new_fingerprints = artifact_fingerprints(&codex_home);
    let backup = latest_backup_path(&codex_home);
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
                ?6, ?7, '1')",
            params![
                next_id,
                old_fingerprints.0,
                new_fingerprints.0,
                old_fingerprints.1,
                new_fingerprints.1,
                backup.to_string_lossy(),
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
fn recovery_stops_on_a_mixed_artifact_state_without_overwriting_it() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply old provider");
    let old_config = fs::read(codex_home.join("config.toml")).expect("read old config");
    let old_credentials = fs::read(codex_home.join("auth.json")).expect("read old credentials");
    let old_fingerprints = artifact_fingerprints(&codex_home);
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
    application
        .apply_provider(next_id, true)
        .expect("apply next provider");
    let new_fingerprints = artifact_fingerprints(&codex_home);
    let backup = latest_backup_path(&codex_home);
    let backup_manifest: Value = serde_json::from_slice(
        &fs::read(backup.join("manifest.json")).expect("read backup manifest"),
    )
    .expect("parse backup manifest");
    let operation_id = backup_manifest["operationId"]
        .as_str()
        .expect("operation id in backup manifest");

    fs::write(codex_home.join("auth.json"), &old_credentials)
        .expect("simulate crash between artifact replacements");
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
             ) VALUES (1, ?1, 'switch_provider', 'prepared', ?2, ?3, ?4, ?5, ?6, ?7, ?8, '1')",
            params![
                operation_id,
                next_id,
                old_fingerprints.0,
                new_fingerprints.0,
                old_fingerprints.1,
                new_fingerprints.1,
                backup.to_string_lossy(),
                target_snapshot.to_string(),
            ],
        )
        .expect("record mixed crash fixture");
    drop(connection);

    let recovery = application
        .recover_pending()
        .expect("recover mixed pending switch");

    assert_eq!(recovery, EnvironmentRecovery::Conflict);
    assert_eq!(
        fs::read(codex_home.join("auth.json")).expect("read mixed credentials"),
        old_credentials
    );
    assert_ne!(
        fs::read(codex_home.join("config.toml")).expect("read mixed config"),
        old_config
    );
    let pending = store
        .bootstrap()
        .contents
        .expect("database contents")
        .has_pending_config_operation;
    assert!(pending);
}

#[test]
fn crash_fault_matrix_converges_to_old_new_or_management_conflict_without_leaking_api_keys() {
    let cases = [
        (
            EnvironmentFailurePoint::AfterBackupCompleted,
            EnvironmentRecovery::NoPendingOperation,
            EnvironmentState::External,
        ),
        (
            EnvironmentFailurePoint::AfterPendingRegistered,
            EnvironmentRecovery::KeptOldState,
            EnvironmentState::External,
        ),
        (
            EnvironmentFailurePoint::AfterConfigReplaced,
            EnvironmentRecovery::Conflict,
            EnvironmentState::Conflict,
        ),
        (
            EnvironmentFailurePoint::AfterAllArtifactsReplaced,
            EnvironmentRecovery::CompletedNewState,
            EnvironmentState::Managed,
        ),
        (
            EnvironmentFailurePoint::BeforeDatabaseCommit,
            EnvironmentRecovery::CompletedNewState,
            EnvironmentState::Managed,
        ),
        (
            EnvironmentFailurePoint::AfterDatabaseCommit,
            EnvironmentRecovery::NoPendingOperation,
            EnvironmentState::Managed,
        ),
    ];

    for (point, expected_recovery, expected_state) in cases {
        let (temp, store, _) = fixture();
        let codex_home = temp.path().join(".codex");
        let interrupted = EnvironmentApplication::with_fault_injector(
            store.clone(),
            &codex_home,
            Arc::new(InterruptAt(point)),
        );

        let failure = interrupted
            .apply_provider(PROVIDER_ID, true)
            .expect_err("fault matrix point must simulate process interruption");
        assert_eq!(
            failure.category,
            EnvironmentFailureCategory::OperationInterrupted
        );

        let restarted = EnvironmentApplication::new(store.clone(), &codex_home);
        let recovery = restarted
            .recover_pending()
            .expect("restart recovery must classify the persisted artifacts");
        assert_eq!(
            recovery, expected_recovery,
            "unexpected recovery at {point:?}"
        );
        let snapshot = restarted
            .inspect()
            .expect("inspect the converged environment");
        assert_eq!(
            snapshot.state, expected_state,
            "unexpected state at {point:?}"
        );
        assert_eq!(
            snapshot.current_provider.is_some(),
            expected_state == EnvironmentState::Managed,
            "current provider evidence must agree with disk at {point:?}"
        );

        let observable_output = format!("{failure:?} {recovery:?} {snapshot:?}");
        assert!(
            !observable_output.contains(API_KEY),
            "recovery output must not expose the API key at {point:?}"
        );
        let pending = store
            .bootstrap()
            .contents
            .expect("database contents")
            .pending_config_operation;
        assert_eq!(
            pending.is_some(),
            expected_recovery == EnvironmentRecovery::Conflict,
            "only a management conflict remains pending at {point:?}"
        );
        if let Some(pending) = pending {
            assert_eq!(pending.stage, "conflict");
        }
    }
}

#[test]
fn confirmed_restore_returns_only_the_latest_managed_artifacts_to_their_previous_state() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let original_config = b"custom_flag = true\n";
    fs::write(codex_home.join("config.toml"), original_config).expect("write original config");

    let applied = application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply provider before restore");
    assert_eq!(applied.restore_availability, RestoreAvailability::Available);
    let preview = applied.restore_preview.expect("available restore preview");
    assert_eq!(
        preview.artifacts,
        vec![ArtifactKind::Config, ArtifactKind::Credentials]
    );
    assert_eq!(preview.target_mode, None);
    assert!(preview.target_provider.is_none());
    let applied_config = fs::read(codex_home.join("config.toml")).expect("read applied config");
    let applied_credentials =
        fs::read(codex_home.join("auth.json")).expect("read applied credentials");

    let unconfirmed = application
        .restore_last_config(false, &applied.revision)
        .expect_err("manual restore requires explicit confirmation");
    assert_eq!(
        unconfirmed.category,
        EnvironmentFailureCategory::RestoreConfirmationRequired
    );
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read unchanged config"),
        applied_config
    );
    assert_eq!(
        fs::read(codex_home.join("auth.json")).expect("read unchanged credentials"),
        applied_credentials
    );

    let restored = application
        .restore_last_config(true, &applied.revision)
        .expect("restore the latest completed GPTEasy modification");

    assert_eq!(restored.state, EnvironmentState::External);
    assert!(restored.current_provider.is_none());
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read restored config"),
        original_config
    );
    assert!(
        !codex_home.join("auth.json").exists(),
        "an originally missing artifact must be restored as missing"
    );
    let contents = store.bootstrap().contents.expect("database contents");
    assert_eq!(
        contents.provider_count, 1,
        "restore must preserve the provider catalog"
    );
    assert!(!contents.has_last_applied_state);
    assert!(!contents.has_pending_config_operation);
    assert_eq!(
        restored.restore_availability,
        RestoreAvailability::Available,
        "the restore itself is a reversible GPTEasy modification"
    );
}

#[test]
fn restore_refuses_to_overwrite_artifacts_changed_after_the_latest_operation() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply provider before external edit");
    let config_path = codex_home.join("config.toml");
    let externally_changed = format!(
        "{}\n[projects.external]\ntrust_level = \"trusted\"\n",
        fs::read_to_string(&config_path).expect("read managed config")
    );
    fs::write(&config_path, &externally_changed).expect("write external config change");
    let snapshot = application.inspect().expect("inspect external change");
    assert_eq!(snapshot.state, EnvironmentState::Managed);
    assert_eq!(
        snapshot.restore_availability,
        RestoreAvailability::ArtifactsChanged
    );

    let failure = application
        .restore_last_config(true, &snapshot.revision)
        .expect_err("restore must not overwrite an external edit");

    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ManagedConflict
    );
    assert_eq!(
        fs::read_to_string(&config_path).expect("read preserved external edit"),
        externally_changed
    );
    assert!(
        !store
            .bootstrap()
            .contents
            .expect("database contents")
            .has_pending_config_operation
    );
}

#[test]
fn restore_rejects_a_corrupted_latest_completed_backup_without_falling_back() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    fs::write(codex_home.join("config.toml"), b"custom_flag = true\n")
        .expect("write original config");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply provider before corrupting backup");
    let current_config = fs::read(codex_home.join("config.toml")).expect("read current config");
    let backup = latest_backup_path(&codex_home);
    fs::write(backup.join("config.toml"), b"not = [valid TOML")
        .expect("corrupt latest completed backup");

    let snapshot = application.inspect().expect("inspect corrupted backup");
    assert_eq!(
        snapshot.restore_availability,
        RestoreAvailability::InvalidBackup
    );
    let failure = application
        .restore_last_config(true, &snapshot.revision)
        .expect_err("corrupt latest backup must be rejected");

    assert_eq!(failure.category, EnvironmentFailureCategory::BackupInvalid);
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read preserved current config"),
        current_config
    );
}

#[test]
fn restore_rejects_inconsistent_preview_metadata() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply provider before tampering with preview metadata");
    let manifest_path = latest_backup_path(&codex_home).join("manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read backup manifest"))
            .expect("parse backup manifest");
    manifest["previousMode"] = Value::String("provider".to_owned());
    manifest["previousProviderId"] = Value::Null;
    fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize tampered manifest"),
    )
    .expect("tamper preview metadata");

    let snapshot = application
        .inspect()
        .expect("inspect tampered preview metadata");

    assert_eq!(
        snapshot.restore_availability,
        RestoreAvailability::InvalidBackup
    );
    assert!(snapshot.restore_preview.is_none());
}

#[test]
fn legacy_backup_without_restore_metadata_infers_its_provider_target() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    let next_id = "924b6c4b-889c-44af-9bd3-e4892e42dac1";
    insert_provider_values(
        &store,
        next_id,
        "Next Provider",
        "https://next.example/v1",
        "next-key",
        "next-model",
        "next-fingerprint",
    );
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply first provider");
    application
        .apply_provider(next_id, true)
        .expect("apply second provider");
    let manifest_path = latest_backup_path(&codex_home).join("manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read backup manifest"))
            .expect("parse backup manifest");
    let object = manifest.as_object_mut().expect("manifest object");
    object.remove("previousMode");
    object.remove("previousProviderId");
    object.remove("restoreTargetRecorded");
    fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize legacy manifest"),
    )
    .expect("write legacy manifest");

    let snapshot = application.inspect().expect("inspect legacy backup");
    let preview = snapshot.restore_preview.expect("legacy restore preview");

    assert_eq!(
        snapshot.restore_availability,
        RestoreAvailability::Available
    );
    assert_eq!(preview.target_mode, Some(AuthenticationMode::Provider));
    assert_eq!(
        preview.target_provider.map(|provider| provider.id),
        Some(PROVIDER_ID.to_owned())
    );
}

#[test]
fn restore_uses_only_the_immediately_previous_completed_configuration() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    let first = application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply first provider");
    let first_config = fs::read(codex_home.join("config.toml")).expect("read first config");
    let first_credentials = fs::read(codex_home.join("auth.json")).expect("read first credentials");
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
    let second = application
        .apply_provider(next_id, true)
        .expect("apply second provider");
    let preview = second.restore_preview.as_ref().expect("restore preview");
    assert_eq!(preview.target_mode, Some(AuthenticationMode::Provider));
    assert_eq!(
        preview
            .target_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(PROVIDER_ID)
    );

    let restored = application
        .restore_last_config(true, &second.revision)
        .expect("restore immediately previous configuration");

    assert_eq!(restored.state, EnvironmentState::Managed);
    assert_eq!(restored.current_provider, first.current_provider);
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read restored first config"),
        first_config
    );
    assert_eq!(
        fs::read(codex_home.join("auth.json")).expect("read restored first credentials"),
        first_credentials
    );
    assert_eq!(
        store
            .bootstrap()
            .contents
            .expect("database contents")
            .provider_count,
        2,
        "restore must not delete either verified provider"
    );
}

#[test]
fn restore_is_disabled_when_its_previous_provider_no_longer_exists() {
    let (temp, store, application) = fixture();
    let codex_home = temp.path().join(".codex");
    let next_id = "924b6c4b-889c-44af-9bd3-e4892e42dac1";
    insert_provider_values(
        &store,
        next_id,
        "Next Provider",
        "https://next.example/v1",
        "next-key",
        "next-model",
        "next-fingerprint",
    );
    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply first provider");
    let second = application
        .apply_provider(next_id, true)
        .expect("apply second provider");
    Connection::open(store.paths().database())
        .expect("open state database")
        .execute("DELETE FROM providers WHERE id = ?1", [PROVIDER_ID])
        .expect("delete previous provider");

    let snapshot = application
        .inspect()
        .expect("inspect missing restore provider");

    assert_eq!(
        snapshot.restore_availability,
        RestoreAvailability::InvalidBackup
    );
    assert!(snapshot.restore_preview.is_none());
    let failure = application
        .restore_last_config(true, &second.revision)
        .expect_err("restore must reject a missing target provider");
    assert_eq!(failure.category, EnvironmentFailureCategory::BackupInvalid);
    assert!(codex_home.join("config.toml").exists());
}

#[test]
fn restore_crash_fault_matrix_uses_the_same_recovery_protocol() {
    let cases = [
        (
            EnvironmentFailurePoint::AfterBackupCompleted,
            EnvironmentRecovery::NoPendingOperation,
            EnvironmentState::Managed,
        ),
        (
            EnvironmentFailurePoint::AfterPendingRegistered,
            EnvironmentRecovery::KeptOldState,
            EnvironmentState::Managed,
        ),
        (
            EnvironmentFailurePoint::AfterConfigReplaced,
            EnvironmentRecovery::Conflict,
            EnvironmentState::Conflict,
        ),
        (
            EnvironmentFailurePoint::AfterAllArtifactsReplaced,
            EnvironmentRecovery::CompletedNewState,
            EnvironmentState::External,
        ),
        (
            EnvironmentFailurePoint::BeforeDatabaseCommit,
            EnvironmentRecovery::CompletedNewState,
            EnvironmentState::External,
        ),
        (
            EnvironmentFailurePoint::AfterDatabaseCommit,
            EnvironmentRecovery::NoPendingOperation,
            EnvironmentState::External,
        ),
    ];

    for (point, expected_recovery, expected_state) in cases {
        let (temp, store, application) = fixture();
        let codex_home = temp.path().join(".codex");
        fs::create_dir_all(&codex_home).expect("create Codex fixture");
        fs::write(codex_home.join("config.toml"), b"custom_flag = true\n")
            .expect("write original config");
        let applied = application
            .apply_provider(PROVIDER_ID, true)
            .expect("apply provider before interrupted restore");
        let interrupted = EnvironmentApplication::with_fault_injector(
            store.clone(),
            &codex_home,
            Arc::new(InterruptAt(point)),
        );

        let failure = interrupted
            .restore_last_config(true, &applied.revision)
            .expect_err("fault matrix point must interrupt the restore");
        assert_eq!(
            failure.category,
            EnvironmentFailureCategory::OperationInterrupted
        );
        let restarted = EnvironmentApplication::new(store.clone(), &codex_home);
        let recovery = restarted
            .recover_pending()
            .expect("recover interrupted restore");
        assert_eq!(
            recovery, expected_recovery,
            "unexpected recovery at {point:?}"
        );
        let snapshot = restarted.inspect().expect("inspect recovered restore");
        assert_eq!(
            snapshot.state, expected_state,
            "unexpected state at {point:?}"
        );
        let observable_output = format!("{failure:?} {recovery:?} {snapshot:?}");
        assert!(!observable_output.contains(API_KEY));
        let pending = store
            .bootstrap()
            .contents
            .expect("database contents")
            .pending_config_operation;
        assert_eq!(
            pending.is_some(),
            expected_recovery == EnvironmentRecovery::Conflict
        );
        if let Some(pending) = pending {
            assert_eq!(pending.stage, "conflict");
        }
    }
}

#[test]
fn configuration_backups_keep_the_latest_five_operations() {
    let (temp, _, application) = fixture();
    let mut created = Vec::new();
    for _ in 0..7 {
        application
            .apply_provider(PROVIDER_ID, true)
            .expect("repeat provider application");
        created.push(latest_backup_path(&temp.path().join(".codex")));
    }
    let backups = fs::read_dir(temp.path().join(".codex/.gpteasy-backups"))
        .expect("read backup root")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 5);
    assert!(!created[0].exists());
    assert!(!created[1].exists());
    assert!(created[2..].iter().all(|path| path.exists()));
}

#[test]
fn duplicate_or_damaged_managed_metadata_is_rejected_without_new_backup() {
    for damage in ["duplicate marker", "duplicate provider id", "invalid TOML"] {
        let (temp, _, application) = fixture();
        let codex_home = temp.path().join(".codex");
        application
            .apply_provider(PROVIDER_ID, true)
            .expect("establish managed environment");
        let config_path = codex_home.join("config.toml");
        let managed = fs::read_to_string(&config_path).expect("read managed config");
        let damaged = match damage {
            "duplicate marker" => managed.replacen(
                "# >>> GPTEasy managed provider >>>",
                "# >>> GPTEasy managed provider >>>\n# >>> GPTEasy managed provider >>>",
                1,
            ),
            "duplicate provider id" => managed.replacen(
                &format!("# GPTEasy provider-id: {PROVIDER_ID}"),
                &format!(
                    "# GPTEasy provider-id: {PROVIDER_ID}\n# GPTEasy provider-id: {PROVIDER_ID}"
                ),
                1,
            ),
            "invalid TOML" => format!("{managed}broken = [\n"),
            _ => unreachable!(),
        };
        fs::write(&config_path, &damaged).expect("write damaged config");
        let preview = application.inspect().expect("inspect damaged config");
        assert_eq!(preview.state, EnvironmentState::Conflict, "{damage}");
        assert!(!preview.takeover_available, "{damage}");
        let backup_root = codex_home.join(".gpteasy-backups");
        let backup_count = fs::read_dir(&backup_root)
            .expect("read initial backup")
            .filter_map(Result::ok)
            .count();

        let failure = application
            .apply_provider_at_revision(PROVIDER_ID, true, &preview.revision)
            .expect_err("damaged metadata must not be repaired");

        assert!(
            matches!(
                failure.category,
                EnvironmentFailureCategory::ManagedConflict
                    | EnvironmentFailureCategory::InvalidConfig
            ),
            "{damage}: {:?}",
            failure.category
        );
        assert_eq!(
            fs::read_to_string(&config_path).expect("read preserved config"),
            damaged
        );
        assert_eq!(
            fs::read_dir(&backup_root)
                .expect("read unchanged backups")
                .filter_map(Result::ok)
                .count(),
            backup_count
        );
    }
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

#[test]
fn force_application_rebuilds_unreadable_artifacts_after_confirmation_and_keeps_backups() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let original_config = [0xff, 0xfe, b'=', b'1'];
    let original_credentials = b"{not valid JSON}";
    fs::write(codex_home.join("config.toml"), original_config).expect("write unreadable config");
    fs::write(codex_home.join("auth.json"), original_credentials)
        .expect("write unreadable credentials");

    let preview = application
        .inspect()
        .expect("inspect unreadable artifacts as conflict");
    let confirmation = application
        .force_apply_provider_at_revision(PROVIDER_ID, &preview.revision, false)
        .expect_err("force recovery must require an explicit rebuild confirmation");
    assert_eq!(
        confirmation.category,
        EnvironmentFailureCategory::ForceRebuildConfirmationRequired
    );
    assert!(!codex_home.join(".gpteasy-backups").exists());

    let applied = application
        .force_apply_provider_at_revision(PROVIDER_ID, &preview.revision, true)
        .expect("confirmed force recovery rebuilds the Codex artifacts");

    assert_eq!(applied.state, EnvironmentState::Managed);
    let config = fs::read_to_string(codex_home.join("config.toml")).expect("read rebuilt config");
    let document = config
        .parse::<toml_edit::DocumentMut>()
        .expect("rebuilt config is valid TOML");
    assert_eq!(document["model_provider"].as_str(), Some(PROVIDER_ID));
    let credentials: Value = serde_json::from_slice(
        &fs::read(codex_home.join("auth.json")).expect("read rebuilt credentials"),
    )
    .expect("rebuilt credentials are JSON");
    assert_eq!(credentials["auth_mode"], "apikey");
    assert_eq!(credentials["OPENAI_API_KEY"], API_KEY);

    let sidecar = fs::read_dir(&codex_home)
        .expect("read Codex home")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("config.toml.") && name.ends_with(".bak"))
        })
        .expect("force recovery creates a timestamped config sidecar backup");
    assert_eq!(
        fs::read(sidecar).expect("read sidecar backup"),
        original_config
    );

    let transaction_backup = latest_backup_path(&codex_home);
    assert_eq!(
        fs::read(transaction_backup.join("config.toml")).expect("read transaction config backup"),
        original_config
    );
    assert_eq!(
        fs::read(transaction_backup.join("auth.json"))
            .expect("read transaction credentials backup"),
        original_credentials
    );
}

#[test]
fn non_utf8_config_is_reported_as_conflict_and_rejected_without_backup() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let original = [0xff, 0xfe, b'=', b'1'];
    fs::write(codex_home.join("config.toml"), original).expect("write non-UTF-8 config");

    let preview = application
        .inspect()
        .expect("inspect non-UTF-8 config as conflict");
    assert_eq!(preview.state, EnvironmentState::Conflict);
    let failure = application
        .apply_provider_at_revision(PROVIDER_ID, true, &preview.revision)
        .expect_err("non-UTF-8 config must be rejected");

    assert_eq!(failure.category, EnvironmentFailureCategory::InvalidConfig);
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read preserved config"),
        original
    );
    assert!(!codex_home.join(".gpteasy-backups").exists());
}

#[test]
fn managed_markers_inside_a_multiline_string_are_never_treated_as_a_block() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let original = concat!(
        "note = \"\"\"\n",
        "# >>> GPTEasy managed provider >>>\n",
        "# GPTEasy provider-id: 9f319739-f219-48ee-be35-22e08d5402d7\n",
        "# <<< GPTEasy managed provider <<<\n",
        "\"\"\"\n",
    );
    fs::write(codex_home.join("config.toml"), original).expect("write multiline fixture");

    let snapshot = application.inspect().expect("inspect multiline fixture");
    assert_eq!(snapshot.state, EnvironmentState::Conflict);
    let failure = application
        .apply_provider(PROVIDER_ID, true)
        .expect_err("multiline markers must be rejected");
    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ManagedConflict
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).expect("read preserved config"),
        original
    );
    assert!(!codex_home.join(".gpteasy-backups").exists());
}

#[test]
fn non_string_credential_store_shape_stops_before_backup_or_write() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let original = b"cli_auth_credentials_store = { kind = 'file' }\ncustom_flag = true\n";
    fs::write(codex_home.join("config.toml"), original).expect("write unsupported config");

    let snapshot = application.inspect().expect("inspect unsupported config");
    assert_eq!(snapshot.state, EnvironmentState::Conflict);

    let failure = application
        .apply_provider(PROVIDER_ID, true)
        .expect_err("non-string credential store must be rejected");
    assert_eq!(failure.category, EnvironmentFailureCategory::InvalidConfig);
    assert_eq!(
        fs::read(codex_home.join("config.toml")).expect("read preserved config"),
        original
    );
    assert!(!codex_home.join(".gpteasy-backups").exists());
}

#[test]
fn redirected_config_artifact_is_rejected_without_following_the_target() {
    let (temp, _, application) = fixture();
    let codex_home = temp.path().join(".codex");
    fs::create_dir_all(&codex_home).expect("create Codex fixture");
    let redirect_target = temp.path().join("external-config.toml");
    let original = b"model = 'external-model'\n";
    fs::write(&redirect_target, original).expect("write redirect target");
    create_file_symlink(&redirect_target, &codex_home.join("config.toml"));

    let failure = application
        .inspect()
        .expect_err("redirected config must be rejected");

    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ArtifactRedirected
    );
    assert_eq!(
        fs::read(&redirect_target).expect("read redirect target"),
        original
    );
    assert!(!codex_home.join(".gpteasy-backups").exists());
}

fn artifact_fingerprints(codex_home: &Path) -> (String, String) {
    let config = fs::read(codex_home.join("config.toml")).expect("read config fingerprint input");
    let credentials =
        fs::read(codex_home.join("auth.json")).expect("read credentials fingerprint input");
    let config_fingerprint = format!("{:x}", Sha256::digest(&config));
    let mut credentials_hasher = Sha256::new();
    credentials_hasher.update(b"file:present:");
    credentials_hasher.update(credentials);
    (
        config_fingerprint,
        format!("{:x}", credentials_hasher.finalize()),
    )
}

fn latest_backup_path(codex_home: &std::path::Path) -> std::path::PathBuf {
    let mut backups = fs::read_dir(codex_home.join(".gpteasy-backups"))
        .expect("read backup root")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    backups.sort();
    backups.pop().expect("latest operation backup")
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("create file symlink fixture");
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("create file symlink fixture");
}

struct FailBeforeCredentials;

impl EnvironmentFaultInjector for FailBeforeCredentials {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool {
        point == EnvironmentFailurePoint::BeforeCredentialsReplace
    }
}

struct InterruptAt(EnvironmentFailurePoint);

impl EnvironmentFaultInjector for InterruptAt {
    fn fails_at(&self, _point: EnvironmentFailurePoint) -> bool {
        false
    }

    fn interrupts_at(&self, point: EnvironmentFailurePoint) -> bool {
        point == self.0
    }
}

struct FailBackupCreation;

impl EnvironmentFaultInjector for FailBackupCreation {
    fn fails_at(&self, _point: EnvironmentFailurePoint) -> bool {
        false
    }

    fn fails_backup_creation(&self) -> bool {
        true
    }
}

struct FailRollback;

impl EnvironmentFaultInjector for FailRollback {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool {
        point == EnvironmentFailurePoint::BeforeCredentialsReplace
    }

    fn fails_rollback(&self) -> bool {
        true
    }
}

struct FixedLoginProbe(LoginStatus);

impl OpenAiLoginProbe for FixedLoginProbe {
    fn inspect(&self) -> LoginInspection {
        LoginInspection {
            status: self.0,
            method: if self.0 == LoginStatus::LoggedIn {
                LoginMethod::ChatGpt
            } else {
                LoginMethod::Unknown
            },
        }
    }
}

struct MethodLoginProbe(LoginStatus, LoginMethod);

impl OpenAiLoginProbe for MethodLoginProbe {
    fn inspect(&self) -> LoginInspection {
        LoginInspection {
            status: self.0,
            method: self.1,
        }
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

#[test]
fn running_consumer_requires_confirmation_and_sets_pending_restart() {
    const SECOND_PROVIDER_ID: &str = "6cde0dd7-9725-462a-ac79-864f5cf63f76";
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    insert_second_provider(&store, SECOND_PROVIDER_ID);
    let scanner = Arc::new(MutableConsumers::new(stopped_consumers()));
    let application = EnvironmentApplication::with_runtime_probes(
        store,
        temp.path().join(".codex"),
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
        scanner.clone(),
    );

    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply initial provider without a running consumer");
    scanner.set(running_cli(42, 1));
    let before = application.inspect().expect("inspect running consumer");
    assert!(before.requires_consumer_confirmation);

    let failure = application
        .apply_provider_at_revision(SECOND_PROVIDER_ID, false, &before.revision)
        .expect_err("running consumer must require confirmation");
    assert_eq!(
        failure.category,
        EnvironmentFailureCategory::ConsumerConfirmationRequired
    );
    assert_eq!(
        application
            .inspect()
            .expect("inspect unchanged provider")
            .current_provider
            .expect("current provider")
            .id,
        PROVIDER_ID
    );

    let switched = application
        .apply_provider_at_revision(SECOND_PROVIDER_ID, true, &before.revision)
        .expect("confirmed switch");
    assert!(switched.pending_restart);
}

#[test]
fn trustworthy_detection_without_consumers_switches_without_extra_confirmation_or_pending_restart()
{
    const SECOND_PROVIDER_ID: &str = "6cde0dd7-9725-462a-ac79-864f5cf63f76";
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    insert_second_provider(&store, SECOND_PROVIDER_ID);
    let application = EnvironmentApplication::with_runtime_probes(
        store,
        temp.path().join(".codex"),
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
        Arc::new(StoppedConsumers),
    );

    application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply initial provider");
    let before = application.inspect().expect("inspect managed environment");
    assert!(!before.requires_consumer_confirmation);

    let switched = application
        .apply_provider_at_revision(SECOND_PROVIDER_ID, false, &before.revision)
        .expect("switch without redundant confirmation");
    assert!(!switched.pending_restart);
}

#[test]
fn pending_restart_clears_after_old_identity_exits_and_ignores_pid_reuse() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    let scanner = Arc::new(MutableConsumers::new(running_cli(42, 1)));
    let application = EnvironmentApplication::with_runtime_probes(
        store,
        temp.path().join(".codex"),
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
        scanner.clone(),
    );

    let applied = application
        .apply_provider(PROVIDER_ID, true)
        .expect("apply provider with a running consumer");
    assert!(applied.pending_restart);

    scanner.set(running_cli(42, u64::MAX));
    let reconciled = application.inspect().expect("reconcile reused PID");
    assert!(!reconciled.pending_restart);
    assert_eq!(reconciled.consumers.cli, ConsumerStatus::Running);
}

#[test]
fn consumer_started_during_artifact_write_blocks_pending_restart_clear() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    let scanner = Arc::new(MutableConsumers::new(stopped_consumers()));
    let application = EnvironmentApplication::with_runtime_dependencies(
        store,
        temp.path().join(".codex"),
        Arc::new(StartsConsumerDuringWrite {
            scanner: scanner.clone(),
        }),
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
        scanner.clone(),
    );

    assert!(
        application
            .apply_provider(PROVIDER_ID, true)
            .expect("apply provider while consumer starts")
            .pending_restart
    );
    assert!(
        application
            .inspect()
            .expect("consumer started before write completed remains old")
            .pending_restart
    );

    scanner.set(stopped_consumers());
    assert!(
        !application
            .inspect()
            .expect("pending clears after the consumer exits")
            .pending_restart
    );
}

#[test]
fn bundled_desktop_child_keeps_pending_restart_after_its_root_exits() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    let scanner = Arc::new(MutableConsumers::new(running_desktop(&[
        (100, 1),
        (101, 2),
    ])));
    let application = EnvironmentApplication::with_runtime_probes(
        store,
        temp.path().join(".codex"),
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
        scanner.clone(),
    );

    assert!(
        application
            .apply_provider(PROVIDER_ID, true)
            .expect("apply provider with desktop process tree")
            .pending_restart
    );
    scanner.set(running_desktop(&[(101, 2)]));

    assert!(
        application
            .inspect()
            .expect("inspect remaining bundled child")
            .pending_restart
    );
}

#[test]
fn missing_or_corrupt_restart_context_keeps_pending_restart() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    let scanner = Arc::new(MutableConsumers::new(running_cli(42, 1)));
    let application = EnvironmentApplication::with_runtime_probes(
        store.clone(),
        temp.path().join(".codex"),
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
        scanner,
    );
    assert!(
        application
            .apply_provider(PROVIDER_ID, true)
            .expect("apply provider")
            .pending_restart
    );
    let connection = Connection::open(store.paths().database()).expect("open state database");

    connection
        .execute(
            "UPDATE app_state SET pending_restart_context = NULL WHERE singleton = 1",
            [],
        )
        .expect("remove restart context");
    assert!(
        application
            .inspect()
            .expect("missing context fails closed")
            .pending_restart
    );

    connection
        .execute(
            "UPDATE app_state SET pending_restart_context = '{not-json' WHERE singleton = 1",
            [],
        )
        .expect("corrupt restart context");
    assert!(
        application
            .inspect()
            .expect("corrupt context fails closed")
            .pending_restart
    );
}

#[test]
fn unknown_detection_stays_pending_until_a_trustworthy_scan() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_provider(&store);
    let scanner = Arc::new(MutableConsumers::new(ConsumerScan::unknown()));
    let application = EnvironmentApplication::with_runtime_probes(
        store,
        temp.path().join(".codex"),
        Arc::new(FixedLoginProbe(LoginStatus::NotLoggedIn)),
        scanner.clone(),
    );

    assert!(
        application
            .apply_provider(PROVIDER_ID, true)
            .expect("confirmed switch after unknown scan")
            .pending_restart
    );
    assert!(
        application
            .inspect()
            .expect("unknown scan remains conservative")
            .pending_restart
    );

    scanner.set(stopped_consumers());
    assert!(
        !application
            .inspect()
            .expect("trustworthy stopped scan clears pending restart")
            .pending_restart
    );
}
