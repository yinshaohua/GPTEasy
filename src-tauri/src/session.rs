use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::state::{SessionProcessOwnership, StateStore};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_IDLE_GRACE: Duration = Duration::from_secs(8);
const INTERACTIVE_SOURCE_KINDS: [&str; 3] = ["cli", "vscode", "appServer"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionAvailabilityStatus {
    Available,
    CodexMissing,
    Incompatible,
    InitializationFailed,
    RecoveryFailed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAvailability {
    pub status: SessionAvailabilityStatus,
    pub message_id: String,
    pub codex_version: Option<String>,
    pub mutation: SessionMutationAvailability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMutationAvailabilityStatus {
    Allowed,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMutationAvailability {
    pub status: SessionMutationAvailabilityStatus,
    pub message_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMutationResultStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionActualState {
    Active,
    Archived,
    Deleted,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMutationResult {
    pub session_id: String,
    pub status: SessionMutationResultStatus,
    pub actual_state: SessionActualState,
    pub message_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQuery {
    pub request_id: Option<String>,
    pub archived: bool,
    pub search_term: Option<String>,
    pub project: Option<String>,
    pub model_provider: Option<String>,
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListPage {
    pub sessions: Vec<SessionSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub forked_from_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub title: String,
    pub preview: String,
    pub project: String,
    pub model_provider: String,
    pub source: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    #[serde(flatten)]
    pub summary: SessionSummary,
    pub entries: Vec<SessionEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEntryKind {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    pub id: String,
    pub kind: SessionEntryKind,
    pub label: String,
    pub content: String,
    pub output: Option<String>,
}

impl SessionDetail {
    pub fn to_markdown(&self) -> String {
        let mut markdown = format!(
            "# {}\n\n- 项目：{}\n- 会话供应商：{}\n- 来源：{}\n- 创建时间：{}\n- 更新时间：{}\n",
            self.summary.title,
            self.summary.project,
            self.summary.model_provider,
            self.summary.source,
            self.summary.created_at,
            self.summary.updated_at,
        );
        for entry in &self.entries {
            match entry.kind {
                SessionEntryKind::User => {
                    markdown.push_str("\n## 用户\n\n");
                    markdown.push_str(&entry.content);
                    markdown.push('\n');
                }
                SessionEntryKind::Assistant => {
                    markdown.push_str("\n## 助手\n\n");
                    markdown.push_str(&entry.content);
                    markdown.push('\n');
                }
                SessionEntryKind::Tool => {
                    markdown.push_str(&format!(
                        "\n<details>\n<summary>{}</summary>\n\n",
                        entry.label
                    ));
                    markdown.push_str("```text\n");
                    markdown.push_str(&entry.content);
                    markdown.push_str("\n```\n");
                    if let Some(output) = &entry.output {
                        markdown.push_str("\n输出：\n\n```text\n");
                        markdown.push_str(output);
                        markdown.push_str("\n```\n");
                    }
                    markdown.push_str("\n</details>\n");
                }
            }
        }
        markdown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionFailureCategory {
    CodexMissing,
    Incompatible,
    InitializationFailed,
    UnexpectedExit,
    Protocol,
    RequestFailed,
    RecoveryFailed,
    WriteFailed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFailure {
    pub category: SessionFailureCategory,
    pub message_id: String,
}

impl SessionFailure {
    pub(crate) fn new(category: SessionFailureCategory, message_id: &str) -> Self {
        Self {
            category,
            message_id: message_id.to_owned(),
        }
    }

    fn recoverable(&self) -> bool {
        matches!(
            self.category,
            SessionFailureCategory::UnexpectedExit | SessionFailureCategory::Protocol
        )
    }
}

#[derive(Clone)]
pub struct SessionApplication {
    gateway: AppServerGateway,
    leases: Arc<Mutex<LeaseState>>,
    list_requests: Arc<Mutex<HashMap<String, CancellationToken>>>,
    idle_grace: Duration,
}

#[derive(Default)]
struct LeaseState {
    active: HashSet<String>,
    generation: u64,
}

#[derive(Clone, Copy)]
enum MutationKind {
    Archive,
    Unarchive,
    Delete,
}

impl MutationKind {
    fn initial_state(self) -> SessionActualState {
        match self {
            Self::Archive | Self::Delete => SessionActualState::Active,
            Self::Unarchive => SessionActualState::Archived,
        }
    }

    fn method(self) -> &'static str {
        match self {
            Self::Archive => "thread/archive",
            Self::Unarchive => "thread/unarchive",
            Self::Delete => "thread/delete",
        }
    }

    fn success_state(self) -> SessionActualState {
        match self {
            Self::Archive => SessionActualState::Archived,
            Self::Unarchive => SessionActualState::Active,
            Self::Delete => SessionActualState::Deleted,
        }
    }

    fn success_message(self) -> &'static str {
        match self {
            Self::Archive => "session.archived",
            Self::Unarchive => "session.unarchived",
            Self::Delete => "session.deleted",
        }
    }

    fn reconciled_message(self) -> &'static str {
        match self {
            Self::Archive => "session.archive_reconciled",
            Self::Unarchive => "session.unarchive_reconciled",
            Self::Delete => "session.delete_reconciled",
        }
    }
}

impl SessionApplication {
    pub fn new(state_store: StateStore) -> Self {
        Self {
            gateway: AppServerGateway::new(state_store),
            leases: Arc::new(Mutex::new(LeaseState::default())),
            list_requests: Arc::new(Mutex::new(HashMap::new())),
            idle_grace: DEFAULT_IDLE_GRACE,
        }
    }

    #[doc(hidden)]
    pub fn with_program_for_harness(
        state_store: StateStore,
        program: PathBuf,
        args_prefix: Vec<OsString>,
        idle_grace: Duration,
    ) -> Self {
        Self {
            gateway: AppServerGateway::with_launch(
                state_store,
                LaunchCommand {
                    identity: program.clone(),
                    program,
                    args_prefix,
                },
            ),
            leases: Arc::new(Mutex::new(LeaseState::default())),
            list_requests: Arc::new(Mutex::new(HashMap::new())),
            idle_grace,
        }
    }

    pub async fn enter(&self, lease_id: &str) -> SessionAvailability {
        {
            let mut leases = self.leases.lock().await;
            leases.active.insert(lease_id.to_owned());
            leases.generation = leases.generation.wrapping_add(1);
        }
        match self.gateway.start_for_entry().await {
            Ok(capability) => SessionAvailability {
                status: SessionAvailabilityStatus::Available,
                message_id: "session.available".to_owned(),
                codex_version: Some(capability.codex_version),
                mutation: mutations_allowed(),
            },
            Err(failure) => availability_from_failure(failure),
        }
    }

    pub async fn leave(&self, lease_id: &str) {
        let generation = {
            let mut leases = self.leases.lock().await;
            leases.active.remove(lease_id);
            leases.generation = leases.generation.wrapping_add(1);
            if !leases.active.is_empty() {
                return;
            }
            leases.generation
        };
        self.schedule_idle_shutdown(generation, true);
    }

    pub async fn list(&self, query: SessionQuery) -> Result<SessionListPage, SessionFailure> {
        let Some(request_id) = query.request_id.clone() else {
            return self.list_uncancellable(query).await;
        };
        let cancellation = CancellationToken::new();
        self.list_requests
            .lock()
            .await
            .insert(request_id.clone(), cancellation.clone());
        let result = tokio::select! {
            _ = cancellation.cancelled() => Err(SessionFailure::new(
                SessionFailureCategory::Cancelled,
                "session.request_cancelled",
            )),
            result = self.list_uncancellable(query) => result,
        };
        self.list_requests.lock().await.remove(&request_id);
        result
    }

    async fn list_uncancellable(
        &self,
        query: SessionQuery,
    ) -> Result<SessionListPage, SessionFailure> {
        if self.gateway.recovery_is_blocked() {
            return Err(recovery_failure());
        }
        match self.gateway.list(&query).await {
            Ok(page) => Ok(page),
            Err(failure) if failure.recoverable() => {
                if !self.gateway.begin_recovery() {
                    return Err(recovery_failure());
                }
                let recovered = match self.gateway.restart().await {
                    Ok(_) => self.gateway.list(&query).await,
                    Err(failure) => Err(failure),
                };
                if recovered.is_ok() {
                    self.gateway.complete_recovery();
                }
                recovered.map_err(|_| recovery_failure())
            }
            Err(failure) => Err(failure),
        }
    }

    pub async fn cancel_list_request(&self, request_id: &str) -> bool {
        let cancellation = self.list_requests.lock().await.get(request_id).cloned();
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
            true
        } else {
            false
        }
    }

    pub async fn read(&self, session_id: &str) -> Result<SessionDetail, SessionFailure> {
        if self.gateway.recovery_is_blocked() {
            return Err(recovery_failure());
        }
        match self.gateway.read(session_id).await {
            Ok(detail) => Ok(detail),
            Err(failure) if failure.recoverable() => {
                if !self.gateway.begin_recovery() {
                    return Err(recovery_failure());
                }
                let recovered = match self.gateway.restart().await {
                    Ok(_) => self.gateway.read(session_id).await,
                    Err(failure) => Err(failure),
                };
                if recovered.is_ok() {
                    self.gateway.complete_recovery();
                }
                recovered.map_err(|_| recovery_failure())
            }
            Err(failure) => Err(failure),
        }
    }

    pub async fn shutdown_now(&self) {
        let mut leases = self.leases.lock().await;
        leases.active.clear();
        leases.generation = leases.generation.wrapping_add(1);
        drop(leases);
        self.gateway.shutdown().await;
    }

    pub async fn suspend(&self) {
        let generation = {
            let mut leases = self.leases.lock().await;
            leases.generation = leases.generation.wrapping_add(1);
            leases.generation
        };
        self.schedule_idle_shutdown(generation, false);
    }

    pub async fn resume(&self) {
        let mut leases = self.leases.lock().await;
        leases.generation = leases.generation.wrapping_add(1);
    }

    pub async fn export_markdown(
        &self,
        detail: &SessionDetail,
        destination: &Path,
    ) -> Result<(), SessionFailure> {
        if destination.as_os_str().is_empty() || destination.is_dir() {
            return Err(SessionFailure::new(
                SessionFailureCategory::WriteFailed,
                "session.export_write_failed",
            ));
        }
        std::fs::write(destination, detail.to_markdown()).map_err(|_| {
            SessionFailure::new(
                SessionFailureCategory::WriteFailed,
                "session.export_write_failed",
            )
        })
    }

    pub async fn archive(&self, session_ids: Vec<String>) -> Vec<SessionMutationResult> {
        self.mutate_many(session_ids, MutationKind::Archive).await
    }

    pub async fn unarchive(&self, session_ids: Vec<String>) -> Vec<SessionMutationResult> {
        self.mutate_many(session_ids, MutationKind::Unarchive).await
    }

    pub async fn delete(&self, session_id: &str) -> SessionMutationResult {
        self.mutate_many(vec![session_id.to_owned()], MutationKind::Delete)
            .await
            .pop()
            .unwrap_or_else(|| SessionMutationResult {
                session_id: session_id.to_owned(),
                status: SessionMutationResultStatus::Failed,
                actual_state: SessionActualState::Unknown,
                message_id: "session.request_failed".to_owned(),
            })
    }

    async fn mutate_many(
        &self,
        session_ids: Vec<String>,
        mutation: MutationKind,
    ) -> Vec<SessionMutationResult> {
        if let Err(failure) = self.gateway.start().await {
            return session_ids
                .into_iter()
                .map(|session_id| SessionMutationResult {
                    session_id,
                    status: SessionMutationResultStatus::Failed,
                    actual_state: SessionActualState::Unknown,
                    message_id: failure.message_id.clone(),
                })
                .collect();
        }

        let mut results = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            results.push(self.mutate_one(&session_id, mutation).await);
        }
        results
    }

    async fn mutate_one(&self, session_id: &str, mutation: MutationKind) -> SessionMutationResult {
        match self.gateway.mutate(mutation, session_id).await {
            Ok(()) => SessionMutationResult {
                session_id: session_id.to_owned(),
                status: SessionMutationResultStatus::Succeeded,
                actual_state: mutation.success_state(),
                message_id: mutation.success_message().to_owned(),
            },
            Err(failure) if failure.recoverable() => {
                self.reconcile_lost_mutation(session_id, mutation).await
            }
            Err(failure) => SessionMutationResult {
                session_id: session_id.to_owned(),
                status: SessionMutationResultStatus::Failed,
                actual_state: mutation.initial_state(),
                message_id: failure.message_id,
            },
        }
    }

    async fn reconcile_lost_mutation(
        &self,
        session_id: &str,
        mutation: MutationKind,
    ) -> SessionMutationResult {
        if !self.gateway.begin_recovery() {
            return SessionMutationResult {
                session_id: session_id.to_owned(),
                status: SessionMutationResultStatus::Failed,
                actual_state: SessionActualState::Unknown,
                message_id: "session.recovery_failed".to_owned(),
            };
        }
        let actual_state = match self.gateway.restart().await {
            Ok(_) => self.gateway.locate_state(session_id).await,
            Err(failure) => Err(failure),
        };
        let Ok(actual_state) = actual_state else {
            return SessionMutationResult {
                session_id: session_id.to_owned(),
                status: SessionMutationResultStatus::Failed,
                actual_state: SessionActualState::Unknown,
                message_id: "session.recovery_failed".to_owned(),
            };
        };
        self.gateway.complete_recovery();
        let applied = actual_state == mutation.success_state();
        SessionMutationResult {
            session_id: session_id.to_owned(),
            status: if applied {
                SessionMutationResultStatus::Succeeded
            } else {
                SessionMutationResultStatus::Failed
            },
            actual_state,
            message_id: if applied {
                mutation.reconciled_message().to_owned()
            } else {
                "session.mutation_not_applied".to_owned()
            },
        }
    }

    fn schedule_idle_shutdown(&self, generation: u64, require_no_active_lease: bool) {
        let application = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(application.idle_grace).await;
            let should_shutdown = {
                let leases = application.leases.lock().await;
                leases.generation == generation
                    && (!require_no_active_lease || leases.active.is_empty())
            };
            if should_shutdown {
                application.gateway.shutdown().await;
            }
        });
    }
}

fn mutations_allowed() -> SessionMutationAvailability {
    SessionMutationAvailability {
        status: SessionMutationAvailabilityStatus::Allowed,
        message_id: "session.mutations_allowed".to_owned(),
    }
}

fn availability_from_failure(failure: SessionFailure) -> SessionAvailability {
    let status = match failure.category {
        SessionFailureCategory::CodexMissing => SessionAvailabilityStatus::CodexMissing,
        SessionFailureCategory::Incompatible => SessionAvailabilityStatus::Incompatible,
        SessionFailureCategory::InitializationFailed => {
            SessionAvailabilityStatus::InitializationFailed
        }
        SessionFailureCategory::RecoveryFailed => SessionAvailabilityStatus::RecoveryFailed,
        _ => SessionAvailabilityStatus::InitializationFailed,
    };
    SessionAvailability {
        status,
        message_id: failure.message_id,
        codex_version: None,
        mutation: SessionMutationAvailability {
            status: SessionMutationAvailabilityStatus::Unavailable,
            message_id: "session.mutations_unavailable".to_owned(),
        },
    }
}

fn recovery_failure() -> SessionFailure {
    SessionFailure::new(
        SessionFailureCategory::RecoveryFailed,
        "session.recovery_failed",
    )
}

#[derive(Clone)]
struct AppServerGateway {
    inner: Arc<Mutex<Option<AppServerConnection>>>,
    state_store: StateStore,
    launch: Option<LaunchCommand>,
    recovery_blocked: Arc<AtomicBool>,
}

#[derive(Clone)]
struct LaunchCommand {
    identity: PathBuf,
    program: PathBuf,
    args_prefix: Vec<OsString>,
}

struct AppServerConnection {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr_task: JoinHandle<()>,
    next_id: u64,
    capability: AppServerCapability,
    ownership_generation: String,
    #[cfg(windows)]
    process_tree: ProcessTreeJob,
}

#[derive(Clone)]
struct AppServerCapability {
    codex_version: String,
}

impl AppServerGateway {
    fn new(state_store: StateStore) -> Self {
        recover_owned_process(&state_store);
        Self {
            inner: Arc::new(Mutex::new(None)),
            state_store,
            launch: None,
            recovery_blocked: Arc::new(AtomicBool::new(false)),
        }
    }

    fn with_launch(state_store: StateStore, launch: LaunchCommand) -> Self {
        recover_owned_process(&state_store);
        Self {
            inner: Arc::new(Mutex::new(None)),
            state_store,
            launch: Some(launch),
            recovery_blocked: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn start_for_entry(&self) -> Result<AppServerCapability, SessionFailure> {
        if self.recovery_blocked.swap(false, Ordering::SeqCst) {
            self.restart().await
        } else {
            self.start().await
        }
    }

    fn recovery_is_blocked(&self) -> bool {
        self.recovery_blocked.load(Ordering::SeqCst)
    }

    fn begin_recovery(&self) -> bool {
        self.recovery_blocked
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn complete_recovery(&self) {
        self.recovery_blocked.store(false, Ordering::SeqCst);
    }

    async fn start(&self) -> Result<AppServerCapability, SessionFailure> {
        let mut inner = self.inner.lock().await;
        if let Some(connection) = inner.as_ref() {
            return Ok(connection.capability.clone());
        }
        let launch = match &self.launch {
            Some(launch) => launch.clone(),
            None => discover_codex(Some(&self.state_store))?,
        };
        let connection = match spawn_connection(&launch, &self.state_store).await {
            Ok(connection) => connection,
            Err(failure) if self.launch.is_none() => {
                let fallback = discover_codex(None)?;
                if fallback.identity == launch.identity {
                    return Err(failure);
                }
                spawn_connection(&fallback, &self.state_store).await?
            }
            Err(failure) => return Err(failure),
        };
        let capability = connection.capability.clone();
        *inner = Some(connection);
        Ok(capability)
    }

    async fn list(&self, query: &SessionQuery) -> Result<SessionListPage, SessionFailure> {
        self.start().await?;
        let mut inner = self.inner.lock().await;
        let connection = inner.as_mut().expect("connection started");
        let mut params = serde_json::Map::new();
        params.insert("archived".to_owned(), json!(query.archived));
        params.insert("limit".to_owned(), json!(query.limit.clamp(1, 100)));
        params.insert("sortKey".to_owned(), json!("recency_at"));
        params.insert("sortDirection".to_owned(), json!("desc"));
        params.insert("sourceKinds".to_owned(), json!(INTERACTIVE_SOURCE_KINDS));
        if let Some(search_term) = query
            .search_term
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            params.insert("searchTerm".to_owned(), json!(search_term));
        }
        if let Some(project) = query
            .project
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            params.insert("cwd".to_owned(), json!(project));
        }
        let model_providers = query
            .model_provider
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|provider| json!([provider]))
            .unwrap_or_else(|| json!([]));
        // Codex defaults an omitted provider filter to the active provider.
        // An explicit empty list is required for the all-providers view.
        params.insert("modelProviders".to_owned(), model_providers);
        if let Some(cursor) = query
            .cursor
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            params.insert("cursor".to_owned(), json!(cursor));
        }
        let result = request_with_timeout(connection, "thread/list", Value::Object(params)).await?;
        parse_list_page(&result)
    }

    async fn read(&self, session_id: &str) -> Result<SessionDetail, SessionFailure> {
        self.start().await?;
        let mut inner = self.inner.lock().await;
        let connection = inner.as_mut().expect("connection started");
        let result = request_with_timeout(
            connection,
            "thread/read",
            json!({ "threadId": session_id, "includeTurns": true }),
        )
        .await?;
        let thread = result.get("thread").ok_or_else(protocol_failure)?;
        parse_detail(thread)
    }

    async fn mutate(&self, mutation: MutationKind, session_id: &str) -> Result<(), SessionFailure> {
        self.start().await?;
        let mut inner = self.inner.lock().await;
        let connection = inner.as_mut().expect("connection started");
        request_with_timeout(
            connection,
            mutation.method(),
            json!({ "threadId": session_id }),
        )
        .await
        .map(|_| ())
    }

    async fn locate_state(&self, session_id: &str) -> Result<SessionActualState, SessionFailure> {
        for (archived, state) in [
            (false, SessionActualState::Active),
            (true, SessionActualState::Archived),
        ] {
            let mut cursor = None;
            let mut seen_cursors = HashSet::new();
            loop {
                let page = self
                    .list(&SessionQuery {
                        request_id: None,
                        archived,
                        search_term: None,
                        project: None,
                        model_provider: None,
                        cursor: cursor.clone(),
                        limit: 100,
                    })
                    .await?;
                if page.sessions.iter().any(|session| session.id == session_id) {
                    return Ok(state);
                }
                let Some(next_cursor) = page.next_cursor else {
                    break;
                };
                if !seen_cursors.insert(next_cursor.clone()) {
                    return Err(protocol_failure());
                }
                cursor = Some(next_cursor);
            }
        }
        Ok(SessionActualState::Deleted)
    }

    async fn restart(&self) -> Result<AppServerCapability, SessionFailure> {
        self.shutdown().await;
        self.start().await
    }

    async fn shutdown(&self) {
        let connection = self.inner.lock().await.take();
        let Some(mut connection) = connection else {
            return;
        };
        close_connection(&mut connection, &self.state_store).await;
    }
}

async fn spawn_connection(
    launch: &LaunchCommand,
    state_store: &StateStore,
) -> Result<AppServerConnection, SessionFailure> {
    let codex_version = read_codex_version(launch).await.ok_or_else(|| {
        SessionFailure::new(
            SessionFailureCategory::InitializationFailed,
            "session.version_probe_failed",
        )
    })?;
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args_prefix)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().map_err(|_| {
        SessionFailure::new(
            SessionFailureCategory::CodexMissing,
            "session.codex_missing",
        )
    })?;
    #[cfg(windows)]
    let process_tree = ProcessTreeJob::assign(&child).map_err(|_| {
        SessionFailure::new(
            SessionFailureCategory::InitializationFailed,
            "session.process_ownership_failed",
        )
    })?;
    let pid = child.id().ok_or_else(|| {
        SessionFailure::new(
            SessionFailureCategory::InitializationFailed,
            "session.initialization_failed",
        )
    })?;
    let stdin = child.stdin.take().ok_or_else(protocol_failure)?;
    let stdout = child.stdout.take().ok_or_else(protocol_failure)?;
    let stderr = child.stderr.take().ok_or_else(protocol_failure)?;
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while matches!(lines.next_line().await, Ok(Some(_))) {}
    });
    let ownership_generation = Uuid::new_v4().to_string();
    let now = epoch_seconds();
    let process_created_at = process_creation_timestamp(&child).unwrap_or(now);
    let executable_path = launch.program.to_string_lossy();
    let capability_path = launch.identity.to_string_lossy();
    let _ = state_store.record_session_process_ownership(
        pid,
        process_created_at,
        &executable_path,
        &ownership_generation,
        now,
    );
    let mut connection = AppServerConnection {
        child,
        stdin: Some(stdin),
        stdout: BufReader::new(stdout),
        stderr_task,
        next_id: 1,
        capability: AppServerCapability {
            codex_version: codex_version.clone(),
        },
        ownership_generation,
        #[cfg(windows)]
        process_tree,
    };
    let initialize = match request_with_timeout(
        &mut connection,
        "initialize",
        json!({
            "clientInfo": {
                "name": "gpteasy",
                "title": "GPTEasy",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "experimentalApi": false,
                "requestAttestation": false
            }
        }),
    )
    .await
    {
        Ok(result) => result,
        Err(failure) => {
            close_connection(&mut connection, state_store).await;
            return Err(match failure.category {
                SessionFailureCategory::Incompatible => failure,
                _ => SessionFailure::new(
                    SessionFailureCategory::InitializationFailed,
                    "session.initialization_failed",
                ),
            });
        }
    };
    if !initialize
        .get("codexHome")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        close_connection(&mut connection, state_store).await;
        return Err(SessionFailure::new(
            SessionFailureCategory::InitializationFailed,
            "session.initialization_failed",
        ));
    }
    if let Err(failure) = connection.notify("initialized", None).await {
        close_connection(&mut connection, state_store).await;
        return Err(failure);
    }
    if let Err(failure) = probe_core_methods(&mut connection).await {
        close_connection(&mut connection, state_store).await;
        return Err(match failure.category {
            SessionFailureCategory::Incompatible => failure,
            _ => SessionFailure::new(
                SessionFailureCategory::InitializationFailed,
                "session.initialization_failed",
            ),
        });
    }
    let _ = state_store.record_session_capability(
        &capability_path,
        &codex_version,
        "available",
        epoch_seconds(),
    );
    Ok(connection)
}

