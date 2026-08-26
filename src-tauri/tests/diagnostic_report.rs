use std::sync::Arc;

use gpteasy_lib::codex::{LoginStatus, LoginStatusCommand};
use gpteasy_lib::consumer::{ConsumerScan, ConsumerScanner, ConsumerStatus};
use gpteasy_lib::diagnostic_report::{
    AuthFileStatus, CodexHomeOverrideStatus, CredentialStore, DiagnosticApplication,
    DiagnosticConfigStatus, DiagnosticObservations, DiagnosticOrigin, DiagnosticRepairSource,
    DiagnosticRepairStatus, DiagnosticScope,
};
use gpteasy_lib::diagnostics::{IssueLogLevel, IssueLogRecord};
use gpteasy_lib::environment::{
    EnvironmentApplication, EnvironmentFailurePoint, EnvironmentFaultInjector, EnvironmentRecovery,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

const BACKUP_SOURCE_PROVIDER_ID: &str = "9f319739-f219-48ee-be35-22e08d5402d7";
const CURRENT_SOURCE_PROVIDER_ID: &str = "f4782322-ad03-4a2e-95b0-a4af3bcff403";

fn stopped_observations() -> DiagnosticObservations {
    DiagnosticObservations {
        login_status: LoginStatus::LoggedIn,
        desktop_status: ConsumerStatus::Stopped,
        cli_status: ConsumerStatus::Stopped,
        codex_cli_version: Some("0.147.0".to_owned()),
    }
}

fn insert_verified_provider(
    store: &StateStore,
    provider_id: &str,
    name: &str,
    base_url: &str,
    api_key: &str,
    model: &str,
    current: bool,
) {
    let connection = Connection::open(store.paths().database()).expect("open state database");
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-provider-combination-v1\0");
    hasher.update(base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(model.as_bytes());
    hasher.update(b"\0");
    hasher.update(api_key.as_bytes());
    let fingerprint = format!("{:x}", hasher.finalize());
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                provider_id,
                name,
                base_url,
                api_key,
                model,
                "1787702400",
                fingerprint,
            ],
        )
        .expect("insert provider fixture");
    if current {
        connection
            .execute(
                "INSERT INTO last_applied_state (
                    singleton, mode, provider_id, config_fingerprint,
                    credentials_fingerprint, applied_at
                 ) VALUES (1, 'provider', ?1, NULL, NULL, ?2)",
                params![provider_id, "1787702400"],
            )
            .expect("mark current provider fixture");
    }
}

