use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;
use rusqlite::{Connection, Error as SqliteError, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use crate::state::StateStore;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryInput {
    pub base_url: String,
    pub api_key: String,
}

impl fmt::Debug for DiscoveryInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveryInput")
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderValidationInput {
    pub base_url: String,
    pub api_key: String,
    pub default_model: String,
}

impl fmt::Debug for ProviderValidationInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderValidationInput")
            .field("base_url", &self.base_url)
            .field("api_key", &"[redacted]")
            .field("default_model", &self.default_model)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDiscovery {
    pub normalized_base_url: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationEvidence {
    pub normalized_base_url: String,
    pub default_model: String,
    pub combination_fingerprint: String,
    pub verified_at_epoch_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderValidationReceipt {
    pub validation_id: String,
    pub normalized_base_url: String,
    pub default_model: String,
    pub combination_fingerprint: String,
    pub verified_at_epoch_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    pub verified_at_epoch_seconds: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct ValidationTimeouts {
    pub connect: Duration,
    pub response_header: Duration,
    pub stream_read: Duration,
    pub response_overall: Duration,
}

impl Default for ValidationTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            response_header: Duration::from_secs(30),
            stream_read: Duration::from_secs(30),
            response_overall: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFailureCategory {
    SecurityPolicy,
    Transport,
    Cancelled,
    ResponseHeaderTimeout,
    FirstEventTimeout,
    StreamIdleTimeout,
    OverallTimeout,
    Authentication,
    RateLimit,
    ModelDiscovery,
    Streaming,
    ResponsesProtocol,
    ToolCall,
    ToolResult,
    InvalidInput,
    DuplicateName,
    VerificationExpired,
    StateUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFailure {
    pub category: ProviderFailureCategory,
    pub message_id: &'static str,
}

impl ProviderFailure {
    fn new(category: ProviderFailureCategory, message_id: &'static str) -> Self {
        Self {
            category,
            message_id,
        }
    }
}

#[derive(Clone)]
pub struct ProviderValidator {
    client: reqwest::Client,
    timeouts: ValidationTimeouts,
}

pub struct ProviderApplication {
    state_store: StateStore,
    validator: ProviderValidator,
    active_requests: Mutex<HashMap<String, CancellationToken>>,
    verified_candidates: Mutex<HashMap<String, VerifiedCandidate>>,
}

#[derive(Clone)]
struct VerifiedCandidate {
    input: ProviderValidationInput,
    evidence: ValidationEvidence,
}

impl ProviderApplication {
    pub fn new(state_store: StateStore, validator: ProviderValidator) -> Self {
        Self {
            state_store,
            validator,
            active_requests: Mutex::new(HashMap::new()),
            verified_candidates: Mutex::new(HashMap::new()),
        }
    }

    pub async fn discover_models(
        &self,
        request_id: String,
        input: DiscoveryInput,
    ) -> Result<ModelDiscovery, ProviderFailure> {
        let cancellation = self.begin_request(&request_id)?;
        let result = self.validator.discover_models(input, cancellation).await;
        self.finish_request(&request_id);
        result
    }

    pub async fn validate_provider(
        &self,
        request_id: String,
        input: ProviderValidationInput,
    ) -> Result<ProviderValidationReceipt, ProviderFailure> {
        let cancellation = self.begin_request(&request_id)?;
        let evidence = self
            .validator
            .validate_provider(input.clone(), cancellation)
            .await;
        self.finish_request(&request_id);
        let evidence = evidence?;
        let validation_id = Uuid::new_v4().to_string();
        self.verified_candidates
            .lock()
            .map_err(|_| state_unavailable())?
            .insert(
                validation_id.clone(),
                VerifiedCandidate {
                    input,
                    evidence: evidence.clone(),
                },
            );
        Ok(ProviderValidationReceipt {
            validation_id,
            normalized_base_url: evidence.normalized_base_url,
            default_model: evidence.default_model,
            combination_fingerprint: evidence.combination_fingerprint,
            verified_at_epoch_seconds: evidence.verified_at_epoch_seconds,
        })
    }

    pub fn cancel_request(&self, request_id: &str) -> bool {
        self.active_requests
            .lock()
            .ok()
            .and_then(|requests| requests.get(request_id).cloned())
            .is_some_and(|cancellation| {
                cancellation.cancel();
                true
            })
    }

    pub fn discard_validation(&self, validation_id: &str) {
        if let Ok(mut candidates) = self.verified_candidates.lock() {
            candidates.remove(validation_id);
        }
    }

    pub fn save_verified_provider(
        &self,
        validation_id: &str,
        name: &str,
    ) -> Result<ProviderSummary, ProviderFailure> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.name_required",
            ));
        }
        let candidate = self
            .verified_candidates
            .lock()
            .map_err(|_| state_unavailable())?
            .get(validation_id)
            .cloned()
            .ok_or_else(verification_expired)?;
        let actual_fingerprint = combination_fingerprint(
            &candidate.evidence.normalized_base_url,
            &candidate.input.api_key,
            &candidate.input.default_model,
        );
        if actual_fingerprint != candidate.evidence.combination_fingerprint {
            self.discard_validation(validation_id);
            return Err(verification_expired());
        }

        let summary = self.insert_provider(name, &candidate)?;
        self.discard_validation(validation_id);
        Ok(summary)
    }

    pub fn list_providers(&self) -> Result<Vec<ProviderSummary>, ProviderFailure> {
        let connection = self.open_catalog()?;
        let mut statement = connection
            .prepare(
                "SELECT id, name, base_url, default_model, verified_at \
                 FROM providers ORDER BY name COLLATE NOCASE, id",
            )
            .map_err(|_| state_unavailable())?;
        statement
            .query_map([], |row| {
                let verified_at = row.get::<_, String>(4)?;
                Ok(ProviderSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    base_url: row.get(2)?,
                    default_model: row.get(3)?,
                    verified_at_epoch_seconds: verified_at.parse().map_err(|error| {
                        SqliteError::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                })
            })
            .map_err(|_| state_unavailable())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| state_unavailable())
    }

    fn begin_request(&self, request_id: &str) -> Result<CancellationToken, ProviderFailure> {
        if request_id.trim().is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.request_id_required",
            ));
        }
        let cancellation = CancellationToken::new();
        let replaced = self
            .active_requests
            .lock()
            .map_err(|_| state_unavailable())?
            .insert(request_id.to_owned(), cancellation.clone());
        if replaced.is_some() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.request_already_running",
            ));
        }
        Ok(cancellation)
    }

    fn finish_request(&self, request_id: &str) {
        if let Ok(mut requests) = self.active_requests.lock() {
            requests.remove(request_id);
        }
    }

    fn open_catalog(&self) -> Result<Connection, ProviderFailure> {
        if !self.state_store.bootstrap().is_ready() {
            return Err(state_unavailable());
        }
        let connection = Connection::open(self.state_store.paths().database())
            .map_err(|_| state_unavailable())?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(|_| state_unavailable())?;
        Ok(connection)
    }

    fn insert_provider(
        &self,
        name: &str,
        candidate: &VerifiedCandidate,
    ) -> Result<ProviderSummary, ProviderFailure> {
        let mut connection = self.open_catalog()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| state_unavailable())?;
        let existing_names = {
            let mut statement = transaction
                .prepare("SELECT name FROM providers")
                .map_err(|_| state_unavailable())?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|_| state_unavailable())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| state_unavailable())?
        };
        let normalized_name = name.to_lowercase();
        if existing_names
            .iter()
            .any(|existing| existing.to_lowercase() == normalized_name)
        {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::DuplicateName,
                "provider.name_duplicate",
            ));
        }

        let summary = ProviderSummary {
            id: Uuid::new_v4().to_string(),
            name: name.to_owned(),
            base_url: candidate.evidence.normalized_base_url.clone(),
            default_model: candidate.input.default_model.clone(),
            verified_at_epoch_seconds: candidate.evidence.verified_at_epoch_seconds,
        };
        transaction
            .execute(
                "INSERT INTO providers (\
                    id, name, base_url, api_key, default_model, verified_at, verification_fingerprint\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    summary.id,
                    summary.name,
                    summary.base_url,
                    candidate.input.api_key,
                    summary.default_model,
                    summary.verified_at_epoch_seconds.to_string(),
                    candidate.evidence.combination_fingerprint,
                ],
            )
            .map_err(|error| match error {
                SqliteError::SqliteFailure(_, Some(message))
                    if message.contains("providers.name") =>
                {
                    ProviderFailure::new(
                        ProviderFailureCategory::DuplicateName,
                        "provider.name_duplicate",
                    )
                }
                _ => state_unavailable(),
            })?;
        transaction.commit().map_err(|_| state_unavailable())?;
        Ok(summary)
    }
}

