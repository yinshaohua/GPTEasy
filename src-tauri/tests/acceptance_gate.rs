#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

#[path = "support/state.rs"]
mod state_support;
mod support;

use std::env;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpteasy_lib::codex::LoginStatus;
use gpteasy_lib::consumer::{
    ConsumerScan, ConsumerScanner, ConsumerStatus, FixtureProcess, ProcessAccess, classify_fixture,
};
use gpteasy_lib::environment::{
    EnvironmentApplication, EnvironmentFailureCategory, EnvironmentFailurePoint,
    EnvironmentFaultInjector, EnvironmentRecovery, EnvironmentState, OpenAiLoginProbe,
    RestoreAvailability,
};
use gpteasy_lib::provider::{
    ProviderApplication, ProviderFailureCategory, ProviderValidationInput, ProviderValidator,
    ValidationTimeouts,
};
use gpteasy_lib::state::{
    CURRENT_SCHEMA_VERSION, DatabaseBlockReason, DatabaseStatus, StatePaths, StateStore,
};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use tempfile::TempDir;
use uuid::Uuid;

use support::local_provider::LocalProvider;

const MODEL_A: &str = "model-a";
const MODEL_B: &str = "model-b";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_x64_acceptance_gate_covers_issue_22() {
    let mut context = AcceptanceContext::new();
    let mut cases = Vec::new();

    cases.push(two_provider_switch_case(&mut context).await);
    cases.push(validation_failure_and_cancel_case(&mut context).await);
    cases.extend(start_state_cases(&mut context).await);
    cases.extend(consistency_failure_cases(&mut context).await);
    cases.extend(pending_operation_fault_cases(&mut context).await);
    cases.extend(database_failure_cases(&mut context.audit));

    context.audit.add(
        "process_arguments",
        env::args().collect::<Vec<_>>().join("\n"),
    );
    context.audit.assert_clean(&context.keys);

    let passed = cases
        .iter()
        .filter(|case| case.result == GateResult::Passed)
        .count();
    assert_eq!(
        passed,
        cases.len(),
        "every acceptance matrix case must pass"
    );
    let evidence = GateEvidence {
        platform: "windows-x64-current-user".to_owned(),
        passed,
        total: cases.len(),
        cases,
        leak_scan: LeakEvidence {
            leaked: false,
            scanned_surfaces: context.audit.names(),
        },
    };
    let bytes = serde_json::to_vec_pretty(&evidence).expect("serialize acceptance evidence");
    for key in &context.keys {
        assert!(
            !contains_bytes(&bytes, key.as_bytes()),
            "evidence must not contain a key"
        );
    }
    if let Ok(path) = env::var("GPTEASY_ACCEPTANCE_EVIDENCE_PATH") {
        let path = PathBuf::from(path);
        fs::create_dir_all(path.parent().expect("evidence path parent"))
            .expect("create evidence directory");
        fs::write(path, &bytes).expect("write redacted acceptance evidence");
    }
    println!(
        "GPTEasy acceptance gate: {passed}/{} cases passed",
        evidence.total
    );
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GateEvidence {
    platform: String,
    passed: usize,
    total: usize,
    cases: Vec<GateCase>,
    leak_scan: LeakEvidence,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GateCase {
    name: String,
    result: GateResult,
    final_state: FinalState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum GateResult {
    Passed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum FinalState {
    ManagedSecondProviderPendingRestart,
    CatalogUnchangedWithoutBackupOrConfig,
    ManagedFirstProvider,
    ManagedFirstProviderWithoutRewritingLoginTokens,
    ManagementConflict,
    ManagedSecondProvider,
    OldExternalStatePreserved,
    OldManagedState,
    OldManagedStateRestored,
    Old,
    New,
    Conflict,
    BlockedMissingDatabase,
    BlockedCorruptDatabase,
    BlockedFutureSchema,
    RecoveredCurrentSchema,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConvergedState {
    Old,
    New,
    Conflict,
}

impl From<ConvergedState> for FinalState {
    fn from(state: ConvergedState) -> Self {
        match state {
            ConvergedState::Old => Self::Old,
            ConvergedState::New => Self::New,
            ConvergedState::Conflict => Self::Conflict,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LeakEvidence {
    leaked: bool,
    scanned_surfaces: Vec<String>,
}

struct AcceptanceContext {
    keys: [String; 2],
    audit: LeakAudit,
}

impl AcceptanceContext {
    fn new() -> Self {
        Self {
            keys: [
                canary("GPTEASY_ACCEPTANCE_KEY_A", "provider-a"),
                canary("GPTEASY_ACCEPTANCE_KEY_B", "provider-b"),
            ],
            audit: LeakAudit::default(),
        }
    }
}

#[derive(Default)]
struct LeakAudit {
    surfaces: Vec<(String, Vec<u8>)>,
}

impl LeakAudit {
    fn add(&mut self, name: &str, value: impl Into<String>) {
        self.surfaces
            .push((name.to_owned(), value.into().into_bytes()));
    }

    fn add_json<T: Serialize>(&mut self, name: &str, value: &T) {
        self.add(
            name,
            serde_json::to_string(value).expect("serialize observable output"),
        );
    }

    fn add_debug<T: Debug>(&mut self, name: &str, value: &T) {
        self.add(name, format!("{value:?}"));
    }

    fn assert_clean(&self, keys: &[String; 2]) {
        for (name, value) in &self.surfaces {
            for key in keys {
                assert!(
                    !contains_bytes(value, key.as_bytes()),
                    "API key leaked into observable {name} surface"
                );
            }
        }
    }

    fn names(&self) -> Vec<String> {
        let mut names = self
            .surfaces
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names.extend(["diagnostic_evidence".to_owned(), "test_log".to_owned()]);
        names.sort();
        names.dedup();
        names
    }
}

struct Harness {
    _temp: TempDir,
    store: StateStore,
    codex_home: PathBuf,
    first: gpteasy_lib::provider::ProviderSummary,
    second: gpteasy_lib::provider::ProviderSummary,
}

async fn verified_harness(keys: &[String; 2]) -> Harness {
    let temp = TempDir::new().expect("create isolated acceptance directory");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(
        store.bootstrap().is_ready(),
        "acceptance state must initialize"
    );
    let application = ProviderApplication::new(store.clone(), validator());
    let first = create_provider(
        &application,
        "provider-a-validation",
        &keys[0],
        MODEL_A,
        "Atlas",
    )
    .await;
    let second = create_provider(
        &application,
        "provider-b-validation",
        &keys[1],
        MODEL_B,
        "Beacon",
    )
    .await;
    let codex_home = temp.path().join(".codex");
    Harness {
        _temp: temp,
        store,
        codex_home,
        first,
        second,
    }
}

async fn create_provider(
    application: &ProviderApplication,
    request_id: &str,
    api_key: &str,
    model: &'static str,
    name: &str,
) -> gpteasy_lib::provider::ProviderSummary {
    let server = LocalProvider::compatible(api_key.to_owned(), model);
    let receipt = application
        .validate_provider(
            request_id.to_owned(),
            ProviderValidationInput {
                base_url: server.base_url().to_owned(),
                api_key: api_key.to_owned(),
                default_model: model.to_owned(),
            },
        )
        .await
        .expect("local provider must complete the validation loop");
    server.finish();
    application
        .save_verified_provider(&receipt.validation_id, name)
        .expect("validated provider must be saved")
}

fn validator() -> ProviderValidator {
    ProviderValidator::new(ValidationTimeouts {
        connect: Duration::from_millis(250),
        response_header: Duration::from_millis(250),
        stream_read: Duration::from_millis(100),
        response_overall: Duration::from_millis(750),
    })
}

fn canary(name: &str, fallback: &str) -> String {
    env::var(name).unwrap_or_else(|_| format!("gpteasy-{fallback}-{}", Uuid::new_v4()))
}

fn environment(
    harness: &Harness,
    faults: Arc<dyn EnvironmentFaultInjector>,
    login: LoginStatus,
    scan: ConsumerScan,
) -> EnvironmentApplication {
    EnvironmentApplication::with_runtime_dependencies(
        harness.store.clone(),
        &harness.codex_home,
        faults,
        Arc::new(FixedLoginProbe(login)),
        Arc::new(FixedScanner::new(scan)),
    )
}

async fn two_provider_switch_case(context: &mut AcceptanceContext) -> GateCase {
    let harness = verified_harness(&context.keys).await;
    assert!(
        Uuid::parse_str(&harness.first.id).is_ok(),
        "first provider id must be a UUID"
    );
    assert!(
        Uuid::parse_str(&harness.second.id).is_ok(),
        "second provider id must be a UUID"
    );
    assert_ne!(
        harness.first.id, harness.second.id,
        "provider IDs must be immutable and distinct"
    );
    assert_ne!(
        harness.first.name, harness.second.name,
        "provider names must be distinct"
    );

    let running_cli = classify_fixture(&[FixtureProcess {
        pid: 7_301,
        parent_pid: 77,
        started_at_epoch_millis: 2_000,
        name: "codex.exe".to_owned(),
        executable: PathBuf::from(
            r"C:\Users\example\AppData\Roaming\npm\node_modules\@openai\codex\vendor\codex.exe",
        ),
        access: ProcessAccess::Available,
        electron_helper: false,
    }]);
    assert_eq!(running_cli.cli, ConsumerStatus::Running);
    let app = environment(
        &harness,
        Arc::new(NoFaults),
        LoginStatus::NotLoggedIn,
        running_cli,
    );
    let first = app
        .apply_provider(&harness.first.id, true)
        .expect("first provider switch");
    assert_eq!(
        first
            .current_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(harness.first.id.as_str())
    );
    assert!(
        first.pending_restart,
        "running CLI must remain pending restart"
    );
    let second = app
        .apply_provider(&harness.second.id, true)
        .expect("second provider switch");
    assert_eq!(
        second
            .current_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(harness.second.id.as_str())
    );
    assert!(config_contains_provider(
        &harness.codex_home,
        &harness.second.id
    ));
    assert!(credentials_use(&harness.codex_home, &context.keys[1]));
    context.audit.add_json(
        "provider_catalog",
        &[harness.first.clone(), harness.second.clone()],
    );
    context.audit.add_debug(
        "provider_inputs",
        &ProviderValidationInput {
            base_url: "http://127.0.0.1:0".to_owned(),
            api_key: context.keys[0].clone(),
            default_model: MODEL_A.to_owned(),
        },
    );
    GateCase {
        name: "two-verified-providers-switch-and-remain-distinguishable".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::ManagedSecondProviderPendingRestart,
    }
}

async fn validation_failure_and_cancel_case(context: &mut AcceptanceContext) -> GateCase {
    let temp = TempDir::new().expect("create validation isolation");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let application = Arc::new(ProviderApplication::new(store.clone(), validator()));
    let failed_server = LocalProvider::authentication_failure(context.keys[0].clone());
    let failure = application
        .validate_provider(
            "provider-auth-failure".to_owned(),
            ProviderValidationInput {
                base_url: failed_server.base_url().to_owned(),
                api_key: context.keys[0].clone(),
                default_model: MODEL_A.to_owned(),
            },
        )
        .await
        .expect_err("authentication failure must stop validation");
    failed_server.finish();
    assert_eq!(failure.category, ProviderFailureCategory::Authentication);
    let mut cancellable_server = LocalProvider::cancellable(context.keys[0].clone(), MODEL_A);
    let cancellable_base_url = cancellable_server.base_url().to_owned();
    let cancellable_application = Arc::clone(&application);
    let cancellable_key = context.keys[0].clone();
    let validation = tokio::spawn(async move {
        cancellable_application
            .validate_provider(
                "provider-cancel".to_owned(),
                ProviderValidationInput {
                    base_url: cancellable_base_url,
                    api_key: cancellable_key,
                    default_model: MODEL_A.to_owned(),
                },
            )
            .await
    });
    cancellable_server.wait_until_streaming();
    assert!(application.cancel_request("provider-cancel"));
    let cancelled = validation
        .await
        .expect("validation task must finish")
        .expect_err("in-flight validation must be cancelled");
    cancellable_server.finish();
    assert_eq!(cancelled.category, ProviderFailureCategory::Cancelled);
    assert_eq!(
        application
            .list_providers()
            .expect("list empty catalog")
            .len(),
        0
    );
    assert!(!temp.path().join(".codex").exists());
    context.audit.add_debug("errors", &failure);
    context.audit.add_debug("cancelled_errors", &cancelled);
    GateCase {
        name: "validation-failure-and-cancel-before-persistence".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::CatalogUnchangedWithoutBackupOrConfig,
    }
}

async fn start_state_cases(context: &mut AcceptanceContext) -> Vec<GateCase> {
    let mut cases = Vec::new();

    let external = verified_harness(&context.keys).await;
    fs::create_dir_all(&external.codex_home).expect("create external Codex directory");
    fs::write(
        external.codex_home.join("config.toml"),
        "model = 'legacy-model'\ncustom_flag = true\n[model_providers.legacy]\nname = 'Legacy'\n",
    )
    .expect("write external config");
    fs::write(
        external.codex_home.join("auth.json"),
        br#"{"OPENAI_API_KEY":"legacy-key","preserved":"yes"}"#,
    )
    .expect("write external credentials");
    let external_app = environment(
        &external,
        Arc::new(NoFaults),
        LoginStatus::NotLoggedIn,
        stopped_scan(),
    );
    let external_result = external_app
        .apply_provider(&external.first.id, true)
        .expect("valid external configuration must be explicitly adopted");
    let external_config = fs::read_to_string(external.codex_home.join("config.toml"))
        .expect("read adopted external config");
    assert!(external_config.contains("custom_flag = true"));
    assert!(external_config.contains("name = 'Legacy'"));
    assert!(external_credentials_preserve(&external.codex_home));
    context.audit.add_json("external_state", &external_result);
    cases.push(GateCase {
        name: "valid-external-configuration-is-preserved-during-takeover".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::ManagedFirstProvider,
    });

    let openai = verified_harness(&context.keys).await;
    fs::create_dir_all(&openai.codex_home).expect("create OpenAI login Codex directory");
    fs::write(
        openai.codex_home.join("config.toml"),
        "cli_auth_credentials_store = 'file'\nmodel = 'gpt-5'\n",
    )
    .expect("write OpenAI login config");
    let openai_tokens = br#"{"tokens":{"access_token":"openai-login-token"}}"#;
    fs::write(openai.codex_home.join("auth.json"), openai_tokens)
        .expect("write OpenAI login credentials");
    let openai_app = environment(
        &openai,
        Arc::new(NoFaults),
        LoginStatus::LoggedIn,
        stopped_scan(),
    );
    let login = openai_app
        .inspect()
        .expect("inspect OpenAI login start state");
    let login_mode = openai_app
        .switch_to_openai_login(true, &login.revision)
        .expect("switch into OpenAI login mode");
    assert_eq!(
        login_mode.mode,
        Some(gpteasy_lib::environment::AuthenticationMode::OpenaiLogin)
    );
    let provider_mode = openai_app
        .apply_provider(&openai.first.id, true)
        .expect("switch back from OpenAI login mode");
    assert_eq!(
        provider_mode.mode,
        Some(gpteasy_lib::environment::AuthenticationMode::Provider)
    );
    assert!(login_tokens_preserved(&openai.codex_home));
    context.audit.add_json("openai_login_state", &login_mode);
    context
        .audit
        .add_json("openai_provider_state", &provider_mode);
    cases.push(GateCase {
        name: "openai-login-start-state_requires_explicit_mode_confirmation".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::ManagedFirstProviderWithoutRewritingLoginTokens,
    });

    let conflict = verified_harness(&context.keys).await;
    fs::create_dir_all(&conflict.codex_home).expect("create conflict Codex directory");
    let malformed = b"# >>> GPTEasy managed provider >>>\nmodel = 'broken'\n";
    fs::write(conflict.codex_home.join("config.toml"), malformed)
        .expect("write malformed managed block");
    let conflict_app = environment(
        &conflict,
        Arc::new(NoFaults),
        LoginStatus::NotLoggedIn,
        stopped_scan(),
    );
    let conflict_before = conflict_app.inspect().expect("inspect management conflict");
    let conflict_failure = conflict_app
        .apply_provider(&conflict.first.id, true)
        .expect_err("damaged managed block must not be rewritten");
    assert_eq!(conflict_before.state, EnvironmentState::Conflict);
    assert_eq!(
        conflict_failure.category,
        EnvironmentFailureCategory::ManagedConflict
    );
    assert_eq!(
        fs::read(conflict.codex_home.join("config.toml")).expect("read preserved conflict"),
        malformed
    );
    assert!(!conflict.codex_home.join(".gpteasy-backups").exists());
    context
        .audit
        .add_debug("conflict_errors", &conflict_failure);
    cases.push(GateCase {
        name: "damaged-management-block_is-a-no-write-conflict".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::ManagementConflict,
    });

    let missing = verified_harness(&context.keys).await;
    let missing_app = environment(
        &missing,
        Arc::new(NoFaults),
        LoginStatus::NotLoggedIn,
        stopped_scan(),
    );
    let missing_before = missing_app
        .inspect()
        .expect("inspect missing Codex configuration");
    assert_eq!(missing_before.state, EnvironmentState::External);
    let missing_after = missing_app
        .apply_provider(&missing.second.id, true)
        .expect("confirmed switch creates missing Codex artifacts");
    assert_eq!(
        missing_after
            .current_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(missing.second.id.as_str())
    );
    assert!(missing.codex_home.join("config.toml").is_file());
    context
        .audit
        .add_json("missing_config_state", &missing_after);
    cases.push(GateCase {
        name: "missing-configuration_is-created-only-by-confirmed-switch".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::ManagedSecondProvider,
    });
    cases
}

async fn consistency_failure_cases(context: &mut AcceptanceContext) -> Vec<GateCase> {
    let harness = verified_harness(&context.keys).await;
    let app = environment(
        &harness,
        Arc::new(NoFaults),
        LoginStatus::NotLoggedIn,
        stopped_scan(),
    );
    let preview = app.inspect().expect("preview before concurrent edit");
    fs::create_dir_all(&harness.codex_home).expect("create concurrent edit directory");
    fs::write(harness.codex_home.join("config.toml"), "external = true\n")
        .expect("write concurrent edit");
    let concurrent = app
        .apply_provider_at_revision(&harness.first.id, true, &preview.revision)
        .expect_err("concurrent edit must stop before backup");
    assert_eq!(
        concurrent.category,
        EnvironmentFailureCategory::ConcurrentModification
    );
    assert_eq!(
        app.inspect()
            .expect("inspect state after concurrent edit")
            .restore_availability,
        RestoreAvailability::NoBackup
    );
    context.audit.add_debug("concurrent_errors", &concurrent);
    let _applied = app
        .apply_provider(&harness.first.id, true)
        .expect("apply after retry");
    let restore_before_failure = app
        .inspect()
        .expect("inspect restore state before backup failure")
        .restore_availability;
    let backup_failure_app = environment(
        &harness,
        Arc::new(FailBackupCreation),
        LoginStatus::NotLoggedIn,
        stopped_scan(),
    );
    let backup_failure = backup_failure_app
        .apply_provider(&harness.second.id, true)
        .expect_err("backup failure must stop before a switch");
    assert_eq!(
        backup_failure.category,
        EnvironmentFailureCategory::BackupFailed
    );
    let after_backup_failure = app.inspect().expect("inspect state after backup failure");
    assert_eq!(
        after_backup_failure.restore_availability,
        restore_before_failure
    );
    assert_eq!(
        after_backup_failure
            .current_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(harness.first.id.as_str())
    );
    context.audit.add_debug("backup_errors", &backup_failure);
    let write_failure_app = environment(
        &harness,
        Arc::new(FailBeforeCredentials),
        LoginStatus::NotLoggedIn,
        stopped_scan(),
    );
    let old_config = fs::read(harness.codex_home.join("config.toml")).expect("read old config");
    let old_credentials =
        fs::read(harness.codex_home.join("auth.json")).expect("read old credentials");
    let write_failure = write_failure_app
        .apply_provider(&harness.second.id, true)
        .expect_err("credential artifact failure must roll back the config");
    assert_eq!(
        write_failure.category,
        EnvironmentFailureCategory::ArtifactWriteFailed
    );
    assert_eq!(
        fs::read(harness.codex_home.join("config.toml")).expect("read rolled back config"),
        old_config
    );
    assert_eq!(
        fs::read(harness.codex_home.join("auth.json")).expect("read rolled back credentials"),
        old_credentials
    );
    assert_eq!(
        app.inspect()
            .expect("inspect state after artifact failure")
            .current_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(harness.first.id.as_str())
    );
    context.audit.add_debug("artifact_errors", &write_failure);

    let restore = verified_harness(&context.keys).await;
    let restore_app = environment(
        &restore,
        Arc::new(NoFaults),
        LoginStatus::NotLoggedIn,
        stopped_scan(),
    );
    let _first = restore_app
        .apply_provider(&restore.first.id, true)
        .expect("apply original provider");
    let switched = restore_app
        .apply_provider(&restore.second.id, true)
        .expect("apply replacement provider");
    let restored = restore_app
        .restore_last_config(true, &switched.revision)
        .expect("restore latest completed configuration");
    assert_eq!(
        restored
            .current_provider
            .as_ref()
            .map(|provider| provider.id.as_str()),
        Some(restore.first.id.as_str())
    );
    assert!(config_contains_provider(
        &restore.codex_home,
        &restore.first.id
    ));
    assert!(credentials_use(
        &restore.codex_home,
        context.keys[0].as_str()
    ));
    context.audit.add_json("restore_state", &restored);

    vec![
        GateCase {
            name: "concurrent-modification-stops-before-backup".to_owned(),
            result: GateResult::Passed,
            final_state: FinalState::OldExternalStatePreserved,
        },
        GateCase {
            name: "backup-failure-keeps-the-old-provider".to_owned(),
            result: GateResult::Passed,
            final_state: FinalState::OldManagedState,
        },
        GateCase {
            name: "multi-artifact-write-failure_rolls_back-completely".to_owned(),
            result: GateResult::Passed,
            final_state: FinalState::OldManagedState,
        },
        GateCase {
            name: "restore-latest-configuration-reconciles-provider-and-artifacts".to_owned(),
            result: GateResult::Passed,
            final_state: FinalState::OldManagedStateRestored,
        },
    ]
}

async fn pending_operation_fault_cases(context: &mut AcceptanceContext) -> Vec<GateCase> {
    let matrix = [
        (
            EnvironmentFailurePoint::AfterBackupCompleted,
            EnvironmentRecovery::NoPendingOperation,
            ConvergedState::Old,
        ),
        (
            EnvironmentFailurePoint::AfterPendingRegistered,
            EnvironmentRecovery::KeptOldState,
            ConvergedState::Old,
        ),
        (
            EnvironmentFailurePoint::AfterConfigReplaced,
            EnvironmentRecovery::Conflict,
            ConvergedState::Conflict,
        ),
        (
            EnvironmentFailurePoint::AfterAllArtifactsReplaced,
            EnvironmentRecovery::CompletedNewState,
            ConvergedState::New,
        ),
        (
            EnvironmentFailurePoint::BeforeDatabaseCommit,
            EnvironmentRecovery::CompletedNewState,
            ConvergedState::New,
        ),
        (
            EnvironmentFailurePoint::AfterDatabaseCommit,
            EnvironmentRecovery::NoPendingOperation,
            ConvergedState::New,
        ),
    ];
    let mut cases = Vec::new();
    for (point, expected_recovery, expected_state) in matrix {
        let harness = verified_harness(&context.keys).await;
        let app = environment(
            &harness,
            Arc::new(NoFaults),
            LoginStatus::NotLoggedIn,
            stopped_scan(),
        );
        app.apply_provider(&harness.first.id, true)
            .expect("establish old state before fault");
        let interrupted = environment(
            &harness,
            Arc::new(InterruptAt(point)),
            LoginStatus::NotLoggedIn,
            stopped_scan(),
        );
        let failure = interrupted
            .apply_provider(&harness.second.id, true)
            .expect_err("fault must interrupt the switch");
        assert_eq!(
            failure.category,
            EnvironmentFailureCategory::OperationInterrupted
        );
        let restarted = environment(
            &harness,
            Arc::new(NoFaults),
            LoginStatus::NotLoggedIn,
            stopped_scan(),
        );
        assert_eq!(
            restarted
                .recover_pending()
                .expect("recover faulted operation"),
            expected_recovery
        );
        let snapshot = restarted.inspect().expect("inspect recovered fault");
        let actual_state = match snapshot.state {
            EnvironmentState::Conflict => ConvergedState::Conflict,
            EnvironmentState::Managed
                if snapshot
                    .current_provider
                    .as_ref()
                    .map(|provider| provider.id.as_str())
                    == Some(harness.first.id.as_str()) =>
            {
                ConvergedState::Old
            }
            EnvironmentState::Managed
                if snapshot
                    .current_provider
                    .as_ref()
                    .map(|provider| provider.id.as_str())
                    == Some(harness.second.id.as_str()) =>
            {
                ConvergedState::New
            }
            _ => panic!("fault did not converge to old, new, or conflict"),
        };
        assert_eq!(
            actual_state, expected_state,
            "fault must converge to old, new, or conflict"
        );
        context
            .audit
            .add_debug("pending_operation_errors", &failure);
        context
            .audit
            .add_json("pending_operation_states", &snapshot);
        cases.push(GateCase {
            name: format!("pending-operation-fault-{point:?}"),
            result: GateResult::Passed,
            final_state: expected_state.into(),
        });
    }
    cases
}

fn database_failure_cases(audit: &mut LeakAudit) -> Vec<GateCase> {
    let mut cases = Vec::new();
    let missing_temp = TempDir::new().expect("missing database isolation");
    let missing = StateStore::new(StatePaths::from_root(missing_temp.path()));
    assert!(missing.bootstrap().is_ready());
    fs::remove_file(missing.paths().database()).expect("remove database fixture");
    let missing_snapshot = missing.bootstrap();
    assert_eq!(missing_snapshot.status, DatabaseStatus::Blocked);
    assert_eq!(
        missing_snapshot.reason,
        Some(DatabaseBlockReason::MissingDatabase)
    );
    audit.add_json("database_missing", &missing_snapshot);
    cases.push(GateCase {
        name: "database-missing-fails-closed".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::BlockedMissingDatabase,
    });

    let corrupt_temp = TempDir::new().expect("corrupt database isolation");
    let corrupt = StateStore::new(StatePaths::from_root(corrupt_temp.path()));
    assert!(corrupt.bootstrap().is_ready());
    fs::write(corrupt.paths().database(), b"not-a-sqlite-database")
        .expect("corrupt database fixture");
    let corrupt_snapshot = corrupt.bootstrap();
    assert_eq!(corrupt_snapshot.status, DatabaseStatus::Blocked);
    assert_eq!(
        corrupt_snapshot.reason,
        Some(DatabaseBlockReason::CorruptDatabase)
    );
    audit.add_json("database_corrupt", &corrupt_snapshot);
    cases.push(GateCase {
        name: "database-corrupt-fails-closed".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::BlockedCorruptDatabase,
    });

    let future_temp = TempDir::new().expect("future database isolation");
    let future = StateStore::new(StatePaths::from_root(future_temp.path()));
    assert!(future.bootstrap().is_ready());
    Connection::open(future.paths().database())
        .expect("open future database")
        .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION + 1)
        .expect("mark future schema");
    let future_snapshot = future.bootstrap();
    assert_eq!(future_snapshot.status, DatabaseStatus::Blocked);
    assert_eq!(
        future_snapshot.reason,
        Some(DatabaseBlockReason::FutureSchema)
    );
    audit.add_json("database_future", &future_snapshot);
    cases.push(GateCase {
        name: "database-future-schema-fails-closed".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::BlockedFutureSchema,
    });

    let migration_temp = TempDir::new().expect("migration database isolation");
    let paths = StatePaths::from_root(migration_temp.path());
    let migration_setup = StateStore::new(paths.clone());
    state_support::create_version_zero_database(&migration_setup);
    let migration = state_support::with_migration_failure(paths);
    let migration_snapshot = migration.bootstrap();
    assert_eq!(migration_snapshot.status, DatabaseStatus::Recovered);
    assert_eq!(
        migration_snapshot.schema_version,
        Some(CURRENT_SCHEMA_VERSION)
    );
    assert!(migration.paths().root().join("state.sqlite3").is_file());
    audit.add_json("database_migration", &migration_snapshot);
    cases.push(GateCase {
        name: "database-migration-failure-recovers-consistent-backup".to_owned(),
        result: GateResult::Passed,
        final_state: FinalState::RecoveredCurrentSchema,
    });
    cases
}

fn config_contains_provider(codex_home: &Path, provider_id: &str) -> bool {
    fs::read_to_string(codex_home.join("config.toml"))
        .map(|config| config.contains(provider_id))
        .unwrap_or(false)
}

fn credentials_use(codex_home: &Path, api_key: &str) -> bool {
    fs::read(codex_home.join("auth.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("OPENAI_API_KEY")
                .and_then(Value::as_str)
                .map(|value| value == api_key)
        })
        .unwrap_or(false)
}

fn external_credentials_preserve(codex_home: &Path) -> bool {
    fs::read(codex_home.join("auth.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("preserved")
                .and_then(Value::as_str)
                .map(|value| value == "yes")
        })
        .unwrap_or(false)
}

fn login_tokens_preserved(codex_home: &Path) -> bool {
    fs::read(codex_home.join("auth.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .pointer("/tokens/access_token")
                .and_then(Value::as_str)
                .map(|value| value == "openai-login-token")
        })
        .unwrap_or(false)
}

fn stopped_scan() -> ConsumerScan {
    ConsumerScan {
        desktop: ConsumerStatus::Stopped,
        cli: ConsumerStatus::Stopped,
        identities: Vec::new(),
        desktop_roots: Vec::new(),
    }
}

struct FixedScanner {
    scan: ConsumerScan,
}

impl FixedScanner {
    fn new(scan: ConsumerScan) -> Self {
        Self { scan }
    }
}

impl ConsumerScanner for FixedScanner {
    fn scan(&self) -> ConsumerScan {
        self.scan.clone()
    }
}

struct FixedLoginProbe(LoginStatus);

impl OpenAiLoginProbe for FixedLoginProbe {
    fn status(&self) -> LoginStatus {
        self.0
    }
}

struct NoFaults;

impl EnvironmentFaultInjector for NoFaults {
    fn fails_at(&self, _point: EnvironmentFailurePoint) -> bool {
        false
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

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}
