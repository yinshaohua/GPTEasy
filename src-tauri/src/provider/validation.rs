use std::collections::HashSet;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;
use serde_json::{Value, json};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use url::{Host, Url};
use uuid::Uuid;

use super::{
    DiscoveryInput, ModelDiscovery, ProviderFailure, ProviderFailureCategory,
    ProviderValidationInput, ProviderValidationStage, ValidationEvidence, ValidationTimeouts,
    cancelled, combination_fingerprint,
};

#[derive(Clone)]
pub struct ProviderValidator {
    client: reqwest::Client,
    timeouts: ValidationTimeouts,
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
        let requested = normalize_base_url(&input.base_url)?;
        let candidates = base_url_candidates(&requested);
        let candidate_count = candidates.len();
        for (index, candidate) in candidates.into_iter().enumerate() {
            match self
                .discover_models_at(&candidate, &input.api_key, cancellation.clone())
                .await
            {
                Ok(models) => {
                    return Ok(ModelDiscovery {
                        requested_base_url: requested.to_string(),
                        normalized_base_url: candidate.to_string(),
                        models,
                    });
                }
                Err(DiscoveryAttemptFailure::EndpointPath) if index + 1 < candidate_count => {}
                Err(DiscoveryAttemptFailure::EndpointPath) => {
                    return Err(ProviderFailure::new(
                        ProviderFailureCategory::ModelDiscovery,
                        "provider.models_request_failed",
                    ));
                }
                Err(DiscoveryAttemptFailure::Failure(failure)) => return Err(failure),
            }
        }
        Err(ProviderFailure::new(
            ProviderFailureCategory::ModelDiscovery,
            "provider.models_request_failed",
        ))
    }

    async fn discover_models_at(
        &self,
        base_url: &Url,
        api_key: &str,
        cancellation: CancellationToken,
    ) -> Result<Vec<String>, DiscoveryAttemptFailure> {
        let request = self
            .client
            .get(endpoint(base_url, "models"))
            .bearer_auth(api_key)
            .send();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled().into()),
            result = timeout(self.timeouts.response_header, request) => {
                result
                    .map_err(|_| ProviderFailure::new(
                        ProviderFailureCategory::ResponseHeaderTimeout,
                        "provider.response_header_timeout",
                    ))?
                    .map_err(|error| transport_failure(&error))?
            }
        };
        if matches!(
            response.status(),
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
        ) {
            return Err(DiscoveryAttemptFailure::EndpointPath);
        }
        classify_status(response.status(), true)?;
        let body = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled().into()),
            result = timeout(self.timeouts.response_overall, response.bytes()) => {
                result
                    .map_err(|_| ProviderFailure::new(
                        ProviderFailureCategory::OverallTimeout,
                        "provider.overall_timeout",
                    ))?
                    .map_err(|error| transport_failure(&error))?
            }
        };
        parse_models(&body).map_err(Into::into)
    }

    pub async fn validate_provider(
        &self,
        input: ProviderValidationInput,
        cancellation: CancellationToken,
    ) -> Result<ValidationEvidence, ProviderFailure> {
        self.validate_provider_with_progress(input, cancellation, |_| {})
            .await
    }

    pub async fn validate_provider_with_progress<F>(
        &self,
        input: ProviderValidationInput,
        cancellation: CancellationToken,
        progress: F,
    ) -> Result<ValidationEvidence, ProviderFailure>
    where
        F: Fn(ProviderValidationStage),
    {
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
        progress(ProviderValidationStage::ModelsConfirmed);
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
            &progress,
        )
        .await?;

        Ok(ValidationEvidence {
            requested_base_url: discovery.requested_base_url,
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

    async fn validate_tool_round_trip<F>(
        &self,
        base_url: &Url,
        api_key: &str,
        model: &str,
        cancellation: CancellationToken,
        progress: &F,
    ) -> Result<(), ProviderFailure>
    where
        F: Fn(ProviderValidationStage),
    {
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
        progress(ProviderValidationStage::ResponsesStream);
        let first = self
            .post_sse(base_url, api_key, &first_payload, cancellation.clone())
            .await?;
        progress(ProviderValidationStage::ToolRoundTrip);
        let mut calls = first.function_calls.into_iter();
        let call = calls.next().ok_or_else(|| {
            ProviderFailure::new(
                ProviderFailureCategory::ToolCall,
                "provider.tool_call_missing",
            )
        })?;
        if calls.next().is_some() || call.call_id.is_empty() || call.name != "gpteasy_probe" {
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
        if !second.function_calls.is_empty() || !second.output_text.contains(&nonce) {
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
    function_calls: Vec<FunctionCall>,
    output_text: String,
}

#[derive(Default)]
struct SseParser {
    event_type: String,
    data_lines: Vec<String>,
    saw_event: bool,
    saw_completed: bool,
    function_calls: Vec<FunctionCall>,
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
                        self.function_calls.push(FunctionCall {
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
            function_calls: self.function_calls,
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

enum DiscoveryAttemptFailure {
    EndpointPath,
    Failure(ProviderFailure),
}

impl From<ProviderFailure> for DiscoveryAttemptFailure {
    fn from(failure: ProviderFailure) -> Self {
        Self::Failure(failure)
    }
}

fn parse_models(body: &[u8]) -> Result<Vec<String>, ProviderFailure> {
    let document: Value = serde_json::from_slice(body).map_err(|_| {
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
    let mut models = Vec::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                ProviderFailure::new(
                    ProviderFailureCategory::ModelDiscovery,
                    "provider.models_invalid",
                )
            })?;
        if seen.insert(id.to_owned()) {
            models.push(id.to_owned());
        }
    }
    if models.is_empty() {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::ModelDiscovery,
            "provider.models_empty",
        ));
    }
    Ok(models)
}

fn base_url_candidates(requested: &Url) -> Vec<Url> {
    let requested_path = requested.path().trim_end_matches('/');
    let stripped_path = strip_endpoint_suffix(requested_path);
    let mut paths = vec![requested_path.to_owned()];
    if stripped_path != requested_path {
        paths.push(stripped_path.to_owned());
    }
    let toggle_source = stripped_path;
    if let Some(without_v1) = toggle_source.strip_suffix("/v1") {
        paths.push(without_v1.to_owned());
    } else {
        paths.push(format!("{}/v1", toggle_source.trim_end_matches('/')));
    }

    let mut seen = HashSet::new();
    paths
        .into_iter()
        .map(|path| {
            let mut candidate = requested.clone();
            candidate.set_path(if path.is_empty() { "/" } else { &path });
            candidate
        })
        .filter(|candidate| seen.insert(candidate.as_str().to_owned()))
        .collect()
}

fn strip_endpoint_suffix(path: &str) -> &str {
    ["/chat/completions", "/responses", "/models"]
        .into_iter()
        .find_map(|suffix| path.strip_suffix(suffix))
        .unwrap_or(path)
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
    if status == StatusCode::UNAUTHORIZED {
        return Err(ProviderFailure::new(
            ProviderFailureCategory::Authentication,
            "provider.invalid_api_key",
        ));
    }
    if status == StatusCode::FORBIDDEN {
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

    let host = url.host().ok_or_else(|| {
        ProviderFailure::new(
            ProviderFailureCategory::SecurityPolicy,
            "provider.url_invalid",
        )
    })?;
    let loopback = match host {
        Host::Domain(host) => host.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
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