impl ProviderValidator {
    pub fn new(timeouts: ValidationTimeouts) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .redirect(Policy::none())
            .build()
            .expect("reqwest client configuration must be valid");
        Self { client, timeouts }
    }

    pub async fn discover_models(
        &self,
        input: DiscoveryInput,
        cancellation: CancellationToken,
    ) -> Result<ModelDiscovery, ProviderFailure> {
        let normalized = normalize_base_url(&input.base_url)?;
        let endpoint = endpoint(&normalized, "models");
        let request = self.client.get(endpoint).bearer_auth(&input.api_key).send();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            result = timeout(self.timeouts.response_header, request) => {
                result
                    .map_err(|_| ProviderFailure::new(
                        ProviderFailureCategory::ResponseHeaderTimeout,
                        "provider.response_header_timeout",
                    ))?
                    .map_err(|error| transport_failure(&error))?
            }
        };
        classify_status(response.status(), true)?;
        let body = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            result = timeout(self.timeouts.response_overall, response.bytes()) => {
                result
                    .map_err(|_| ProviderFailure::new(
                        ProviderFailureCategory::OverallTimeout,
                        "provider.overall_timeout",
                    ))?
                    .map_err(|error| transport_failure(&error))?
            }
        };
        let document: Value = serde_json::from_slice(&body).map_err(|_| {
            ProviderFailure::new(
                ProviderFailureCategory::ModelDiscovery,
                "provider.models_invalid",
            )
        })?;
        let data = document
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProviderFailure::new(
                    ProviderFailureCategory::ModelDiscovery,
                    "provider.models_invalid",
                )
            })?;
        let mut seen = HashSet::new();
        let models = data
            .iter()
            .filter_map(|item| item.get("id").and_then(Value::as_str))
            .filter(|id| !id.is_empty() && seen.insert((*id).to_owned()))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if models.is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::ModelDiscovery,
                "provider.models_empty",
            ));
        }
        Ok(ModelDiscovery {
            normalized_base_url: normalized.to_string(),
            models,
        })
    }

    pub async fn validate_provider(
        &self,
        input: ProviderValidationInput,
        cancellation: CancellationToken,
    ) -> Result<ValidationEvidence, ProviderFailure> {
        if input.default_model.is_empty() {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::InvalidInput,
                "provider.default_model_required",
            ));
        }
        let discovery = self
            .discover_models(
                DiscoveryInput {
                    base_url: input.base_url.clone(),
                    api_key: input.api_key.clone(),
                },
                cancellation.clone(),
            )
            .await?;
        if !discovery
            .models
            .iter()
            .any(|model| model == &input.default_model)
        {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::ModelDiscovery,
                "provider.default_model_missing",
            ));
        }
        let base_url = Url::parse(&discovery.normalized_base_url).map_err(|_| {
            ProviderFailure::new(
                ProviderFailureCategory::SecurityPolicy,
                "provider.url_invalid",
            )
        })?;
        self.validate_tool_round_trip(
            &base_url,
            &input.api_key,
            &input.default_model,
            cancellation,
        )
        .await?;

        Ok(ValidationEvidence {
            combination_fingerprint: combination_fingerprint(
                &discovery.normalized_base_url,
                &input.api_key,
                &input.default_model,
            ),
            normalized_base_url: discovery.normalized_base_url,
            default_model: input.default_model,
            verified_at_epoch_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
    }

    async fn validate_tool_round_trip(
        &self,
        base_url: &Url,
        api_key: &str,
        model: &str,
        cancellation: CancellationToken,
    ) -> Result<(), ProviderFailure> {
        let nonce = format!("gpteasy-{}", Uuid::new_v4());
        let user_input = json!({
            "role": "user",
            "content": [{
                "type": "input_text",
                "text": format!(
                    "Call gpteasy_probe exactly once with nonce `{nonce}`. After receiving the tool output, reply with the nonce."
                )
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
        let first = self
            .post_sse(base_url, api_key, &first_payload, cancellation.clone())
            .await?;
        let call = first.function_call.ok_or_else(|| {
            ProviderFailure::new(
                ProviderFailureCategory::ToolCall,
                "provider.tool_call_missing",
            )
        })?;
        if call.call_id.is_empty() || call.name != "gpteasy_probe" {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::ToolCall,
                "provider.tool_call_invalid",
            ));
        }
        let arguments: Value = serde_json::from_str(&call.arguments).map_err(|_| {
            ProviderFailure::new(
                ProviderFailureCategory::ToolCall,
                "provider.tool_arguments_invalid",
            )
        })?;
        let strict_arguments = arguments.as_object().is_some_and(|object| {
            object.len() == 1 && object.get("nonce").and_then(Value::as_str) == Some(nonce.as_str())
        });
        if !strict_arguments {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::ToolCall,
                "provider.tool_arguments_invalid",
            ));
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
            "model": model,
            "input": [user_input, function_call_item, function_output],
            "tools": [tool],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "stream": true
        });
        let second = self
            .post_sse(base_url, api_key, &second_payload, cancellation)
            .await?;
        if !second.output_text.contains(&nonce) {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::ToolResult,
                "provider.tool_result_invalid",
            ));
        }
        Ok(())
    }

    async fn post_sse(
        &self,
        base_url: &Url,
        api_key: &str,
        payload: &Value,
        cancellation: CancellationToken,
    ) -> Result<SseResult, ProviderFailure> {
        let started = Instant::now();
        let request = self
            .client
            .post(endpoint(base_url, "responses"))
            .bearer_auth(api_key)
            .json(payload)
            .send();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            result = timeout(self.timeouts.response_header, request) => {
                result
                    .map_err(|_| ProviderFailure::new(
                        ProviderFailureCategory::ResponseHeaderTimeout,
                        "provider.response_header_timeout",
                    ))?
                    .map_err(|error| transport_failure(&error))?
            }
        };
        classify_status(response.status(), false)?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !content_type.starts_with("text/event-stream") {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::Streaming,
                "provider.responses_not_sse",
            ));
        }

        let mut stream = response.bytes_stream();
        let mut parser = SseParser::default();
        let mut buffer = Vec::new();
        loop {
            let elapsed = started.elapsed();
            if elapsed >= self.timeouts.response_overall {
                return Err(overall_timeout());
            }
            let remaining = self.timeouts.response_overall.saturating_sub(elapsed);
            let wait = remaining.min(self.timeouts.stream_read);
            let next = tokio::select! {
                _ = cancellation.cancelled() => return Err(cancelled()),
                result = timeout(wait, stream.next()) => result,
            };
            let chunk = match next {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(Some(Err(_))) => {
                    return Err(ProviderFailure::new(
                        ProviderFailureCategory::Streaming,
                        "provider.responses_stream_broken",
                    ));
                }
                Ok(None) => break,
                Err(_) if started.elapsed() >= self.timeouts.response_overall => {
                    return Err(overall_timeout());
                }
                Err(_) if !parser.saw_event => {
                    return Err(ProviderFailure::new(
                        ProviderFailureCategory::FirstEventTimeout,
                        "provider.first_event_timeout",
                    ));
                }
                Err(_) => {
                    return Err(ProviderFailure::new(
                        ProviderFailureCategory::StreamIdleTimeout,
                        "provider.stream_idle_timeout",
                    ));
                }
            };
            buffer.extend_from_slice(&chunk);
            consume_complete_lines(&mut buffer, &mut parser)?;
        }
        if !buffer.is_empty() {
            let final_line = std::str::from_utf8(&buffer).map_err(|_| protocol_failure())?;
            parser.consume_line(final_line.trim_end_matches('\r'))?;
        }
        parser.finish_event()?;
        parser.into_result()
    }
}

