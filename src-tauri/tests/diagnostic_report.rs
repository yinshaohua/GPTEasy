use gpteasy_lib::codex::LoginStatus;
use gpteasy_lib::consumer::ConsumerStatus;
use gpteasy_lib::diagnostic_report::{
    AuthFileStatus, CodexHomeOverrideStatus, CredentialStore, DiagnosticApplication,
    DiagnosticConfigStatus, DiagnosticObservations, DiagnosticOrigin, DiagnosticScope,
};
use gpteasy_lib::diagnostics::{IssueLogLevel, IssueLogRecord};
use tempfile::tempdir;

fn stopped_observations() -> DiagnosticObservations {
    DiagnosticObservations {
        login_status: LoginStatus::LoggedIn,
        desktop_status: ConsumerStatus::Stopped,
        cli_status: ConsumerStatus::Stopped,
        codex_cli_version: Some("0.147.0".to_owned()),
    }
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
