//! Codex 推理能力的隔离探测、版本绑定快照和精确合并。
//!
//! 这个模块故意不根据供应商、模型名称或模型前缀猜测能力。Codex 的
//! `model/list` 是默认能力来源；探测得到的明确未识别状态可以由显式、版本化
//! 的供应商兼容规则精确补充。探测失败时仍保留明确的 unknown 状态。

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const MAX_MODELS: usize = 256;
const MAX_PAGES: usize = 8;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const CHILD_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityState {
    KnownNonEmpty,
    KnownEmpty,
    NotFound,
    ProbeFailed,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilitySource {
    CodexBuiltin,
    ProviderDeclared,
    VendorCompatibility,
    Unknown,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReasoningCapability {
    pub state: CapabilityState,
    pub levels: Vec<String>,
    pub default_level: Option<String>,
    pub source: CapabilitySource,
    pub source_id: Option<String>,
    pub codex_version: Option<String>,
    pub observed_at_epoch_seconds: u64,
    pub message_id: String,
}

impl ReasoningCapability {
    fn from_codex_metadata(metadata: &CodexModelMetadata, version: Option<&str>) -> Self {
        let default_level = metadata.default_reasoning_effort.clone().filter(|level| {
            metadata
                .supported_reasoning_efforts
                .iter()
                .any(|item| item == level)
        });
        let state = if metadata.state == CapabilityState::Conflict
            || (metadata.default_reasoning_effort.is_some() && default_level.is_none())
        {
            CapabilityState::Conflict
        } else {
            metadata.state
        };
        Self {
            state,
            levels: metadata.supported_reasoning_efforts.clone(),
            default_level,
            source: metadata.source,
            source_id: Some(metadata.source_id.clone()),
            codex_version: version.map(str::to_owned),
            observed_at_epoch_seconds: epoch_seconds(),
            message_id: match state {
                CapabilityState::KnownNonEmpty => "reasoning_metadata.codex_known_nonempty",
                CapabilityState::KnownEmpty => "reasoning_metadata.codex_known_empty",
                CapabilityState::NotFound => "reasoning_metadata.codex_not_found",
                CapabilityState::Conflict => "reasoning_metadata.codex_conflict",
                CapabilityState::ProbeFailed => "reasoning_metadata.codex_probe_failed",
            }
            .to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SnapshotStatus {
    Complete,
    ProbeFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexModelMetadata {
    pub model_id: String,
    pub state: CapabilityState,
    pub default_reasoning_effort: Option<String>,
    pub supported_reasoning_efforts: Vec<String>,
    pub source: CapabilitySource,
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexMetadataSnapshot {
    pub schema_version: u32,
    pub target_environment: String,
    pub codex_version: Option<String>,
    pub executable_fingerprint: Option<String>,
    pub observed_at_epoch_seconds: u64,
    pub status: SnapshotStatus,
    pub models: Vec<CodexModelMetadata>,
}

impl CodexMetadataSnapshot {
    pub(crate) fn capability_for(&self, model_id: &str) -> Option<ReasoningCapability> {
        let matches = self
            .models
            .iter()
            .filter(|model| model.model_id == model_id)
            .collect::<Vec<_>>();
        (matches.len() == 1).then(|| {
            ReasoningCapability::from_codex_metadata(matches[0], self.codex_version.as_deref())
        })
    }

    pub(crate) fn covers_models_in_environment(
        &self,
        model_ids: &[String],
        environment: &str,
    ) -> bool {
        if self.target_environment != environment {
            return false;
        }
        let mut scoped = self.clone();
        scoped.target_environment = "native".to_owned();
        scoped.covers_exact_models(model_ids)
    }

    pub(crate) fn covers_exact_models(&self, model_ids: &[String]) -> bool {
        let requested = model_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let snapshot_ids = self
            .models
            .iter()
            .map(|metadata| metadata.model_id.as_str())
            .collect::<HashSet<_>>();
        self.schema_version == SNAPSHOT_SCHEMA_VERSION
            && self.target_environment == "native"
            && self.status == SnapshotStatus::Complete
            && requested.len() == model_ids.len()
            && snapshot_ids.len() == self.models.len()
            && self
                .models
                .iter()
                .all(|metadata| !metadata.model_id.is_empty())
            && requested == snapshot_ids
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProbeFailure {
    pub message_id: &'static str,
}

impl ProbeFailure {
    fn new(message_id: &'static str) -> Self {
        Self { message_id }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReaderEvent {
    Message(Value),
    LimitExceeded,
    Eof,
    Failed,
}

/// 只合并精确模型 ID。第一阶段没有供应商声明和厂商规则的默认实现，保留
/// 参数是为了让未来扩展不会把优先级逻辑散落到目录、Windows、WSL 和 shell。
pub(crate) fn merge_exact(
    codex: Option<&ReasoningCapability>,
    provider: Option<&ReasoningCapability>,
    vendor: Option<&ReasoningCapability>,
) -> ReasoningCapability {
    if let Some(codex) = codex {
        match codex.state {
            CapabilityState::KnownNonEmpty => {
                if let Some(provider) = provider {
                    return intersect_capabilities(codex, provider);
                }
                return codex.clone();
            }
            CapabilityState::KnownEmpty | CapabilityState::Conflict => return codex.clone(),
            CapabilityState::NotFound | CapabilityState::ProbeFailed => {}
        }
    }
    if let Some(provider) = provider {
        return provider.clone();
    }
    if let Some(vendor) = vendor {
        return vendor.clone();
    }
    ReasoningCapability {
        state: CapabilityState::NotFound,
        levels: Vec::new(),
        default_level: None,
        source: CapabilitySource::Unknown,
        source_id: None,
        codex_version: None,
        observed_at_epoch_seconds: epoch_seconds(),
        message_id: "reasoning_metadata.unknown".to_owned(),
    }
}

fn intersect_capabilities(
    codex: &ReasoningCapability,
    provider: &ReasoningCapability,
) -> ReasoningCapability {
    let levels = codex
        .levels
        .iter()
        .filter(|level| provider.levels.iter().any(|candidate| candidate == *level))
        .cloned()
        .collect::<Vec<_>>();
    if levels.is_empty() {
        return ReasoningCapability {
            state: CapabilityState::Conflict,
            levels,
            default_level: None,
            source: CapabilitySource::Conflict,
            source_id: Some("codex.model/list+provider".to_owned()),
            codex_version: codex.codex_version.clone(),
            observed_at_epoch_seconds: epoch_seconds(),
            message_id: "reasoning_metadata.conflict".to_owned(),
        };
    }
    let default_level = provider
        .default_level
        .as_ref()
        .or(codex.default_level.as_ref())
        .filter(|level| levels.iter().any(|candidate| candidate == *level))
        .cloned();
    ReasoningCapability {
        state: CapabilityState::KnownNonEmpty,
        levels,
        default_level,
        source: CapabilitySource::CodexBuiltin,
        source_id: Some("codex.model/list+provider".to_owned()),
        codex_version: codex.codex_version.clone(),
        observed_at_epoch_seconds: epoch_seconds(),
        message_id: "reasoning_metadata.codex_intersected".to_owned(),
    }
}

pub(crate) fn probe_codex_metadata(
    model_ids: &[String],
) -> Result<CodexMetadataSnapshot, ProbeFailure> {
    let executable = crate::session::discover_codex_native_executable()
        .ok_or_else(|| ProbeFailure::new("reasoning_metadata.codex_executable_unavailable"))?;
    probe_codex_metadata_with_executable(&executable, model_ids)
}

pub(crate) fn snapshot_matches_current_codex(
    snapshot: &CodexMetadataSnapshot,
    model_ids: &[String],
) -> bool {
    let Some(executable) = crate::session::discover_codex_native_executable() else {
        return false;
    };
    let version = read_codex_version(&executable);
    let fingerprint = executable_fingerprint(&executable);
    snapshot_matches_metadata(
        snapshot,
        model_ids,
        version.as_deref(),
        fingerprint.as_deref(),
    )
}

pub(crate) fn snapshot_matches_metadata(
    snapshot: &CodexMetadataSnapshot,
    model_ids: &[String],
    codex_version: Option<&str>,
    executable_fingerprint: Option<&str>,
) -> bool {
    snapshot.covers_exact_models(model_ids)
        && snapshot.codex_version.as_deref() == codex_version
        && snapshot.executable_fingerprint.as_deref() == executable_fingerprint
        && codex_version.is_some()
        && executable_fingerprint.is_some()
}

pub(crate) fn apply_deepseek_compatibility(
    snapshot: &mut CodexMetadataSnapshot,
    default_model: &str,
) {
    for model in &mut snapshot.models {
        if model.model_id != default_model || model.state != CapabilityState::NotFound {
            continue;
        }
        model.state = CapabilityState::KnownNonEmpty;
        model.default_reasoning_effort = Some("high".to_owned());
        model.supported_reasoning_efforts =
            vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()];
        model.source = CapabilitySource::VendorCompatibility;
        model.source_id = "deepseek-codex-effort-v1".to_owned();
    }
}

pub(crate) fn apply_dayway_ds_compatibility(snapshot: &mut CodexMetadataSnapshot) {
    for model in &mut snapshot.models {
        if !matches!(
            model.model_id.as_str(),
            "deepseek-v4-flash" | "deepseek-v4-pro"
        ) || model.state != CapabilityState::NotFound
        {
            continue;
        }
        model.state = CapabilityState::KnownNonEmpty;
        model.default_reasoning_effort = Some("high".to_owned());
        model.supported_reasoning_efforts =
            vec!["low".to_owned(), "medium".to_owned(), "high".to_owned()];
        model.source = CapabilitySource::VendorCompatibility;
        model.source_id = "dayway-ds-deepseek-effort-v1".to_owned();
    }
}

fn probe_codex_metadata_with_executable(
    executable: &Path,
    model_ids: &[String],
) -> Result<CodexMetadataSnapshot, ProbeFailure> {
    if model_ids.is_empty() {
        return Ok(CodexMetadataSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            target_environment: "native".to_owned(),
            codex_version: read_codex_version(executable),
            executable_fingerprint: executable_fingerprint(executable),
            observed_at_epoch_seconds: epoch_seconds(),
            status: SnapshotStatus::Complete,
            models: Vec::new(),
        });
    }
    let workspace = ProbeWorkspace::new()?;
    write_probe_config(
        workspace.path(),
        model_ids.first().map(String::as_str).unwrap_or(""),
    )?;
    let mut process = spawn_probe_process(executable, workspace.path())?;
    let result = probe_process(&mut process, model_ids);
    cleanup_process(&mut process);
    result.map(|mut snapshot| {
        snapshot.codex_version = read_codex_version(executable);
        snapshot.executable_fingerprint = executable_fingerprint(executable);
        snapshot
    })
}

/// Reuse the bounded protocol parser over a transport scoped to another environment.
pub(crate) fn probe_via_command(
    mut command: Command,
    model_ids: &[String],
    target_environment: &str,
) -> Result<CodexMetadataSnapshot, ProbeFailure> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut process = command
        .spawn()
        .map_err(|_| ProbeFailure::new("reasoning_metadata.codex_probe_spawn_failed"))?;
    let result = probe_process(&mut process, model_ids);
    cleanup_process(&mut process);
    result.map(|mut snapshot| {
        snapshot.target_environment = target_environment.to_owned();
        snapshot
    })
}

fn spawn_probe_process(executable: &Path, home: &Path) -> Result<Child, ProbeFailure> {
    let mut command = Command::new(executable);
    command
        .args(["app-server", "--stdio"])
        .env("CODEX_HOME", home)
        .env("HOME", home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("OPENAI_BASE_URL")
        .env_remove("OPENAI_API_BASE")
        .env_remove("OPENAI_API_HOST")
        .env_remove("OPENAI_ORG_ID")
        .env_remove("OPENAI_ORGANIZATION")
        .env_remove("OPENAI_PROJECT")
        .env_remove("CODEX_API_KEY")
        .env_remove("CODEX_API_URL")
        .env_remove("CODEX_BASE_URL")
        .env_remove("CODEX_API_BASE")
        .env_remove("AZURE_OPENAI_API_KEY")
        .env_remove("AZURE_OPENAI_ENDPOINT")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GOOGLE_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(home);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .spawn()
        .map_err(|_| ProbeFailure::new("reasoning_metadata.codex_probe_spawn_failed"))
}

fn probe_process(
    process: &mut Child,
    model_ids: &[String],
) -> Result<CodexMetadataSnapshot, ProbeFailure> {
    let stdout = process
        .stdout
        .take()
        .ok_or_else(|| ProbeFailure::new("reasoning_metadata.codex_probe_protocol_failed"))?;
    let (sender, receiver) = mpsc::sync_channel(32);
    thread::spawn(move || read_messages(stdout, sender));
    let mut stdin = process
        .stdin
        .take()
        .ok_or_else(|| ProbeFailure::new("reasoning_metadata.codex_probe_protocol_failed"))?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    send_request(
        &mut stdin,
        1,
        "initialize",
        json!({
            "clientInfo": {"name": "gpteasy_reasoning_probe", "version": "1"},
            "capabilities": {"experimentalApi": false, "requestAttestation": false}
        }),
    )?;
    wait_for_response(&receiver, 1, deadline)?;
    send_notification(&mut stdin, "initialized", None)?;

    let mut cursor: Option<String> = None;
    let mut seen_cursors = HashSet::new();
    let mut pages = 0;
    let mut by_id = HashMap::<String, CodexModelMetadata>::new();
    let mut returned_model_ids = HashSet::<String>::new();
    loop {
        if pages >= MAX_PAGES {
            return Err(ProbeFailure::new(
                "reasoning_metadata.codex_probe_pagination_limit",
            ));
        }
        let mut params = Map::new();
        params.insert("includeHidden".to_owned(), Value::Bool(true));
        if let Some(cursor) = cursor.as_deref() {
            params.insert("cursor".to_owned(), Value::String(cursor.to_owned()));
        }
        let request_id = pages as u64 + 2;
        send_request(&mut stdin, request_id, "model/list", Value::Object(params))?;
        let result = wait_for_response(&receiver, request_id, deadline)?;
        let data = result
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| ProbeFailure::new("reasoning_metadata.codex_probe_bad_models"))?;
        for item in data {
            let model_id = model_id_from_item(item)?;
            if !returned_model_ids.insert(model_id.to_owned())
                && !model_ids.iter().any(|requested| requested == model_id)
            {
                continue;
            }
            if returned_model_ids.len() > MAX_MODELS {
                return Err(ProbeFailure::new(
                    "reasoning_metadata.codex_probe_model_limit",
                ));
            }
            if !model_ids.iter().any(|requested| requested == model_id) {
                continue;
            }
            let metadata = parse_model_metadata(model_id, item)?;
            if let Some(previous) = by_id.get(model_id) {
                if previous != &metadata {
                    by_id.insert(
                        model_id.to_owned(),
                        CodexModelMetadata {
                            model_id: model_id.to_owned(),
                            state: CapabilityState::Conflict,
                            default_reasoning_effort: None,
                            supported_reasoning_efforts: Vec::new(),
                            source: CapabilitySource::Conflict,
                            source_id: "codex.model/list.duplicate".to_owned(),
                        },
                    );
                }
            } else {
                by_id.insert(model_id.to_owned(), metadata);
            }
        }
        pages += 1;
        cursor = next_cursor(&result, cursor.as_deref(), &mut seen_cursors)?;
        if cursor.is_none() {
            break;
        }
    }
    let models = model_ids
        .iter()
        .map(|model_id| {
            by_id
                .remove(model_id)
                .unwrap_or_else(|| CodexModelMetadata {
                    model_id: model_id.clone(),
                    state: CapabilityState::NotFound,
                    default_reasoning_effort: None,
                    supported_reasoning_efforts: Vec::new(),
                    source: CapabilitySource::CodexBuiltin,
                    source_id: "codex.model/list".to_owned(),
                })
        })
        .collect();
    Ok(CodexMetadataSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        target_environment: "native".to_owned(),
        codex_version: None,
        executable_fingerprint: None,
        observed_at_epoch_seconds: epoch_seconds(),
        status: SnapshotStatus::Complete,
        models,
    })
}

fn parse_model_metadata(model_id: &str, item: &Value) -> Result<CodexModelMetadata, ProbeFailure> {
    let Some(supported) = item.get("supportedReasoningEfforts") else {
        return Err(ProbeFailure::new(
            "reasoning_metadata.codex_probe_missing_efforts",
        ));
    };
    let Some(items) = supported.as_array() else {
        return Err(ProbeFailure::new(
            "reasoning_metadata.codex_probe_bad_efforts",
        ));
    };
    let mut efforts = Vec::new();
    for value in items {
        let effort = value
            .as_str()
            .or_else(|| value.get("effort").and_then(Value::as_str))
            .or_else(|| value.get("reasoningEffort").and_then(Value::as_str))
            .filter(|value| safe_effort(value));
        let Some(effort) = effort else {
            return Err(ProbeFailure::new(
                "reasoning_metadata.codex_probe_bad_efforts",
            ));
        };
        if !efforts.iter().any(|existing| existing == effort) {
            efforts.push(effort.to_owned());
        }
    }
    let default_reasoning_effort = match item.get("defaultReasoningEffort") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if safe_effort(value) => Some(value.clone()),
        Some(_) => {
            return Err(ProbeFailure::new(
                "reasoning_metadata.codex_probe_bad_default_effort",
            ));
        }
    };
    let state = if default_reasoning_effort
        .as_ref()
        .is_some_and(|default| !efforts.iter().any(|effort| effort == default))
    {
        CapabilityState::Conflict
    } else if efforts.is_empty() {
        CapabilityState::KnownEmpty
    } else {
        CapabilityState::KnownNonEmpty
    };
    Ok(CodexModelMetadata {
        model_id: model_id.to_owned(),
        state,
        default_reasoning_effort,
        supported_reasoning_efforts: efforts,
        source: CapabilitySource::CodexBuiltin,
        source_id: "codex.model/list".to_owned(),
    })
}

fn model_id_from_item(item: &Value) -> Result<&str, ProbeFailure> {
    let model = item.get("model").and_then(Value::as_str);
    let id = item.get("id").and_then(Value::as_str);
    let model_id = match (model, id) {
        (Some(model), Some(id)) if model != id => {
            return Err(ProbeFailure::new(
                "reasoning_metadata.codex_probe_conflicting_model_id",
            ));
        }
        (Some(model), _) => model,
        (_, Some(id)) => id,
        (None, None) => {
            return Err(ProbeFailure::new(
                "reasoning_metadata.codex_probe_bad_model_id",
            ));
        }
    };
    if model_id.is_empty() {
        return Err(ProbeFailure::new(
            "reasoning_metadata.codex_probe_bad_model_id",
        ));
    }
    Ok(model_id)
}

fn next_cursor(
    result: &Value,
    current_cursor: Option<&str>,
    seen_cursors: &mut HashSet<String>,
) -> Result<Option<String>, ProbeFailure> {
    let left = result
        .get("nextCursor")
        .map(parse_cursor_value)
        .transpose()?;
    let right = result
        .get("next_cursor")
        .map(parse_cursor_value)
        .transpose()?;
    let next = match (left, right) {
        (Some(left), Some(right)) if left != right => {
            return Err(ProbeFailure::new(
                "reasoning_metadata.codex_probe_conflicting_cursor",
            ));
        }
        (Some(cursor), _) | (_, Some(cursor)) => cursor,
        (None, None) => None,
    };
    let Some(next) = next else {
        return Ok(None);
    };
    if next.is_empty()
        || current_cursor == Some(next.as_str())
        || !seen_cursors.insert(next.clone())
    {
        return Err(ProbeFailure::new(
            "reasoning_metadata.codex_probe_cursor_loop",
        ));
    }
    Ok(Some(next))
}

fn parse_cursor_value(value: &Value) -> Result<Option<String>, ProbeFailure> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ProbeFailure::new("reasoning_metadata.codex_probe_bad_cursor"))
        .map(Some)
}

fn read_messages(stdout: ChildStdout, sender: SyncSender<ReaderEvent>) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    let mut total = 0usize;
    loop {
        line.clear();
        let read = match reader.read_until(b'\n', &mut line) {
            Ok(read) => read,
            Err(_) => {
                let _ = sender.send(ReaderEvent::Failed);
                return;
            }
        };
        if read == 0 {
            let _ = sender.send(ReaderEvent::Eof);
            return;
        }
        total = total.saturating_add(read);
        if total > MAX_RESPONSE_BYTES || line.len() > MAX_LINE_BYTES {
            let _ = sender.send(ReaderEvent::LimitExceeded);
            return;
        }
        let value = match serde_json::from_slice::<Value>(&line) {
            Ok(value) => value,
            Err(_) => {
                let _ = sender.send(ReaderEvent::Failed);
                return;
            }
        };
        if sender.send(ReaderEvent::Message(value)).is_err() {
            return;
        }
    }
}

fn send_request(
    stdin: &mut impl Write,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), ProbeFailure> {
    let mut message = serde_json::to_vec(&json!({"id": id, "method": method, "params": params}))
        .map_err(|_| ProbeFailure::new("reasoning_metadata.codex_probe_protocol_failed"))?;
    message.push(b'\n');
    stdin
        .write_all(&message)
        .and_then(|_| stdin.flush())
        .map_err(|_| ProbeFailure::new("reasoning_metadata.codex_probe_unexpected_exit"))
}

fn send_notification(
    stdin: &mut impl Write,
    method: &str,
    params: Option<Value>,
) -> Result<(), ProbeFailure> {
    let mut object = Map::new();
    object.insert("method".to_owned(), Value::String(method.to_owned()));
    if let Some(params) = params {
        object.insert("params".to_owned(), params);
    }
    let mut message = serde_json::to_vec(&Value::Object(object))
        .map_err(|_| ProbeFailure::new("reasoning_metadata.codex_probe_protocol_failed"))?;
    message.push(b'\n');
    stdin
        .write_all(&message)
        .and_then(|_| stdin.flush())
        .map_err(|_| ProbeFailure::new("reasoning_metadata.codex_probe_unexpected_exit"))
}

fn wait_for_response(
    receiver: &Receiver<ReaderEvent>,
    id: u64,
    deadline: Instant,
) -> Result<Value, ProbeFailure> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ProbeFailure::new("reasoning_metadata.codex_probe_timeout"));
        }
        match receiver.recv_timeout(remaining) {
            Ok(ReaderEvent::Message(value))
                if value.get("id").and_then(Value::as_u64) == Some(id) =>
            {
                if value.get("error").is_some() {
                    return Err(ProbeFailure::new(
                        "reasoning_metadata.codex_probe_rpc_error",
                    ));
                }
                return value.get("result").cloned().ok_or_else(|| {
                    ProbeFailure::new("reasoning_metadata.codex_probe_protocol_failed")
                });
            }
            Ok(ReaderEvent::Message(_)) => continue,
            Ok(ReaderEvent::Eof) | Ok(ReaderEvent::Failed) => {
                return Err(ProbeFailure::new(
                    "reasoning_metadata.codex_probe_unexpected_exit",
                ));
            }
            Ok(ReaderEvent::LimitExceeded) => {
                return Err(ProbeFailure::new(
                    "reasoning_metadata.codex_probe_response_too_large",
                ));
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(ProbeFailure::new("reasoning_metadata.codex_probe_timeout"));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(ProbeFailure::new(
                    "reasoning_metadata.codex_probe_unexpected_exit",
                ));
            }
        }
    }
}