fn insert_backup_source_provider(store: &StateStore) {
    insert_verified_provider(
        store,
        BACKUP_SOURCE_PROVIDER_ID,
        "Current Provider",
        "https://current.example/v1",
        "local-secret",
        "gpt-5",
        false,
    );
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

struct FailAfterConfigReplace;

impl EnvironmentFaultInjector for FailAfterConfigReplace {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool {
        point == EnvironmentFailurePoint::AfterConfigReplaced
    }
}

struct InterruptAfterConfigReplace;

impl EnvironmentFaultInjector for InterruptAfterConfigReplace {
    fn fails_at(&self, _point: EnvironmentFailurePoint) -> bool {
        false
    }

    fn interrupts_at(&self, point: EnvironmentFailurePoint) -> bool {
        point == EnvironmentFailurePoint::AfterConfigReplaced
    }
}

struct ChangeCredentialsAfterConfigReplace {
    path: std::path::PathBuf,
}

impl EnvironmentFaultInjector for ChangeCredentialsAfterConfigReplace {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool {
        if point == EnvironmentFailurePoint::AfterConfigReplaced {
            std::fs::write(
                &self.path,
                r#"{"auth_mode":"apikey","OPENAI_API_KEY":"changed-secret"}"#,
            )
            .expect("change credentials concurrently");
        }
        false
    }
}

fn repair_fixture_with_faults(
    faults: Arc<dyn EnvironmentFaultInjector>,
) -> (
    tempfile::TempDir,
    EnvironmentApplication,
    DiagnosticApplication,
    String,
) {
    let directory = tempdir().expect("create repair fixture");
    let original = concat!(
        "model_provider = \"custom\"\n",
        "model = \"gpt-5\"\n",
        "[model_providers.f4782322-ad03-4a2e-95b0-a4af3bcff403]\n",
        "name = \"custom\"\n",
        "base_url = \"https://provider.example/v1\"\n",
        "wire_api = \"responses\"\n",
        "requires_openai_auth = true\n",
    )
    .to_owned();
    std::fs::write(directory.path().join("config.toml"), &original).expect("write repair config");
    std::fs::write(
        directory.path().join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-secret"}"#,
    )
    .expect("write repair credentials");
    let store = StateStore::new(StatePaths::from_root(directory.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_verified_provider(
        &store,
        CURRENT_SOURCE_PROVIDER_ID,
        "custom",
        "https://provider.example/v1",
        "local-secret",
        "gpt-5",
        true,
    );
    let environment = EnvironmentApplication::with_runtime_dependencies(
        store,
        directory.path(),
        faults,
        Arc::new(LoginStatusCommand::codex_default()),
        Arc::new(StoppedConsumers),
    );
    let application =
        DiagnosticApplication::with_environment(directory.path(), None, environment.clone());
    (directory, environment, application, original)
}

#[test]
fn repairs_dangling_custom_from_one_compatible_current_definition_after_preview() {
    let directory = tempdir().expect("create temporary Codex home");
    let original = concat!(
        "model_provider = \"custom\"\n",
        "model = \"gpt-5\"\n",
        "[model_providers.f4782322-ad03-4a2e-95b0-a4af3bcff403]\n",
        "name = \"custom\"\n",
        "base_url = \"https://provider.example/v1\"\n",
        "wire_api = \"responses\"\n",
        "requires_openai_auth = true\n",
        "supports_websockets = false\n",
    );
    std::fs::write(directory.path().join("config.toml"), original).expect("write config fixture");
    std::fs::write(
        directory.path().join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-secret"}"#,
    )
    .expect("write credential fixture");
    let store = StateStore::new(StatePaths::from_root(directory.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_verified_provider(
        &store,
        CURRENT_SOURCE_PROVIDER_ID,
        "custom",
        "https://provider.example/v1",
        "local-secret",
        "gpt-5",
        true,
    );
    let environment = EnvironmentApplication::new(store, directory.path());
    let application = DiagnosticApplication::with_environment(directory.path(), None, environment);

    let report = application.inspect_with(&stopped_observations(), &[]);
    let preview = report.repair_preview.as_ref().expect("safe repair preview");

    assert_eq!(preview.source, DiagnosticRepairSource::CurrentConfig);
    assert_eq!(preview.provider_name, "custom");
    assert_eq!(preview.base_url, "https://provider.example/v1");
    assert_eq!(preview.model, "gpt-5");
    for export in [report.redacted_json(), report.redacted_markdown()] {
        assert!(!export.contains("https://provider.example/v1"));
        assert!(!export.contains("gpt-5"));
        assert!(!export.contains("local-secret"));
    }
    assert!(report.findings.iter().any(|finding| {
        finding.code == "model_provider_missing_definition" && finding.repairable
    }));

    let result = application.repair_custom_provider(&preview.preview_id);

    assert_eq!(result.status, DiagnosticRepairStatus::Succeeded);
    let repaired = std::fs::read_to_string(directory.path().join("config.toml"))
        .expect("read repaired config");
    let document = repaired
        .parse::<toml_edit::DocumentMut>()
        .expect("repaired config remains valid TOML");
    assert_eq!(
        document["model_providers"]["custom"]["base_url"].as_str(),
        Some("https://provider.example/v1")
    );
    assert_eq!(
        document["model_providers"]["custom"]["requires_openai_auth"].as_bool(),
        Some(true)
    );
    assert!(
        application
            .inspect_with(&stopped_observations(), &[])
            .findings
            .iter()
            .all(|finding| finding.code != "model_provider_missing_definition")
    );
    assert!(directory.path().join(".gpteasy-backups").is_dir());
}

#[test]
fn refuses_a_unique_current_supplier_without_explicit_custom_identity() {
    let directory = tempdir().expect("create non-custom current source fixture");
    let original = concat!(
        "model_provider = \"custom\"\n",
        "model = \"gpt-5\"\n",
        "[model_providers.only_source]\n",
        "name = \"Current Supplier\"\n",
        "base_url = \"https://current.example/v1\"\n",
        "wire_api = \"responses\"\n",
        "requires_openai_auth = true\n",
    );
    std::fs::write(directory.path().join("config.toml"), original)
        .expect("write non-custom source config");
    std::fs::write(
        directory.path().join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-secret"}"#,
    )
    .expect("write non-custom source credentials");
    let store = StateStore::new(StatePaths::from_root(directory.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_verified_provider(
        &store,
        "only_source",
        "Current Supplier",
        "https://current.example/v1",
        "local-secret",
        "gpt-5",
        true,
    );
    let environment = EnvironmentApplication::new(store, directory.path());
    let application = DiagnosticApplication::with_environment(directory.path(), None, environment);

    let report = application.inspect_with(&stopped_observations(), &[]);

    assert!(report.repair_preview.is_none());
    assert!(report.findings.iter().any(|finding| {
        finding.code == "model_provider_missing_definition" && !finding.repairable
    }));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("config.toml"))
            .expect("read unmodified non-custom config"),
        original
    );
}

#[test]
fn codex_home_mismatch_blocks_repair_preview_and_execution() {
    let (directory, environment, application, original) =
        repair_fixture_with_faults(Arc::new(FailAfterConfigReplace));
    let preview_id = application
        .inspect_with(&stopped_observations(), &[])
        .repair_preview
        .expect("baseline repair preview")
        .preview_id;
    let mismatched = DiagnosticApplication::with_environment(
        directory.path(),
        Some(directory.path().join("other-codex-home")),
        environment,
    );

    let report = mismatched.inspect_with(&stopped_observations(), &[]);
    let result = mismatched.repair_custom_provider(&preview_id);

    assert_eq!(
        report.environment.codex_home_override_status,
        CodexHomeOverrideStatus::Differs
    );
    assert!(report.repair_preview.is_none());
    assert!(report.findings.iter().any(|finding| {
        finding.code == "model_provider_missing_definition" && !finding.repairable
    }));
    assert_eq!(result.status, DiagnosticRepairStatus::ManualRequired);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("config.toml"))
            .expect("read mismatch-blocked config"),
        original
    );
}

#[test]
fn repairs_dangling_custom_from_one_valid_gpteasy_backup() {
    let directory = tempdir().expect("create temporary Codex home");
    let codex_home = directory.path().join(".codex");
    std::fs::create_dir_all(&codex_home).expect("create Codex home");
    std::fs::write(
        codex_home.join("config.toml"),
        concat!(
            "model_provider = \"custom\"\n",
            "model = \"gpt-5\"\n",
            "[model_providers.custom]\n",
            "name = \"Historical Custom\"\n",
            "base_url = \"https://historical.example/v1\"\n",
            "wire_api = \"responses\"\n",
            "requires_openai_auth = true\n",
        ),
    )
    .expect("write historical config");
    std::fs::write(
        codex_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-secret"}"#,
    )
    .expect("write historical credentials");
    let store = StateStore::new(StatePaths::from_root(directory.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_backup_source_provider(&store);
    let environment = EnvironmentApplication::new(store, &codex_home);
    environment
        .apply_provider(BACKUP_SOURCE_PROVIDER_ID, true)
        .expect("create completed GPTEasy backup");
    std::fs::write(
        codex_home.join("config.toml"),
        "model_provider = \"custom\"\nmodel = \"gpt-5\"\n",
    )
    .expect("write dangling current config");
    let application =
        DiagnosticApplication::with_environment(&codex_home, None, environment.clone());

    let report = application.inspect_with(&stopped_observations(), &[]);
    let preview = report.repair_preview.expect("backup repair preview");

    assert_eq!(preview.source, DiagnosticRepairSource::GpteasyBackup);
    assert_eq!(preview.provider_name, "Historical Custom");
    assert_eq!(
        application
            .repair_custom_provider(&preview.preview_id)
            .status,
        DiagnosticRepairStatus::Succeeded
    );
    let repaired =
        std::fs::read_to_string(codex_home.join("config.toml")).expect("read repaired config");
    assert!(repaired.contains("https://historical.example/v1"));
}

#[test]
fn refuses_ambiguous_sources_and_missing_credentials_without_modifying_config() {
    let ambiguous = tempdir().expect("create ambiguous fixture");
    let ambiguous_home = ambiguous.path().join(".codex");
    std::fs::create_dir_all(&ambiguous_home).expect("create ambiguous Codex home");
    let historical_config = |name: &str, base_url: &str| {
        format!(
            "model_provider = \"custom\"\nmodel = \"gpt-5\"\n\
             [model_providers.custom]\nname = \"{name}\"\nbase_url = \"{base_url}\"\n\
             wire_api = \"responses\"\nrequires_openai_auth = true\n"
        )
    };
    std::fs::write(
        ambiguous_home.join("config.toml"),
        historical_config("First", "https://first.example/v1"),
    )
    .expect("write first historical config");
    std::fs::write(
        ambiguous_home.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-secret"}"#,
    )
    .expect("write credentials");
    let ambiguous_store = StateStore::new(StatePaths::from_root(ambiguous.path().join("state")));
    assert!(ambiguous_store.bootstrap().is_ready());
    insert_backup_source_provider(&ambiguous_store);
    let ambiguous_environment = EnvironmentApplication::new(ambiguous_store, &ambiguous_home);
    ambiguous_environment
        .apply_provider(BACKUP_SOURCE_PROVIDER_ID, true)
        .expect("back up first historical custom definition");
    std::fs::write(
        ambiguous_home.join("config.toml"),
        historical_config("Second", "https://second.example/v1"),
    )
    .expect("write second historical config");
    ambiguous_environment
        .apply_provider(BACKUP_SOURCE_PROVIDER_ID, true)
        .expect("back up second historical custom definition");
    let ambiguous_config = "model_provider = \"custom\"\nmodel = \"gpt-5\"\n";
    std::fs::write(ambiguous_home.join("config.toml"), ambiguous_config)
        .expect("write ambiguous dangling config");
    let ambiguous_application =
        DiagnosticApplication::with_environment(&ambiguous_home, None, ambiguous_environment);

    let ambiguous_report = ambiguous_application.inspect_with(&stopped_observations(), &[]);

    assert!(ambiguous_report.repair_preview.is_none());
    assert!(ambiguous_report.findings.iter().any(|finding| {
        finding.code == "model_provider_missing_definition" && !finding.repairable
    }));
    assert_eq!(
        std::fs::read_to_string(ambiguous_home.join("config.toml"))
            .expect("read untouched ambiguous config"),
        ambiguous_config
    );

    let missing_credentials = tempdir().expect("create missing credential fixture");
    std::fs::write(
        missing_credentials.path().join("config.toml"),
        concat!(
            "model_provider = \"custom\"\n",
            "model = \"gpt-5\"\n",
            "[model_providers.f4782322-ad03-4a2e-95b0-a4af3bcff403]\n",
            "name = \"custom\"\n",
            "base_url = \"https://provider.example/v1\"\n",
            "wire_api = \"responses\"\n",
            "requires_openai_auth = true\n",
        ),
    )
    .expect("write config without credentials");
    let missing_store = StateStore::new(StatePaths::from_root(
        missing_credentials.path().join("state"),
    ));
    assert!(missing_store.bootstrap().is_ready());
    insert_verified_provider(
        &missing_store,
        CURRENT_SOURCE_PROVIDER_ID,
        "custom",
        "https://provider.example/v1",
        "local-secret",
        "gpt-5",
        true,
    );
    let missing_environment =
        EnvironmentApplication::new(missing_store, missing_credentials.path());
    let missing_application = DiagnosticApplication::with_environment(
        missing_credentials.path(),
        None,
        missing_environment,
    );

    let missing_report = missing_application.inspect_with(&stopped_observations(), &[]);

    assert!(missing_report.repair_preview.is_none());
    assert!(!missing_credentials.path().join(".gpteasy-backups").exists());
}

#[test]
fn stale_preview_does_not_overwrite_a_concurrent_config_change() {
    let (directory, _, application, _) =
        repair_fixture_with_faults(Arc::new(FailAfterConfigReplace));
    let preview = application
        .inspect_with(&stopped_observations(), &[])
        .repair_preview
        .expect("repair preview");
    let changed = "model_provider = \"custom\"\nmodel = \"changed-model\"\n";
    std::fs::write(directory.path().join("config.toml"), changed).expect("write concurrent change");

    let result = application.repair_custom_provider(&preview.preview_id);

    assert_eq!(result.status, DiagnosticRepairStatus::NotModified);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("config.toml"))
            .expect("read concurrent config"),
        changed
    );
}

#[test]
fn post_write_failure_rolls_back_to_the_original_config() {
    let (directory, _, application, original) =
        repair_fixture_with_faults(Arc::new(FailAfterConfigReplace));
    let preview = application
        .inspect_with(&stopped_observations(), &[])
        .repair_preview
        .expect("repair preview");

    let result = application.repair_custom_provider(&preview.preview_id);

    assert_eq!(result.status, DiagnosticRepairStatus::RolledBack);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("config.toml"))
            .expect("read rolled back config"),
        original
    );
}

#[test]
fn interrupted_repair_is_restored_on_pending_operation_recovery() {
    let (directory, environment, application, original) =
        repair_fixture_with_faults(Arc::new(InterruptAfterConfigReplace));
    let preview = application
        .inspect_with(&stopped_observations(), &[])
        .repair_preview
        .expect("repair preview");

    let interrupted = application.repair_custom_provider(&preview.preview_id);

    assert_eq!(interrupted.status, DiagnosticRepairStatus::ManualRequired);
    assert_ne!(
        std::fs::read_to_string(directory.path().join("config.toml"))
            .expect("read interrupted config"),
        original
    );
    assert_eq!(
        environment
            .recover_pending()
            .expect("recover interrupted repair"),
        EnvironmentRecovery::KeptOldState
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("config.toml"))
            .expect("read recovered config"),
        original
    );
}

