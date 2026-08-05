use anyhow::{bail, Context, Result};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderSecretFile {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct ProviderInput {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationStage {
    pub name: String,
    pub ok: bool,
    pub duration_ms: u128,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationEvidence {
    pub ok: bool,
    pub category: String,
    pub provider_host: String,
    pub model: String,
    pub combination_fingerprint: String,
    pub stages: Vec<ValidationStage>,
}

#[derive(Debug, Clone)]
pub struct VerifiedProvider {
    pub input: ProviderInput,
    pub evidence: ValidationEvidence,
}

#[derive(Debug)]
struct FunctionCall {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Debug)]
struct SseResult {
    content_type: String,
    events: Vec<String>,
    saw_completed: bool,
    function_call: Option<FunctionCall>,
    output_text: String,
}

pub fn mock_verified_provider() -> VerifiedProvider {
    let input = ProviderInput {
        id: "provider-new".to_string(),
        name: "Validated Mock Provider".to_string(),
        base_url: "https://validated.example/v1".to_string(),
        api_key: "spike-012-fake-secret".to_string(),
        model: "new-model".to_string(),
    };
    let fingerprint = combination_fingerprint(&input);
    VerifiedProvider {
        evidence: ValidationEvidence {
            ok: true,
            category: "validated".to_string(),
            provider_host: "validated.example".to_string(),
            model: input.model.clone(),
            combination_fingerprint: fingerprint,
            stages: vec![
                ValidationStage {
                    name: "model_discovery".to_string(),
                    ok: true,
                    duration_ms: 5,
                    details: json!({"mode": "deterministic", "found": true}),
                },
                ValidationStage {
                    name: "responses_stream_and_tool_call".to_string(),
                    ok: true,
                    duration_ms: 8,
                    details: json!({"mode": "deterministic", "saw_completed": true}),
                },
                ValidationStage {
                    name: "tool_result_round_trip".to_string(),
                    ok: true,
                    duration_ms: 7,
                    details: json!({"mode": "deterministic", "nonce_round_trip": true}),
                },
            ],
        },
        input,
    }
}

pub fn load_secret(path: &Path) -> Result<ProviderInput> {
    if !path.exists() {
        bail!("provider secret file does not exist: {}", path.display());
    }
    let ignored = Command::new("git")
        .args(["check-ignore", "--quiet", "--"])
        .arg(path)
        .status()
        .context("run git check-ignore")?
        .success();
    if !ignored {
        bail!("provider secret file is not ignored by Git");
    }
    let secret: ProviderSecretFile =
        serde_json::from_slice(&fs::read(path)?).context("parse provider secret JSON")?;
    if secret.api_key.is_empty() || secret.model.is_empty() {
        bail!("provider secret fields must be non-empty");
    }
    Ok(ProviderInput {
        id: "provider-live".to_string(),
        name: "Live Validated Provider".to_string(),
        base_url: secret.base_url,
        api_key: secret.api_key,
        model: secret.model,
    })
}

pub fn validate_live(input: ProviderInput) -> Result<VerifiedProvider> {
    let mut stages = Vec::new();
    let normalized = normalize_base_url(&input.base_url)?;
    let host = normalized.host_str().unwrap_or("unknown").to_string();
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()?;

    let model_start = Instant::now();
    let models_url = normalized.join("models")?;
    let response = client
        .get(models_url)
        .bearer_auth(&input.api_key)
        .send()
        .context("model discovery request")?;
    if matches!(response.status().as_u16(), 401 | 403) {
        bail!("authentication: model discovery returned {}", response.status());
    }
    if response.status().as_u16() == 429 {
        bail!("rate_limit: model discovery returned 429");
    }
    let status = response.status();
    let models: Value = response.json().context("parse model discovery JSON")?;
    let found = models
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|item| item.get("id").and_then(Value::as_str) == Some(input.model.as_str()));
    stages.push(ValidationStage {
        name: "model_discovery".to_string(),
        ok: status.is_success() && found,
        duration_ms: model_start.elapsed().as_millis(),
        details: json!({"status": status.as_u16(), "found": found}),
    });
    if !status.is_success() || !found {
        bail!("model_discovery: default model was not found");
    }

    let nonce = format!(
        "gpteasy-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let tool = json!({
        "type": "function",
        "name": "gpteasy_probe",
        "description": "Return the nonce supplied by the user.",
        "strict": true,
        "parameters": {
            "type": "object",
            "properties": {"nonce": {"type": "string"}},
            "required": ["nonce"],
            "additionalProperties": false
        }
    });
    let user_input = json!({
        "role": "user",
        "content": [{"type": "input_text", "text": format!("Call gpteasy_probe with nonce {nonce}.")}]
    });
    let first_payload = json!({
        "model": input.model,
        "input": [user_input.clone()],
        "tools": [tool.clone()],
        "tool_choice": {"type": "function", "name": "gpteasy_probe"},
        "parallel_tool_calls": false,
        "stream": true
    });
    let first_start = Instant::now();
    let first = post_sse(&client, &normalized, &input.api_key, &first_payload)?;
    let call = first
        .function_call
        .context("tool_call: stream completed without gpteasy_probe")?;
    let arguments: Value =
        serde_json::from_str(&call.arguments).context("tool_call: invalid arguments JSON")?;
    let first_ok = first.saw_completed
        && call.name == "gpteasy_probe"
        && arguments.get("nonce").and_then(Value::as_str) == Some(nonce.as_str());
    stages.push(ValidationStage {
        name: "responses_stream_and_tool_call".to_string(),
        ok: first_ok,
        duration_ms: first_start.elapsed().as_millis(),
        details: json!({
            "content_type": first.content_type,
            "event_count": first.events.len(),
            "saw_completed": first.saw_completed,
            "function_name": call.name
        }),
    });
    if !first_ok {
        bail!("tool_call: nonce or completion event did not match");
    }

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
        "model": input.model,
        "input": [user_input, function_call_item, function_output],
        "tools": [tool],
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "stream": true
    });
    let second_start = Instant::now();
    let second = post_sse(&client, &normalized, &input.api_key, &second_payload)?;
    let second_ok = second.saw_completed && second.output_text.contains(&nonce);
    stages.push(ValidationStage {
        name: "tool_result_round_trip".to_string(),
        ok: second_ok,
        duration_ms: second_start.elapsed().as_millis(),
        details: json!({
            "content_type": second.content_type,
            "event_count": second.events.len(),
            "saw_completed": second.saw_completed,
            "output_contains_nonce": second.output_text.contains(&nonce),
            "output_length": second.output_text.len()
        }),
    });
    if !second_ok {
        bail!("tool_result: final streamed answer did not contain the nonce");
    }

    let fingerprint = combination_fingerprint(&input);
    Ok(VerifiedProvider {
        evidence: ValidationEvidence {
            ok: true,
            category: "validated".to_string(),
            provider_host: host,
            model: input.model.clone(),
            combination_fingerprint: fingerprint,
            stages,
        },
        input,
    })
}

