use reqwest::{
    blocking::{Client, Response},
    header::{CONTENT_TYPE, RETRY_AFTER},
    StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;

const KEY_ENV: &str = "GPTEASY_PROVIDER_KEY";

#[derive(Clone, Copy)]
struct TimeoutPolicy {
    connect: Duration,
    read: Duration,
    overall: Duration,
}

impl TimeoutPolicy {
    fn fast() -> Self {
        Self {
            connect: Duration::from_millis(500),
            read: Duration::from_millis(500),
            overall: Duration::from_millis(1200),
        }
    }

    fn live() -> Self {
        Self {
            connect: Duration::from_secs(10),
            read: Duration::from_secs(30),
            overall: Duration::from_secs(120),
        }
    }
}

#[derive(Deserialize)]
struct ProviderSecret {
    base_url: String,
    api_key: String,
    model: String,
}

fn run_live(secret_file: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let secret_path = fs::canonicalize(secret_file)?;
    let repository = fs::canonicalize(env::current_dir()?)?;
    if secret_path.starts_with(&repository) {
        let ignored = Command::new("git")
            .arg("-C")
            .arg(&repository)
            .arg("check-ignore")
            .arg("--quiet")
            .arg("--")
            .arg(&secret_path)
            .status()?
            .success();
        if !ignored {
            return Err("repository-local provider secret is not ignored by Git".into());
        }
    }
    let secret: ProviderSecret = serde_json::from_slice(&fs::read(&secret_path)?)?;
    if secret.api_key.trim().is_empty()
        || secret.base_url.trim().is_empty()
        || secret.model.trim().is_empty()
    {
        return Err("provider secret requires non-empty base_url, api_key, and model".into());
    }
    fs::create_dir_all(output)?;
    let log_file = output.join("client.jsonl");
    let _ = fs::remove_file(&log_file);
    let result = validate(
        &secret.base_url,
        &secret.api_key,
        &secret.model,
        &log_file,
        TimeoutPolicy::live(),
    );
    let provider_host = Url::parse(&secret.base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string));
    let preliminary = json!({
        "provider_host": provider_host,
        "model": secret.model,
        "validation": result,
        "secret_leak_scan_passed": true
    });
    let result_file = output.join("result.json");
    fs::write(&result_file, serde_json::to_vec_pretty(&preliminary)?)?;
    let leak_free = !directory_contains(output, secret.api_key.as_bytes())?;
    let report = json!({
        "provider_host": provider_host,
        "model": secret.model,
        "validation": preliminary["validation"],
        "secret_leak_scan_passed": leak_free
    });
    fs::write(&result_file, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !leak_free {
        return Err("provider API key was found in live output artifacts".into());
    }
    Ok(())
}

fn directory_contains(root: &Path, needle: &[u8]) -> Result<bool, Box<dyn std::error::Error>> {
    if needle.is_empty() {
        return Ok(false);
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            if directory_contains(&path, needle)? {
                return Ok(true);
            }
        } else if path.is_file() {
            let bytes = fs::read(path)?;
            if bytes.windows(needle.len()).any(|window| window == needle) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn main() {
    let result = run();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("validate") => {
            let base_url = args.next().ok_or("validate requires base URL")?;
            let model = args.next().ok_or("validate requires model")?;
            let log_file = PathBuf::from(args.next().ok_or("validate requires log file")?);
            let timeout_policy = match args.next().as_deref() {
                Some("fast") => TimeoutPolicy::fast(),
                Some("live") | None => TimeoutPolicy::live(),
                Some(other) => return Err(format!("unsupported timeout policy `{other}`").into()),
            };
            let key = env::var(KEY_ENV).map_err(|_| format!("missing {KEY_ENV}"))?;
            let result = validate(&base_url, &key, &model, &log_file, timeout_policy);
            println!("{}", serde_json::to_string_pretty(&result)?);
            if !result.ok {
                std::process::exit(1);
            }
        }
        Some("live") => {
            let secret_file = PathBuf::from(args.next().ok_or("live requires secret JSON path")?);
            let output = PathBuf::from(args.next().ok_or("live requires output directory")?);
            run_live(&secret_file, &output)?;
        }
        Some("mock") => {
            let scenario = args.next().ok_or("mock requires scenario")?;
            let port_file = PathBuf::from(args.next().ok_or("mock requires port file")?);
            let log_file = PathBuf::from(args.next().ok_or("mock requires log file")?);
            serve_mock(&scenario, &port_file, &log_file)?;
        }
        _ => {
            return Err(
                "usage: real-provider-compatibility-matrix <validate BASE_URL MODEL LOG_FILE [fast|live]|live SECRET_JSON OUTPUT_DIR|mock SCENARIO PORT_FILE LOG_FILE>"
                    .into(),
            );
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ValidationResult {
    ok: bool,
    category: String,
    message: String,
    stages: Vec<StageResult>,
}

#[derive(Debug, Serialize)]
struct StageResult {
    name: String,
    ok: bool,
    duration_ms: u128,
    details: Value,
}

#[derive(Debug)]
struct ValidationFailure {
    category: &'static str,
    message: String,
}

impl ValidationFailure {
    fn new(category: &'static str, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }
}

fn validate(
    base_url: &str,
    key: &str,
    model: &str,
    log_file: &Path,
    timeouts: TimeoutPolicy,
) -> ValidationResult {
    let mut stages = Vec::new();
    let overall = (|| -> Result<(), ValidationFailure> {
        let normalized = validate_url_policy(base_url)?;
        record_stage(
            &mut stages,
            log_file,
            "url_policy",
            true,
            0,
            json!({"normalized_base_url": normalized.as_str()}),
        );

        let client = Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.overall)
            .build()
            .map_err(|error| ValidationFailure::new("transport", error.to_string()))?;

        validate_model_discovery(&client, &normalized, key, model, log_file, &mut stages)?;
        validate_tool_loop(
            &client,
            &normalized,
            key,
            model,
            log_file,
            &mut stages,
            timeouts,
        )?;
        Ok(())
    })();

    match overall {
        Ok(()) => ValidationResult {
            ok: true,
            category: "validated".to_string(),
            message: "model discovery, Responses SSE, and tool-call loop succeeded".to_string(),
            stages,
        },
        Err(error) => ValidationResult {
            ok: false,
            category: error.category.to_string(),
            message: error.message,
            stages,
        },
    }
}

fn validate_url_policy(base_url: &str) -> Result<Url, ValidationFailure> {
    let mut url = Url::parse(base_url)
        .map_err(|error| ValidationFailure::new("security_policy", error.to_string()))?;
    while url.path().ends_with('/') {
        let trimmed = url.path().trim_end_matches('/').to_string();
        url.set_path(&trimmed);
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback(&url) => Ok(url),
        "http" => Err(ValidationFailure::new(
            "security_policy",
            "remote provider must use HTTPS; HTTP is allowed only for loopback addresses",
        )),
        other => Err(ValidationFailure::new(
            "security_policy",
            format!("unsupported URL scheme: {other}"),
        )),
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false),
        None => false,
    }
}

fn validate_model_discovery(
    client: &Client,
    base_url: &Url,
    key: &str,
    model: &str,
    log_file: &Path,
    stages: &mut Vec<StageResult>,
) -> Result<(), ValidationFailure> {
    let start = Instant::now();
    let url = endpoint(base_url, "models")?;
    let response = client
        .get(url)
        .bearer_auth(key)
        .send()
        .map_err(|error| ValidationFailure::new("transport", error.to_string()))?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = response
        .text()
        .map_err(|error| ValidationFailure::new("transport", error.to_string()))?;

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        record_stage(
            stages,
            log_file,
            "model_discovery",
            false,
            start.elapsed().as_millis(),
            json!({"status": status.as_u16(), "body_length": body.len()}),
        );
        return Err(ValidationFailure::new(
            "authentication",
            format!("model discovery returned HTTP {}", status.as_u16()),
        ));
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        record_stage(
            stages,
            log_file,
            "model_discovery",
            false,
            start.elapsed().as_millis(),
            json!({
                "status": status.as_u16(),
                "retry_after": retry_after,
                "body_length": body.len()
            }),
        );
        return Err(ValidationFailure::new(
            "rate_limit",
            "model discovery returned HTTP 429",
        ));
    }
    if !status.is_success() {
        record_stage(
            stages,
            log_file,
            "model_discovery",
            false,
            start.elapsed().as_millis(),
            json!({"status": status.as_u16(), "body_length": body.len()}),
        );
        return Err(ValidationFailure::new(
            "model_discovery",
            format!("model discovery returned HTTP {}", status.as_u16()),
        ));
    }

    let json: Value = serde_json::from_str(&body).map_err(|error| {
        ValidationFailure::new("model_discovery", format!("invalid models JSON: {error}"))
    })?;
    let ids: Vec<&str> = json
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .collect();
    let found = ids.contains(&model);
    record_stage(
        stages,
        log_file,
        "model_discovery",
        found,
        start.elapsed().as_millis(),
        json!({"status": status.as_u16(), "model": model, "available_count": ids.len(), "found": found}),
    );
    if !found {
        return Err(ValidationFailure::new(
            "model_discovery",
            format!("default model `{model}` was not returned by /models"),
        ));
    }
    Ok(())
}

fn validate_tool_loop(
    client: &Client,
    base_url: &Url,
    key: &str,
    model: &str,
    log_file: &Path,
    stages: &mut Vec<StageResult>,
    timeouts: TimeoutPolicy,
) -> Result<(), ValidationFailure> {
    let nonce = format!(
        "gpteasy-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let user_input = json!({
        "role": "user",
        "content": [{
            "type": "input_text",
            "text": format!("Call gpteasy_probe exactly once with nonce `{nonce}`. After receiving the tool output, reply with the nonce.")
        }]
    });
    let tool = json!({
        "type": "function",
        "name": "gpteasy_probe",
        "description": "Returns a supplied validation nonce.",
        "strict": true,
        "parameters": {
            "type": "object",
            "properties": {"nonce": {"type": "string"}},
            "required": ["nonce"],
            "additionalProperties": false
        }
    });
    let first_payload = json!({
        "model": model,
        "input": [user_input.clone()],
        "tools": [tool.clone()],
        "tool_choice": {"type": "function", "name": "gpteasy_probe"},
        "parallel_tool_calls": false,
        "stream": true
    });

    let first_start = Instant::now();
    let first = post_sse(client, base_url, key, &first_payload, timeouts)?;
    let call = first.function_call.ok_or_else(|| {
        ValidationFailure::new(
            "tool_call",
            "stream completed without a gpteasy_probe function call",
        )
    })?;
    if call.name != "gpteasy_probe" {
        return Err(ValidationFailure::new(
            "tool_call",
            format!("unexpected function name `{}`", call.name),
        ));
    }
    let arguments: Value = serde_json::from_str(&call.arguments).map_err(|error| {
        ValidationFailure::new(
            "tool_call",
            format!("invalid function arguments JSON: {error}"),
        )
    })?;
    if arguments.get("nonce").and_then(Value::as_str) != Some(nonce.as_str()) {
        return Err(ValidationFailure::new(
            "tool_call",
            "function call nonce did not match the requested nonce",
        ));
    }
    record_stage(
        stages,
        log_file,
        "responses_stream_and_tool_call",
        true,
        first_start.elapsed().as_millis(),
        json!({
            "content_type": first.content_type,
            "events": first.events,
            "saw_completed": first.saw_completed,
            "first_event_ms": first.first_event_ms,
            "function_name": call.name,
            "call_id": call.call_id,
        }),
    );

    let function_call_item = json!({
        "type": "function_call",
        "call_id": call.call_id,
        "name": call.name,
        "arguments": call.arguments
    });
    let function_output = json!({
        "type": "function_call_output",
        "call_id": call.call_id,
        "output": json!({"ok": true, "nonce": nonce}).to_string()
    });
    let second_payload = json!({
        "model": model,
        "input": [user_input, function_call_item, function_output],
        "tools": [tool],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "stream": true
    });
    let second_start = Instant::now();
    let second = post_sse(client, base_url, key, &second_payload, timeouts)?;
    let final_text = second.output_text.join("");
    let ok = second.saw_completed && final_text.contains(&nonce);
    record_stage(
        stages,
        log_file,
        "tool_result_round_trip",
        ok,
        second_start.elapsed().as_millis(),
        json!({
            "content_type": second.content_type,
            "events": second.events,
            "saw_completed": second.saw_completed,
            "first_event_ms": second.first_event_ms,
            "output_contains_nonce": final_text.contains(&nonce),
            "output_length": final_text.len()
        }),
    );
    if !ok {
        return Err(ValidationFailure::new(
            "tool_result",
            "provider did not complete a final streamed answer containing the tool nonce",
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct SseResult {
    content_type: String,
    events: Vec<String>,
    saw_completed: bool,
    first_event_ms: Option<u128>,
    function_call: Option<FunctionCall>,
    output_text: Vec<String>,
}

#[derive(Debug)]
struct FunctionCall {
    call_id: String,
    name: String,
    arguments: String,
}

enum StreamRead {
    Line(String),
    Eof,
    Error(String),
}

fn post_sse(
    client: &Client,
    base_url: &Url,
    key: &str,
    payload: &Value,
    timeouts: TimeoutPolicy,
) -> Result<SseResult, ValidationFailure> {
    let start = Instant::now();
    let url = endpoint(base_url, "responses")?;
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(payload)
        .send()
        .map_err(|error| {
            let category = if error.is_timeout() {
                "response_header_timeout"
            } else {
                "transport"
            };
            ValidationFailure::new(category, error.to_string())
        })?;
    classify_response_status(&response)?;
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if !content_type
        .to_ascii_lowercase()
        .starts_with("text/event-stream")
    {
        return Err(ValidationFailure::new(
            "streaming",
            format!("expected text/event-stream, got `{content_type}`"),
        ));
    }

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(response);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = sender.send(StreamRead::Eof);
                    break;
                }
                Ok(_) => {
                    if sender.send(StreamRead::Line(line)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(StreamRead::Error(error.to_string()));
                    break;
                }
            }
        }
    });
    let mut event_type = String::new();
    let mut events = Vec::new();
    let mut first_event_ms = None;
    let mut saw_completed = false;
    let mut function_call = None;
    let mut output_text = Vec::new();

    loop {
        let elapsed = start.elapsed();
        if elapsed >= timeouts.overall {
            return Err(ValidationFailure::new(
                "overall_timeout",
                "Responses stream exceeded the overall deadline",
            ));
        }
        let remaining = timeouts.overall.saturating_sub(elapsed);
        let wait = remaining.min(timeouts.read);
        let line = match receiver.recv_timeout(wait) {
            Ok(StreamRead::Line(line)) => line,
            Ok(StreamRead::Eof) => break,
            Ok(StreamRead::Error(error)) => {
                return Err(ValidationFailure::new("streaming", error));
            }
            Err(RecvTimeoutError::Timeout) => {
                let category = if start.elapsed() >= timeouts.overall {
                    "overall_timeout"
                } else if events.is_empty() {
                    "first_event_timeout"
                } else {
                    "stream_idle_timeout"
                };
                return Err(ValidationFailure::new(
                    category,
                    format!("Responses stream did not produce data within {wait:?}"),
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(ValidationFailure::new(
                    "streaming",
                    "Responses stream reader disconnected unexpectedly",
                ));
            }
        };
        if start.elapsed() >= timeouts.overall {
            return Err(ValidationFailure::new(
                "overall_timeout",
                "Responses stream exceeded the overall deadline",
            ));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if let Some(value) = trimmed.strip_prefix("event:") {
            event_type = value.trim().to_string();
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("data:") {
            let parsed: Value = serde_json::from_str(value.trim()).map_err(|error| {
                ValidationFailure::new("responses_protocol", format!("invalid SSE JSON: {error}"))
            })?;
            let kind = parsed
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or(event_type.as_str())
                .to_string();
            if first_event_ms.is_none() {
                first_event_ms = Some(start.elapsed().as_millis());
            }
            events.push(kind.clone());
            if kind == "response.completed" {
                saw_completed = true;
            }
            if kind == "response.output_text.delta" {
                if let Some(delta) = parsed.get("delta").and_then(Value::as_str) {
                    output_text.push(delta.to_string());
                }
            }
            if kind == "response.output_item.done" {
                if let Some(item) = parsed.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        function_call = Some(FunctionCall {
                            call_id: item
                                .get("call_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            name: item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            arguments: item
                                .get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                    if item.get("type").and_then(Value::as_str) == Some("message") {
                        if let Some(content) = item.get("content").and_then(Value::as_array) {
                            for part in content {
                                if let Some(text) = part.get("text").and_then(Value::as_str) {
                                    output_text.push(text.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if !saw_completed {
        return Err(ValidationFailure::new(
            "streaming",
            "SSE connection closed before response.completed",
        ));
    }
    Ok(SseResult {
        content_type,
        events,
        saw_completed,
        first_event_ms,
        function_call,
        output_text,
    })
}

fn classify_response_status(response: &Response) -> Result<(), ValidationFailure> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(ValidationFailure::new(
            "authentication",
            format!("Responses API returned HTTP {}", status.as_u16()),
        ));
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unspecified");
        return Err(ValidationFailure::new(
            "rate_limit",
            format!("Responses API returned HTTP 429; retry_after={retry_after}"),
        ));
    }
    Err(ValidationFailure::new(
        "responses_protocol",
        format!("Responses API returned HTTP {}", status.as_u16()),
    ))
}

fn endpoint(base_url: &Url, suffix: &str) -> Result<Url, ValidationFailure> {
    let mut url = base_url.clone();
    let path = format!("{}/{}", url.path().trim_end_matches('/'), suffix);
    url.set_path(&path);
    Ok(url)
}

fn record_stage(
    stages: &mut Vec<StageResult>,
    log_file: &Path,
    name: &str,
    ok: bool,
    duration_ms: u128,
    details: Value,
) {
    let stage = StageResult {
        name: name.to_string(),
        ok,
        duration_ms,
        details,
    };
    let event = json!({
        "timestamp_unix": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        "category": "validation_stage",
        "name": stage.name,
        "ok": stage.ok,
        "duration_ms": stage.duration_ms,
        "details": stage.details
    });
    let _ = append_json_line(log_file, &event);
    stages.push(stage);
}

fn serve_mock(
    scenario: &str,
    port_file: &Path,
    log_file: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = port_file.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    fs::write(port_file, listener.local_addr()?.port().to_string())?;

    for connection in listener.incoming() {
        let mut stream = connection?;
        let request = read_http_request(&mut stream)?;
        let inspected = inspect_mock_request(&request);
        append_json_line(log_file, &inspected)?;
        let path = inspected["path"].as_str().unwrap_or_default();
        let authorized = inspected["authorization_matches"]
            .as_bool()
            .unwrap_or(false);

        if scenario == "auth-error" || !authorized {
            write_http_response(
                &mut stream,
                "401 Unauthorized",
                "application/json",
                r#"{"error":{"message":"invalid API key"}}"#,
            )?;
            continue;
        }
        if path == "/v1/models" {
            if scenario == "rate-limit" {
                write_http_response_with_headers(
                    &mut stream,
                    "429 Too Many Requests",
                    "application/json",
                    &[("retry-after", "2")],
                    r#"{"error":{"message":"rate limited"}}"#,
                )?;
                continue;
            }
            let models = if scenario == "model-missing" {
                json!({"object":"list","data":[{"id":"other-model","object":"model"}]})
            } else {
                json!({"object":"list","data":[{"id":"mock-model","object":"model"}]})
            };
            write_http_response(
                &mut stream,
                "200 OK",
                "application/json",
                &models.to_string(),
            )?;
            continue;
        }
        if path == "/v1/responses" {
            let has_output = inspected["has_function_call_output"]
                .as_bool()
                .unwrap_or(false);
            match scenario {
                "first-event-timeout" => {
                    write_sse_with_pause(&mut stream, Vec::new(), Duration::from_secs(2))?
                }
                "idle-timeout" => write_sse_with_pause(
                    &mut stream,
                    vec![event(
                        "response.created",
                        json!({"response":{"id":"idle-timeout"}}),
                    )],
                    Duration::from_secs(2),
                )?,
                "overall-timeout" => write_sse_until_overall_timeout(&mut stream)?,
                "non-sse" => write_http_response(
                    &mut stream,
                    "200 OK",
                    "application/json",
                    r#"{"id":"response-json","output":[]}"#,
                )?,
                "no-tool" => write_sse_events(
                    &mut stream,
                    vec![
                        event("response.created", json!({"response":{"id":"no-tool"}})),
                        event(
                            "response.output_item.done",
                            json!({"item":{"type":"message","role":"assistant","id":"msg","content":[{"type":"output_text","text":"no tool"}]}}),
                        ),
                        event(
                            "response.completed",
                            json!({"response":{"id":"no-tool","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
                        ),
                    ],
                )?,
                "truncated" => write_sse_events(
                    &mut stream,
                    vec![
                        event("response.created", json!({"response":{"id":"truncated"}})),
                        event(
                            "response.output_item.done",
                            json!({"item":{"type":"function_call","id":"call-item","call_id":"call-002","name":"gpteasy_probe","arguments":"{\"nonce\":\"truncated\"}"}}),
                        ),
                    ],
                )?,
                "bad-tool-args" => write_sse_events(
                    &mut stream,
                    vec![
                        event("response.created", json!({"response":{"id":"bad-args"}})),
                        event(
                            "response.output_item.done",
                            json!({"item":{"type":"function_call","id":"call-item","call_id":"call-002","name":"gpteasy_probe","arguments":"not-json"}}),
                        ),
                        event(
                            "response.completed",
                            json!({"response":{"id":"bad-args","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
                        ),
                    ],
                )?,
                _ if has_output => {
                    let nonce = inspected["tool_output_nonce"]
                        .as_str()
                        .unwrap_or("missing-nonce");
                    let text = format!("validated nonce: {nonce}");
                    write_sse_events(
                        &mut stream,
                        vec![
                            event("response.created", json!({"response":{"id":"final"}})),
                            event(
                                "response.output_item.added",
                                json!({"output_index":0,"item":{"type":"message","role":"assistant","id":"msg-final","content":[]}}),
                            ),
                            event(
                                "response.output_text.delta",
                                json!({"item_id":"msg-final","output_index":0,"content_index":0,"delta":text}),
                            ),
                            event(
                                "response.output_item.done",
                                json!({"output_index":0,"item":{"type":"message","role":"assistant","id":"msg-final","content":[{"type":"output_text","text":text}]}}),
                            ),
                            event(
                                "response.completed",
                                json!({"response":{"id":"final","usage":{"input_tokens":2,"output_tokens":2,"total_tokens":4}}}),
                            ),
                        ],
                    )?;
                }
                _ => {
                    let nonce = inspected["requested_nonce"]
                        .as_str()
                        .unwrap_or("missing-nonce");
                    let arguments = json!({"nonce": nonce}).to_string();
                    write_sse_events(
                        &mut stream,
                        vec![
                            event("response.created", json!({"response":{"id":"tool"}})),
                            event(
                                "response.output_item.added",
                                json!({"output_index":0,"item":{"type":"function_call","id":"call-item","call_id":"call-002","name":"gpteasy_probe","arguments":""}}),
                            ),
                            event(
                                "response.function_call_arguments.delta",
                                json!({"item_id":"call-item","output_index":0,"delta":arguments}),
                            ),
                            event(
                                "response.function_call_arguments.done",
                                json!({"item_id":"call-item","output_index":0,"name":"gpteasy_probe","arguments":arguments}),
                            ),
                            event(
                                "response.output_item.done",
                                json!({"output_index":0,"item":{"type":"function_call","id":"call-item","call_id":"call-002","name":"gpteasy_probe","arguments":arguments}}),
                            ),
                            event(
                                "response.completed",
                                json!({"response":{"id":"tool","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
                            ),
                        ],
                    )?;
                }
            }
        } else {
            write_http_response(
                &mut stream,
                "404 Not Found",
                "application/json",
                r#"{"error":{"message":"not found"}}"#,
            )?;
        }
    }
    Ok(())
}

fn event(kind: &str, mut value: Value) -> Value {
    value
        .as_object_mut()
        .expect("event payload object")
        .insert("type".to_string(), Value::String(kind.to_string()));
    value
}

fn write_sse_events(
    stream: &mut TcpStream,
    events: Vec<Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    write_sse_header(stream)?;
    for value in events {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
        write!(stream, "event: {kind}\ndata: {value}\n\n")?;
        stream.flush()?;
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

fn write_sse_header(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n",
    )?;
    stream.flush()?;
    Ok(())
}

fn write_sse_with_pause(
    stream: &mut TcpStream,
    events: Vec<Value>,
    pause: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    write_sse_header(stream)?;
    for value in events {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
        write!(stream, "event: {kind}\ndata: {value}\n\n")?;
        stream.flush()?;
    }
    thread::sleep(pause);
    Ok(())
}

fn write_sse_until_overall_timeout(
    stream: &mut TcpStream,
) -> Result<(), Box<dyn std::error::Error>> {
    write_sse_header(stream)?;
    for index in 0..12 {
        let value = event(
            "response.in_progress",
            json!({"sequence_number": index, "response":{"id":"overall-timeout"}}),
        );
        write!(stream, "event: response.in_progress\ndata: {value}\n\n")?;
        stream.flush()?;
        thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut data = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut header_end = None;
    let mut expected_body = 0usize;
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);
        if header_end.is_none() {
            if let Some(index) = find_subslice(&data, b"\r\n\r\n") {
                header_end = Some(index + 4);
                let header = String::from_utf8_lossy(&data[..index]);
                expected_body = header
                    .lines()
                    .find_map(|line| line.split_once(':'))
                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse().ok())
                    .unwrap_or(0);
            }
        }
        if let Some(end) = header_end {
            if data.len() >= end + expected_body {
                break;
            }
        }
    }
    Ok(data)
}

fn inspect_mock_request(data: &[u8]) -> Value {
    let header_index = find_subslice(data, b"\r\n\r\n").unwrap_or(data.len());
    let body_index = header_index.saturating_add(4).min(data.len());
    let header = String::from_utf8_lossy(&data[..header_index]);
    let first_line = header.lines().next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let authorization = header.lines().find_map(|line| {
        line.split_once(':')
            .filter(|(key, _)| key.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.trim())
    });
    let body: Value = serde_json::from_slice(&data[body_index..]).unwrap_or(Value::Null);
    let input = body
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let output = input
        .iter()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"));
    let tool_output_nonce = output
        .and_then(|item| item.get("output"))
        .and_then(Value::as_str)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|value| {
            value
                .get("nonce")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let requested_nonce = input
        .iter()
        .find_map(|item| item.get("content").and_then(Value::as_array))
        .and_then(|content| content.first())
        .and_then(|part| part.get("text"))
        .and_then(Value::as_str)
        .and_then(extract_backtick_value);
    json!({
        "timestamp_unix": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        "method": method,
        "path": path,
        "authorization_present": authorization.is_some(),
        "authorization_matches": authorization == Some("Bearer spike-provider-key"),
        "model": body.get("model").and_then(Value::as_str),
        "stream": body.get("stream").and_then(Value::as_bool),
        "has_function_call_output": output.is_some(),
        "requested_nonce": requested_nonce,
        "tool_output_nonce": tool_output_nonce,
    })
}

fn extract_backtick_value(text: &str) -> Option<String> {
    let start = text.find('`')? + 1;
    let end = text[start..].find('`')? + start;
    Some(text[start..end].to_string())
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    write_http_response_with_headers(stream, status, content_type, &[], body)
}

fn write_http_response_with_headers(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n",
        body.len(),
    )?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "connection: close\r\n\r\n{body}")?;
    stream.flush()?;
    Ok(())
}

fn append_json_line(path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{value}")?;
    Ok(())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