#[test]
fn concurrent_credential_change_rolls_back_config_without_overwriting_credentials() {
    let directory = tempdir().expect("create concurrent credential fixture");
    let credentials_path = directory.path().join("auth.json");
    let original = concat!(
        "model_provider = \"custom\"\n",
        "model = \"gpt-5\"\n",
        "[model_providers.f4782322-ad03-4a2e-95b0-a4af3bcff403]\n",
        "name = \"custom\"\n",
        "base_url = \"https://provider.example/v1\"\n",
        "wire_api = \"responses\"\n",
        "requires_openai_auth = true\n",
    );
    std::fs::write(directory.path().join("config.toml"), original).expect("write repair config");
    std::fs::write(
        &credentials_path,
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-secret"}"#,
    )
    .expect("write repair credentials");
    let store = StateStore::new(StatePaths::from_root(directory.path().join("state")));
    assert!(store.bootstrap().is_ready());
    insert_verified_provider(
        &store,
        CURRENT_SOURCE_PROVIDER_ID,
        "custom",
        "https://provider.example/v1",
        "local-secret",
        "gpt-5",
        true,
    );
    let environment = EnvironmentApplication::with_runtime_dependencies(
        store,
        directory.path(),
        Arc::new(ChangeCredentialsAfterConfigReplace {
            path: credentials_path.clone(),
        }),
        Arc::new(LoginStatusCommand::codex_default()),
        Arc::new(StoppedConsumers),
    );
    let application = DiagnosticApplication::with_environment(directory.path(), None, environment);
    let preview = application
        .inspect_with(&stopped_observations(), &[])
        .repair_preview
        .expect("repair preview");

    let result = application.repair_custom_provider(&preview.preview_id);

    assert_eq!(result.status, DiagnosticRepairStatus::RolledBack);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("config.toml"))
            .expect("read rolled back config"),
        original
    );
    assert!(
        std::fs::read_to_string(credentials_path)
            .expect("read concurrent credentials")
            .contains("changed-secret")
    );
}