#[derive(Debug)]
struct FunctionCall {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Debug)]
struct SseResult {
    function_call: Option<FunctionCall>,
    output_text: String,
}

#[derive(Default)]
struct SseParser {
    event_type: String,
    data_lines: Vec<String>,
    saw_event: bool,
    saw_completed: bool,
    function_call: Option<FunctionCall>,
    output_text: String,
}

impl SseParser {
    fn consume_line(&mut self, line: &str) -> Result<(), ProviderFailure> {
        if line.is_empty() {
            return self.finish_event();
        }
        if line.starts_with(':') {
            return Ok(());
        }
        if let Some(value) = line.strip_prefix("event:") {
            self.event_type = value.trim().to_owned();
        } else if let Some(value) = line.strip_prefix("data:") {
            self.data_lines.push(value.trim_start().to_owned());
        }
        Ok(())
    }

    fn finish_event(&mut self) -> Result<(), ProviderFailure> {
        if self.data_lines.is_empty() {
            self.event_type.clear();
            return Ok(());
        }
        let data = self.data_lines.join("\n");
        self.data_lines.clear();
        if data == "[DONE]" {
            self.event_type.clear();
            return Ok(());
        }
        let event: Value = serde_json::from_str(&data).map_err(|_| protocol_failure())?;
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(&self.event_type)
            .to_owned();
        self.event_type.clear();
        self.saw_event = true;
        match kind.as_str() {
            "response.completed" => {
                self.saw_completed = true;
                collect_completed_text(&event, &mut self.output_text);
            }
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.output_text.push_str(delta);
                }
            }
            "response.output_item.done" => {
                if let Some(item) = event.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        self.function_call = Some(FunctionCall {
                            call_id: item
                                .get("call_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            name: item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            arguments: item
                                .get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        });
                    } else {
                        collect_item_text(item, &mut self.output_text);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn into_result(self) -> Result<SseResult, ProviderFailure> {
        if !self.saw_completed {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::Streaming,
                "provider.responses_stream_incomplete",
            ));
        }
        Ok(SseResult {
            function_call: self.function_call,
            output_text: self.output_text,
        })
    }
}