async fn probe_core_methods(connection: &mut AppServerConnection) -> Result<(), SessionFailure> {
    let list = request_with_timeout(
        connection,
        "thread/list",
        json!({
            "archived": false,
            "limit": 1,
            "sortKey": "recency_at",
            "sortDirection": "desc",
            "sourceKinds": INTERACTIVE_SOURCE_KINDS,
        }),
    )
    .await?;
    if !list.get("data").is_some_and(Value::is_array) {
        return Err(protocol_failure());
    }

    match request_with_timeout(
        connection,
        "thread/read",
        json!({
            "threadId": "gpteasy-capability-probe-invalid-session",
            "includeTurns": true,
        }),
    )
    .await
    {
        Ok(value) if value.get("thread").is_some_and(Value::is_object) => {}
        Ok(_) => return Err(protocol_failure()),
        Err(SessionFailure {
            category: SessionFailureCategory::RequestFailed,
            ..
        }) => {}
        Err(failure) => return Err(failure),
    }

    for method in ["thread/archive", "thread/unarchive", "thread/delete"] {
        match request_with_timeout(
            connection,
            method,
            json!({ "threadId": "gpteasy-capability-probe-invalid-session" }),
        )
        .await
        {
            Ok(value) if value.is_object() => {}
            Ok(_) => return Err(protocol_failure()),
            Err(SessionFailure {
                category: SessionFailureCategory::RequestFailed,
                ..
            }) => {}
            Err(failure) => return Err(failure),
        }
    }
    Ok(())
}