#[test]
fn reports_a_dangling_custom_model_provider_as_a_local_configuration_finding() {
    let directory = tempdir().expect("create temporary Codex home");
    std::fs::write(
        directory.path().join("config.toml"),
        "model_provider = \"custom\"\nmodel = \"gpt-5\"\n",
    )
    .expect("write config fixture");
    let application = DiagnosticApplication::new(directory.path(), None);

    let report = application.inspect_with(&stopped_observations(), &[]);

    assert_eq!(
        report.environment.config_status,
        DiagnosticConfigStatus::Valid
    );
    assert_eq!(
        report.environment.active_provider.as_deref(),
        Some("custom")
    );
    assert!(report.environment.declared_providers.is_empty());
    assert!(report.findings.iter().any(|finding| {
        finding.code == "model_provider_missing_definition"
            && finding.origin == DiagnosticOrigin::Local
            && finding.summary.contains("custom")
            && !finding.repairable
    }));
}

#[test]
fn distinguishes_missing_unreadable_encoding_and_toml_syntax_failures() {
    let missing = tempdir().expect("create missing fixture");
    let unreadable = tempdir().expect("create unreadable fixture");
    std::fs::create_dir(unreadable.path().join("config.toml"))
        .expect("create non-file config fixture");
    let encoding = tempdir().expect("create encoding fixture");
    std::fs::write(encoding.path().join("config.toml"), [0xff, 0xfe])
        .expect("write invalid UTF-8 fixture");
    let syntax = tempdir().expect("create syntax fixture");
    std::fs::write(syntax.path().join("config.toml"), "model_provider = [\n")
        .expect("write invalid TOML fixture");

    let cases = [
        (
            missing.path(),
            DiagnosticConfigStatus::Missing,
            "config_missing",
        ),
        (
            unreadable.path(),
            DiagnosticConfigStatus::Unreadable,
            "config_unreadable",
        ),
        (
            encoding.path(),
            DiagnosticConfigStatus::EncodingError,
            "config_encoding_error",
        ),
        (
            syntax.path(),
            DiagnosticConfigStatus::TomlSyntaxError,
            "config_toml_syntax_error",
        ),
    ];

    for (codex_home, expected_status, expected_finding) in cases {
        let report =
            DiagnosticApplication::new(codex_home, None).inspect_with(&stopped_observations(), &[]);
        assert_eq!(report.environment.config_status, expected_status);
        assert_eq!(report.findings[0].code, expected_finding);
        assert_eq!(report.findings[0].origin, DiagnosticOrigin::Local);
    }
}

