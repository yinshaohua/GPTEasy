use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

const STATE_DATABASE: &str = "state_5.sqlite";
const REQUIRED_THREAD_COLUMNS: [&str; 6] = [
    "id",
    "rollout_path",
    "source",
    "model_provider",
    "has_user_event",
    "archived",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityTargetMode {
    OpenaiLogin,
    Provider,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityTarget {
    pub mode: VisibilityTargetMode,
    pub model_provider: String,
    pub environment_revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityAppServerCapability {
    Available,
    Unavailable,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityScanContext {
    pub target: VisibilityTarget,
    pub codex_version: Option<String>,
    pub app_server: VisibilityAppServerCapability,
    pub execution_blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilitySummary {
    pub candidates: u32,
    pub unchanged: u32,
    pub missing_index: u32,
    pub skipped: u32,
    pub blocked: u32,
    pub encrypted_content_risk: u32,
    pub active: u32,
    pub archived: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilitySchemaCapability {
    pub status: String,
    pub database: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityReason {
    pub code: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionVisibilityPreview {
    pub target: VisibilityTarget,
    pub codex_version: Option<String>,
    pub app_server: VisibilityAppServerCapability,
    pub schema: VisibilitySchemaCapability,
    pub summary: VisibilitySummary,
    pub can_execute: bool,
    pub blockers: Vec<String>,
    pub reasons: Vec<VisibilityReason>,
}

impl SessionVisibilityPreview {
    pub fn diagnostic_details(&self) -> String {
        let target_mode = match self.target.mode {
            VisibilityTargetMode::OpenaiLogin => "openai_login",
            VisibilityTargetMode::Provider => "provider",
            VisibilityTargetMode::Unknown => "unknown",
        };
        let codex_version = self
            .codex_version
            .as_deref()
            .map(safe_diagnostic_value)
            .unwrap_or_else(|| "unknown".to_owned());
        let error_codes = if self.blockers.is_empty() {
            "none".to_owned()
        } else {
            self.blockers.join(",")
        };
        format!(
            "stage=scan; target_mode={target_mode}; codex_version={codex_version}; \
             schema={}; candidates={}; unchanged={}; missing_index={}; skipped={}; \
             blocked={}; encrypted_content_risk={}; active={}; archived={}; \
             error_codes={error_codes}",
            self.schema.status,
            self.summary.candidates,
            self.summary.unchanged,
            self.summary.missing_index,
            self.summary.skipped,
            self.summary.blocked,
            self.summary.encrypted_content_risk,
            self.summary.active,
            self.summary.archived,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibilityFailure {
    pub message_id: &'static str,
}

#[derive(Clone)]
pub struct SessionVisibilityApplication {
    codex_home: PathBuf,
}

impl SessionVisibilityApplication {
    pub fn new(codex_home: impl AsRef<Path>) -> Self {
        Self {
            codex_home: codex_home.as_ref().to_path_buf(),
        }
    }

    pub fn scan(
        &self,
        context: VisibilityScanContext,
    ) -> Result<SessionVisibilityPreview, VisibilityFailure> {
        let index = read_index(&self.codex_home);
        let mut summary = VisibilitySummary {
            candidates: 0,
            unchanged: 0,
            missing_index: 0,
            skipped: 0,
            blocked: 0,
            encrypted_content_risk: 0,
            active: 0,
            archived: 0,
        };
        let mut reasons = BTreeMap::<String, u32>::new();
        self.scan_directory(
            &self.codex_home.join("sessions"),
            false,
            &context.target.model_provider,
            &index,
            &mut summary,
            &mut reasons,
        )?;
        self.scan_directory(
            &self.codex_home.join("archived_sessions"),
            true,
            &context.target.model_provider,
            &index,
            &mut summary,
            &mut reasons,
        )?;

        let mut blockers = context.execution_blockers;
        if context.app_server != VisibilityAppServerCapability::Available {
            blockers.push(
                match context.app_server {
                    VisibilityAppServerCapability::Unavailable => "app_server_unavailable",
                    VisibilityAppServerCapability::Incompatible => "app_server_incompatible",
                    VisibilityAppServerCapability::Available => unreachable!(),
                }
                .to_owned(),
            );
        }
        if index.schema_status != "supported" {
            blockers.push("unsupported_index_schema".to_owned());
        }
        blockers.sort();
        blockers.dedup();
        let can_execute = blockers.is_empty();
        if !can_execute {
            summary.blocked = summary.candidates;
            for blocker in &blockers {
                increment(&mut reasons, blocker);
            }
        }

        Ok(SessionVisibilityPreview {
            target: context.target,
            codex_version: context.codex_version,
            app_server: context.app_server,
            schema: VisibilitySchemaCapability {
                status: index.schema_status,
                database: STATE_DATABASE.to_owned(),
            },
            summary,
            can_execute,
            blockers,
            reasons: reasons
                .into_iter()
                .map(|(code, count)| VisibilityReason { code, count })
                .collect(),
        })
    }

    pub fn add_execution_blocker(preview: &mut SessionVisibilityPreview, blocker: &str) {
        if preview.blockers.iter().any(|existing| existing == blocker) {
            return;
        }
        preview.blockers.push(blocker.to_owned());
        preview.blockers.sort();
        preview.can_execute = false;
        preview.summary.blocked = preview.summary.candidates;
        if let Some(reason) = preview
            .reasons
            .iter_mut()
            .find(|reason| reason.code == blocker)
        {
            reason.count += 1;
        } else {
            preview.reasons.push(VisibilityReason {
                code: blocker.to_owned(),
                count: 1,
            });
            preview
                .reasons
                .sort_by(|left, right| left.code.cmp(&right.code));
        }
    }

    fn scan_directory(
        &self,
        root: &Path,
        archived: bool,
        target_provider: &str,
        index: &IndexSnapshot,
        summary: &mut VisibilitySummary,
        reasons: &mut BTreeMap<String, u32>,
    ) -> Result<(), VisibilityFailure> {
        if !root.exists() {
            return Ok(());
        }
        let mut paths = Vec::new();
        collect_rollouts(root, &mut paths)?;
        paths.sort();
        for path in paths {
            if archived {
                summary.archived += 1;
            } else {
                summary.active += 1;
            }
            let rollout = match read_rollout(&path) {
                Ok(rollout) => rollout,
                Err(code) => {
                    summary.skipped += 1;
                    increment(reasons, code);
                    continue;
                }
            };
            let Some(source) = rollout.source.as_deref() else {
                summary.skipped += 1;
                increment(reasons, "identity_ambiguous");
                continue;
            };
            if let Some(code) = excluded_source_reason(source, rollout.thread_source.as_deref()) {
                summary.skipped += 1;
                increment(reasons, code);
                continue;
            }
            if rollout.has_derived_identity || !rollout.has_user_event {
                summary.skipped += 1;
                increment(
                    reasons,
                    if rollout.has_derived_identity {
                        "excluded_derived"
                    } else {
                        "no_user_event"
                    },
                );
                continue;
            }
            let Some(indexed) = index.rows.get(&rollout.id) else {
                summary.candidates += 1;
                summary.missing_index += 1;
                increment(reasons, "index_missing");
                if rollout.has_encrypted_content {
                    summary.encrypted_content_risk += 1;
                    increment(reasons, "encrypted_content");
                }
                continue;
            };
            if !same_rollout_path(&indexed.rollout_path, &path)
                || indexed.source != source
                || indexed.archived != archived
                || !indexed.has_user_event
            {
                summary.skipped += 1;
                increment(reasons, "identity_ambiguous");
                continue;
            }
            if rollout.model_provider.as_deref() == Some(target_provider)
                && indexed.model_provider == target_provider
            {
                summary.unchanged += 1;
                continue;
            }
            summary.candidates += 1;
            increment(reasons, "provider_mismatch");
            if rollout.has_encrypted_content {
                summary.encrypted_content_risk += 1;
                increment(reasons, "encrypted_content");
            }
        }
        Ok(())
    }
}

struct IndexedThread {
    rollout_path: PathBuf,
    source: String,
    model_provider: String,
    has_user_event: bool,
    archived: bool,
}

struct IndexSnapshot {
    schema_status: String,
    rows: HashMap<String, IndexedThread>,
}

fn read_index(codex_home: &Path) -> IndexSnapshot {
    let database = codex_home.join(STATE_DATABASE);
    let Ok(connection) = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return IndexSnapshot {
            schema_status: "missing".to_owned(),
            rows: HashMap::new(),
        };
    };
    let Ok(columns) = thread_columns(&connection) else {
        return IndexSnapshot {
            schema_status: "unknown".to_owned(),
            rows: HashMap::new(),
        };
    };
    if !REQUIRED_THREAD_COLUMNS
        .iter()
        .all(|required| columns.iter().any(|column| column == required))
    {
        return IndexSnapshot {
            schema_status: "unknown".to_owned(),
            rows: HashMap::new(),
        };
    }
    let Ok(mut statement) = connection.prepare(
        "SELECT id, rollout_path, source, model_provider, has_user_event, archived FROM threads",
    ) else {
        return IndexSnapshot {
            schema_status: "unknown".to_owned(),
            rows: HashMap::new(),
        };
    };
    let Ok(mapped) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            IndexedThread {
                rollout_path: PathBuf::from(row.get::<_, String>(1)?),
                source: row.get(2)?,
                model_provider: row.get(3)?,
                has_user_event: row.get(4)?,
                archived: row.get(5)?,
            },
        ))
    }) else {
        return IndexSnapshot {
            schema_status: "unknown".to_owned(),
            rows: HashMap::new(),
        };
    };
    let mut rows = HashMap::new();
    for row in mapped {
        let Ok((id, indexed)) = row else {
            return IndexSnapshot {
                schema_status: "unknown".to_owned(),
                rows: HashMap::new(),
            };
        };
        rows.insert(id, indexed);
    }
    IndexSnapshot {
        schema_status: "supported".to_owned(),
        rows,
    }
}

fn thread_columns(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare("PRAGMA table_info(threads)")?;
    statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect()
}

fn same_rollout_path(indexed: &Path, observed: &Path) -> bool {
    match (fs::canonicalize(indexed), fs::canonicalize(observed)) {
        (Ok(indexed), Ok(observed)) => indexed == observed,
        _ => false,
    }
}

struct RolloutObservation {
    id: String,
    source: Option<String>,
    thread_source: Option<String>,
    model_provider: Option<String>,
    has_derived_identity: bool,
    has_user_event: bool,
    has_encrypted_content: bool,
}

fn read_rollout(path: &Path) -> Result<RolloutObservation, &'static str> {
    let file = fs::File::open(path).map_err(|_| "rollout_unreadable")?;
    let mut lines = BufReader::new(file).lines();
    let first = lines
        .next()
        .ok_or("rollout_damaged")?
        .map_err(|_| "rollout_unreadable")?;
    let first = serde_json::from_str::<Value>(&first).map_err(|_| "rollout_damaged")?;
    if first.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Err("identity_ambiguous");
    }
    let payload = first
        .get("payload")
        .and_then(Value::as_object)
        .ok_or("identity_ambiguous")?;
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| Uuid::parse_str(id).is_ok())
        .ok_or("identity_ambiguous")?
        .to_owned();
    let source = payload
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let thread_source = payload
        .get("thread_source")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let model_provider = payload
        .get("model_provider")
        .and_then(Value::as_str)
        .filter(|provider| !provider.trim().is_empty())
        .map(str::to_owned);
    let has_derived_identity = [
        "forked_from_id",
        "parent_thread_id",
        "agent_nickname",
        "agent_role",
        "agent_path",
    ]
    .iter()
    .any(|field| payload.get(*field).is_some_and(|value| !value.is_null()));
    let mut observation = RolloutObservation {
        id,
        source,
        thread_source,
        model_provider,
        has_derived_identity,
        has_user_event: false,
        has_encrypted_content: contains_encrypted_content(&first),
    };
    for line in lines {
        let line = line.map_err(|_| "rollout_unreadable")?;
        let value = serde_json::from_str::<Value>(&line).map_err(|_| "rollout_damaged")?;
        observation.has_user_event |= is_user_event(&value);
        observation.has_encrypted_content |= contains_encrypted_content(&value);
    }
    Ok(observation)
}

fn is_user_event(value: &Value) -> bool {
    let item_type = value.get("type").and_then(Value::as_str);
    let payload = value.get("payload");
    matches!(
        (
            item_type,
            payload
                .and_then(|payload| payload.get("type"))
                .and_then(Value::as_str),
            payload
                .and_then(|payload| payload.get("role"))
                .and_then(Value::as_str),
        ),
        (Some("event_msg"), Some("user_message"), _)
            | (Some("response_item"), Some("message"), Some("user"))
    )
}

fn contains_encrypted_content(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "encrypted_content" && !value.is_null()) || contains_encrypted_content(value)
        }),
        Value::Array(values) => values.iter().any(contains_encrypted_content),
        _ => false,
    }
}

