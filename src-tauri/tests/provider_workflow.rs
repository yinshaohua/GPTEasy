use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gpteasy_lib::provider::{
    DiscoveryInput, ProviderApplication, ProviderFailureCategory, ProviderValidationInput,
    ProviderValidator, ValidationTimeouts,
};
use gpteasy_lib::state::{StatePaths, StateStore};
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
            for index in 0..request_count {
                let (mut stream, _) = listener.accept().expect("accept validation request");
                let request = read_request(&mut stream);
                if index == 0 {
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