async fn close_connection(connection: &mut AppServerConnection, state_store: &StateStore) {
    connection.stdin.take();
    if timeout(SHUTDOWN_TIMEOUT, connection.child.wait())
        .await
        .is_err()
    {
        #[cfg(windows)]
        connection.process_tree.terminate();
        let _ = connection.child.kill().await;
        let _ = connection.child.wait().await;
    }
    connection.stderr_task.abort();
    let _ = state_store.clear_session_process_ownership(&connection.ownership_generation);
}

async fn read_codex_version(launch: &LaunchCommand) -> Option<String> {
    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args_prefix)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = timeout(Duration::from_secs(3), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

async fn request_with_timeout(
    connection: &mut AppServerConnection,
    method: &str,
    params: Value,
) -> Result<Value, SessionFailure> {
    timeout(REQUEST_TIMEOUT, connection.request(method, params))
        .await
        .map_err(|_| {
            SessionFailure::new(
                SessionFailureCategory::Protocol,
                "session.request_timed_out",
            )
        })?
}

impl AppServerConnection {
    async fn notify(&mut self, method: &str, params: Option<Value>) -> Result<(), SessionFailure> {
        let mut message = serde_json::Map::new();
        message.insert("method".to_owned(), json!(method));
        if let Some(params) = params {
            message.insert("params".to_owned(), params);
        }
        self.write_line(&Value::Object(message)).await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, SessionFailure> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.write_line(&json!({ "method": method, "id": id, "params": params }))
            .await?;
        loop {
            let mut line = String::new();
            let read = self
                .stdout
                .read_line(&mut line)
                .await
                .map_err(|_| unexpected_exit())?;
            if read == 0 {
                return Err(unexpected_exit());
            }
            let message: Value =
                serde_json::from_str(line.trim()).map_err(|_| protocol_failure())?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                if message.get("id").is_some() && message.get("method").is_some() {
                    self.respond_method_not_found(&message).await?;
                }
                continue;
            }
            if let Some(error) = message.get("error") {
                let category = if is_method_not_found_error(error) {
                    SessionFailureCategory::Incompatible
                } else {
                    SessionFailureCategory::RequestFailed
                };
                let message_id = if category == SessionFailureCategory::Incompatible {
                    "session.incompatible"
                } else {
                    "session.request_failed"
                };
                return Err(SessionFailure::new(category, message_id));
            }
            return message.get("result").cloned().ok_or_else(protocol_failure);
        }
    }

    async fn respond_method_not_found(&mut self, message: &Value) -> Result<(), SessionFailure> {
        self.write_line(&json!({
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "error": { "code": -32601, "message": "method not supported" }
        }))
        .await
    }

    async fn write_line(&mut self, value: &Value) -> Result<(), SessionFailure> {
        let stdin = self.stdin.as_mut().ok_or_else(unexpected_exit)?;
        let mut encoded = serde_json::to_vec(value).map_err(|_| protocol_failure())?;
        encoded.push(b'\n');
        stdin
            .write_all(&encoded)
            .await
            .map_err(|_| unexpected_exit())?;
        stdin.flush().await.map_err(|_| unexpected_exit())
    }
}