fn cleanup_process(process: &mut Child) {
    if process.try_wait().ok().flatten().is_some() {
        return;
    }
    let _ = process.kill();
    let deadline = Instant::now() + CHILD_CLEANUP_TIMEOUT;
    while Instant::now() < deadline {
        if process.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = process.wait();
}

struct ProbeWorkspace {
    path: PathBuf,
}

impl ProbeWorkspace {
    fn new() -> Result<Self, ProbeFailure> {
        let root = std::env::temp_dir();
        for _ in 0..3 {
            let path = root.join(format!("gpteasy-reasoning-probe-{}", Uuid::new_v4()));
            if fs::create_dir(&path).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).is_err() {
                        let _ = fs::remove_dir(&path);
                        continue;
                    }
                }
                return Ok(Self { path });
            }
        }
        Err(ProbeFailure::new(
            "reasoning_metadata.codex_probe_workspace_failed",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ProbeWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_probe_config(home: &Path, model: &str) -> Result<(), ProbeFailure> {
    let config = format!(
        "model = {}\nmodel_provider = \"offline_probe\"\n[analytics]\nenabled = false\n[model_providers.offline_probe]\nname = \"GPTEasy offline reasoning probe\"\nbase_url = \"http://127.0.0.1:9/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = false\n",
        toml_string(model),
    );
    let path = home.join("config.toml");
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| ProbeFailure::new("reasoning_metadata.codex_probe_workspace_failed"))?;
    file.write_all(config.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| ProbeFailure::new("reasoning_metadata.codex_probe_workspace_failed"))
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn read_codex_version(executable: &Path) -> Option<String> {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .env_remove("OPENAI_API_KEY")
        .env_remove("OPENAI_BASE_URL")
        .env_remove("OPENAI_API_BASE")
        .env_remove("OPENAI_API_HOST")
        .env_remove("OPENAI_ORG_ID")
        .env_remove("OPENAI_ORGANIZATION")
        .env_remove("OPENAI_PROJECT")
        .env_remove("CODEX_API_KEY")
        .env_remove("CODEX_API_URL")
        .env_remove("CODEX_BASE_URL")
        .env_remove("CODEX_API_BASE")
        .env_remove("AZURE_OPENAI_API_KEY")
        .env_remove("AZURE_OPENAI_ENDPOINT")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GOOGLE_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().ok().flatten() {
            if !status.success() {
                return None;
            }
            let mut output = Vec::new();
            child
                .stdout
                .take()?
                .take(1025)
                .read_to_end(&mut output)
                .ok()?;
            let text = String::from_utf8(output).ok()?.trim().to_owned();
            return (!text.is_empty()
                && text.len() <= 128
                && text.chars().all(|c| !c.is_control()))
            .then_some(text);
        }
        if started.elapsed() >= VERSION_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn executable_fingerprint(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(format!("sha256:{:x}", hasher.finalize()))
}

fn safe_effort(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(state: CapabilityState, levels: &[&str]) -> ReasoningCapability {
        ReasoningCapability {
            state,
            levels: levels.iter().map(|level| (*level).to_owned()).collect(),
            default_level: levels.first().map(|level| (*level).to_owned()),
            source: CapabilitySource::CodexBuiltin,
            source_id: Some("test".to_owned()),
            codex_version: Some("0.160.1".to_owned()),
            observed_at_epoch_seconds: 1,
            message_id: "test".to_owned(),
        }
    }

    #[test]
    fn exact_merge_preserves_known_empty_and_does_not_fallback() {
        let codex = capability(CapabilityState::KnownEmpty, &[]);
        let vendor = capability(CapabilityState::KnownNonEmpty, &["low", "high"]);
        let result = merge_exact(Some(&codex), None, Some(&vendor));
        assert_eq!(result.state, CapabilityState::KnownEmpty);
        assert!(result.levels.is_empty());
    }

    #[test]
    fn exact_merge_intersects_provider_declaration() {
        let codex = capability(CapabilityState::KnownNonEmpty, &["low", "medium", "high"]);
        let mut provider = capability(CapabilityState::KnownNonEmpty, &["medium", "high"]);
        provider.default_level = Some("high".to_owned());
        let result = merge_exact(Some(&codex), Some(&provider), None);
        assert_eq!(result.levels, vec!["medium", "high"]);
        assert_eq!(result.default_level.as_deref(), Some("high"));
    }

    #[test]
    fn model_metadata_requires_explicit_supported_efforts_field() {
        let error = parse_model_metadata(
            "model-a",
            &json!({
                "model": "model-a",
                "defaultReasoningEffort": "high"
            }),
        )
        .expect_err("missing field must not become unknown success");
        assert_eq!(
            error.message_id,
            "reasoning_metadata.codex_probe_missing_efforts"
        );
    }

    #[test]
    fn model_metadata_distinguishes_known_empty() {
        let metadata = parse_model_metadata(
            "model-a",
            &json!({"model":"model-a","supportedReasoningEfforts":[]}),
        )
        .unwrap();
        assert_eq!(metadata.state, CapabilityState::KnownEmpty);
        assert!(metadata.supported_reasoning_efforts.is_empty());
    }

    #[test]
    fn model_metadata_accepts_codex_reasoning_effort_objects() {
        let metadata = parse_model_metadata(
            "model-a",
            &json!({
                "model": "model-a",
                "defaultReasoningEffort": "medium",
                "supportedReasoningEfforts": [
                    {"reasoningEffort": "low", "description": "lighter"},
                    {"reasoningEffort": "medium", "description": "balanced"}
                ]
            }),
        )
        .unwrap();
        assert_eq!(metadata.state, CapabilityState::KnownNonEmpty);
        assert_eq!(metadata.default_reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(metadata.supported_reasoning_efforts, ["low", "medium"]);
    }

    #[test]
    fn snapshot_cache_requires_matching_version_and_executable_fingerprint() {
        let snapshot = CodexMetadataSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            target_environment: "native".to_owned(),
            codex_version: Some("0.160.1".to_owned()),
            executable_fingerprint: Some("sha256:one".to_owned()),
            observed_at_epoch_seconds: 1,
            status: SnapshotStatus::Complete,
            models: vec![CodexModelMetadata {
                model_id: "model-a".to_owned(),
                state: CapabilityState::KnownNonEmpty,
                default_reasoning_effort: Some("medium".to_owned()),
                supported_reasoning_efforts: vec!["medium".to_owned()],
                source: CapabilitySource::CodexBuiltin,
                source_id: "codex.model/list".to_owned(),
            }],
        };
        let model_ids = vec!["model-a".to_owned()];
        assert!(snapshot_matches_metadata(
            &snapshot,
            &model_ids,
            Some("0.160.1"),
            Some("sha256:one")
        ));
        assert!(!snapshot_matches_metadata(
            &snapshot,
            &model_ids,
            Some("0.160.1"),
            Some("sha256:two")
        ));
        assert!(!snapshot_matches_metadata(
            &snapshot,
            &model_ids,
            Some("0.160.0"),
            Some("sha256:one")
        ));
        assert!(!snapshot_matches_metadata(
            &snapshot,
            &model_ids,
            Some("0.160.1"),
            None
        ));

        let mut without_fingerprint = snapshot.clone();
        without_fingerprint.executable_fingerprint = None;
        assert!(!snapshot_matches_metadata(
            &without_fingerprint,
            &model_ids,
            Some("0.160.1"),
            Some("sha256:one")
        ));
    }

    #[test]
    fn snapshot_serialization_is_versioned_and_exact() {
        let snapshot = CodexMetadataSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            target_environment: "native".to_owned(),
            codex_version: Some("0.160.1".to_owned()),
            executable_fingerprint: Some("sha256:test".to_owned()),
            observed_at_epoch_seconds: 1,
            status: SnapshotStatus::Complete,
            models: vec![CodexModelMetadata {
                model_id: "gpt-6.1-sol".to_owned(),
                state: CapabilityState::KnownNonEmpty,
                default_reasoning_effort: Some("medium".to_owned()),
                supported_reasoning_efforts: vec!["low".to_owned(), "medium".to_owned()],
                source: CapabilitySource::CodexBuiltin,
                source_id: "codex.model/list".to_owned(),
            }],
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded: CodexMetadataSnapshot = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.covers_exact_models(&["gpt-6.1-sol".to_owned()]));
        assert!(!decoded.covers_exact_models(&["gpt-6.1-sol-v2".to_owned()]));
    }

    #[test]
    fn dayway_ds_compatibility_matches_only_the_two_supported_models() {
        let mut snapshot = CodexMetadataSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            target_environment: "native".to_owned(),
            codex_version: Some("0.160.1".to_owned()),
            executable_fingerprint: Some("sha256:test".to_owned()),
            observed_at_epoch_seconds: 1,
            status: SnapshotStatus::Complete,
            models: [
                "deepseek-v4-flash",
                "deepseek-v4-pro",
                "deepseek-v4-pro-preview",
            ]
            .into_iter()
            .map(|model_id| CodexModelMetadata {
                model_id: model_id.to_owned(),
                state: CapabilityState::NotFound,
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
                source: CapabilitySource::CodexBuiltin,
                source_id: "codex.model/list".to_owned(),
            })
            .collect(),
        };

        apply_dayway_ds_compatibility(&mut snapshot);

        assert_eq!(snapshot.models[0].state, CapabilityState::KnownNonEmpty);
        assert_eq!(snapshot.models[1].state, CapabilityState::KnownNonEmpty);
        assert_eq!(
            snapshot.models[0].default_reasoning_effort.as_deref(),
            Some("high")
        );
        assert_eq!(
            snapshot.models[1].supported_reasoning_efforts,
            ["low", "medium", "high"]
        );
        assert_eq!(
            snapshot.models[0].source,
            CapabilitySource::VendorCompatibility
        );
        assert_eq!(snapshot.models[2].state, CapabilityState::NotFound);
        assert!(snapshot.models[2].supported_reasoning_efforts.is_empty());
    }
}