#[test]
fn includes_redacted_environment_authentication_consumer_and_version_facts() {
    let directory = tempdir().expect("create temporary Codex home");
    std::fs::write(
        directory.path().join("config.toml"),
        concat!(
            "model_provider = \"custom\"\n",
            "cli_auth_credentials_store = \"file\"\n",
            "[model_providers.custom]\n",
            "name = \"Custom\"\n",
            "base_url = \"https://secret.example/v1\"\n",
        ),
    )
    .expect("write config fixture");
    std::fs::write(
        directory.path().join("auth.json"),
        r#"{"OPENAI_API_KEY":"raw-secret-api-key"}"#,
    )
    .expect("write auth fixture");
    let overridden_home = directory.path().join("other-codex-home");
    let observations = DiagnosticObservations {
        login_status: LoginStatus::NotLoggedIn,
        desktop_status: ConsumerStatus::Running,
        cli_status: ConsumerStatus::Unknown,
        codex_cli_version: Some("0.147.0".to_owned()),
    };

    let report = DiagnosticApplication::new(directory.path(), Some(overridden_home))
        .inspect_with(&observations, &[]);

    assert_eq!(report.environment.scope, DiagnosticScope::CurrentUser);
    assert_eq!(report.environment.codex_home, "~/.codex");
    assert_eq!(
        report.environment.codex_home_override_status,
        CodexHomeOverrideStatus::Differs
    );
    assert_eq!(report.environment.declared_providers, ["custom"]);
    assert_eq!(report.authentication.login_status, LoginStatus::NotLoggedIn);
    assert_eq!(
        report.authentication.auth_file_status,
        AuthFileStatus::Present
    );
    assert_eq!(
        report.authentication.credential_store,
        CredentialStore::File
    );
    assert_eq!(report.consumers.desktop, ConsumerStatus::Running);
    assert_eq!(report.consumers.cli, ConsumerStatus::Unknown);
    assert_eq!(report.versions.gpteasy, env!("CARGO_PKG_VERSION"));
    assert_eq!(report.versions.codex_cli.as_deref(), Some("0.147.0"));
    assert!(report.findings.iter().any(|finding| {
        finding.code == "codex_home_mismatch"
            && finding.origin == DiagnosticOrigin::Local
            && !finding.repairable
    }));
}