fn is_method_not_found_error(error: &Value) -> bool {
    if error.get("code").and_then(Value::as_i64) == Some(-32601) {
        return true;
    }
    error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .is_some_and(|message| {
            message.contains("method not found")
                || message.contains("unknown method")
                || message.contains("method not supported")
        })
}

fn parse_list_page(value: &Value) -> Result<SessionListPage, SessionFailure> {
    let sessions = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(protocol_failure)?
        .iter()
        // Older App Server builds may accept `sourceKinds` but fail to apply
        // the filter. Keep the product boundary enforced by the consumer too.
        .filter(|thread| is_interactive_thread(thread.get("source")))
        .map(parse_summary)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = value
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(SessionListPage {
        sessions,
        next_cursor,
    })
}

fn is_interactive_thread(source: Option<&Value>) -> bool {
    match source {
        Some(Value::String(kind)) => {
            INTERACTIVE_SOURCE_KINDS.contains(&kind.as_str())
                || !matches!(
                    kind.as_str(),
                    "exec"
                        | "subAgent"
                        | "subAgentReview"
                        | "subAgentCompact"
                        | "subAgentThreadSpawn"
                        | "subAgentOther"
                ) && kind != "unknown"
        }
        Some(Value::Object(value)) => value
            .get("custom")
            .and_then(Value::as_str)
            .is_some_and(|custom| !custom.trim().is_empty()),
        _ => false,
    }
}