pub fn combination_fingerprint(input: &ProviderInput) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-provider-combination-v1\0");
    hasher.update(input.base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(input.model.as_bytes());
    hasher.update(b"\0");
    hasher.update(input.api_key.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn normalize_base_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value).context("parse provider base URL")?;
    match url.scheme() {
        "https" => {}
        "http" if url.host_str().is_some_and(is_loopback_host) => {}
        "http" => bail!("security_policy: remote provider must use HTTPS"),
        other => bail!("security_policy: unsupported URL scheme {other}"),
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn post_sse(client: &Client, base: &Url, key: &str, payload: &Value) -> Result<SseResult> {
    let response = client
        .post(base.join("responses")?)
        .bearer_auth(key)
        .json(payload)
        .send()
        .context("Responses request")?;
    if matches!(response.status().as_u16(), 401 | 403) {
        bail!("authentication: Responses returned {}", response.status());
    }
    if response.status().as_u16() == 429 {
        bail!("rate_limit: Responses returned 429");
    }
    if !response.status().is_success() {
        bail!("responses_protocol: Responses returned {}", response.status());
    }
    parse_sse(response)
}

fn parse_sse(response: Response) -> Result<SseResult> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !content_type.starts_with("text/event-stream") {
        bail!("streaming: expected text/event-stream");
    }
    let mut reader = BufReader::new(response);
    let mut line = String::new();
    let mut data = Vec::new();
    let mut events = Vec::new();
    let mut saw_completed = false;
    let mut function_call = None;
    let mut output_text = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            if !data.is_empty() {
                consume_event(
                    &data.join("\n"),
                    &mut events,
                    &mut saw_completed,
                    &mut function_call,
                    &mut output_text,
                )?;
            }
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if !data.is_empty() {
                consume_event(
                    &data.join("\n"),
                    &mut events,
                    &mut saw_completed,
                    &mut function_call,
                    &mut output_text,
                )?;
                data.clear();
            }
        } else if let Some(value) = trimmed.strip_prefix("data:") {
            data.push(value.trim_start().to_string());
        }
    }
    Ok(SseResult {
        content_type,
        events,
        saw_completed,
        function_call,
        output_text,
    })
}

fn consume_event(
    data: &str,
    events: &mut Vec<String>,
    saw_completed: &mut bool,
    function_call: &mut Option<FunctionCall>,
    output_text: &mut String,
) -> Result<()> {
    if data == "[DONE]" {
        return Ok(());
    }
    let value: Value = serde_json::from_str(data).context("parse SSE data JSON")?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    events.push(kind.clone());
    match kind.as_str() {
        "response.completed" => *saw_completed = true,
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                output_text.push_str(delta);
            }
        }
        "response.output_item.done" => {
            if let Some(item) = value.get("item") {
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    *function_call = Some(FunctionCall {
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
                                output_text.push_str(text);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