fn excluded_source_reason(source: &str, thread_source: Option<&str>) -> Option<&'static str> {
    if thread_source.is_some_and(|source| source != "user") {
        return Some("excluded_internal");
    }
    match source {
        "cli" | "vscode" | "appServer" => None,
        "exec" => Some("excluded_exec"),
        "subAgent"
        | "subagent"
        | "subAgentReview"
        | "subAgentCompact"
        | "subAgentThreadSpawn"
        | "subAgentOther" => Some("excluded_subagent"),
        "mcp" | "automation" | "internal" => Some("excluded_internal"),
        "remote" | "remoteHost" => Some("excluded_remote"),
        _ => Some("identity_ambiguous"),
    }
}

fn collect_rollouts(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), VisibilityFailure> {
    let entries = fs::read_dir(root).map_err(|_| VisibilityFailure {
        message_id: "session_visibility.scan_failed",
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| VisibilityFailure {
            message_id: "session_visibility.scan_failed",
        })?;
        let file_type = entry.file_type().map_err(|_| VisibilityFailure {
            message_id: "session_visibility.scan_failed",
        })?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_rollouts(&entry.path(), paths)?;
        } else if file_type.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn increment(reasons: &mut BTreeMap<String, u32>, code: &str) {
    *reasons.entry(code.to_owned()).or_default() += 1;
}

fn safe_diagnostic_value(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '.' | '-' | '_')
        })
        .take(64)
        .collect()
}