fn parse_summary(thread: &Value) -> Result<SessionSummary, SessionFailure> {
    let id = required_string(thread, "id")?;
    let preview = string_value(thread, "preview");
    let title = thread
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            preview
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| line.trim().to_owned())
        })
        .unwrap_or_else(|| "未命名会话".to_owned());
    Ok(SessionSummary {
        id,
        forked_from_id: optional_string(thread, "forkedFromId"),
        parent_thread_id: optional_string(thread, "parentThreadId"),
        title,
        preview,
        // These fields are metadata, not protocol prerequisites. Older
        // App Server versions may omit them; keep the session usable while
        // preserving the values when they are present.
        project: string_value(thread, "cwd"),
        model_provider: string_value(thread, "modelProvider"),
        source: source_label(thread.get("source")),
        created_at: optional_i64(thread, "createdAt"),
        updated_at: thread
            .get("recencyAt")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| optional_i64(thread, "updatedAt")),
    })
}

fn parse_detail(thread: &Value) -> Result<SessionDetail, SessionFailure> {
    let summary = parse_summary(thread)?;
    let turns = thread
        .get("turns")
        .and_then(Value::as_array)
        .ok_or_else(protocol_failure)?;
    let mut entries = Vec::new();
    for turn in turns {
        let items = turn
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(protocol_failure)?;
        for item in items {
            if let Some(entry) = parse_entry(item)? {
                entries.push(entry);
            }
        }
    }
    Ok(SessionDetail { summary, entries })
}