#[test]
fn distinguishes_local_provider_lookup_failures_from_remote_invalid_api_keys() {
    let directory = tempdir().expect("create temporary Codex home");
    std::fs::write(
        directory.path().join("config.toml"),
        "model_provider = \"openai\"\n[model_providers.openai]\nname = \"OpenAI\"\n",
    )
    .expect("write config fixture");
    let logs = [
        IssueLogRecord {
            timestamp_epoch_seconds: 10,
            level: IssueLogLevel::Error,
            event: "codex.session.open".to_owned(),
            message: "session.model_provider_not_found".to_owned(),
            details: Some("api_key=must-not-leak".to_owned()),
        },
        IssueLogRecord {
            timestamp_epoch_seconds: 20,
            level: IssueLogLevel::Error,
            event: "provider.responses".to_owned(),
            message: "provider.invalid_api_key".to_owned(),
            details: Some("category=Authentication response_body=must-not-leak".to_owned()),
        },
        IssueLogRecord {
            timestamp_epoch_seconds: 30,
            level: IssueLogLevel::Error,
            event: "provider.responses".to_owned(),
            message: "provider.authentication_failed".to_owned(),
            details: Some("status=403".to_owned()),
        },
        IssueLogRecord {
            timestamp_epoch_seconds: 40,
            level: IssueLogLevel::Error,
            event: "legacy.raw_error".to_owned(),
            message: "Model provider stale not found; 401 invalid_api_key".to_owned(),
            details: None,
        },
    ];

    let report = DiagnosticApplication::new(directory.path(), None)
        .inspect_with(&stopped_observations(), &logs);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "historical_provider_missing" && finding.origin == DiagnosticOrigin::Local
    }));
    assert!(report.findings.iter().any(|finding| {
        finding.code == "remote_invalid_api_key" && finding.origin == DiagnosticOrigin::Remote
    }));
    assert_eq!(report.errors.len(), 2);
    assert_eq!(report.errors[0].error_code, "model_provider_not_found");
    assert_eq!(report.errors[0].origin, DiagnosticOrigin::Local);
    assert_eq!(report.errors[0].last_seen_epoch_seconds, 10);
    assert_eq!(report.errors[0].occurrences, 1);
    assert_eq!(report.errors[1].error_code, "invalid_api_key");
    assert_eq!(report.errors[1].origin, DiagnosticOrigin::Remote);
    assert_eq!(report.errors[1].last_seen_epoch_seconds, 20);
    assert_eq!(report.errors[1].occurrences, 1);
}