fn consume_complete_lines(
    buffer: &mut Vec<u8>,
    parser: &mut SseParser,
) -> Result<(), ProviderFailure> {
    while let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
        let bytes = buffer.drain(..=index).collect::<Vec<_>>();
        let line = std::str::from_utf8(&bytes[..bytes.len() - 1])
            .map_err(|_| protocol_failure())?
            .trim_end_matches('\r');
        parser.consume_line(line)?;
    }
    Ok(())
}

fn collect_completed_text(event: &Value, output: &mut String) {
    if let Some(items) = event
        .get("response")
        .and_then(|response| response.get("output"))
        .and_then(Value::as_array)
    {
        for item in items {
            collect_item_text(item, output);
        }
    }
}

fn collect_item_text(item: &Value, output: &mut String) {
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for part in content {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                output.push_str(text);
            }
        }
    }
}

fn combination_fingerprint(base_url: &str, api_key: &str, model: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-provider-combination-v1\0");
    hasher.update(base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(model.as_bytes());
    hasher.update(b"\0");
    hasher.update(api_key.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn protocol_failure() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::ResponsesProtocol,
        "provider.responses_protocol_invalid",
    )
}

fn overall_timeout() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::OverallTimeout,
        "provider.overall_timeout",
    )
}

fn verification_expired() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::VerificationExpired,
        "provider.verification_expired",
    )
}