fn parse_entry(item: &Value) -> Result<Option<SessionEntry>, SessionFailure> {
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(protocol_failure)?;
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(item_type)
        .to_owned();
    let entry = match item_type {
        "userMessage" => {
            let content = item
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(protocol_failure)?
                .iter()
                .filter_map(user_content)
                .collect::<Vec<_>>()
                .join("\n");
            SessionEntry {
                id,
                kind: SessionEntryKind::User,
                label: "用户".to_owned(),
                content,
                output: None,
            }
        }
        "agentMessage" | "plan" => SessionEntry {
            id,
            kind: SessionEntryKind::Assistant,
            label: "助手".to_owned(),
            content: string_value(item, "text"),
            output: None,
        },
        "commandExecution" => SessionEntry {
            id,
            kind: SessionEntryKind::Tool,
            label: "命令".to_owned(),
            content: string_value(item, "command"),
            output: item
                .get("aggregatedOutput")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        "fileChange" => SessionEntry {
            id,
            kind: SessionEntryKind::Tool,
            label: "文件修改".to_owned(),
            content: serde_json::to_string_pretty(item.get("changes").unwrap_or(&Value::Null))
                .unwrap_or_default(),
            output: None,
        },
        "mcpToolCall"
        | "dynamicToolCall"
        | "collabAgentToolCall"
        | "webSearch"
        | "imageView"
        | "imageGeneration"
        | "reasoning" => SessionEntry {
            id,
            kind: SessionEntryKind::Tool,
            label: tool_label(item_type),
            content: serde_json::to_string_pretty(item).unwrap_or_default(),
            output: None,
        },
        _ => return Ok(None),
    };
    Ok(Some(entry))
}

fn user_content(value: &Value) -> Option<String> {
    match value.get("type").and_then(Value::as_str)? {
        "text" => value.get("text")?.as_str().map(str::to_owned),
        "image" => Some("[图片]".to_owned()),
        "localImage" => Some(format!("[本地图片：{}]", string_value(value, "path"))),
        "audio" | "localAudio" => Some("[音频]".to_owned()),
        "skill" => Some(format!("[技能：{}]", string_value(value, "name"))),
        "mention" => Some(format!("[引用：{}]", string_value(value, "name"))),
        _ => None,
    }
}

fn tool_label(item_type: &str) -> String {
    match item_type {
        "mcpToolCall" => "MCP 工具",
        "dynamicToolCall" => "动态工具",
        "collabAgentToolCall" => "协作活动",
        "webSearch" => "网页搜索",
        "imageView" => "查看图片",
        "imageGeneration" => "生成图片",
        "reasoning" => "推理活动",
        _ => "工具活动",
    }
    .to_owned()
}

fn source_label(source: Option<&Value>) -> String {
    match source {
        Some(Value::String(value)) => match value.as_str() {
            "cli" => "Codex CLI".to_owned(),
            "vscode" => "IDE".to_owned(),
            "appServer" => "ChatGPT/Codex 桌面版".to_owned(),
            value => value.to_owned(),
        },
        Some(Value::Object(value)) => value
            .get("custom")
            .and_then(Value::as_str)
            .unwrap_or("未知来源")
            .to_owned(),
        _ => "未知来源".to_owned(),
    }
}

fn required_string(value: &Value, key: &str) -> Result<String, SessionFailure> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(protocol_failure)
}

