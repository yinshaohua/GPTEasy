use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gpteasy_lib::environment::{
    EnvironmentApplication, EnvironmentFailurePoint, EnvironmentFaultInjector,
};
use gpteasy_lib::provider::{
    DiscoveryInput, ProviderApplication, ProviderFailureCategory, ProviderUpdateDiscoveryInput,
    ProviderUpdateValidationInput, ProviderValidationInput, ProviderValidationStage,
    ProviderValidator, ValidationTimeouts,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;

fn validator() -> ProviderValidator {
    ProviderValidator::new(ValidationTimeouts {
        connect: Duration::from_millis(250),
        response_header: Duration::from_millis(250),
        stream_read: Duration::from_millis(100),
        response_overall: Duration::from_millis(750),
    })
}

#[tokio::test]
async fn model_discovery_enforces_the_provider_url_policy_before_network_access() {
    let validator = validator();

    for rejected in [
        "http://provider.example/v1",
        "https://user@provider.example/v1",
        "https://provider.example/v1?tenant=secret",
        "https://provider.example/v1#fragment",
    ] {
        let failure = validator
            .discover_models(
                DiscoveryInput {
                    base_url: rejected.to_owned(),
                    api_key: "must-not-leak".to_owned(),
                },
                Default::default(),
            )
            .await
            .expect_err("unsafe URL must fail before a request is sent");

        assert_eq!(failure.category, ProviderFailureCategory::SecurityPolicy);
        assert!(!failure.message_id.contains("must-not-leak"));
    }
}

#[tokio::test]
async fn model_discovery_allows_http_for_each_loopback_host_form() {
    for (bind_host, url_host) in [
        ("127.0.0.1", "127.0.0.1"),
        ("localhost", "localhost"),
        ("::1", "[::1]"),
    ] {
        let server = ModelServer::start_on(
            bind_host,
            url_host,
            "200 OK",
            r#"{"object":"list","data":[{"id":"model-a"}]}"#,
        );

        let discovery = validator()
            .discover_models(
                DiscoveryInput {
                    base_url: server.base_url,
                    api_key: "test-provider-key".to_owned(),
                },
                Default::default(),
            )
            .await
            .unwrap_or_else(|failure| {
                panic!("loopback HTTP must be allowed for {url_host}: {failure:?}")
            });

        assert_eq!(discovery.models, ["model-a"]);
    }
}

#[tokio::test]
async fn model_discovery_preserves_the_path_prefix_and_returns_actual_models() {
    let server = ModelServer::start(
        "200 OK",
        r#"{"object":"list","data":[{"id":"model-b"},{"id":"model-a"},{"id":"model-b"}]}"#,
    );

    let discovery = validator()
        .discover_models(
            DiscoveryInput {
                base_url: format!("{}/tenant/v1/", server.base_url),
                api_key: "test-provider-key".to_owned(),
            },
            Default::default(),
        )
        .await
        .expect("model discovery succeeds");

    assert_eq!(
        discovery.normalized_base_url,
        format!("{}/tenant/v1", server.base_url)
    );
    assert_eq!(discovery.models, ["model-b", "model-a"]);
    let request = server.request.recv().expect("captured models request");
    assert!(request.starts_with("GET /tenant/v1/models HTTP/1.1"));
    assert!(request.contains("authorization: Bearer test-provider-key"));
}

#[tokio::test]
async fn model_discovery_probes_bounded_path_candidates_in_serial_order() {
    let server = ScriptedModelServer::start([
        ("404 Not Found", r#"{"error":"missing endpoint"}"#),
        ("404 Not Found", r#"{"error":"missing endpoint"}"#),
        (
            "200 OK",
            r#"{"object":"list","data":[{"id":"candidate-model"}]}"#,
        ),
    ]);
    let requested_base_url = format!("{}/tenant/chat/completions", server.base_url);

    let discovery = validator()
        .discover_models(
            DiscoveryInput {
                base_url: requested_base_url.clone(),
                api_key: "test-provider-key".to_owned(),
            },
            Default::default(),
        )
        .await
        .expect("a bounded path candidate succeeds");

    assert_eq!(discovery.requested_base_url, requested_base_url);
    assert_eq!(
        discovery.normalized_base_url,
        format!("{}/tenant/v1", server.base_url)
    );
    assert_eq!(discovery.models, ["candidate-model"]);
    assert_eq!(
        server.finish(),
        [
            "/tenant/chat/completions/models",
            "/tenant/models",
            "/tenant/v1/models",
        ]
    );
}

#[tokio::test]
async fn model_discovery_does_not_guess_paths_for_non_path_failures() {
    for (status, expected) in [
        ("401 Unauthorized", ProviderFailureCategory::Authentication),
        ("429 Too Many Requests", ProviderFailureCategory::RateLimit),
        ("302 Found", ProviderFailureCategory::SecurityPolicy),
        (
            "500 Internal Server Error",
            ProviderFailureCategory::ModelDiscovery,
        ),
    ] {
        let server = ScriptedModelServer::start([(status, r#"{"error":"stop"}"#)]);
        let failure = validator()
            .discover_models(
                DiscoveryInput {
                    base_url: format!("{}/wrong", server.base_url),
                    api_key: "test-provider-key".to_owned(),
                },
                Default::default(),
            )
            .await
            .expect_err("non-path failures must stop candidate probing");

        assert_eq!(failure.category, expected, "status {status}");
        assert_eq!(server.finish(), ["/wrong/models"], "status {status}");
    }

    let server = ScriptedModelServer::start([("200 OK", "not-json")]);
    let failure = validator()
        .discover_models(
            DiscoveryInput {
                base_url: format!("{}/wrong", server.base_url),
                api_key: "test-provider-key".to_owned(),
            },
            Default::default(),
        )
        .await
        .expect_err("protocol errors must stop candidate probing");
    assert_eq!(failure.category, ProviderFailureCategory::ModelDiscovery);
    assert_eq!(server.finish(), ["/wrong/models"]);
}

#[tokio::test]
async fn model_discovery_stops_guessing_for_timeout_security_and_tls_failures() {
    let timeout_server = DelayedModelServer::start(Duration::from_millis(400));
    let timed_out = validator()
        .discover_models(
            DiscoveryInput {
                base_url: format!("{}/wrong", timeout_server.base_url),
                api_key: "test-provider-key".to_owned(),
            },
            Default::default(),
        )
        .await
        .expect_err("response timeout stops candidate probing");
    assert_eq!(
        timed_out.category,
        ProviderFailureCategory::ResponseHeaderTimeout
    );
    assert_eq!(timeout_server.finish(), ["/wrong/models"]);

    let security_failure = validator()
        .discover_models(
            DiscoveryInput {
                base_url: "http://provider.example/wrong".to_owned(),
                api_key: "test-provider-key".to_owned(),
            },
            Default::default(),
        )
        .await
        .expect_err("security policy stops before candidate probing");
    assert_eq!(
        security_failure.category,
        ProviderFailureCategory::SecurityPolicy
    );

    let tls_failure = validator()
        .discover_models(
            DiscoveryInput {
                base_url: "https://127.0.0.1:9/wrong".to_owned(),
                api_key: "test-provider-key".to_owned(),
            },
            Default::default(),
        )
        .await
        .expect_err("TLS or transport failure stops candidate probing");
    assert!(matches!(
        tls_failure.category,
        ProviderFailureCategory::Transport | ProviderFailureCategory::ResponseHeaderTimeout
    ));
}

#[tokio::test]
async fn model_discovery_covers_each_bounded_suffix_candidate_without_changing_origin() {
    for (requested_path, expected_paths, resolved_path) in [
        (
            "/tenant/models",
            vec!["/tenant/models/models", "/tenant/models"],
            "/tenant",
        ),
        (
            "/tenant/responses",
            vec!["/tenant/responses/models", "/tenant/models"],
            "/tenant",
        ),
        (
            "/tenant/chat/completions",
            vec!["/tenant/chat/completions/models", "/tenant/models"],
            "/tenant",
        ),
        (
            "/tenant",
            vec!["/tenant/models", "/tenant/v1/models"],
            "/tenant/v1",
        ),
        (
            "/tenant/v1",
            vec!["/tenant/v1/models", "/tenant/models"],
            "/tenant",
        ),
    ] {
        let server = ScriptedModelServer::start([
            ("404 Not Found", r#"{"error":"missing endpoint"}"#),
            (
                "200 OK",
                r#"{"object":"list","data":[{"id":"candidate-model"}]}"#,
            ),
        ]);
        let expected_resolved_base_url = format!("{}{resolved_path}", server.base_url);
        let discovery = validator()
            .discover_models(
                DiscoveryInput {
                    base_url: format!("{}{requested_path}", server.base_url),
                    api_key: "test-provider-key".to_owned(),
                },
                Default::default(),
            )
            .await
            .expect("bounded suffix candidate succeeds");

        assert_eq!(server.finish(), expected_paths, "path {requested_path}");
        assert_eq!(
            discovery.normalized_base_url, expected_resolved_base_url,
            "path {requested_path}"
        );
    }
}

#[tokio::test]
async fn validation_completes_a_fragmented_strict_tool_round_trip() {
    let server = ValidationServer::start(ValidationScenario::Success);

    let evidence = validator()
        .validate_provider(
            ProviderValidationInput {
                base_url: format!("{}/tenant/v1", server.base_url),
                api_key: "test-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
            Default::default(),
        )
        .await
        .expect("complete provider validation succeeds");

    assert_eq!(
        evidence.normalized_base_url,
        format!("{}/tenant/v1", server.base_url)
    );
    assert_eq!(evidence.default_model, "model-a");
    assert_eq!(evidence.combination_fingerprint.len(), 64);
    let requests = server
        .requests
        .recv()
        .expect("captured validation requests");
    assert_eq!(requests.len(), 3);
    let first: Value = serde_json::from_slice(request_body(&requests[1])).expect("first payload");
    assert_eq!(first["tools"][0]["strict"], true);
    assert_eq!(
        first["tools"][0]["parameters"]["additionalProperties"],
        false
    );
    assert_eq!(first["tool_choice"]["name"], "gpteasy_probe");
    assert_eq!(first["parallel_tool_calls"], false);
    let second: Value = serde_json::from_slice(request_body(&requests[2])).expect("second payload");
    assert_eq!(second["input"][1]["call_id"], second["input"][2]["call_id"]);
}

#[tokio::test]
async fn candidate_validation_requires_explicit_address_confirmation_before_save() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store, validator());
    let server = PathFixValidationServer::start();
    let requested_base_url = format!("{}/tenant", server.base_url);
    let resolved_base_url = format!("{}/tenant/v1", server.base_url);

    let receipt = application
        .validate_provider(
            "candidate-validation".to_owned(),
            ProviderValidationInput {
                base_url: requested_base_url.clone(),
                api_key: "test-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("candidate completes full validation");

    assert_eq!(receipt.requested_base_url, requested_base_url);
    assert_eq!(receipt.normalized_base_url, resolved_base_url);
    let blocked = application
        .save_verified_provider(&receipt.validation_id, "Candidate Provider")
        .expect_err("an unconfirmed suggested address cannot be saved");
    assert_eq!(
        blocked.category,
        ProviderFailureCategory::VerificationExpired
    );
    assert!(
        application
            .list_providers()
            .expect("empty catalog")
            .is_empty()
    );

    application
        .confirm_validation_base_url(&receipt.validation_id, &receipt.normalized_base_url)
        .expect("user confirms the validated candidate address");
    let saved = application
        .save_verified_provider(&receipt.validation_id, "Candidate Provider")
        .expect("confirmed candidate can be saved");
    assert_eq!(saved.base_url, resolved_base_url);
    assert_eq!(
        server.finish(),
        [
            "/tenant/models",
            "/tenant/v1/models",
            "/tenant/v1/responses",
            "/tenant/v1/responses",
        ]
    );
}

#[tokio::test]
async fn expired_candidate_receipt_cannot_be_confirmed_or_saved() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application =
        ProviderApplication::with_validation_receipt_ttl(store, validator(), Duration::ZERO);
    let server = PathFixValidationServer::start();
    let receipt = application
        .validate_provider(
            "expiring-candidate".to_owned(),
            ProviderValidationInput {
                base_url: format!("{}/tenant", server.base_url),
                api_key: "test-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("candidate validates before receipt expiry");

    let expired = application
        .confirm_validation_base_url(&receipt.validation_id, &receipt.normalized_base_url)
        .expect_err("expired receipt cannot be confirmed");
    assert_eq!(
        expired.category,
        ProviderFailureCategory::VerificationExpired
    );
    let expired = application
        .save_verified_provider(&receipt.validation_id, "Expired Candidate")
        .expect_err("expired receipt cannot be saved");
    assert_eq!(
        expired.category,
        ProviderFailureCategory::VerificationExpired
    );
    assert!(
        application
            .list_providers()
            .expect("empty catalog")
            .is_empty()
    );
    server.finish();
}

#[tokio::test]
async fn validation_reports_ordered_stages_without_exposing_credentials() {
    let server = ValidationServer::start(ValidationScenario::Success);
    let input = ProviderValidationInput {
        base_url: server.base_url,
        api_key: "stage-secret-canary".to_owned(),
        default_model: "model-a".to_owned(),
    };
    let debug_input = format!("{input:?}");
    let stages = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_stages = Arc::clone(&stages);

    let evidence = validator()
        .validate_provider_with_progress(input, Default::default(), move |stage| {
            captured_stages.lock().expect("capture stage").push(stage);
        })
        .await
        .expect("validation succeeds");

    assert_eq!(
        *stages.lock().expect("validation stages"),
        [
            ProviderValidationStage::ModelsConfirmed,
            ProviderValidationStage::ResponsesStream,
            ProviderValidationStage::ToolRoundTrip,
        ]
    );
    let observable_output = format!("{debug_input}\n{evidence:?}");
    assert!(!observable_output.contains("stage-secret-canary"));
    assert!(debug_input.contains("[redacted]"));

    let invalid_server = ValidationServer::start(ValidationScenario::WrongNonce);
    let failure_stages = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_failure_stages = Arc::clone(&failure_stages);
    let failure = validator()
        .validate_provider_with_progress(
            ProviderValidationInput {
                base_url: invalid_server.base_url,
                api_key: "failure-secret-canary".to_owned(),
                default_model: "model-a".to_owned(),
            },
            Default::default(),
            move |stage| {
                captured_failure_stages
                    .lock()
                    .expect("capture failure stage")
                    .push(stage);
            },
        )
        .await
        .expect_err("invalid strict tool arguments fail validation");
    assert_eq!(failure.category, ProviderFailureCategory::ToolCall);
    assert_eq!(
        *failure_stages.lock().expect("failure stages"),
        [
            ProviderValidationStage::ModelsConfirmed,
            ProviderValidationStage::ResponsesStream,
            ProviderValidationStage::ToolRoundTrip,
        ]
    );
    assert!(!format!("{failure:?}").contains("failure-secret-canary"));
}

#[tokio::test]
async fn model_discovery_rejects_empty_or_malformed_model_lists() {
    for body in [
        r#"{"object":"list","data":[]}"#,
        r#"{"object":"list"}"#,
        r#"{"object":"list","data":[{"id":"model-a"},{"name":"missing-id"}]}"#,
        "not-json",
    ] {
        let server = ModelServer::start("200 OK", body);
        let failure = validator()
            .discover_models(
                DiscoveryInput {
                    base_url: server.base_url,
                    api_key: "test-provider-key".to_owned(),
                },
                Default::default(),
            )
            .await
            .expect_err("unusable model list must fail");

        assert_eq!(failure.category, ProviderFailureCategory::ModelDiscovery);
        assert!(!failure.message_id.contains("test-provider-key"));
    }
}

#[tokio::test]
async fn validation_rejects_broken_streams_and_nonce_or_schema_mismatches() {
    for (scenario, expected) in [
        (
            ValidationScenario::Truncated,
            ProviderFailureCategory::Streaming,
        ),
        (
            ValidationScenario::ExtraArgument,
            ProviderFailureCategory::ToolCall,
        ),
        (
            ValidationScenario::WrongNonce,
            ProviderFailureCategory::ToolCall,
        ),
        (
            ValidationScenario::WrongFinalNonce,
            ProviderFailureCategory::ToolResult,
        ),
        (
            ValidationScenario::MultipleToolCalls,
            ProviderFailureCategory::ToolCall,
        ),
        (
            ValidationScenario::SecondRoundToolCall,
            ProviderFailureCategory::ToolResult,
        ),
    ] {
        let server = ValidationServer::start(scenario);
        let failure = validator()
            .validate_provider(
                ProviderValidationInput {
                    base_url: server.base_url,
                    api_key: "test-provider-key".to_owned(),
                    default_model: "model-a".to_owned(),
                },
                Default::default(),
            )
            .await
            .expect_err("invalid Responses behavior must fail validation");

        assert_eq!(failure.category, expected);
        assert!(!failure.message_id.contains("test-provider-key"));
    }
}

#[tokio::test]
async fn validation_distinguishes_first_event_timeout_and_user_cancellation() {
    let timeout_server = ValidationServer::start(ValidationScenario::IdleBeforeFirstEvent);
    let timed_out = validator()
        .validate_provider(
            ProviderValidationInput {
                base_url: timeout_server.base_url,
                api_key: "test-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
            Default::default(),
        )
        .await
        .expect_err("idle stream must time out");
    assert_eq!(
        timed_out.category,
        ProviderFailureCategory::FirstEventTimeout
    );

    let cancel_server = ValidationServer::start(ValidationScenario::IdleBeforeFirstEvent);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let cancel_signal = cancellation.clone();
    let cancel_validator = validator();
    let validation = cancel_validator.validate_provider(
        ProviderValidationInput {
            base_url: cancel_server.base_url,
            api_key: "test-provider-key".to_owned(),
            default_model: "model-a".to_owned(),
        },
        cancellation,
    );
    let (_, cancelled) = tokio::join!(
        async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            cancel_signal.cancel();
        },
        validation
    );
    let failure = cancelled.expect_err("cancelled validation must stop");
    assert_eq!(failure.category, ProviderFailureCategory::Cancelled);
}

#[tokio::test]
async fn a_verified_provider_is_persisted_only_after_explicit_save() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    let initial = store.bootstrap();
    assert!(initial.is_ready());
    let application = ProviderApplication::new(store.clone(), validator());
    let first_server = ValidationServer::start(ValidationScenario::Success);

    let receipt = application
        .validate_provider(
            "validation-1".to_owned(),
            ProviderValidationInput {
                base_url: first_server.base_url,
                api_key: "test-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("provider validates");

    assert!(
        application
            .list_providers()
            .expect("list providers")
            .is_empty()
    );
    let after_validation = store.bootstrap().contents.expect("state contents");
    assert_eq!(after_validation.provider_count, 0);
    assert!(!after_validation.has_pending_config_operation);
    assert_eq!(
        std::fs::read_dir(store.paths().backups())
            .expect("read backup directory")
            .count(),
        0
    );

    let saved = application
        .save_verified_provider(&receipt.validation_id, "  Example Provider  ")
        .expect("save validated provider");
    assert_eq!(saved.name, "Example Provider");
    assert!(uuid::Uuid::parse_str(&saved.id).is_ok());
    assert_eq!(
        application.list_providers().expect("list providers"),
        [saved]
    );
    assert_eq!(
        store
            .bootstrap()
            .contents
            .expect("state contents")
            .provider_count,
        1
    );

    let duplicate_server = ValidationServer::start(ValidationScenario::Success);
    let duplicate = application
        .validate_provider(
            "validation-2".to_owned(),
            ProviderValidationInput {
                base_url: duplicate_server.base_url,
                api_key: "another-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("second provider validates");
    let failure = application
        .save_verified_provider(&duplicate.validation_id, "example provider")
        .expect_err("provider names are unique without case");
    assert_eq!(failure.category, ProviderFailureCategory::DuplicateName);
    assert_eq!(
        application.list_providers().expect("list providers").len(),
        1
    );
}

#[tokio::test]
async fn dayway_recommendation_identity_is_explicit_pinned_and_recoverable() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store.clone(), validator());

    let ordinary_server = ValidationServer::start(ValidationScenario::Success);
    let ordinary_receipt = application
        .validate_provider(
            "ordinary-dayway".to_owned(),
            ProviderValidationInput {
                base_url: ordinary_server.base_url,
                api_key: "ordinary-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("ordinary combination validates");
    let reserved = application
        .save_verified_provider(&ordinary_receipt.validation_id, "DayWay")
        .expect_err("ordinary providers cannot reserve the recommended name");
    assert_eq!(reserved.category, ProviderFailureCategory::InvalidInput);

    let ordinary_server = ValidationServer::start(ValidationScenario::Success);
    let ordinary = create_provider(
        &application,
        "ordinary-provider",
        ordinary_server.base_url,
        "Ordinary Provider",
        "ordinary-key",
    )
    .await;
    assert_eq!(ordinary.recommendation_id, None);
    let impersonation = application
        .rename_provider(&ordinary.id, "dayway")
        .expect_err("ordinary rename cannot impersonate DayWay");
    assert_eq!(
        impersonation.category,
        ProviderFailureCategory::InvalidInput
    );
    let recommended_server = ValidationServer::start(ValidationScenario::Success);
    let receipt = application
        .validate_provider(
            "recommended-dayway".to_owned(),
            ProviderValidationInput {
                base_url: recommended_server.base_url,
                api_key: "recommended-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("recommended combination validates");
    let recommended = application
        .save_dayway_provider(&receipt.validation_id)
        .expect("explicit DayWay save");

    assert_eq!(recommended.name, "DayWay");
    assert_eq!(recommended.recommendation_id.as_deref(), Some("dayway"));
    assert!(!recommended.has_recommendation_update);
    assert_eq!(
        application.list_providers().expect("recommended is pinned")[0].id,
        recommended.id
    );
    let reorder = application
        .reorder_providers(&[ordinary.id.clone(), recommended.id.clone()])
        .expect_err("recommended provider cannot move from first position");
    assert_eq!(reorder.category, ProviderFailureCategory::InvalidInput);
    assert_eq!(
        application
            .list_providers()
            .expect("pinned after rejection")[0]
            .id,
        recommended.id
    );
    let saved_base_url = recommended.base_url.clone();
    let connection = Connection::open(store.paths().database()).expect("open provider database");
    connection
        .execute(
            "UPDATE providers SET recommendation_template_base_url = 'https://old.dayway.example/v1' WHERE id = ?1",
            [&recommended.id],
        )
        .expect("simulate a newer built-in template after upgrade");
    drop(connection);
    let after_upgrade = application.list_providers().expect("list after upgrade");
    assert!(after_upgrade[0].has_recommendation_update);
    assert_eq!(after_upgrade[0].base_url, saved_base_url);
    let rename = application
        .rename_provider(&recommended.id, "DayWay Renamed")
        .expect_err("recommended name is fixed");
    assert_eq!(rename.category, ProviderFailureCategory::InvalidInput);

    application
        .delete_provider(&recommended.id)
        .expect("non-current recommendation can be deleted");
    assert!(
        application
            .list_providers()
            .expect("recommendation removed")
            .iter()
            .all(|provider| provider.recommendation_id.is_none())
    );
}

#[tokio::test]
async fn legacy_dayway_name_conflict_requires_confirmation_and_preserves_the_old_provider() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("open provider database");
    connection
        .execute(
            "INSERT INTO providers (id, name, base_url, api_key, default_model, verified_at, verification_fingerprint, sort_order)
             VALUES ('legacy-id', 'DayWay', 'https://legacy.example/v1', 'legacy-key', 'legacy-model', '123', 'legacy-fingerprint', 0)",
            [],
        )
        .expect("simulate migrated ordinary DayWay");
    drop(connection);
    let application = ProviderApplication::new(store.clone(), validator());
    let server = ValidationServer::start(ValidationScenario::Success);
    let receipt = application
        .validate_provider(
            "recommended-with-conflict".to_owned(),
            ProviderValidationInput {
                base_url: server.base_url,
                api_key: "recommended-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("recommended combination validates");

    let conflict = application
        .save_dayway_provider(&receipt.validation_id)
        .expect_err("name conflict requires explicit confirmation");
    assert_eq!(conflict.message_id, "provider.recommended_name_conflict");
    assert_eq!(
        application.list_providers().expect("unchanged catalog")[0].name,
        "DayWay"
    );

    let recommended = application
        .save_dayway_provider_with_name_conflict_confirmation(&receipt.validation_id, true)
        .expect("confirmed conflict resolution saves recommendation");
    assert_eq!(recommended.recommendation_id.as_deref(), Some("dayway"));
    let connection =
        Connection::open(store.paths().database()).expect("inspect preserved provider");
    let legacy = connection
        .query_row(
            "SELECT name, base_url, api_key, default_model, verification_fingerprint
             FROM providers WHERE id = 'legacy-id'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .expect("legacy provider remains");
    assert_eq!(
        legacy,
        (
            "DayWay (原供应商)".to_owned(),
            "https://legacy.example/v1".to_owned(),
            "legacy-key".to_owned(),
            "legacy-model".to_owned(),
            "legacy-fingerprint".to_owned(),
        )
    );
}

#[tokio::test]
async fn failed_cancelled_or_discarded_validation_never_changes_persistent_state() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let before = store.bootstrap().contents.expect("initial state contents");
    let application = Arc::new(ProviderApplication::new(store.clone(), validator()));

    let invalid_server = ValidationServer::start(ValidationScenario::WrongNonce);
    let failure = application
        .validate_provider(
            "invalid-validation".to_owned(),
            ProviderValidationInput {
                base_url: invalid_server.base_url,
                api_key: "test-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect_err("nonce mismatch fails validation");
    assert_eq!(failure.category, ProviderFailureCategory::ToolCall);
    assert_eq!(
        store.bootstrap().contents.expect("state after failure"),
        before
    );

    let idle_server = ValidationServer::start(ValidationScenario::IdleBeforeFirstEvent);
    let request_id = "cancelled-validation".to_owned();
    let task_application = Arc::clone(&application);
    let task_request_id = request_id.clone();
    let task = tokio::spawn(async move {
        task_application
            .validate_provider(
                task_request_id,
                ProviderValidationInput {
                    base_url: idle_server.base_url,
                    api_key: "test-provider-key".to_owned(),
                    default_model: "model-a".to_owned(),
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(application.cancel_request(&request_id));
    let cancelled = task
        .await
        .expect("join cancelled validation")
        .expect_err("request cancellation fails validation");
    assert_eq!(cancelled.category, ProviderFailureCategory::Cancelled);
    assert_eq!(
        store
            .bootstrap()
            .contents
            .expect("state after cancellation"),
        before
    );

    let valid_server = ValidationServer::start(ValidationScenario::Success);
    let receipt = application
        .validate_provider(
            "discarded-validation".to_owned(),
            ProviderValidationInput {
                base_url: valid_server.base_url,
                api_key: "test-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("provider validates before discard");
    application.discard_validation(&receipt.validation_id);
    let discarded = application
        .save_verified_provider(&receipt.validation_id, "Discarded")
        .expect_err("discarded validation cannot be saved");
    assert_eq!(
        discarded.category,
        ProviderFailureCategory::VerificationExpired
    );
    assert_eq!(
        store.bootstrap().contents.expect("state after discard"),
        before
    );

    let cancelled_receipt_server = ValidationServer::start(ValidationScenario::Success);
    let cancelled_receipt = application
        .validate_provider(
            "cancelled-after-success".to_owned(),
            ProviderValidationInput {
                base_url: cancelled_receipt_server.base_url,
                api_key: "test-provider-key".to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("provider validates before its request is cancelled");
    assert!(application.cancel_request("cancelled-after-success"));
    let cancelled_after_success = application
        .save_verified_provider(&cancelled_receipt.validation_id, "Cancelled after success")
        .expect_err("cancelling the originating request invalidates its receipt");
    assert_eq!(
        cancelled_after_success.category,
        ProviderFailureCategory::VerificationExpired
    );
    assert_eq!(
        store
            .bootstrap()
            .contents
            .expect("state after post-success cancellation"),
        before
    );
}

#[tokio::test]
async fn failed_catalog_revalidation_keeps_verification_time_and_current_provider() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store.clone(), validator());
    let valid_server = ValidationServer::start(ValidationScenario::Success);
    let provider = create_provider(
        &application,
        "revalidation-create",
        valid_server.base_url,
        "Revalidation Target",
        "revalidation-secret-canary",
    )
    .await;
    mark_current_provider(&store, &provider.id);
    let before = application
        .list_providers()
        .expect("catalog before failure");

    let failure = application
        .revalidate_provider("failed-catalog-revalidation".to_owned(), provider.id)
        .await
        .expect_err("unavailable provider fails revalidation");

    assert!(matches!(
        failure.category,
        ProviderFailureCategory::Transport | ProviderFailureCategory::ResponseHeaderTimeout
    ));
    assert_eq!(
        application.list_providers().expect("catalog after failure"),
        before
    );
}

#[tokio::test]
async fn catalog_revalidation_returns_a_candidate_receipt_without_persisting_it() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store.clone(), validator());
    let server = PathFixValidationServer::start();
    let requested_base_url = format!("{}/tenant", server.base_url);
    let resolved_base_url = format!("{}/tenant/v1", server.base_url);
    insert_provider_record(
        &store,
        "candidate-revalidation-provider",
        &requested_base_url,
        "test-provider-key",
        123,
    );

    let result = application
        .revalidate_provider(
            "candidate-revalidation".to_owned(),
            "candidate-revalidation-provider".to_owned(),
        )
        .await
        .expect("candidate completes revalidation");

    assert_eq!(result.provider.base_url, requested_base_url);
    assert_eq!(result.provider.verified_at_epoch_seconds, 123);
    let receipt = result
        .validation_receipt
        .expect("candidate revalidation requires address confirmation");
    assert_eq!(receipt.normalized_base_url, resolved_base_url);
    assert_eq!(
        application
            .list_providers()
            .expect("catalog remains unchanged")[0]
            .base_url,
        requested_base_url
    );
}

#[tokio::test]
async fn catalog_keeps_provider_identity_across_rename_and_protects_the_current_provider() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store.clone(), validator());

    let first_server = ValidationServer::start(ValidationScenario::Success);
    let first = create_provider(
        &application,
        "create-first",
        first_server.base_url,
        "First Provider",
        "first-provider-key",
    )
    .await;
    let second_server = ValidationServer::start(ValidationScenario::Success);
    let second = create_provider(
        &application,
        "create-second",
        second_server.base_url,
        "Second Provider",
        "second-provider-key",
    )
    .await;

    assert_ne!(first.id, second.id);
    let renamed = application
        .rename_provider(&first.id, "  Renamed Provider  ")
        .expect("rename provider");
    assert_eq!(renamed.id, first.id);
    assert_eq!(renamed.name, "Renamed Provider");
    assert_eq!(
        renamed.verified_at_epoch_seconds,
        first.verified_at_epoch_seconds
    );

    mark_current_provider(&store, &first.id);
    let providers = application.list_providers().expect("list providers");
    assert_eq!(providers.len(), 2);
    assert!(
        providers
            .iter()
            .find(|item| item.id == first.id)
            .unwrap()
            .is_current
    );
    assert!(
        !providers
            .iter()
            .find(|item| item.id == second.id)
            .unwrap()
            .is_current
    );

    let protected = application
        .delete_provider(&first.id)
        .expect_err("current provider cannot be deleted");
    assert_eq!(
        protected.category,
        ProviderFailureCategory::CurrentProviderProtected
    );
    application
        .delete_provider(&second.id)
        .expect("non-current provider can be deleted");
    let remaining = application.list_providers().expect("list providers");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, renamed.id);
    assert_eq!(remaining[0].name, renamed.name);
    assert!(remaining[0].is_current);
}

#[tokio::test]
async fn catalog_order_is_persistent_and_invalid_reorders_are_atomic() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store.clone(), validator());
    let first_server = ValidationServer::start(ValidationScenario::Success);
    let first = create_provider(
        &application,
        "order-first",
        first_server.base_url.clone(),
        "First Provider",
        "first-key",
    )
    .await;
    let second_server = ValidationServer::start(ValidationScenario::Success);
    let second = create_provider(
        &application,
        "order-second",
        second_server.base_url.clone(),
        "Second Provider",
        "second-key",
    )
    .await;
    let third_server = ValidationServer::start(ValidationScenario::Success);
    let third = create_provider(
        &application,
        "order-third",
        third_server.base_url.clone(),
        "Third Provider",
        "third-key",
    )
    .await;

    let reordered = application
        .reorder_providers(&[third.id.clone(), first.id.clone(), second.id.clone()])
        .expect("valid reorder");
    assert_eq!(
        reordered
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![third.id.as_str(), first.id.as_str(), second.id.as_str()]
    );
    assert_eq!(
        application.list_providers().expect("persistent order")[0].id,
        third.id
    );

    let invalid = application
        .reorder_providers(&[first.id.clone(), first.id.clone(), second.id.clone()])
        .expect_err("duplicate ids are rejected");
    assert_eq!(invalid.category, ProviderFailureCategory::InvalidInput);
    assert_eq!(
        application.list_providers().expect("order remains")[0].id,
        third.id
    );

    application
        .delete_provider(&first.id)
        .expect("delete provider");
    let compacted = application.list_providers().expect("compact order");
    assert_eq!(
        compacted
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![third.id.as_str(), second.id.as_str()]
    );
}

#[tokio::test]
async fn provider_updates_replace_only_the_freshly_validated_non_current_record() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store.clone(), validator());
    let compatible_server = ValidationServer::start_for(ValidationScenario::Success, 4);
    let original = create_provider(
        &application,
        "create-update-target",
        compatible_server.base_url.clone(),
        "Update Target",
        "original-provider-key",
    )
    .await;

    let incompatible_server = ValidationServer::start(ValidationScenario::WrongNonce);
    let failure = application
        .validate_provider_update(
            "failed-update".to_owned(),
            ProviderUpdateValidationInput {
                provider_id: original.id.clone(),
                base_url: incompatible_server.base_url,
                api_key: Some("rejected-provider-key".to_owned()),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect_err("failed validation keeps the old provider");
    assert_eq!(failure.category, ProviderFailureCategory::ToolCall);
    let providers = application.list_providers().expect("list providers");
    assert_eq!(providers.as_slice(), std::slice::from_ref(&original));

    let receipt = application
        .validate_provider_update(
            "successful-update".to_owned(),
            ProviderUpdateValidationInput {
                provider_id: original.id.clone(),
                base_url: compatible_server.base_url.clone(),
                api_key: Some("replacement-provider-key".to_owned()),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("updated combination validates");
    let providers = application.list_providers().expect("list providers");
    assert_eq!(providers.as_slice(), std::slice::from_ref(&original));
    let updated = application
        .save_provider_update(&receipt.validation_id, &original.id, "Update Target")
        .expect("save validated non-current update");
    assert_eq!(updated.id, original.id);

    let secret = application
        .reveal_provider_api_key(&original.id)
        .expect("reveal API key after explicit request");
    let serialized = serde_json::to_value(secret).expect("serialize API key response");
    assert!(
        serialized["value"]
            .as_str()
            .is_some_and(|value| value == "replacement-provider-key")
    );

    let revalidated = application
        .revalidate_provider("manual-revalidation".to_owned(), original.id.clone())
        .await
        .expect("manual revalidation succeeds");
    assert_eq!(revalidated.provider.id, original.id);
    assert!(revalidated.validation_receipt.is_none());

    mark_current_provider(&store, &original.id);
    let current_receipt = application
        .validate_provider_update(
            "current-update".to_owned(),
            ProviderUpdateValidationInput {
                provider_id: original.id.clone(),
                base_url: compatible_server.base_url,
                api_key: Some("current-replacement-key".to_owned()),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("current provider combination can be validated");
    let apply_required = application
        .save_provider_update(
            &current_receipt.validation_id,
            &original.id,
            "Update Target",
        )
        .expect_err("current provider update cannot change only the catalog");
    assert_eq!(
        apply_required.category,
        ProviderFailureCategory::SaveAndApplyRequired
    );
    let still_current = application.list_providers().expect("list providers");
    assert_eq!(still_current.len(), 1);
    assert_eq!(still_current[0].id, original.id);
    assert!(still_current[0].is_current);
}

#[tokio::test]
async fn current_provider_update_commits_catalog_and_codex_artifacts_together() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store.clone(), validator());
    let creation_server = ValidationServer::start(ValidationScenario::Success);
    let original = create_provider(
        &application,
        "create-current-target",
        creation_server.base_url,
        "Current Provider",
        "original-current-key",
    )
    .await;
    let codex_home = temp.path().join(".codex");
    let environment = EnvironmentApplication::new(store.clone(), &codex_home);
    environment
        .apply_provider(&original.id, true)
        .expect("apply original provider");
    let original_config = std::fs::read(codex_home.join("config.toml")).expect("read old config");
    let original_auth = std::fs::read(codex_home.join("auth.json")).expect("read old auth");

    let update_server = ValidationServer::start(ValidationScenario::Success);
    let receipt = application
        .validate_provider_update(
            "validate-current-update".to_owned(),
            ProviderUpdateValidationInput {
                provider_id: original.id.clone(),
                base_url: update_server.base_url.clone(),
                api_key: Some("replacement-current-key".to_owned()),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("updated current combination validates");
    let confirmation_required = application
        .save_and_apply_provider_update(
            &environment,
            &receipt.validation_id,
            &original.id,
            "Updated Current Provider",
            false,
        )
        .expect_err("current provider update must require consumer confirmation");
    assert_eq!(
        confirmation_required.category,
        ProviderFailureCategory::SaveAndApplyFailed
    );
    assert_eq!(
        confirmation_required.message_id,
        "environment.consumer_confirmation_required"
    );
    let failing_environment = EnvironmentApplication::with_fault_injector(
        store.clone(),
        &codex_home,
        Arc::new(FailBeforeDatabaseCommit),
    );

    let failure = application
        .save_and_apply_provider_update(
            &failing_environment,
            &receipt.validation_id,
            &original.id,
            "Updated Current Provider",
            true,
        )
        .expect_err("database commit failure must roll back the whole update");

    assert_eq!(
        failure.category,
        ProviderFailureCategory::SaveAndApplyFailed
    );
    assert_eq!(
        std::fs::read(codex_home.join("config.toml")).expect("read rolled back config"),
        original_config
    );
    assert_eq!(
        std::fs::read(codex_home.join("auth.json")).expect("read rolled back auth"),
        original_auth
    );
    let unchanged = application
        .list_providers()
        .expect("list unchanged provider");
    assert_eq!(unchanged.len(), 1);
    assert_eq!(unchanged[0].id, original.id);
    assert_eq!(unchanged[0].name, original.name);
    assert_eq!(unchanged[0].base_url, original.base_url);
    assert_eq!(unchanged[0].default_model, original.default_model);
    assert_eq!(
        unchanged[0].verified_at_epoch_seconds,
        original.verified_at_epoch_seconds
    );
    assert!(unchanged[0].is_current);

    let updated = application
        .save_and_apply_provider_update(
            &environment,
            &receipt.validation_id,
            &original.id,
            "Updated Current Provider",
            true,
        )
        .expect("retry save and apply");
    let updated = updated.provider;

    assert_eq!(updated.id, original.id);
    assert_eq!(updated.name, "Updated Current Provider");
    assert_eq!(updated.base_url, format!("{}/", update_server.base_url));
    assert!(updated.is_current);
    let config =
        std::fs::read_to_string(codex_home.join("config.toml")).expect("read updated config");
    let document = config
        .parse::<toml_edit::DocumentMut>()
        .expect("updated config is TOML");
    assert_eq!(
        document["model_providers"][&original.id]["base_url"].as_str(),
        Some(updated.base_url.as_str())
    );
    let auth: Value = serde_json::from_slice(
        &std::fs::read(codex_home.join("auth.json")).expect("read updated auth"),
    )
    .expect("updated auth is JSON");
    assert_eq!(auth["OPENAI_API_KEY"], "replacement-current-key");
}

struct FailBeforeDatabaseCommit;

impl EnvironmentFaultInjector for FailBeforeDatabaseCommit {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool {
        point == EnvironmentFailurePoint::BeforeDatabaseCommit
    }
}

#[tokio::test]
async fn editing_connection_fields_can_discover_models_without_revealing_the_saved_key() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store, validator());
    let creation_server = ValidationServer::start(ValidationScenario::Success);
    let provider = create_provider(
        &application,
        "create-discovery-target",
        creation_server.base_url,
        "Discovery Target",
        "saved-provider-key",
    )
    .await;
    let discovery_server =
        ModelServer::start("200 OK", r#"{"object":"list","data":[{"id":"model-b"}]}"#);

    let discovery = application
        .discover_models_for_update(
            "update-discovery".to_owned(),
            ProviderUpdateDiscoveryInput {
                provider_id: provider.id,
                base_url: discovery_server.base_url,
                api_key: None,
            },
        )
        .await
        .expect("saved API key can be used without exposing it to the UI");

    assert_eq!(discovery.models, ["model-b"]);
    let request = discovery_server
        .request
        .recv()
        .expect("captured models request");
    assert!(request.contains("authorization: Bearer saved-provider-key"));
}

#[tokio::test]
async fn validated_update_expires_after_a_concurrent_rename() {
    let temp = TempDir::new().expect("temp state directory");
    let store = StateStore::new(StatePaths::from_root(temp.path()));
    assert!(store.bootstrap().is_ready());
    let application = ProviderApplication::new(store, validator());
    let compatible_server = ValidationServer::start_for(ValidationScenario::Success, 2);
    let provider = create_provider(
        &application,
        "create-concurrent-target",
        compatible_server.base_url.clone(),
        "Before Rename",
        "original-provider-key",
    )
    .await;
    let receipt = application
        .validate_provider_update(
            "validate-before-rename".to_owned(),
            ProviderUpdateValidationInput {
                provider_id: provider.id.clone(),
                base_url: compatible_server.base_url,
                api_key: Some("replacement-provider-key".to_owned()),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("updated combination validates");

    application
        .rename_provider(&provider.id, "After Rename")
        .expect("concurrent rename succeeds");
    let expired = application
        .save_provider_update(&receipt.validation_id, &provider.id, "Before Rename")
        .expect_err("stale update cannot overwrite a concurrent rename");

    assert_eq!(
        expired.category,
        ProviderFailureCategory::VerificationExpired
    );
    let remaining = application.list_providers().expect("list providers");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].name, "After Rename");
}

async fn create_provider(
    application: &ProviderApplication,
    request_id: &str,
    base_url: String,
    name: &str,
    api_key: &str,
) -> gpteasy_lib::provider::ProviderSummary {
    let receipt = application
        .validate_provider(
            request_id.to_owned(),
            ProviderValidationInput {
                base_url,
                api_key: api_key.to_owned(),
                default_model: "model-a".to_owned(),
            },
        )
        .await
        .expect("provider validates");
    application
        .save_verified_provider(&receipt.validation_id, name)
        .expect("save provider")
}

fn mark_current_provider(store: &StateStore, provider_id: &str) {
    let connection = rusqlite::Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO last_applied_state (\
                singleton, mode, provider_id, config_fingerprint, credentials_fingerprint, applied_at\
             ) VALUES (1, 'provider', ?1, 'config', 'credentials', '1')",
            [provider_id],
        )
        .expect("mark current provider");
}

fn insert_provider_record(
    store: &StateStore,
    provider_id: &str,
    base_url: &str,
    api_key: &str,
    verified_at: u64,
) {
    let connection = Connection::open(store.paths().database()).expect("open state");
    connection
        .execute(
            "INSERT INTO providers (id, name, base_url, api_key, default_model, verified_at, verification_fingerprint, sort_order)
             VALUES (?1, 'Candidate Revalidation', ?2, ?3, 'model-a', ?4, 'original-fingerprint', 0)",
            rusqlite::params![provider_id, base_url, api_key, verified_at.to_string()],
        )
        .expect("insert saved provider fixture");
}

struct ModelServer {
    base_url: String,
    request: mpsc::Receiver<String>,
}

impl ModelServer {
    fn start(status: &'static str, body: &'static str) -> Self {
        Self::start_on("127.0.0.1", "127.0.0.1", status, body)
    }

    fn start_on(bind_host: &str, url_host: &str, status: &'static str, body: &'static str) -> Self {
        let listener = TcpListener::bind((bind_host, 0)).expect("bind mock provider");
        let address = listener.local_addr().expect("mock provider address");
        let (sender, request) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept models request");
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).expect("read models request");
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            sender
                .send(String::from_utf8_lossy(&bytes).to_string())
                .expect("capture models request");
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("write models response");
            stream.flush().expect("flush models response");
        });
        Self {
            base_url: format!("http://{url_host}:{}", address.port()),
            request,
        }
    }
}

struct ScriptedModelServer {
    base_url: String,
    requests: mpsc::Receiver<Vec<String>>,
    worker: thread::JoinHandle<()>,
}

impl ScriptedModelServer {
    fn start<const N: usize>(responses: [(&'static str, &'static str); N]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind scripted provider");
        let address = listener.local_addr().expect("scripted provider address");
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut paths = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().expect("accept scripted request");
                let request = read_request(&mut stream);
                let request_line = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .expect("request path")
                    .to_owned();
                paths.push(path);
                write_json(&mut stream, status, body);
            }
            sender.send(paths).expect("capture scripted paths");
        });
        Self {
            base_url: format!("http://{address}"),
            requests,
            worker,
        }
    }

    fn finish(self) -> Vec<String> {
        self.worker.join().expect("scripted provider worker");
        self.requests.recv().expect("scripted request paths")
    }
}

struct DelayedModelServer {
    base_url: String,
    requests: mpsc::Receiver<Vec<String>>,
    worker: thread::JoinHandle<()>,
}

impl DelayedModelServer {
    fn start(delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind delayed provider");
        let address = listener.local_addr().expect("delayed provider address");
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept delayed request");
            let request = read_request(&mut stream);
            let path = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("delayed request path")
                .to_owned();
            sender.send(vec![path]).expect("capture delayed path");
            thread::sleep(delay);
            write_json(&mut stream, "200 OK", r#"{"object":"list","data":[]}"#);
        });
        Self {
            base_url: format!("http://{address}"),
            requests,
            worker,
        }
    }

    fn finish(self) -> Vec<String> {
        self.worker.join().expect("delayed provider worker");
        self.requests.recv().expect("delayed request paths")
    }
}

struct PathFixValidationServer {
    base_url: String,
    requests: mpsc::Receiver<Vec<String>>,
    worker: thread::JoinHandle<()>,
}

impl PathFixValidationServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind path-fix provider");
        let address = listener.local_addr().expect("path-fix provider address");
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut paths = Vec::new();
            for index in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept path-fix request");
                let request = read_request(&mut stream);
                let path = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("path-fix request path")
                    .to_owned();
                paths.push(path);
                match index {
                    0 => write_json(
                        &mut stream,
                        "404 Not Found",
                        r#"{"error":"missing endpoint"}"#,
                    ),
                    1 => write_json(
                        &mut stream,
                        "200 OK",
                        r#"{"object":"list","data":[{"id":"model-a"}]}"#,
                    ),
                    _ => {
                        let payload: Value = serde_json::from_slice(request_body(&request))
                            .expect("path-fix validation payload");
                        write_validation_sse(&mut stream, ValidationScenario::Success, &payload);
                    }
                }
            }
            sender.send(paths).expect("capture path-fix requests");
        });
        Self {
            base_url: format!("http://{address}"),
            requests,
            worker,
        }
    }

    fn finish(self) -> Vec<String> {
        self.worker.join().expect("path-fix provider worker");
        self.requests.recv().expect("path-fix request paths")
    }
}

#[derive(Clone, Copy)]
enum ValidationScenario {
    Success,
    Truncated,
    ExtraArgument,
    WrongNonce,
    WrongFinalNonce,
    MultipleToolCalls,
    SecondRoundToolCall,
    IdleBeforeFirstEvent,
}

struct ValidationServer {
    base_url: String,
    requests: mpsc::Receiver<Vec<Vec<u8>>>,
}

impl ValidationServer {
    fn start(scenario: ValidationScenario) -> Self {
        Self::start_for(scenario, 1)
    }

    fn start_for(scenario: ValidationScenario, repetitions: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind validation provider");
        let address = listener.local_addr().expect("validation provider address");
        let (sender, requests) = mpsc::channel();
        thread::spawn(move || {
            let mut captured = Vec::new();
            let request_count = match scenario {
                ValidationScenario::Truncated
                | ValidationScenario::ExtraArgument
                | ValidationScenario::WrongNonce
                | ValidationScenario::MultipleToolCalls
                | ValidationScenario::IdleBeforeFirstEvent => 2,
                ValidationScenario::Success
                | ValidationScenario::WrongFinalNonce
                | ValidationScenario::SecondRoundToolCall => 3,
            };
            for index in 0..(request_count * repetitions) {
                let (mut stream, _) = listener.accept().expect("accept validation request");
                let request = read_request(&mut stream);
                if index % request_count == 0 {
                    write_json(
                        &mut stream,
                        "200 OK",
                        r#"{"object":"list","data":[{"id":"model-a"}]}"#,
                    );
                } else {
                    let payload: Value =
                        serde_json::from_slice(request_body(&request)).expect("validation payload");
                    write_validation_sse(&mut stream, scenario, &payload);
                }
                captured.push(request);
            }
            let _ = sender.send(captured);
        });
        Self {
            base_url: format!("http://{address}"),
            requests,
        }
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set request read timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    let mut expected = None;
    loop {
        let count = stream.read(&mut chunk).expect("read validation request");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if expected.is_none() {
            if let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                expected = Some(header_end + 4 + content_length);
            }
        }
        if expected.is_some_and(|length| bytes.len() >= length) {
            break;
        }
    }
    bytes
}

fn request_body(request: &[u8]) -> &[u8] {
    find_bytes(request, b"\r\n\r\n")
        .map(|index| &request[index + 4..])
        .unwrap_or_default()
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_json(stream: &mut std::net::TcpStream, status: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write JSON response");
    stream.flush().expect("flush JSON response");
}

fn write_validation_sse(
    stream: &mut std::net::TcpStream,
    scenario: ValidationScenario,
    payload: &Value,
) {
    if matches!(scenario, ValidationScenario::IdleBeforeFirstEvent) {
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
            )
            .expect("write idle SSE headers");
        stream.flush().expect("flush idle SSE headers");
        thread::sleep(Duration::from_millis(250));
        return;
    }
    let has_tool_output = payload["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    let events = if has_tool_output {
        let output = payload["input"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["type"] == "function_call_output")
            })
            .and_then(|item| item["output"].as_str())
            .and_then(|output| serde_json::from_str::<Value>(output).ok())
            .expect("valid function output");
        let nonce = if matches!(scenario, ValidationScenario::WrongFinalNonce) {
            "wrong-final-nonce"
        } else {
            output["nonce"].as_str().expect("output nonce")
        };
        vec![
            sse_event(
                "response.output_text.delta",
                json!({"delta": format!("validated {nonce}")}),
            ),
            if matches!(scenario, ValidationScenario::SecondRoundToolCall) {
                sse_event(
                    "response.output_item.done",
                    json!({"item":{"type":"function_call","id":"unexpected-call-item","call_id":"unexpected-call-001","name":"gpteasy_probe","arguments":output.to_string()}}),
                )
            } else {
                String::new()
            },
            sse_event(
                "response.completed",
                json!({"response":{"id":"response-final"}}),
            ),
        ]
    } else {
        let requested_nonce = payload["input"][0]["content"][0]["text"]
            .as_str()
            .and_then(extract_backtick_value)
            .expect("requested nonce");
        let arguments = match scenario {
            ValidationScenario::ExtraArgument => {
                json!({"nonce": requested_nonce, "unexpected": true}).to_string()
            }
            ValidationScenario::WrongNonce => json!({"nonce": "wrong-nonce"}).to_string(),
            _ => json!({"nonce": requested_nonce}).to_string(),
        };
        let mut events = vec![
            sse_event(
                "response.function_call_arguments.delta",
                json!({"item_id":"call-item","delta":arguments}),
            ),
            sse_event(
                "response.output_item.done",
                json!({"item":{"type":"function_call","id":"call-item","call_id":"call-001","name":"gpteasy_probe","arguments":arguments}}),
            ),
        ];
        if !matches!(scenario, ValidationScenario::Truncated) {
            if matches!(scenario, ValidationScenario::MultipleToolCalls) {
                events.push(sse_event(
                    "response.output_item.done",
                    json!({"item":{"type":"function_call","id":"call-item-2","call_id":"call-002","name":"gpteasy_probe","arguments":arguments}}),
                ));
            }
            events.push(sse_event(
                "response.completed",
                json!({"response":{"id":"response-tool"}}),
            ));
        }
        events
    };
    let body = events.join("");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
    )
    .expect("write SSE headers");
    for fragment in body.as_bytes().chunks(7) {
        write!(stream, "{:X}\r\n", fragment.len()).expect("write chunk length");
        stream.write_all(fragment).expect("write SSE fragment");
        stream.write_all(b"\r\n").expect("write chunk boundary");
        stream.flush().expect("flush SSE fragment");
    }
    stream.write_all(b"0\r\n\r\n").expect("finish SSE body");
    stream.flush().expect("flush SSE body");
}

fn sse_event(kind: &str, mut payload: Value) -> String {
    payload
        .as_object_mut()
        .expect("event object")
        .insert("type".to_owned(), Value::String(kind.to_owned()));
    format!("event: {kind}\ndata: {payload}\n\n")
}

fn extract_backtick_value(text: &str) -> Option<&str> {
    let start = text.find('`')? + 1;
    let end = text[start..].find('`')? + start;
    Some(&text[start..end])
}