fn state_unavailable() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::StateUnavailable,
        "provider.state_unavailable",
    )
}

fn endpoint(base_url: &Url, suffix: &str) -> Url {
    let mut url = base_url.clone();
    url.set_path(&format!(
        "{}/{}",
        base_url.path().trim_end_matches('/'),
        suffix
    ));
    url
}

fn classify_status(status: StatusCode, model_discovery: bool) -> Result<(), ProviderFailure> {
    if status.is_success() {
        return Ok(());
    }
    if status.is_redirection() {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::SecurityPolicy,
            "provider.redirect_forbidden",
        ));
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::Authentication,
            "provider.authentication_failed",
        ));
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::RateLimit,
            "provider.rate_limited",
        ));
    }
    let (category, message_id) = if model_discovery {
        (
            ProviderFailureCategory::ModelDiscovery,
            "provider.models_request_failed",
        )
    } else {
        (
            ProviderFailureCategory::ResponsesProtocol,
            "provider.responses_request_failed",
        )
    };
    Err(ProviderFailure::new(category, message_id))
}

fn transport_failure(error: &reqwest::Error) -> ProviderFailure {
    if error.is_timeout() {
        ProviderFailure::new(
            ProviderFailureCategory::ResponseHeaderTimeout,
            "provider.response_header_timeout",
        )
    } else {
        ProviderFailure::new(
            ProviderFailureCategory::Transport,
            "provider.transport_failed",
        )
    }
}

fn cancelled() -> ProviderFailure {
    ProviderFailure::new(
        ProviderFailureCategory::Cancelled,
        "provider.request_cancelled",
    )
}

fn normalize_base_url(base_url: &str) -> Result<Url, ProviderFailure> {
    let mut url = Url::parse(base_url).map_err(|_| {
        ProviderFailure::new(
            ProviderFailureCategory::SecurityPolicy,
            "provider.url_invalid",
        )
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::SecurityPolicy,
            "provider.url_components_forbidden",
        ));
    }

    let host = url.host_str().ok_or_else(|| {
        ProviderFailure::new(
            ProviderFailureCategory::SecurityPolicy,
            "provider.url_invalid",
        )
    })?;
    let loopback = host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1";
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::SecurityPolicy,
                "provider.remote_https_required",
            ));
        }
        _ => {
            return Err(ProviderFailure::new(
                ProviderFailureCategory::SecurityPolicy,
                "provider.url_scheme_unsupported",
            ));
        }
    }

    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url)
}