fn string_value(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn optional_i64(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}

fn protocol_failure() -> SessionFailure {
    SessionFailure::new(SessionFailureCategory::Protocol, "session.protocol_error")
}

fn unexpected_exit() -> SessionFailure {
    SessionFailure::new(
        SessionFailureCategory::UnexpectedExit,
        "session.unexpected_exit",
    )
}

fn discover_codex(state_store: Option<&StateStore>) -> Result<LaunchCommand, SessionFailure> {
    if let Some(override_path) = std::env::var_os("GPTEASY_CODEX_EXECUTABLE") {
        let path = PathBuf::from(override_path);
        if path.is_file() {
            return Ok(launch_from_path(path));
        }
    }
    if let Some(preferred) = state_store
        .and_then(StateStore::preferred_session_executable)
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return Ok(launch_from_path(preferred));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(windows)]
    let current_user_path = read_current_user_path();
    #[cfg(not(windows))]
    let current_user_path: Option<OsString> = None;
    match discover_codex_in_paths(&path, current_user_path.as_deref()) {
        Ok(launch) => Ok(launch),
        Err(_) => {
            #[cfg(windows)]
            if desktop_app_detected() {
                return Err(SessionFailure::new(
                    SessionFailureCategory::CodexMissing,
                    "session.desktop_app_server_unavailable",
                ));
            }
            Err(SessionFailure::new(
                SessionFailureCategory::CodexMissing,
                "session.codex_missing",
            ))
        }
    }
}

fn launch_from_path(path: PathBuf) -> LaunchCommand {
    #[cfg(windows)]
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
    {
        return LaunchCommand {
            identity: path.clone(),
            program: PathBuf::from("cmd.exe"),
            args_prefix: vec!["/D".into(), "/S".into(), "/C".into(), path.into_os_string()],
        };
    }
    LaunchCommand {
        identity: path.clone(),
        program: path,
        args_prefix: Vec::new(),
    }
}

fn discover_codex_in_paths(
    process_path: &OsStr,
    current_user_path: Option<&OsStr>,
) -> Result<LaunchCommand, SessionFailure> {
    let mut directories = std::env::split_paths(process_path).collect::<Vec<_>>();
    if let Some(current_user_path) = current_user_path {
        directories.extend(std::env::split_paths(current_user_path));
    }
    #[cfg(windows)]
    directories.extend(desktop_codex_directories());
    for directory in directories {
        for name in candidate_names() {
            let candidate = directory.join(name);
            if !candidate.is_file() {
                continue;
            }
            #[cfg(windows)]
            if candidate
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
            {
                if let Some(native) = npm_native_codex(&candidate) {
                    return Ok(LaunchCommand {
                        identity: native.clone(),
                        program: native,
                        args_prefix: Vec::new(),
                    });
                }
                return Ok(LaunchCommand {
                    identity: candidate.clone(),
                    program: PathBuf::from("cmd.exe"),
                    args_prefix: vec![
                        "/D".into(),
                        "/S".into(),
                        "/C".into(),
                        candidate.into_os_string(),
                    ],
                });
            }
            return Ok(LaunchCommand {
                identity: candidate.clone(),
                program: candidate,
                args_prefix: Vec::new(),
            });
        }
    }
    Err(SessionFailure::new(
        SessionFailureCategory::CodexMissing,
        "session.codex_missing",
    ))
}

#[cfg(windows)]
fn desktop_codex_directories() -> Vec<PathBuf> {
    let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") else {
        return Vec::new();
    };
    let packages = PathBuf::from(local_app_data).join("Packages");
    let Ok(entries) = std::fs::read_dir(packages) else {
        return Vec::new();
    };
    let mut directories = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.starts_with("openai.codex_") && !name.starts_with("openai.chatgpt_") {
            continue;
        }
        let root = entry.path();
        collect_desktop_codex_dirs(&root, 0, &mut directories);
        for relative in [
            "LocalCache/Local/codex",
            "LocalCache/Local/resources/codex",
            "LocalCache/Local/resources/codex/bin",
            "LocalCache/Local/Programs/codex",
            "LocalCache/Local/Programs/codex/bin",
            "LocalCache/Local/Programs/resources/codex",
        ] {
            directories.push(root.join(relative));
        }
    }
    directories
}

#[cfg(windows)]
fn collect_desktop_codex_dirs(root: &Path, depth: usize, directories: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_desktop_codex_dirs(&path, depth + 1, directories);
        } else if path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
        {
            if let Some(parent) = path.parent() {
                directories.push(parent.to_owned());
            }
        }
    }
}

#[cfg(windows)]
fn desktop_app_detected() -> bool {
    let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") else {
        return false;
    };
    let packages = PathBuf::from(local_app_data).join("Packages");
    std::fs::read_dir(packages)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("openai.codex_") || name.starts_with("openai.chatgpt_")
        })
}

#[cfg(windows)]
fn read_current_user_path() -> Option<OsString> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ)
        .ok()?
        .get_value::<OsString, _>("Path")
        .ok()
}