#[test]
fn json_and_markdown_exports_exclude_credentials_and_full_config_values() {
    let directory = tempdir().expect("create temporary Codex home");
    std::fs::write(
        directory.path().join("config.toml"),
        concat!(
            "model_provider = \"custom\"\n",
            "model = \"private-model-name\"\n",
            "[model_providers.custom]\n",
            "name = \"Custom\"\n",
            "base_url = \"https://private-provider.example/v1\"\n",
            "experimental_bearer_token = \"config-secret-token\"\n",
        ),
    )
    .expect("write config fixture");
    std::fs::write(
        directory.path().join("auth.json"),
        r#"{"OPENAI_API_KEY":"auth-file-secret-key","tokens":{"access_token":"raw-access-token"}}"#,
    )
    .expect("write auth fixture");
    let logs = [IssueLogRecord {
        timestamp_epoch_seconds: 30,
        level: IssueLogLevel::Error,
        event: "provider.responses".to_owned(),
        message: "provider.invalid_api_key".to_owned(),
        details: Some("response_body=raw-response-body api_key=log-secret-key".to_owned()),
    }];
    let report = DiagnosticApplication::new(directory.path(), None)
        .inspect_with(&stopped_observations(), &logs);

    for export in [report.redacted_json(), report.redacted_markdown()] {
        for forbidden in [
            "private-model-name",
            "https://private-provider.example/v1",
            "config-secret-token",
            "auth-file-secret-key",
            "raw-access-token",
            "raw-response-body",
            "log-secret-key",
        ] {
            assert!(!export.contains(forbidden), "export leaked {forbidden}");
        }
        assert!(export.contains("custom"));
        assert!(export.contains("invalid_api_key"));
    }
}