#[cfg(windows)]
fn candidate_names() -> [&'static str; 3] {
    ["codex.exe", "codex.cmd", "codex"]
}

#[cfg(not(windows))]
fn candidate_names() -> [&'static str; 1] {
    ["codex"]
}

#[cfg(windows)]
fn npm_native_codex(command: &Path) -> Option<PathBuf> {
    let root = command.parent()?;
    let (package, triple) = if cfg!(target_arch = "aarch64") {
        ("codex-win32-arm64", "aarch64-pc-windows-msvc")
    } else {
        ("codex-win32-x64", "x86_64-pc-windows-msvc")
    };
    let candidate = root
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("node_modules")
        .join("@openai")
        .join(package)
        .join("vendor")
        .join(triple)
        .join("bin")
        .join("codex.exe");
    candidate.is_file().then_some(candidate)
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(not(windows))]
fn process_creation_timestamp(_child: &Child) -> Option<i64> {
    None
}

#[cfg(windows)]
fn process_creation_timestamp(child: &Child) -> Option<i64> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetProcessTimes;

    let raw_handle = child.raw_handle()?;
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let success = unsafe {
        GetProcessTimes(
            raw_handle as HANDLE,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } != 0;
    success.then_some(
        ((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime)) as i64,
    )
}

fn recover_owned_process(state_store: &StateStore) {
    let Some(ownership) = state_store.session_process_ownership() else {
        return;
    };
    // The generation is a fencing token. Claim the exact persisted record
    // before inspecting or terminating the process so a newer owner can
    // replace it without being affected by a stale recovery attempt.
    if !state_store.clear_session_process_ownership(&ownership.ownership_generation) {
        return;
    }
    #[cfg(windows)]
    let _terminated = terminate_exact_owned_process(&ownership);
    #[cfg(not(windows))]
    let _terminated = false;
}

#[cfg(windows)]
fn terminate_exact_owned_process(ownership: &SessionProcessOwnership) -> bool {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess,
        WaitForSingleObject,
    };

    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            0,
            ownership.pid,
        )
    };
    if handle.is_null() {
        return false;
    }
    let mut path = vec![0u16; 32_768];
    let mut path_len = path.len() as u32;
    let path_matches = unsafe {
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut path_len)
            != 0
    };
    let actual_path = if path_matches {
        OsString::from_wide(&path[..path_len as usize])
            .to_string_lossy()
            .to_ascii_lowercase()
    } else {
        String::new()
    };
    let expected_path = std::fs::canonicalize(&ownership.executable_path)
        .unwrap_or_else(|_| PathBuf::from(&ownership.executable_path))
        .to_string_lossy()
        .to_ascii_lowercase();
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let creation_matches =
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) != 0 }
            && (((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
                as i64)
                == ownership.process_created_at;
    let exact = path_matches && actual_path == expected_path && creation_matches;
    let terminated = exact && unsafe { TerminateProcess(handle, 1) != 0 };
    if terminated {
        unsafe {
            let _ = WaitForSingleObject(handle, 2_000);
        }
    }
    unsafe {
        let _ = CloseHandle(handle);
    }
    terminated
}

#[cfg(windows)]
struct ProcessTreeJob {
    handle: HANDLE,
}

#[cfg(windows)]
unsafe impl Send for ProcessTreeJob {}

#[cfg(windows)]
impl ProcessTreeJob {
    fn assign(child: &Child) -> std::io::Result<Self> {
        let Some(raw_handle) = child.raw_handle() else {
            return Err(std::io::Error::other("child process handle unavailable"));
        };
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } != 0;
        let assigned =
            configured && unsafe { AssignProcessToJobObject(handle, raw_handle as HANDLE) != 0 };
        if !assigned {
            unsafe { CloseHandle(handle) };
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { handle })
    }

    fn terminate(&self) {
        unsafe {
            let _ = TerminateJobObject(self.handle, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTreeJob {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn codex_installed_after_process_start_is_discovered_from_current_user_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let stale_directory = temp.path().join("stale-path");
        let npm_directory = temp.path().join("npm");
        std::fs::create_dir_all(&stale_directory).expect("create stale path");
        std::fs::create_dir_all(&npm_directory).expect("create npm path");
        std::fs::write(npm_directory.join("codex.cmd"), "@echo off\r\n").expect("create npm shim");
        let native = npm_directory
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("node_modules")
            .join("@openai")
            .join(if cfg!(target_arch = "aarch64") {
                "codex-win32-arm64"
            } else {
                "codex-win32-x64"
            })
            .join("vendor")
            .join(if cfg!(target_arch = "aarch64") {
                "aarch64-pc-windows-msvc"
            } else {
                "x86_64-pc-windows-msvc"
            })
            .join("bin")
            .join("codex.exe");
        std::fs::create_dir_all(native.parent().expect("native parent"))
            .expect("create native directory");
        std::fs::write(&native, []).expect("create native executable");
        let stale_path = std::env::join_paths([stale_directory]).expect("stale process path");
        let current_user_path = std::env::join_paths([npm_directory]).expect("current user path");

        let launch = discover_codex_in_paths(&stale_path, Some(&current_user_path))
            .expect("newly installed Codex must be discovered without restarting GPTEasy");

        assert_eq!(launch.identity, native);
        assert_eq!(launch.program, native);
        assert!(launch.args_prefix.is_empty());
    }

    #[test]
    fn custom_interactive_sources_are_preserved_but_internal_sources_are_filtered() {
        let custom = json!({ "custom": "JetBrains" });
        assert!(is_interactive_thread(Some(&custom)));
        assert_eq!(source_label(Some(&custom)), "JetBrains");
        assert_eq!(
            source_label(Some(&json!("appServer"))),
            "ChatGPT/Codex 桌面版"
        );
        assert!(!is_interactive_thread(Some(&json!("subAgent"))));
        assert!(!is_interactive_thread(Some(&json!("unknown"))));
    }
}
