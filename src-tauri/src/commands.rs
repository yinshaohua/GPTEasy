use std::process::Command;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

use crate::consumer::{ConsumerScanner, ConsumerStatus, WindowsConsumerScanner};
use crate::desktop::{DesktopApplication, DesktopFailure, DesktopFailureCategory, DesktopSnapshot};
use crate::diagnostics::{IssueLogLevel, IssueLogRecord, IssueLogStore};
use crate::environment::{
    EnvironmentApplication, EnvironmentFailure, EnvironmentFailureCategory, EnvironmentSnapshot,
    EnvironmentVisibilityContext,
};
use crate::provider::{
    AppliedProviderUpdate, DAYWAY_WEBSITE, DiscoveryInput, LinuxExportFailure, LinuxExportResult,
    LinuxShell, ModelDiscovery, ProviderApiKey, ProviderApplication, ProviderFailure,
    ProviderFailureCategory, ProviderRevalidationResult, ProviderSummary,
    ProviderUpdateDiscoveryInput, ProviderUpdateValidationInput, ProviderValidationInput,
    ProviderValidationReceipt, ProviderValidationStage,
};
use crate::session::{
    SessionApplication, SessionAvailability, SessionDetail, SessionFailure, SessionListPage,
    SessionMutationResult, SessionQuery,
};
use crate::session_visibility::{
    SessionVisibilityApplication, SessionVisibilityPreview, VisibilityAppServerCapability,
    VisibilityConsumerState, VisibilityCoordinationOutcome, VisibilityCoordinationStatus,
    VisibilityExecutionRequest, VisibilityExecutionResult, VisibilityExecutionRuntime,
    VisibilityFailure, VisibilityRuntimeFuture, VisibilityScanContext, VisibilityTarget,
    VisibilityTargetMode, VisibilityThreadView, VisibilityVerificationViews,
};
use crate::startup::{StartupCoordinator, StartupSnapshot};
use crate::state::PendingSessionVisibilitySnapshot;
use crate::tray;
use crate::update::{
    UpdateActivityGate, UpdateCoordinator, UpdateFailureCategory, UpdateInstallFailure,
    UpdateInstallFailureCategory, UpdateSnapshot, UpdateState,
};
use crate::wsl::{
    WslApplication, WslApplyResult, WslDeletionAuditError, WslEnvironmentSummary, WslFailure,
    WslLifecycleOutcome, WslLifecycleResult, WslReclaimProgress, WslRefreshResult,
    summarize_wsl_inventory,
};

pub(crate) struct StartupRuntime {
    coordinator: Mutex<StartupCoordinator>,
}

pub(crate) struct ProviderRuntime {
    application: ProviderApplication,
}

pub(crate) struct EnvironmentRuntime {
    application: EnvironmentApplication,
}

pub(crate) struct DesktopRuntime {
    application: DesktopApplication,
}

pub(crate) struct WslRuntime {
    application: WslApplication,
}

pub(crate) struct SessionRuntime {
    application: SessionApplication,
    visibility: SessionVisibilityApplication,
    availability: Mutex<Option<SessionAvailability>>,
}

pub(crate) struct IssueLogRuntime {
    pub(crate) store: Arc<IssueLogStore>,
}

impl IssueLogRuntime {
    pub(crate) fn new(store: IssueLogStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }
}

pub(crate) struct UpdateRuntime {
    pub(crate) coordinator: UpdateCoordinator,
    pub(crate) activity: UpdateActivityGate,
    notified_version: Mutex<Option<String>>,
}

impl UpdateRuntime {
    pub(crate) fn new(coordinator: UpdateCoordinator) -> Self {
        Self {
            coordinator,
            activity: UpdateActivityGate::default(),
            notified_version: Mutex::new(None),
        }
    }

    fn notify_if_hidden(&self, app: &AppHandle, snapshot: &UpdateSnapshot) {
        let Some(version) = snapshot.available_version.as_deref() else {
            return;
        };
        if snapshot.state != UpdateState::Pending {
            return;
        }
        let hidden = match app.get_webview_window("main") {
            Some(window) => match window.is_visible() {
                Ok(visible) => !visible,
                Err(_) => {
                    log_runtime_error(
                        app,
                        "update.notification_visibility",
                        "window.visibility_unavailable",
                        "category=window",
                    );
                    true
                }
            },
            None => true,
        };
        if !hidden {
            return;
        }
        let Ok(mut notified) = self.notified_version.lock() else {
            log_runtime_error(
                app,
                "update.notification_state",
                "update.notification_state_unavailable",
                "category=state_unavailable",
            );
            return;
        };
        if notified.as_deref() == Some(version) {
            return;
        }
        match app
            .notification()
            .builder()
            .title("GPTEasy 有待安装更新")
            .body(format!(
                "版本 {version} 已下载并通过签名验证。打开设置查看。"
            ))
            .extra("open_settings", true)
            .show()
        {
            Ok(()) => *notified = Some(version.to_owned()),
            Err(_) => log_runtime_error(
                app,
                "update.notification",
                "notification.show_failed",
                "category=notification",
            ),
        }
    }
}

impl ProviderRuntime {
    pub(crate) fn new(application: ProviderApplication) -> Self {
        Self { application }
    }

    pub(crate) fn list(&self) -> Result<Vec<ProviderSummary>, ProviderFailure> {
        self.application.list_providers()
    }

    pub(crate) fn shutdown_requests(&self) -> usize {
        self.application.shutdown_requests()
    }

    pub(crate) fn assistant_provider(
        &self,
        provider_id: &str,
    ) -> Result<(ProviderSummary, String), ProviderFailure> {
        self.application.assistant_provider(provider_id)
    }
}

impl EnvironmentRuntime {
    pub(crate) fn new(application: EnvironmentApplication) -> Self {
        Self { application }
    }

    pub(crate) fn inspect(&self) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        self.application.inspect()
    }

    pub(crate) fn has_pending_restart(&self) -> Result<bool, EnvironmentFailure> {
        self.application.has_pending_restart()
    }

    pub(crate) fn session_visibility_context(
        &self,
    ) -> Result<EnvironmentVisibilityContext, EnvironmentFailure> {
        self.application.inspect_for_session_visibility()
    }
}

impl DesktopRuntime {
    pub(crate) fn new(application: DesktopApplication) -> Self {
        Self { application }
    }
}

impl WslRuntime {
    pub(crate) fn new(application: WslApplication) -> Self {
        Self { application }
    }

    pub(crate) fn inspect(&self) -> Result<Vec<WslEnvironmentSummary>, WslFailure> {
        self.application.list()
    }
}

impl SessionRuntime {
    pub(crate) fn new(
        application: SessionApplication,
        visibility: SessionVisibilityApplication,
    ) -> Self {
        Self {
            application,
            visibility,
            availability: Mutex::new(None),
        }
    }

    pub(crate) fn shutdown(&self) {
        tauri::async_runtime::block_on(self.application.shutdown_now());
    }

    pub(crate) fn suspend(&self) {
        tauri::async_runtime::block_on(self.application.suspend());
    }

    pub(crate) fn resume(&self) {
        tauri::async_runtime::block_on(self.application.resume());
    }

    fn record_availability(&self, availability: &SessionAvailability) {
        if let Ok(mut current) = self.availability.lock() {
            *current = Some(availability.clone());
        }
    }

    fn availability(&self) -> Option<SessionAvailability> {
        self.availability.lock().ok()?.clone()
    }
}

impl StartupRuntime {
    pub(crate) fn new(coordinator: StartupCoordinator) -> Self {
        Self {
            coordinator: Mutex::new(coordinator),
        }
    }

    fn inspect(&self) -> Result<StartupSnapshot, CommandFailure> {
        self.coordinator
            .lock()
            .map(|coordinator| coordinator.inspect())
            .map_err(|_| CommandFailure {
                message_id: "startup.internal_state_unavailable",
            })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandFailure {
    message_id: &'static str,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontendFailureEvent {
    UpdateProgressListener,
    ProviderSwitchListener,
    ProviderValidationProgressListener,
    UnhandledError,
    UnhandledRejection,
}

trait IssueLoggableFailure {
    fn message_id(&self) -> &str;
    fn category_name(&self) -> String;

    fn diagnostic_details(&self) -> String {
        format!("category={}", self.category_name())
    }
}

macro_rules! impl_issue_loggable_failure {
    ($failure:ty) => {
        impl IssueLoggableFailure for $failure {
            fn message_id(&self) -> &str {
                self.message_id.as_ref()
            }

            fn category_name(&self) -> String {
                stable_category_name(&self.category)
            }
        }
    };
}

impl_issue_loggable_failure!(EnvironmentFailure);
impl_issue_loggable_failure!(DesktopFailure);
impl_issue_loggable_failure!(LinuxExportFailure);
impl_issue_loggable_failure!(ProviderFailure);
impl_issue_loggable_failure!(SessionFailure);
impl_issue_loggable_failure!(WslFailure);

impl IssueLoggableFailure for VisibilityFailure {
    fn message_id(&self) -> &str {
        self.message_id
    }

    fn category_name(&self) -> String {
        "session_visibility".to_owned()
    }

    fn diagnostic_details(&self) -> String {
        format!(
            "category=session_visibility stage={} error_code={}",
            self.stage, self.message_id
        )
    }
}

impl IssueLoggableFailure for CommandFailure {
    fn message_id(&self) -> &str {
        self.message_id
    }

    fn category_name(&self) -> String {
        "command".to_owned()
    }
}

fn stable_category_name(category: &impl std::fmt::Debug) -> String {
    let mut result = String::new();
    for character in format!("{category:?}").chars() {
        if character.is_ascii_uppercase() {
            if !result.is_empty() {
                result.push('_');
            }
            result.push(character.to_ascii_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

impl IssueLoggableFailure for DeleteProviderFailure {
    fn message_id(&self) -> &str {
        self.message_id
    }

    fn category_name(&self) -> String {
        self.category.to_owned()
    }
}

fn finish_command<T, E: IssueLoggableFailure>(
    store: &IssueLogStore,
    event: &'static str,
    result: Result<T, E>,
) -> Result<T, E> {
    if let Err(failure) = &result {
        store.append(
            IssueLogLevel::Error,
            event,
            failure.message_id(),
            Some(failure.diagnostic_details()),
        );
    }
    result
}

#[derive(Debug, Clone, Copy)]
enum WslReclaimAuditPhase {
    Revalidated,
    SafeApplyCheck,
    RebuildRequired,
    UserConfirmed,
    Prepared,
    ArtifactsReplaced,
    StateCommitted,
    Succeeded,
}

fn log_wsl_reclaim_phase(store: &IssueLogStore, phase: WslReclaimAuditPhase) {
    let (message, details) = match phase {
        WslReclaimAuditPhase::Revalidated => (
            "wsl.reclaim_revalidated",
            "phase=revalidated status=succeeded",
        ),
        WslReclaimAuditPhase::SafeApplyCheck => {
            ("wsl.reclaim_safe_apply_check", "phase=safe_apply_check")
        }
        WslReclaimAuditPhase::RebuildRequired => {
            ("wsl.reclaim_rebuild_required", "phase=rebuild_required")
        }
        WslReclaimAuditPhase::UserConfirmed => {
            ("wsl.reclaim_user_confirmed", "phase=user_confirmed")
        }
        WslReclaimAuditPhase::Prepared => ("wsl.reclaim_prepared", "phase=prepared"),
        WslReclaimAuditPhase::ArtifactsReplaced => (
            "wsl.reclaim_artifacts_replaced",
            "phase=backup_and_write status=completed",
        ),
        WslReclaimAuditPhase::StateCommitted => {
            ("wsl.reclaim_state_committed", "phase=state_committed")
        }
        WslReclaimAuditPhase::Succeeded => {
            ("wsl.reclaim_succeeded", "phase=result status=succeeded")
        }
    };
    store.append(
        IssueLogLevel::Info,
        "wsl.reclaim_provider",
        message,
        Some(details.to_owned()),
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderRevalidationAuditContext {
    WslReclaim,
}

fn audit_provider_revalidation_result(
    store: &IssueLogStore,
    context: Option<ProviderRevalidationAuditContext>,
    result: &Result<ProviderRevalidationResult, ProviderFailure>,
) {
    if context == Some(ProviderRevalidationAuditContext::WslReclaim)
        && matches!(result, Ok(revalidated) if revalidated.validation_receipt.is_none())
    {
        log_wsl_reclaim_phase(store, WslReclaimAuditPhase::Revalidated);
    }
}

fn log_wsl_reclaim_progress(store: &IssueLogStore, progress: WslReclaimProgress) {
    let phase = match progress {
        WslReclaimProgress::RebuildRequired => WslReclaimAuditPhase::RebuildRequired,
        WslReclaimProgress::Prepared => WslReclaimAuditPhase::Prepared,
        WslReclaimProgress::ArtifactsReplaced => WslReclaimAuditPhase::ArtifactsReplaced,
        WslReclaimProgress::StateCommitted => WslReclaimAuditPhase::StateCommitted,
    };
    log_wsl_reclaim_phase(store, phase);
}

fn wsl_inventory_details(provider_count: usize, environments: &[WslEnvironmentSummary]) -> String {
    let stats = summarize_wsl_inventory(environments);
    let message_count = |message_id: &str| {
        environments
            .iter()
            .filter(|environment| environment.message_id.as_deref() == Some(message_id))
            .count()
    };
    format!(
        concat!(
            "provider_count={} environment_count={} manageable_count={} ",
            "legacy_count={} conflict_count={} busy_count={} ",
            "unknown_schema_count={} invalid_markers_count={} ",
            "invalid_credential_reference_count={} invalid_config_count={}"
        ),
        provider_count,
        stats.environment_count,
        stats.manageable_count,
        stats.legacy_count,
        stats.conflict_count,
        stats.busy_count,
        message_count("wsl.schema_unknown"),
        message_count("wsl.markers_invalid"),
        message_count("wsl.credential_reference_invalid"),
        message_count("wsl.config_invalid"),
    )
}

fn finish_command_with_desktop_restart(
    store: &IssueLogStore,
    result: Result<DesktopSnapshot, DesktopFailure>,
    expected_root_count: usize,
    observed_after_failure: Option<&DesktopSnapshot>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    match &result {
        Ok(snapshot) => store.append(
            IssueLogLevel::Info,
            "desktop.restart",
            snapshot.message_id,
            Some(format!(
                "phase=completed expected_root_count={expected_root_count} observed_status={} observed_root_count={}",
                stable_category_name(&snapshot.status),
                snapshot.roots.len()
            )),
        ),
        Err(failure) => {
            let observed_status = observed_after_failure
                .map(|snapshot| stable_category_name(&snapshot.status))
                .unwrap_or_else(|| "unavailable".to_owned());
            let observed_root_count = observed_after_failure
                .map(|snapshot| snapshot.roots.len().to_string())
                .unwrap_or_else(|| "unavailable".to_owned());
            store.append(
                IssueLogLevel::Error,
                "desktop.restart",
                failure.message_id,
                Some(format!(
                    "category={} phase={} expected_root_count={expected_root_count} observed_status={observed_status} observed_root_count={observed_root_count}",
                    stable_category_name(&failure.category),
                    desktop_failure_phase(failure.category),
                )),
            );
        }
    }
    result
}

fn desktop_failure_phase(category: DesktopFailureCategory) -> &'static str {
    match category {
        DesktopFailureCategory::ActionUnavailable => "availability",
        DesktopFailureCategory::IdentityChanged => "identity_recheck",
        DesktopFailureCategory::CloseFailed => "close_request",
        DesktopFailureCategory::CloseTimedOut => "close_observation",
        DesktopFailureCategory::TerminationFailed => "termination_request",
        DesktopFailureCategory::TerminationTimedOut => "termination_observation",
        DesktopFailureCategory::ActivationFailed => "activation",
        DesktopFailureCategory::LaunchNotObserved => "launch_observation",
    }
}

fn log_runtime_error(
    app: &AppHandle,
    event: &'static str,
    message_id: &'static str,
    details: &'static str,
) {
    app.state::<IssueLogRuntime>().store.append(
        IssueLogLevel::Error,
        event,
        message_id,
        Some(details.to_owned()),
    );
}

fn desktop_task_failed() -> DesktopFailure {
    DesktopFailure {
        category: DesktopFailureCategory::ActionUnavailable,
        message_id: "desktop.state_unavailable",
    }
}

#[tauri::command]
pub(crate) fn get_startup_snapshot(
    state: State<'_, StartupRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<StartupSnapshot, CommandFailure> {
    let result = state.inspect();
    log_startup_inspection(&logs.store, &result);
    result
}

#[tauri::command]
pub(crate) fn get_update_snapshot(state: State<'_, UpdateRuntime>) -> UpdateSnapshot {
    state.coordinator.snapshot()
}

#[tauri::command]
pub(crate) async fn get_desktop_snapshot(
    state: State<'_, DesktopRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || application.inspect())
        .await
        .map_err(|_| desktop_task_failed());
    finish_command(&logs.store, "desktop.inspect", result)
}

#[tauri::command]
pub(crate) async fn start_desktop_application(
    app: AppHandle,
    state: State<'_, DesktopRuntime>,
    sessions: State<'_, SessionRuntime>,
    environment: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    ensure_session_visibility_restart_allowed(&sessions.visibility, &logs.store)?;
    let coordination = coordinate_pending_session_visibility(
        Some(&app),
        &sessions.application,
        &sessions.visibility,
        &environment.application,
        &logs.store,
    )
    .await;
    ensure_coordination_allows_restart(coordination, &sessions.visibility)?;
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || application.start())
        .await
        .map_err(|_| desktop_task_failed())?;
    finish_command(&logs.store, "desktop.start", result)
}

#[tauri::command]
pub(crate) async fn restart_desktop_application(
    app: AppHandle,
    state: State<'_, DesktopRuntime>,
    sessions: State<'_, SessionRuntime>,
    environment: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
    expected_roots: Vec<crate::consumer::ConsumerIdentity>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    ensure_session_visibility_restart_allowed(&sessions.visibility, &logs.store)?;
    let application = state.application.clone();
    let session_application = sessions.application.clone();
    let visibility = sessions.visibility.clone();
    let environment_application = environment.application.clone();
    let checkpoint_app = app.clone();
    let checkpoint_logs = logs.store.clone();
    let expected_root_count = expected_roots.len();
    let (result, observed_after_failure) = tauri::async_runtime::spawn_blocking(move || {
        let result = application.restart_with_checkpoint(&expected_roots, move || {
            let coordination =
                tauri::async_runtime::block_on(coordinate_pending_session_visibility(
                    Some(&checkpoint_app),
                    &session_application,
                    &visibility,
                    &environment_application,
                    &checkpoint_logs,
                ));
            ensure_coordination_allows_restart(coordination, &visibility)
        });
        let observed_after_failure = result.is_err().then(|| application.inspect());
        (result, observed_after_failure)
    })
    .await
    .map_err(|_| desktop_task_failed())?;
    finish_command_with_desktop_restart(
        &logs.store,
        result,
        expected_root_count,
        observed_after_failure.as_ref(),
    )
}

fn ensure_coordination_allows_restart(
    coordination: Result<VisibilityCoordinationOutcome, VisibilityFailure>,
    visibility: &SessionVisibilityApplication,
) -> Result<(), DesktopFailure> {
    match coordination {
        Ok(outcome) if outcome.block_codex_restart => Err(visibility_restart_blocked()),
        Ok(_) => Ok(()),
        Err(_) => {
            let recovery = visibility.assess_recovery();
            if recovery.is_err() || recovery.is_ok_and(|assessment| assessment.block_codex_restart)
            {
                Err(visibility_restart_blocked())
            } else {
                Ok(())
            }
        }
    }
}

fn visibility_restart_blocked() -> DesktopFailure {
    DesktopFailure {
        category: DesktopFailureCategory::ActionUnavailable,
        message_id: "session_visibility.recovery_indeterminate",
    }
}

fn ensure_session_visibility_restart_allowed(
    visibility: &SessionVisibilityApplication,
    logs: &IssueLogStore,
) -> Result<(), DesktopFailure> {
    let assessment = visibility.assess_recovery().map_err(|failure| {
        logs.append(
            IssueLogLevel::Error,
            "desktop.session_visibility_gate",
            failure.message_id,
            Some(format!(
                "category=session_visibility stage={} error_code={}",
                failure.stage, failure.message_id
            )),
        );
        DesktopFailure {
            category: DesktopFailureCategory::ActionUnavailable,
            message_id: "session_visibility.recovery_indeterminate",
        }
    })?;
    if assessment.block_codex_restart {
        logs.append(
            IssueLogLevel::Error,
            "desktop.session_visibility_gate",
            "session_visibility.recovery_indeterminate",
            Some("category=session_visibility stage=recovery_gate error_code=session_visibility.recovery_indeterminate".to_owned()),
        );
        return Err(DesktopFailure {
            category: DesktopFailureCategory::ActionUnavailable,
            message_id: "session_visibility.recovery_indeterminate",
        });
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn install_update(
    app: AppHandle,
    state: State<'_, UpdateRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<UpdateSnapshot, UpdateInstallFailure> {
    let Some(guard) = state.activity.try_begin_install() else {
        let failure = UpdateInstallFailure {
            category: UpdateInstallFailureCategory::Busy,
            message_id: "update.busy",
        };
        log_update_install_failure(&logs.store, &failure, &state.coordinator.snapshot());
        return Err(failure);
    };
    let snapshot = match state.coordinator.confirm_install() {
        Ok(snapshot) => snapshot,
        Err(failure) => {
            log_update_install_failure(&logs.store, &failure, &state.coordinator.snapshot());
            return Err(failure);
        }
    };
    guard.commit_install();
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        if app
            .emit("update-install-started", snapshot.clone())
            .is_err()
        {
            log_runtime_error(
                &app,
                "update.install_started_event",
                "event.emit_failed",
                "category=event",
            );
        }
        app.exit(0);
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    let _ = app;
    Ok(snapshot)
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub(crate) async fn perform_update_check(
    app: &AppHandle,
    retry_incomplete: bool,
) -> UpdateSnapshot {
    let runtime = app.state::<UpdateRuntime>();
    let coordinator = runtime.coordinator.clone();
    let event_app = app.clone();
    let progress = move |snapshot| {
        if event_app.emit("update-progress", snapshot).is_err() {
            log_runtime_error(
                &event_app,
                "update.progress_event",
                "event.emit_failed",
                "category=event",
            );
        }
    };
    let snapshot = if retry_incomplete {
        coordinator.check_and_download(progress).await
    } else {
        coordinator.scheduled_check_and_download(progress).await
    };
    log_update_check_failure(&app.state::<IssueLogRuntime>().store, &snapshot);
    runtime.notify_if_hidden(app, &snapshot);
    if app.emit("update-progress", snapshot.clone()).is_err() {
        log_runtime_error(
            app,
            "update.progress_event",
            "event.emit_failed",
            "category=event",
        );
    }
    snapshot
}

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
pub(crate) async fn perform_update_check(
    app: &AppHandle,
    _retry_incomplete: bool,
) -> UpdateSnapshot {
    app.state::<UpdateRuntime>().coordinator.snapshot()
}

#[tauri::command]
pub(crate) async fn check_for_updates(app: AppHandle) -> UpdateSnapshot {
    perform_update_check(&app, true).await
}

fn log_update_check_failure(store: &IssueLogStore, snapshot: &UpdateSnapshot) {
    let Some(category) = snapshot.failure_category else {
        return;
    };
    store.append(
        IssueLogLevel::Error,
        "update.check",
        "应用更新检查失败",
        Some(format!(
            "category={} target_version={}",
            update_failure_category_name(category),
            snapshot.available_version.as_deref().unwrap_or("none"),
        )),
    );
}

fn log_update_install_failure(
    store: &IssueLogStore,
    failure: &UpdateInstallFailure,
    snapshot: &UpdateSnapshot,
) {
    store.append(
        IssueLogLevel::Error,
        "update.install",
        failure.message_id,
        Some(format!(
            "category={} target_version={}",
            update_install_failure_category_name(&failure.category),
            snapshot.available_version.as_deref().unwrap_or("none"),
        )),
    );
}

fn update_failure_category_name(category: UpdateFailureCategory) -> &'static str {
    match category {
        UpdateFailureCategory::CheckFailed => "check_failed",
        UpdateFailureCategory::ManifestInvalid => "manifest_invalid",
        UpdateFailureCategory::DownloadFailed => "download_failed",
        UpdateFailureCategory::SignatureInvalid => "signature_invalid",
    }
}

fn update_install_failure_category_name(category: &UpdateInstallFailureCategory) -> &'static str {
    match category {
        UpdateInstallFailureCategory::NoPendingUpdate => "no_pending_update",
        UpdateInstallFailureCategory::Busy => "busy",
        UpdateInstallFailureCategory::UnsupportedPlatform => "unsupported_platform",
        UpdateInstallFailureCategory::StateUnavailable => "state_unavailable",
        UpdateInstallFailureCategory::LaunchFailed => "launch_failed",
    }
}

#[tauri::command]
pub(crate) fn open_update_manual_download(
    logs: State<'_, IssueLogRuntime>,
) -> Result<(), CommandFailure> {
    finish_command(
        &logs.store,
        "update.open_manual_download",
        open_external_url(crate::update::MANUAL_DOWNLOAD_URL),
    )
}

#[tauri::command]
pub(crate) fn open_update_release_notes(
    logs: State<'_, IssueLogRuntime>,
    url: String,
) -> Result<(), CommandFailure> {
    let result = if !is_valid_update_release_notes_url(&url) {
        Err(CommandFailure {
            message_id: "update.release_notes_invalid",
        })
    } else {
        open_external_url(&url)
    };
    finish_command(&logs.store, "update.open_release_notes", result)
}

fn is_valid_update_release_notes_url(url: &str) -> bool {
    let prefix = format!("{}/v", crate::update::GITEE_RELEASES_URL);
    let Some(version) = url.strip_prefix(&prefix) else {
        return false;
    };
    let parts = version.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && (part == &"0" || !part.starts_with('0'))
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u64>().is_ok()
        })
}

#[cfg(test)]
mod update_release_notes_tests {
    use super::is_valid_update_release_notes_url;

    #[test]
    fn accepts_the_expected_gitee_release_url() {
        assert!(is_valid_update_release_notes_url(
            "https://gitee.com/ericshaohua/gpteasy-releases/releases/tag/v1.1.4"
        ));
    }

    #[test]
    fn rejects_untrusted_or_malformed_release_urls() {
        for url in [
            "https://example.com/releases/tag/v1.1.4",
            "https://gitee.com/ericshaohua/gpteasy-releases/releases/tag/v1.1",
            "https://gitee.com/ericshaohua/gpteasy-releases/releases/tag/v1.1.4/notes",
            "https://gitee.com/ericshaohua/gpteasy-releases/releases/tag/v1.1.4?next=evil",
            "https://gitee.com/ericshaohua/gpteasy-releases/releases/tag/v01.1.4",
            "https://gitee.com/ericshaohua/gpteasy-releases/releases/tag/v1.1.4-beta",
        ] {
            assert!(!is_valid_update_release_notes_url(url), "accepted {url}");
        }
    }
}

fn open_external_url(url: &str) -> Result<(), CommandFailure> {
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(url).spawn();
    result.map(|_| ()).map_err(|_| CommandFailure {
        message_id: "update.manual_download_failed",
    })
}

#[tauri::command]
pub(crate) fn refresh_startup_snapshot(
    state: State<'_, StartupRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<StartupSnapshot, CommandFailure> {
    let result = state.inspect();
    log_startup_inspection(&logs.store, &result);
    result
}

fn log_startup_inspection(store: &IssueLogStore, result: &Result<StartupSnapshot, CommandFailure>) {
    match result {
        Ok(snapshot) if snapshot.mode == crate::startup::ApplicationMode::Blocked => store.append(
            IssueLogLevel::Error,
            "startup.inspect",
            snapshot.message_id,
            Some(format!(
                "block_reason={:?}; database_status={:?}; config_status={:?}; \
                 last_applied_mode={:?}; managed_config_state={:?}; login_status={:?}",
                snapshot.block_reason,
                snapshot.database.status,
                snapshot.codex.config_status,
                snapshot
                    .database
                    .contents
                    .as_ref()
                    .and_then(|contents| contents.last_applied_mode),
                snapshot.codex.managed_config_state,
                snapshot.codex.login_status,
            )),
        ),
        Err(failure) => store.append(
            IssueLogLevel::Error,
            "startup.inspect",
            failure.message_id,
            None,
        ),
        _ => {}
    }
}

#[tauri::command]
pub(crate) fn list_providers(
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    finish_command(
        &logs.store,
        "provider.list",
        state.application.list_providers(),
    )
}

#[tauri::command]
pub(crate) async fn enter_session_management(
    logs: State<'_, IssueLogRuntime>,
    state: State<'_, SessionRuntime>,
    lease_id: String,
) -> Result<SessionAvailability, CommandFailure> {
    let availability = state.application.enter(&lease_id).await;
    state.record_availability(&availability);
    if availability.status != crate::session::SessionAvailabilityStatus::Available {
        logs.store.append(
            IssueLogLevel::Error,
            "session.enter",
            &availability.message_id,
            Some(format!(
                "status={:?}; version={:?}",
                availability.status, availability.codex_version
            )),
        );
    }
    Ok(availability)
}

#[tauri::command]
pub(crate) fn get_session_visibility_status(
    state: State<'_, SessionRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<Option<PendingSessionVisibilitySnapshot>, VisibilityFailure> {
    finish_command(
        &logs.store,
        "session_visibility.status",
        state.visibility.pending_status(),
    )
}

fn visibility_target_from_environment(
    context: &EnvironmentVisibilityContext,
) -> Result<VisibilityTarget, VisibilityFailure> {
    if context.state != crate::environment::EnvironmentState::Managed || context.pending_operation {
        return Err(VisibilityFailure {
            message_id: "session_visibility.environment_unavailable",
            stage: "target_snapshot",
        });
    }
    let (mode, model_provider) = match (&context.mode, &context.provider_id) {
        (Some(crate::environment::AuthenticationMode::OpenaiLogin), _) => {
            (VisibilityTargetMode::OpenaiLogin, "openai".to_owned())
        }
        (Some(crate::environment::AuthenticationMode::Provider), Some(provider_id)) => {
            (VisibilityTargetMode::Provider, provider_id.clone())
        }
        _ => {
            return Err(VisibilityFailure {
                message_id: "session_visibility.environment_unavailable",
                stage: "target_snapshot",
            });
        }
    };
    Ok(VisibilityTarget {
        mode,
        model_provider,
        environment_revision: context.revision.clone(),
    })
}

async fn record_and_coordinate_mode_switch(app: &AppHandle, logs: &IssueLogStore) {
    let sessions = app.state::<SessionRuntime>();
    let environment = app.state::<EnvironmentRuntime>();
    if record_mode_switch_pending_visibility(&environment.application, &sessions.visibility, logs)
        .is_err()
    {
        return;
    }
    emit_session_visibility_status(app, &sessions.visibility, logs);
    let _ = coordinate_pending_session_visibility(
        Some(app),
        &sessions.application,
        &sessions.visibility,
        &environment.application,
        logs,
    )
    .await;
}

fn record_mode_switch_pending_visibility(
    environment: &EnvironmentApplication,
    visibility: &SessionVisibilityApplication,
    logs: &IssueLogStore,
) -> Result<VisibilityTarget, VisibilityFailure> {
    let result = environment
        .inspect_for_session_visibility()
        .map_err(|_| VisibilityFailure {
            message_id: "session_visibility.environment_unavailable",
            stage: "mode_switch",
        })
        .and_then(|context| visibility_target_from_environment(&context))
        .and_then(|target| {
            visibility.record_pending(&target)?;
            Ok(target)
        });
    match &result {
        Ok(_) => log_visibility_pending_recorded(logs),
        Err(failure) => log_visibility_coordination_failure(logs, failure),
    }
    result
}

async fn coordinate_pending_session_visibility(
    app: Option<&AppHandle>,
    session: &SessionApplication,
    visibility: &SessionVisibilityApplication,
    environment: &EnvironmentApplication,
    logs: &IssueLogStore,
) -> Result<VisibilityCoordinationOutcome, VisibilityFailure> {
    let pending = match visibility.pending_status() {
        Ok(pending) => pending,
        Err(failure) => {
            log_visibility_coordination_failure(logs, &failure);
            return Err(failure);
        }
    };
    if pending.is_none() {
        return Ok(VisibilityCoordinationOutcome {
            status: VisibilityCoordinationStatus::Idle,
            block_codex_restart: false,
            error_code: "none".to_owned(),
            execution: None,
        });
    }
    let environment_context = match environment.inspect_for_session_visibility() {
        Ok(context) => context,
        Err(_) => {
            let failure = VisibilityFailure {
                message_id: "session_visibility.environment_unavailable",
                stage: "target_snapshot",
            };
            log_visibility_coordination_failure(logs, &failure);
            return Err(failure);
        }
    };
    let target = match visibility_target_from_environment(&environment_context) {
        Ok(target) => target,
        Err(failure) => {
            log_visibility_coordination_failure(logs, &failure);
            return Err(failure);
        }
    };
    let runtime = CommandVisibilityRuntime {
        session,
        environment,
    };
    let consumer = runtime.consumers(true);
    let mut availability = None;
    let (app_server, codex_version) = if consumer == VisibilityConsumerState::NoConsumers {
        let entered = session.enter("session-visibility-auto").await;
        let active_version =
            if entered.status == crate::session::SessionAvailabilityStatus::Available {
                session.active_app_server_version().await
            } else {
                None
            };
        let capability = match (entered.status, active_version.as_ref()) {
            (crate::session::SessionAvailabilityStatus::Available, Some(_)) => {
                VisibilityAppServerCapability::Available
            }
            (crate::session::SessionAvailabilityStatus::Incompatible, _) => {
                VisibilityAppServerCapability::Incompatible
            }
            _ => VisibilityAppServerCapability::Unavailable,
        };
        let version = active_version.or(entered.codex_version.clone());
        availability = Some(entered);
        (capability, version)
    } else {
        (VisibilityAppServerCapability::Unavailable, None)
    };
    if consumer == VisibilityConsumerState::NoConsumers
        && app_server == VisibilityAppServerCapability::Available
    {
        if let Err(failure) = visibility.mark_pending_running(&target) {
            session.leave("session-visibility-auto").await;
            log_visibility_coordination_failure(logs, &failure);
            return Err(failure);
        }
        if let Some(app) = app {
            emit_session_visibility_status(app, visibility, logs);
        }
    }
    let outcome = visibility
        .coordinate_pending(
            VisibilityScanContext {
                target,
                codex_version,
                app_server,
                consumer_state: consumer,
                execution_blockers: Vec::new(),
            },
            &runtime,
        )
        .await;
    if availability.is_some() {
        session.leave("session-visibility-auto").await;
    }
    match &outcome {
        Ok(outcome) => {
            log_visibility_coordination(logs, outcome);
            if let Some(app) = app {
                emit_session_visibility_status(app, visibility, logs);
                if matches!(
                    outcome.status,
                    VisibilityCoordinationStatus::Partial | VisibilityCoordinationStatus::Blocked
                ) {
                    let body = if outcome.block_codex_restart {
                        "会话可见性状态需要先确认，Codex 暂未重新启动"
                    } else {
                        "部分会话可见性仍需修复，可在会话管理中重试"
                    };
                    let _ = app
                        .notification()
                        .builder()
                        .title("GPTEasy")
                        .body(body)
                        .show();
                }
            }
        }
        Err(failure) => {
            log_visibility_coordination_failure(logs, failure);
            if let Some(app) = app {
                emit_session_visibility_status(app, visibility, logs);
            }
        }
    }
    outcome
}

fn log_visibility_coordination_failure(store: &IssueLogStore, failure: &VisibilityFailure) {
    store.append(
        IssueLogLevel::Error,
        "session_visibility.auto",
        failure.message_id,
        Some(format!(
            "stage={}; status=failed; error_codes={}",
            failure.stage, failure.message_id
        )),
    );
}

fn log_visibility_pending_recorded(store: &IssueLogStore) {
    store.append(
        IssueLogLevel::Info,
        "session_visibility.auto",
        "session_visibility.pending_recorded",
        Some("stage=mode_switch; status=pending; error_codes=none".to_owned()),
    );
}

fn log_visibility_coordination(store: &IssueLogStore, outcome: &VisibilityCoordinationOutcome) {
    let details = outcome.execution.as_ref().map_or_else(
        || {
            let consumer_state = match outcome.error_code.as_str() {
                "session_visibility.cli_running" => "cli_running",
                "session_visibility.desktop_running" => "desktop_running",
                "session_visibility.consumer_unknown" => "unknown",
                _ => "none",
            };
            format!(
                "stage=consumer_recheck; status={}; succeeded=0; retryable=0; \
                 verification_failed=0; schema_variant=unknown; consumer_state={consumer_state}; \
                 writes_started=false; recovery_required={}; block_codex_restart={}; error_code={}",
                outcome.status.as_str(),
                outcome.block_codex_restart,
                outcome.block_codex_restart,
                outcome.error_code
            )
        },
        VisibilityExecutionResult::diagnostic_details,
    );
    store.append(
        if matches!(
            outcome.status,
            VisibilityCoordinationStatus::Complete | VisibilityCoordinationStatus::Idle
        ) {
            IssueLogLevel::Info
        } else {
            IssueLogLevel::Warn
        },
        "session_visibility.auto",
        "session_visibility.coordinated",
        Some(details),
    );
}

fn log_visibility_status_event_failure(
    store: &IssueLogStore,
    message_id: &str,
    category: &str,
    stage: &str,
) {
    store.append(
        IssueLogLevel::Error,
        "session_visibility.status_event",
        message_id,
        Some(format!("category={category} stage={stage}")),
    );
}

fn emit_session_visibility_status(
    app: &AppHandle,
    visibility: &SessionVisibilityApplication,
    logs: &IssueLogStore,
) {
    let status = match visibility.pending_status() {
        Ok(status) => status,
        Err(failure) => {
            log_visibility_status_event_failure(
                logs,
                failure.message_id,
                "session_visibility",
                "pending_state",
            );
            return;
        }
    };
    if app
        .emit("session-visibility-status-changed", status)
        .is_err()
    {
        log_visibility_status_event_failure(
            logs,
            "runtime.event_unavailable",
            "runtime",
            "status_event",
        );
    }
}

#[tauri::command]
pub(crate) async fn preview_session_visibility(
    state: State<'_, SessionRuntime>,
    environment: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<SessionVisibilityPreview, VisibilityFailure> {
    let environment_before =
        environment
            .session_visibility_context()
            .map_err(|_| VisibilityFailure {
                message_id: "session_visibility.environment_unavailable",
                stage: "scan_context",
            });
    let environment_before =
        match finish_command(&logs.store, "session_visibility.scan", environment_before) {
            Ok(context) => context,
            Err(failure) => return Err(failure),
        };
    let availability = state.availability();
    let active_app_server_version = if availability
        .as_ref()
        .is_some_and(|value| value.status == crate::session::SessionAvailabilityStatus::Available)
    {
        state.application.active_app_server_version().await
    } else {
        None
    };
    let app_server = match (
        availability.as_ref().map(|value| value.status),
        active_app_server_version.as_ref(),
    ) {
        (Some(crate::session::SessionAvailabilityStatus::Available), Some(_)) => {
            VisibilityAppServerCapability::Available
        }
        (Some(crate::session::SessionAvailabilityStatus::Incompatible), _) => {
            VisibilityAppServerCapability::Incompatible
        }
        _ => VisibilityAppServerCapability::Unavailable,
    };
    let codex_version =
        active_app_server_version.or_else(|| availability.and_then(|value| value.codex_version));
    let mut execution_blockers = Vec::new();
    match environment_before.state {
        crate::environment::EnvironmentState::External => {
            execution_blockers.push("external_configuration".to_owned());
        }
        crate::environment::EnvironmentState::Conflict => {
            execution_blockers.push("managed_conflict".to_owned());
        }
        crate::environment::EnvironmentState::Managed => {}
    }
    if environment_before.pending_operation {
        execution_blockers.push("pending_config_operation".to_owned());
    }
    let (mode, model_provider) = match environment_before.mode {
        Some(crate::environment::AuthenticationMode::OpenaiLogin) => {
            (VisibilityTargetMode::OpenaiLogin, "openai".to_owned())
        }
        Some(crate::environment::AuthenticationMode::Provider) => {
            match environment_before.provider_id.clone() {
                Some(provider_id) => (VisibilityTargetMode::Provider, provider_id),
                None => {
                    execution_blockers.push("target_mode_unresolved".to_owned());
                    (VisibilityTargetMode::Unknown, "unknown".to_owned())
                }
            }
        }
        None => {
            execution_blockers.push("target_mode_unresolved".to_owned());
            (VisibilityTargetMode::Unknown, "unknown".to_owned())
        }
    };
    let visibility = state.visibility.clone();
    let consumer_state = scan_visibility_consumers(&state.application, true);
    let scan_context = VisibilityScanContext {
        target: VisibilityTarget {
            mode,
            model_provider,
            environment_revision: environment_before.revision.clone(),
        },
        codex_version,
        app_server,
        consumer_state,
        execution_blockers,
    };
    let scan = tokio::task::spawn_blocking(move || visibility.scan(scan_context))
        .await
        .map_err(|_| VisibilityFailure {
            message_id: "session_visibility.scan_failed",
            stage: "scan",
        })
        .and_then(|result| result);
    let mut preview = match finish_command(&logs.store, "session_visibility.scan", scan) {
        Ok(preview) => preview,
        Err(failure) => return Err(failure),
    };
    if preview.app_server == VisibilityAppServerCapability::Available
        && state
            .application
            .active_app_server_version()
            .await
            .is_none()
    {
        preview.app_server = VisibilityAppServerCapability::Unavailable;
        SessionVisibilityApplication::add_execution_blocker(&mut preview, "app_server_unavailable");
    }
    if let Ok(environment_after) = environment.session_visibility_context()
        && (environment_after.revision != environment_before.revision
            || environment_after.mode != environment_before.mode
            || environment_after.provider_id != environment_before.provider_id)
    {
        SessionVisibilityApplication::add_execution_blocker(
            &mut preview,
            "environment_revision_changed",
        );
    }
    log_session_visibility_preview(&logs.store, &preview);
    Ok(preview)
}

struct CommandVisibilityRuntime<'a> {
    session: &'a SessionApplication,
    environment: &'a EnvironmentApplication,
}

impl VisibilityExecutionRuntime for CommandVisibilityRuntime<'_> {
    fn current_target(&self) -> Result<VisibilityTarget, VisibilityFailure> {
        let context = self
            .environment
            .inspect_for_session_visibility()
            .map_err(|_| VisibilityFailure {
                message_id: "session_visibility.rescan_required",
                stage: "target_snapshot",
            })?;
        if context.state != crate::environment::EnvironmentState::Managed
            || context.pending_operation
        {
            return Err(VisibilityFailure {
                message_id: "session_visibility.rescan_required",
                stage: "target_snapshot",
            });
        }
        let (mode, model_provider) = match (context.mode, context.provider_id) {
            (Some(crate::environment::AuthenticationMode::OpenaiLogin), _) => {
                (VisibilityTargetMode::OpenaiLogin, "openai".to_owned())
            }
            (Some(crate::environment::AuthenticationMode::Provider), Some(provider_id)) => {
                (VisibilityTargetMode::Provider, provider_id)
            }
            _ => {
                return Err(VisibilityFailure {
                    message_id: "session_visibility.rescan_required",
                    stage: "target_snapshot",
                });
            }
        };
        Ok(VisibilityTarget {
            mode,
            model_provider,
            environment_revision: context.revision,
        })
    }

    fn baseline_views<'a>(
        &'a self,
        target_provider: &'a str,
    ) -> VisibilityRuntimeFuture<'a, VisibilityVerificationViews> {
        Box::pin(async move {
            let all_providers = collect_visibility_view(self.session, None).await?;
            let target_provider =
                collect_visibility_view(self.session, Some(target_provider)).await?;
            Ok(VisibilityVerificationViews {
                all_providers,
                target_provider,
            })
        })
    }

    fn shutdown_owned_app_server(&self) -> VisibilityRuntimeFuture<'_, ()> {
        Box::pin(async move {
            self.session.shutdown_now().await;
            Ok(())
        })
    }

    fn consumers(&self, exclude_owned_app_server: bool) -> VisibilityConsumerState {
        scan_visibility_consumers(self.session, exclude_owned_app_server)
    }

    fn verification_views<'a>(
        &'a self,
        target_provider: &'a str,
    ) -> VisibilityRuntimeFuture<'a, VisibilityVerificationViews> {
        Box::pin(async move {
            let all_providers = collect_visibility_view(self.session, None).await?;
            let target_provider =
                collect_visibility_view(self.session, Some(target_provider)).await?;
            Ok(VisibilityVerificationViews {
                all_providers,
                target_provider,
            })
        })
    }
}

fn scan_visibility_consumers(
    session: &SessionApplication,
    exclude_owned_app_server: bool,
) -> VisibilityConsumerState {
    let scanner = WindowsConsumerScanner::new();
    let exclusion = exclude_owned_app_server
        .then(|| session.owned_consumer_exclusion())
        .flatten();
    let scan = exclusion
        .as_ref()
        .map(|exclusion| scanner.scan_excluding(std::slice::from_ref(exclusion)))
        .unwrap_or_else(|| scanner.scan());
    if scan.desktop == ConsumerStatus::Unknown || scan.cli == ConsumerStatus::Unknown {
        VisibilityConsumerState::Unknown
    } else if scan.cli == ConsumerStatus::Running {
        VisibilityConsumerState::CliRunning
    } else if scan.desktop == ConsumerStatus::Running {
        VisibilityConsumerState::DesktopRunning
    } else {
        VisibilityConsumerState::NoConsumers
    }
}

async fn collect_visibility_view(
    application: &SessionApplication,
    model_provider: Option<&str>,
) -> Result<Vec<VisibilityThreadView>, VisibilityFailure> {
    let mut threads = Vec::new();
    for archived in [false, true] {
        let mut cursor = None;
        let mut seen_cursors = std::collections::HashSet::new();
        loop {
            let page = application
                .list(SessionQuery {
                    request_id: None,
                    archived,
                    search_term: None,
                    project: None,
                    model_provider: model_provider.map(str::to_owned),
                    cursor: cursor.clone(),
                    limit: 100,
                })
                .await
                .map_err(|_| VisibilityFailure {
                    message_id: "session_visibility.app_server_verification_failed",
                    stage: "app_server_query",
                })?;
            threads.extend(
                page.sessions
                    .into_iter()
                    .map(|session| VisibilityThreadView {
                        id: session.id,
                        archived,
                    }),
            );
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err(VisibilityFailure {
                    message_id: "session_visibility.app_server_verification_failed",
                    stage: "app_server_query",
                });
            }
            cursor = Some(next_cursor);
        }
    }
    Ok(threads)
}

#[tauri::command]
pub(crate) async fn execute_session_visibility(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    environment: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
    request: VisibilityExecutionRequest,
) -> Result<VisibilityExecutionResult, VisibilityFailure> {
    let runtime = CommandVisibilityRuntime {
        session: &state.application,
        environment: &environment.application,
    };
    let index_hash_before = state.visibility.diagnostic_index_hash();
    let result = state.visibility.execute_pending(request, &runtime).await;
    match &result {
        Ok(result) => {
            log_session_visibility_execution_result(&logs.store, result);
            if result.status != "complete" {
                let _ = app
                    .notification()
                    .builder()
                    .title("GPTEasy")
                    .body("部分会话仍需重试修复")
                    .show();
            }
        }
        Err(failure) => log_session_visibility_execution_failure(
            &logs.store,
            &state.visibility,
            failure,
            index_hash_before.as_deref(),
            runtime.consumers(true),
        ),
    }
    emit_session_visibility_status(&app, &state.visibility, &logs.store);
    result
}

fn log_session_visibility_execution_failure(
    store: &IssueLogStore,
    visibility: &SessionVisibilityApplication,
    failure: &VisibilityFailure,
    index_hash_before: Option<&str>,
    observed_consumer: VisibilityConsumerState,
) {
    let (manifest_writes_started, recovery_required, retryable) =
        visibility.diagnostic_recovery_evidence();
    let index_changed = visibility.diagnostic_index_hash().as_deref() != index_hash_before;
    let definitely_before_write = matches!(
        failure.stage,
        "pending_state"
            | "scan"
            | "storage_capability_recheck"
            | "confirmation_recheck"
            | "target_snapshot"
            | "target_recheck"
            | "consumer_preflight"
    );
    let consumer = match failure.message_id {
        "session_visibility.cli_running" => VisibilityConsumerState::CliRunning,
        "session_visibility.desktop_running" => VisibilityConsumerState::DesktopRunning,
        "session_visibility.consumer_unknown" => VisibilityConsumerState::Unknown,
        _ => observed_consumer,
    };
    store.append(
        IssueLogLevel::Error,
        "session_visibility.execute",
        failure.message_id,
        Some(format!(
            "stage={}; status=failed; error_code={}; succeeded=0; retryable={retryable}; \
             verification_failed=0; schema_variant={}; consumer_state={}; writes_started={}; \
             recovery_required={recovery_required}",
            failure.stage,
            failure.message_id,
            visibility.diagnostic_schema_variant(),
            consumer.diagnostic_name(),
            !definitely_before_write && (manifest_writes_started || index_changed),
        )),
    );
}

fn log_session_visibility_execution_result(
    store: &IssueLogStore,
    result: &VisibilityExecutionResult,
) {
    store.append(
        if result.status == "complete" {
            IssueLogLevel::Info
        } else {
            IssueLogLevel::Warn
        },
        "session_visibility.execute",
        result.message_id,
        Some(result.diagnostic_details()),
    );
}

fn log_session_visibility_preview(store: &IssueLogStore, preview: &SessionVisibilityPreview) {
    store.append(
        IssueLogLevel::Info,
        "session_visibility.scan",
        if preview.can_execute {
            "session_visibility.scan_completed"
        } else {
            "session_visibility.scan_blocked"
        },
        Some(preview.diagnostic_details()),
    );
}

#[tauri::command]
pub(crate) async fn leave_session_management(
    state: State<'_, SessionRuntime>,
    _logs: State<'_, IssueLogRuntime>,
    lease_id: String,
) -> Result<(), CommandFailure> {
    state.application.leave(&lease_id).await;
    Ok(())
}

#[tauri::command]
pub(crate) async fn list_sessions(
    state: State<'_, SessionRuntime>,
    logs: State<'_, IssueLogRuntime>,
    query: SessionQuery,
) -> Result<SessionListPage, SessionFailure> {
    finish_command(
        &logs.store,
        "session.list",
        state.application.list(query).await,
    )
}

#[tauri::command]
pub(crate) async fn cancel_session_request(
    state: State<'_, SessionRuntime>,
    _logs: State<'_, IssueLogRuntime>,
    request_id: String,
) -> Result<bool, CommandFailure> {
    Ok(state.application.cancel_list_request(&request_id).await)
}

#[tauri::command]
pub(crate) async fn read_session(
    state: State<'_, SessionRuntime>,
    logs: State<'_, IssueLogRuntime>,
    session_id: String,
) -> Result<SessionDetail, SessionFailure> {
    finish_command(
        &logs.store,
        "session.read",
        state.application.read(&session_id).await,
    )
}

#[tauri::command]
pub(crate) async fn archive_sessions(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    logs: State<'_, IssueLogRuntime>,
    session_ids: Vec<String>,
) -> Result<Vec<SessionMutationResult>, CommandFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("会话修改") else {
        return finish_command(
            &logs.store,
            "session.archive",
            Err(CommandFailure {
                message_id: "update.installing",
            }),
        );
    };
    finish_command(
        &logs.store,
        "session.archive",
        Ok(state.application.archive(session_ids).await),
    )
}

#[tauri::command]
pub(crate) async fn unarchive_sessions(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    logs: State<'_, IssueLogRuntime>,
    session_ids: Vec<String>,
) -> Result<Vec<SessionMutationResult>, CommandFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("会话修改") else {
        return finish_command(
            &logs.store,
            "session.unarchive",
            Err(CommandFailure {
                message_id: "update.installing",
            }),
        );
    };
    finish_command(
        &logs.store,
        "session.unarchive",
        Ok(state.application.unarchive(session_ids).await),
    )
}

#[tauri::command]
pub(crate) async fn delete_session(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    logs: State<'_, IssueLogRuntime>,
    session_id: String,
) -> Result<SessionMutationResult, CommandFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("会话修改") else {
        return finish_command(
            &logs.store,
            "session.delete",
            Err(CommandFailure {
                message_id: "update.installing",
            }),
        );
    };
    finish_command(
        &logs.store,
        "session.delete",
        Ok(state.application.delete(&session_id).await),
    )
}

#[tauri::command]
pub(crate) fn choose_session_export_destination(
    app: AppHandle,
    logs: State<'_, IssueLogRuntime>,
    suggested_title: String,
) -> Result<Option<String>, CommandFailure> {
    let selected = app
        .dialog()
        .file()
        .set_file_name(format!("{}.md", safe_export_name(&suggested_title)))
        .add_filter("Markdown", &["md"])
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let result = selected
        .into_path()
        .map(|path| Some(path.to_string_lossy().into_owned()))
        .map_err(|_| CommandFailure {
            message_id: "session.export_destination_invalid",
        });
    finish_command(&logs.store, "session.choose_export_destination", result)
}

#[tauri::command]
pub(crate) async fn export_session_markdown(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    logs: State<'_, IssueLogRuntime>,
    detail: SessionDetail,
    destination: String,
) -> Result<(), SessionFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("会话导出") else {
        return finish_command(
            &logs.store,
            "session.export",
            Err(SessionFailure::new(
                crate::session::SessionFailureCategory::WriteFailed,
                "update.installing",
            )),
        );
    };
    let result = state
        .application
        .export_markdown(&detail, std::path::Path::new(&destination))
        .await;
    finish_command(&logs.store, "session.export", result)
}

fn safe_export_name(title: &str) -> String {
    let sanitized = title
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches([' ', '.']);
    if sanitized.is_empty() {
        "Codex 会话".to_owned()
    } else {
        sanitized.chars().take(80).collect()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinuxExportDestination {
    path: String,
    exists: bool,
}

#[tauri::command]
pub(crate) fn choose_linux_export_destination(
    app: AppHandle,
    logs: State<'_, IssueLogRuntime>,
    shell: LinuxShell,
) -> Result<Option<LinuxExportDestination>, LinuxExportFailure> {
    let selected = app
        .dialog()
        .file()
        .set_file_name(shell.suggested_file_name())
        .add_filter(shell.display_name(), &[shell.extension()])
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = match selected.into_path().map_err(|_| LinuxExportFailure {
        category: crate::provider::LinuxExportFailureCategory::UnsafeDestination,
        message_id: "linux_export.unsafe_destination",
    }) {
        Ok(path) => path,
        Err(failure) => {
            return finish_command(&logs.store, "linux_export.choose_destination", Err(failure));
        }
    };
    finish_command(
        &logs.store,
        "linux_export.choose_destination",
        Ok(Some(LinuxExportDestination {
            exists: path.exists(),
            path: path.to_string_lossy().into_owned(),
        })),
    )
}

#[tauri::command]
pub(crate) fn export_linux_script(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    shell: LinuxShell,
    destination: String,
    confirm_overwrite: bool,
) -> Result<LinuxExportResult, LinuxExportFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("Linux 导出")
    else {
        return finish_command(
            &logs.store,
            "linux_export.write",
            Err(LinuxExportFailure {
                category: crate::provider::LinuxExportFailureCategory::StateUnavailable,
                message_id: "update.installing",
            }),
        );
    };
    let result = state.application.export_linux_script(
        shell,
        std::path::Path::new(&destination),
        confirm_overwrite,
    );
    finish_command(&logs.store, "linux_export.write", result)
}

#[tauri::command]
pub(crate) async fn get_environment_snapshot(
    state: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let application = state.application.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || application.inspect()).await {
        Ok(result) => result,
        Err(_) => Err(environment_task_failed()),
    };
    if let Ok(snapshot) = &result {
        if snapshot.state == crate::environment::EnvironmentState::Conflict {
            logs.store.append(
                IssueLogLevel::Error,
                "environment.inspect",
                snapshot.message_id,
                Some("state=conflict".to_owned()),
            );
        }
    }
    finish_command(&logs.store, "environment.inspect", result)
}

#[tauri::command]
pub(crate) fn list_issue_logs(
    state: State<'_, IssueLogRuntime>,
    since_epoch_seconds: i64,
    level: Option<IssueLogLevel>,
    query: Option<String>,
) -> Vec<IssueLogRecord> {
    state
        .store
        .list(since_epoch_seconds, level, query.as_deref())
}

#[tauri::command]
pub(crate) fn get_issue_log_path(state: State<'_, IssueLogRuntime>) -> String {
    state.store.path().to_string_lossy().into_owned()
}

#[tauri::command]
pub(crate) fn record_frontend_failure(
    state: State<'_, IssueLogRuntime>,
    event: FrontendFailureEvent,
) {
    let (event, message_id) = match event {
        FrontendFailureEvent::UpdateProgressListener => (
            "frontend.update_progress_listener",
            "event.listener_registration_failed",
        ),
        FrontendFailureEvent::ProviderSwitchListener => (
            "frontend.provider_switch_listener",
            "event.listener_registration_failed",
        ),
        FrontendFailureEvent::ProviderValidationProgressListener => (
            "frontend.provider_validation_progress_listener",
            "event.listener_registration_failed",
        ),
        FrontendFailureEvent::UnhandledError => {
            ("frontend.unhandled_error", "frontend.unhandled_error")
        }
        FrontendFailureEvent::UnhandledRejection => (
            "frontend.unhandled_rejection",
            "frontend.unhandled_rejection",
        ),
    };
    state.store.append(
        IssueLogLevel::Error,
        event,
        message_id,
        Some("category=frontend".to_owned()),
    );
}

#[tauri::command]
pub(crate) fn copy_issue_logs(
    app: AppHandle,
    state: State<'_, IssueLogRuntime>,
    since_epoch_seconds: i64,
    level: Option<IssueLogLevel>,
    query: Option<String>,
) -> Result<usize, CommandFailure> {
    let records = state
        .store
        .list(since_epoch_seconds, level, query.as_deref());
    let result = app
        .clipboard()
        .write_text(IssueLogStore::format(&records))
        .map_err(|_| CommandFailure {
            message_id: "diagnostics.copy_failed",
        })
        .map(|_| records.len());
    finish_command(&state.store, "diagnostics.copy", result)
}

#[tauri::command]
pub(crate) fn choose_issue_log_export_destination(
    app: AppHandle,
    logs: State<'_, IssueLogRuntime>,
) -> Result<Option<String>, CommandFailure> {
    let selected = app
        .dialog()
        .file()
        .set_file_name("gpteasy-issue-log.jsonl")
        .add_filter("JSON Lines", &["jsonl", "log"])
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(None);
    };
    let result = selected
        .into_path()
        .map(|path| Some(path.to_string_lossy().into_owned()))
        .map_err(|_| CommandFailure {
            message_id: "diagnostics.export_destination_invalid",
        });
    finish_command(&logs.store, "diagnostics.choose_export_destination", result)
}

#[tauri::command]
pub(crate) fn export_issue_logs(
    state: State<'_, IssueLogRuntime>,
    since_epoch_seconds: i64,
    level: Option<IssueLogLevel>,
    query: Option<String>,
    destination: String,
) -> Result<usize, CommandFailure> {
    let records = state
        .store
        .list(since_epoch_seconds, level, query.as_deref());
    let result = std::fs::write(destination, IssueLogStore::format(&records))
        .map_err(|_| CommandFailure {
            message_id: "diagnostics.export_failed",
        })
        .map(|_| records.len());
    finish_command(&state.store, "diagnostics.export", result)
}

#[tauri::command]
pub(crate) fn export_all_issue_logs(
    state: State<'_, IssueLogRuntime>,
    destination: String,
) -> Result<usize, CommandFailure> {
    let records = state.store.list_all(0, None, None);
    let result = std::fs::write(destination, IssueLogStore::format(&records))
        .map_err(|_| CommandFailure {
            message_id: "diagnostics.export_failed",
        })
        .map(|_| records.len());
    finish_command(&state.store, "diagnostics.export_all", result)
}

#[tauri::command]
pub(crate) async fn list_wsl_environments(
    state: State<'_, WslRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<Vec<WslEnvironmentSummary>, WslFailure> {
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let environments = application.list()?;
        let provider_count = application.verified_provider_count()?;
        Ok((environments, provider_count))
    })
    .await
    .map_err(|_| {
        WslFailure::new(
            crate::wsl::WslFailureCategory::StateUnavailable,
            "wsl.state_unavailable",
        )
    })
    .and_then(|result| result);
    if let Ok((environments, provider_count)) = &result {
        logs.store.append(
            IssueLogLevel::Info,
            "wsl.selection_state",
            "wsl.selection_state_observed",
            Some(wsl_inventory_details(*provider_count, environments)),
        );
    }
    let result = result.map(|(environments, _)| environments);
    finish_command(&logs.store, "wsl.list", result)
}

#[tauri::command]
pub(crate) async fn apply_wsl_provider(
    app: AppHandle,
    state: State<'_, WslRuntime>,
    logs: State<'_, IssueLogRuntime>,
    environment_id: String,
    provider_id: String,
    expected_revision: String,
    confirm: bool,
) -> Result<WslApplyResult, WslFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("WSL2 应用") else {
        return finish_command(
            &logs.store,
            "wsl.apply_provider",
            Err(WslFailure::new(
                crate::wsl::WslFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.apply_provider(&environment_id, &provider_id, &expected_revision, confirm)
    })
    .await
    .map_err(|_| {
        WslFailure::new(
            crate::wsl::WslFailureCategory::StateUnavailable,
            "wsl.state_unavailable",
        )
    })
    .and_then(|result| result);
    finish_command(&logs.store, "wsl.apply_provider", result)
}

#[tauri::command]
pub(crate) async fn reclaim_wsl_provider(
    app: AppHandle,
    state: State<'_, WslRuntime>,
    logs: State<'_, IssueLogRuntime>,
    environment_id: String,
    provider_id: String,
    expected_revision: String,
    authorize_start: bool,
    confirm_reclaim: bool,
) -> Result<WslApplyResult, WslFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("WSL2 重新接管")
    else {
        return finish_command(
            &logs.store,
            "wsl.reclaim_provider",
            Err(WslFailure::new(
                crate::wsl::WslFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    log_wsl_reclaim_phase(&logs.store, WslReclaimAuditPhase::SafeApplyCheck);
    if confirm_reclaim {
        log_wsl_reclaim_phase(&logs.store, WslReclaimAuditPhase::UserConfirmed);
    }
    let application = state.application.clone();
    let log_store = logs.store.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.reclaim_provider_with_progress(
            &environment_id,
            &provider_id,
            &expected_revision,
            authorize_start,
            confirm_reclaim,
            &mut |progress| log_wsl_reclaim_progress(&log_store, progress),
        )
    })
    .await
    .map_err(|_| {
        WslFailure::new(
            crate::wsl::WslFailureCategory::StateUnavailable,
            "wsl.state_unavailable",
        )
    })
    .and_then(|result| result);
    if matches!(
        &result,
        Err(failure) if failure.message_id == "wsl.reclaim_confirmation_required"
    ) {
        return result;
    }
    match &result {
        Ok(_) => log_wsl_reclaim_phase(&logs.store, WslReclaimAuditPhase::Succeeded),
        Err(_) => {}
    }
    finish_command(&logs.store, "wsl.reclaim_provider", result)
}

#[tauri::command]
pub(crate) async fn refresh_wsl_environment(
    app: AppHandle,
    state: State<'_, WslRuntime>,
    logs: State<'_, IssueLogRuntime>,
    environment_id: String,
    expected_revision: String,
    authorize_start: bool,
) -> Result<WslRefreshResult, WslFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("WSL2 环境协调")
    else {
        return finish_command(
            &logs.store,
            "wsl.refresh",
            Err(WslFailure::new(
                crate::wsl::WslFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.refresh_environment(&environment_id, &expected_revision, authorize_start)
    })
    .await
    .map_err(|_| {
        WslFailure::new(
            crate::wsl::WslFailureCategory::StateUnavailable,
            "wsl.state_unavailable",
        )
    })
    .and_then(|result| result);
    finish_command(&logs.store, "wsl.refresh", result)
}

#[tauri::command]
pub(crate) async fn apply_environment_provider(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
    provider_id: String,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("配置写入") else {
        return finish_command(
            &logs.store,
            "environment.apply_provider",
            Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let application = state.application.clone();
    let requested_provider = provider_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        match application.apply_provider_at_revision(&provider_id, true, &expected_revision) {
            Err(failure)
                if failure.category == EnvironmentFailureCategory::ConcurrentModification =>
            {
                let latest = application.inspect()?;
                if latest.state == crate::environment::EnvironmentState::Managed {
                    application.apply_provider_at_revision(&provider_id, true, &latest.revision)
                } else {
                    Err(failure)
                }
            }
            result => result,
        }
    })
    .await
    .map_err(|_| environment_task_failed())
    .and_then(|result| result);
    if let Ok(snapshot) = &result {
        logs.store.append(
            IssueLogLevel::Info,
            "environment.apply_provider",
            "供应商配置已写入",
            Some(format!(
                "provider_id={requested_provider}; pending_restart={}",
                snapshot.pending_restart
            )),
        );
        record_and_coordinate_mode_switch(&app, &logs.store).await;
    }
    let result = refresh_environment_tray_after(&app, result);
    finish_command(&logs.store, "environment.apply_provider", result)
}

#[tauri::command]
pub(crate) async fn force_apply_environment_provider(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
    provider_id: String,
    expected_revision: String,
    confirm_rebuild: bool,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("强制设置供应商")
    else {
        return finish_command(
            &logs.store,
            "environment.force_apply_provider",
            Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.force_apply_provider_at_revision(
            &provider_id,
            &expected_revision,
            confirm_rebuild,
        )
    })
    .await
    .map_err(|_| environment_task_failed())
    .and_then(|result| result);
    if result.is_ok() {
        record_and_coordinate_mode_switch(&app, &logs.store).await;
    }
    finish_command(&logs.store, "environment.force_apply_provider", result)
}

#[tauri::command]
pub(crate) async fn restore_last_environment_config(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
    confirm_restore: bool,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("配置恢复") else {
        return finish_command(
            &logs.store,
            "environment.restore",
            Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.restore_last_config(confirm_restore, &expected_revision)
    })
    .await
    .map_err(|_| environment_task_failed())
    .and_then(|result| result);
    let result = refresh_environment_tray_after(&app, result);
    finish_command(&logs.store, "environment.restore", result)
}

#[tauri::command]
pub(crate) async fn switch_to_openai_login(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    logs: State<'_, IssueLogRuntime>,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("配置写入") else {
        return finish_command(
            &logs.store,
            "environment.switch_to_openai_login",
            Err(EnvironmentFailure::new(
                EnvironmentFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.switch_to_openai_login(true, &expected_revision)
    })
    .await
    .map_err(|_| environment_task_failed())
    .and_then(|result| result);
    if let Ok(snapshot) = &result {
        logs.store.append(
            IssueLogLevel::Info,
            "environment.switch_to_openai_login",
            "已切换到 OpenAI 登录",
            Some(format!("state={:?}", snapshot.state)),
        );
        record_and_coordinate_mode_switch(&app, &logs.store).await;
    }
    let result = refresh_environment_tray_after(&app, result);
    finish_command(&logs.store, "environment.switch_to_openai_login", result)
}

#[tauri::command]
pub(crate) async fn discover_provider_models(
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    request_id: String,
    input: DiscoveryInput,
) -> Result<ModelDiscovery, ProviderFailure> {
    finish_command(
        &logs.store,
        "provider.discover_models",
        state.application.discover_models(request_id, input).await,
    )
}

#[tauri::command]
pub(crate) async fn discover_provider_models_for_update(
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    request_id: String,
    input: ProviderUpdateDiscoveryInput,
) -> Result<ModelDiscovery, ProviderFailure> {
    let result = state
        .application
        .discover_models_for_update(request_id, input)
        .await;
    finish_command(&logs.store, "provider.discover_models_for_update", result)
}

#[tauri::command]
pub(crate) async fn validate_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    request_id: String,
    input: ProviderValidationInput,
) -> Result<ProviderValidationReceipt, ProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商验证")
    else {
        return finish_command(
            &logs.store,
            "provider.validate",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let progress_request_id = request_id.clone();
    let result = state
        .application
        .validate_provider_with_progress(request_id, input, move |stage| {
            if app
                .emit(
                    "provider-validation-progress",
                    ProviderValidationProgress {
                        request_id: progress_request_id.clone(),
                        stage,
                    },
                )
                .is_err()
            {
                log_runtime_error(
                    &app,
                    "provider.validation_progress_event",
                    "event.emit_failed",
                    "category=event",
                );
            }
        })
        .await;
    finish_command(&logs.store, "provider.validate", result)
}

#[tauri::command]
pub(crate) async fn validate_provider_update(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    request_id: String,
    input: ProviderUpdateValidationInput,
) -> Result<ProviderValidationReceipt, ProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商验证")
    else {
        return finish_command(
            &logs.store,
            "provider.validate_update",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let progress_request_id = request_id.clone();
    let result = state
        .application
        .validate_provider_update_with_progress(request_id, input, move |stage| {
            if app
                .emit(
                    "provider-validation-progress",
                    ProviderValidationProgress {
                        request_id: progress_request_id.clone(),
                        stage,
                    },
                )
                .is_err()
            {
                log_runtime_error(
                    &app,
                    "provider.validation_progress_event",
                    "event.emit_failed",
                    "category=event",
                );
            }
        })
        .await;
    finish_command(&logs.store, "provider.validate_update", result)
}

#[tauri::command]
pub(crate) async fn revalidate_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    request_id: String,
    provider_id: String,
    audit_context: Option<ProviderRevalidationAuditContext>,
) -> Result<ProviderRevalidationResult, ProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商验证")
    else {
        return finish_command(
            &logs.store,
            "provider.revalidate",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let progress_request_id = request_id.clone();
    let result = state
        .application
        .revalidate_provider_with_progress(request_id, provider_id, move |stage| {
            if app
                .emit(
                    "provider-validation-progress",
                    ProviderValidationProgress {
                        request_id: progress_request_id.clone(),
                        stage,
                    },
                )
                .is_err()
            {
                log_runtime_error(
                    &app,
                    "provider.validation_progress_event",
                    "event.emit_failed",
                    "category=event",
                );
            }
        })
        .await;
    audit_provider_revalidation_result(&logs.store, audit_context, &result);
    finish_command(&logs.store, "provider.revalidate", result)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderValidationProgress {
    request_id: String,
    stage: ProviderValidationStage,
}

#[tauri::command]
pub(crate) fn cancel_provider_request(
    state: State<'_, ProviderRuntime>,
    request_id: String,
) -> bool {
    state.application.cancel_request(&request_id)
}

#[tauri::command]
pub(crate) fn confirm_provider_validation_base_url(
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    validation_id: String,
    base_url: String,
) -> Result<(), ProviderFailure> {
    let result = state
        .application
        .confirm_validation_base_url(&validation_id, &base_url);
    finish_command(&logs.store, "provider.confirm_base_url", result)
}

#[tauri::command]
pub(crate) fn save_verified_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    validation_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入")
    else {
        return finish_command(
            &logs.store,
            "provider.save",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let result = state
        .application
        .save_verified_provider(&validation_id, &name);
    finish_command(
        &logs.store,
        "provider.save",
        refresh_tray_after(&app, result),
    )
}

#[tauri::command]
pub(crate) fn save_dayway_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    validation_id: String,
    confirm_name_conflict: bool,
) -> Result<ProviderSummary, ProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入")
    else {
        return finish_command(
            &logs.store,
            "provider.save_dayway",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let result = state
        .application
        .save_dayway_provider_with_name_conflict_confirmation(
            &validation_id,
            confirm_name_conflict,
        );
    finish_command(
        &logs.store,
        "provider.save_dayway",
        refresh_tray_after(&app, result),
    )
}

#[tauri::command]
pub(crate) fn open_dayway_website(logs: State<'_, IssueLogRuntime>) -> Result<(), ProviderFailure> {
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe").arg(DAYWAY_WEBSITE).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(DAYWAY_WEBSITE).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(DAYWAY_WEBSITE).spawn();
    let result = result.map(|_| ()).map_err(|_| {
        ProviderFailure::new(
            ProviderFailureCategory::StateUnavailable,
            "provider.website_open_failed",
        )
    });
    finish_command(&logs.store, "provider.open_dayway_website", result)
}

#[tauri::command]
pub(crate) fn rename_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    provider_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入")
    else {
        return finish_command(
            &logs.store,
            "provider.rename",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let result = state.application.rename_provider(&provider_id, &name);
    finish_command(
        &logs.store,
        "provider.rename",
        refresh_tray_after(&app, result),
    )
}

#[tauri::command]
pub(crate) fn save_provider_update(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    validation_id: String,
    provider_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入")
    else {
        return finish_command(
            &logs.store,
            "provider.save_update",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let environment = app.state::<EnvironmentRuntime>();
    let result = state.application.save_provider_update_for_environment(
        &environment.application,
        &validation_id,
        &provider_id,
        &name,
    );
    finish_command(
        &logs.store,
        "provider.save_update",
        refresh_tray_after(&app, result),
    )
}

#[tauri::command]
pub(crate) async fn save_and_apply_provider_update(
    app: AppHandle,
    logs: State<'_, IssueLogRuntime>,
    validation_id: String,
    provider_id: String,
    name: String,
) -> Result<AppliedProviderUpdate, ProviderFailure> {
    let Some(_activity) = app.state::<UpdateRuntime>().activity.try_begin("配置写入") else {
        return finish_command(
            &logs.store,
            "provider.save_and_apply_update",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let provider_state = task_app.state::<ProviderRuntime>();
        let environment_state = task_app.state::<EnvironmentRuntime>();
        provider_state.application.save_and_apply_provider_update(
            &environment_state.application,
            &validation_id,
            &provider_id,
            &name,
            true,
        )
    })
    .await
    .map_err(|_| {
        ProviderFailure::new(
            ProviderFailureCategory::StateUnavailable,
            "provider.state_unavailable",
        )
    })
    .and_then(|result| result);
    let result = match result {
        Ok(applied) => {
            let _ = tray::refresh_with_snapshot(&app, &applied.environment);
            Ok(applied)
        }
        Err(failure) => Err(failure),
    };
    finish_command(&logs.store, "provider.save_and_apply_update", result)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteProviderResult {
    lifecycle_results: Vec<WslLifecycleResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteProviderFailure {
    category: &'static str,
    message_id: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    lifecycle_outcome: Option<WslLifecycleOutcome>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lifecycle_results: Vec<WslLifecycleResult>,
}

#[tauri::command]
pub(crate) async fn delete_provider(
    app: AppHandle,
    logs: State<'_, IssueLogRuntime>,
    provider_id: String,
    authorize_stopped_wsl: bool,
) -> Result<DeleteProviderResult, DeleteProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入")
    else {
        return finish_command(
            &logs.store,
            "provider.delete",
            Err(DeleteProviderFailure {
                category: "state_unavailable",
                message_id: "update.installing",
                lifecycle_outcome: None,
                lifecycle_results: Vec::new(),
            }),
        );
    };
    let task_app = app.clone();
    let audit_provider_id = provider_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let provider_state = task_app.state::<ProviderRuntime>();
        let wsl_state = task_app.state::<WslRuntime>();
        wsl_state.application.audit_provider_deletion_then(
            &audit_provider_id,
            authorize_stopped_wsl,
            || {
                provider_state
                    .application
                    .delete_provider(&audit_provider_id)
            },
        )
    })
    .await
    .map_err(|_| DeleteProviderFailure {
        category: "state_unavailable",
        message_id: "wsl.state_unavailable",
        lifecycle_outcome: None,
        lifecycle_results: Vec::new(),
    });
    let result = match result {
        Err(failure) => Err(failure),
        Ok(Ok((audit, ()))) => Ok(DeleteProviderResult {
            lifecycle_results: audit.lifecycle_results,
        }),
        Ok(Err(WslDeletionAuditError::Verification(failure))) => Err(DeleteProviderFailure {
            category: "wsl_verification",
            message_id: failure.message_id,
            lifecycle_outcome: failure.lifecycle_outcome,
            lifecycle_results: Vec::new(),
        }),
        Ok(Err(WslDeletionAuditError::Deletion {
            failure,
            lifecycle_results,
        })) => Err(DeleteProviderFailure {
            category: "provider",
            message_id: failure.message_id,
            lifecycle_outcome: None,
            lifecycle_results,
        }),
    };
    finish_command(
        &logs.store,
        "provider.delete",
        refresh_tray_after(&app, result),
    )
}

#[tauri::command]
pub(crate) fn reorder_providers(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    provider_ids: Vec<String>,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    let Some(_activity) = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入")
    else {
        return finish_command(
            &logs.store,
            "provider.reorder",
            Err(ProviderFailure::new(
                ProviderFailureCategory::StateUnavailable,
                "update.installing",
            )),
        );
    };
    let result = state.application.reorder_providers(&provider_ids);
    finish_command(
        &logs.store,
        "provider.reorder",
        refresh_tray_after(&app, result),
    )
}

#[tauri::command]
pub(crate) fn reveal_provider_api_key(
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    provider_id: String,
) -> Result<ProviderApiKey, ProviderFailure> {
    finish_command(
        &logs.store,
        "provider.reveal_api_key",
        state.application.reveal_provider_api_key(&provider_id),
    )
}

#[tauri::command]
pub(crate) fn copy_provider_api_key(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    logs: State<'_, IssueLogRuntime>,
    provider_id: String,
) -> Result<(), ProviderFailure> {
    let result = state
        .application
        .reveal_provider_api_key(&provider_id)
        .and_then(|api_key| {
            app.clipboard().write_text(api_key.expose()).map_err(|_| {
                ProviderFailure::new(
                    ProviderFailureCategory::ClipboardUnavailable,
                    "provider.clipboard_unavailable",
                )
            })
        });
    finish_command(&logs.store, "provider.copy_api_key", result)
}

#[tauri::command]
pub(crate) fn discard_provider_validation(
    state: State<'_, ProviderRuntime>,
    validation_id: String,
) {
    state.application.discard_validation(&validation_id);
}

fn refresh_tray_after<T, E>(app: &AppHandle, result: Result<T, E>) -> Result<T, E> {
    if result.is_ok() {
        let _ = tray::refresh(app);
    }
    result
}

fn refresh_environment_tray_after(
    app: &AppHandle,
    result: Result<EnvironmentSnapshot, EnvironmentFailure>,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    if let Ok(snapshot) = &result {
        let _ = tray::refresh_with_snapshot(app, snapshot);
    }
    result
}

fn environment_task_failed() -> EnvironmentFailure {
    EnvironmentFailure::new(
        EnvironmentFailureCategory::StateUnavailable,
        "environment.state_unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        DeleteProviderFailure, IssueLogLevel, IssueLogStore, ProviderFailure,
        ProviderFailureCategory, ProviderRevalidationAuditContext, ProviderRevalidationResult,
        ProviderSummary, ProviderValidationReceipt, UpdateFailureCategory, UpdateInstallFailure,
        UpdateInstallFailureCategory, UpdateSnapshot, UpdateState, WslReclaimAuditPhase,
        audit_provider_revalidation_result, ensure_coordination_allows_restart,
        ensure_session_visibility_restart_allowed, finish_command,
        finish_command_with_desktop_restart, log_session_visibility_execution_failure,
        log_session_visibility_execution_result, log_session_visibility_preview,
        log_update_check_failure, log_update_install_failure, log_visibility_coordination,
        log_visibility_coordination_failure, log_visibility_pending_recorded,
        log_visibility_status_event_failure, log_wsl_reclaim_phase,
        record_mode_switch_pending_visibility, wsl_inventory_details,
    };
    use crate::codex::{LoginInspection, LoginMethod, LoginStatus};
    use crate::consumer::{
        ConsumerIdentity, ConsumerRole, ConsumerScan, ConsumerScanner, ConsumerStatus,
    };
    use crate::desktop::{DesktopAction, DesktopFailure, DesktopFailureCategory, DesktopSnapshot};
    use crate::environment::{AuthenticationMode, EnvironmentApplication, OpenAiLoginProbe};
    use crate::session_visibility::{
        SessionVisibilityApplication, SessionVisibilityPreview, VisibilityAppServerCapability,
        VisibilityConsumerState, VisibilityCoordinationOutcome, VisibilityCoordinationStatus,
        VisibilityExecutionBreakdown, VisibilityExecutionReadiness, VisibilityExecutionResult,
        VisibilityFailure, VisibilityIndexPlan, VisibilityReason, VisibilitySchemaCapability,
        VisibilitySummary, VisibilityTarget, VisibilityTargetMode,
    };
    use crate::state::{PendingSessionVisibilityTargetMode, StatePaths, StateStore};
    use crate::wsl::{
        WslAvailability, WslConfigurationState, WslEnvironmentSummary, WslReclaimPreview,
        WslReclaimScope,
    };
    use rusqlite::{Connection, params};
    use std::sync::Arc;
    use tempfile::tempdir;

    struct LoggedOutProbe;

    impl OpenAiLoginProbe for LoggedOutProbe {
        fn inspect(&self) -> LoginInspection {
            LoginInspection {
                status: LoginStatus::NotLoggedIn,
                method: LoginMethod::Unknown,
            }
        }
    }

    struct StoppedConsumerScanner;

    impl ConsumerScanner for StoppedConsumerScanner {
        fn scan(&self) -> ConsumerScan {
            ConsumerScan {
                desktop: ConsumerStatus::Stopped,
                cli: ConsumerStatus::Stopped,
                identities: Vec::new(),
                desktop_roots: Vec::new(),
            }
        }
    }

    fn failed_update_snapshot(category: UpdateFailureCategory) -> UpdateSnapshot {
        UpdateSnapshot {
            current_version: "1.0.0".to_owned(),
            state: UpdateState::Failed,
            available_version: Some("1.1.0".to_owned()),
            notes: None,
            published_at: None,
            checked_at_epoch_seconds: Some(1),
            downloaded_bytes: 0,
            total_bytes: None,
            progress_percent: None,
            failure_category: Some(category),
            error_message: None,
            manual_download_url: "https://example.invalid/download".to_owned(),
            release_notes_url: None,
        }
    }

    #[test]
    fn session_visibility_scan_log_keeps_minimum_evidence_without_sensitive_identity() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let preview = SessionVisibilityPreview {
            confirmation_id: "redacted-confirmation".to_owned(),
            target: VisibilityTarget {
                mode: VisibilityTargetMode::Provider,
                model_provider: "sensitive-provider-id".to_owned(),
                environment_revision: "sensitive-environment-revision".to_owned(),
            },
            codex_version: Some("codex-cli 0.150.1".to_owned()),
            app_server: VisibilityAppServerCapability::Unavailable,
            schema: VisibilitySchemaCapability {
                status: "supported".to_owned(),
                database: "state_5.sqlite".to_owned(),
                variant: "legacy".to_owned(),
            },
            index_plan: VisibilityIndexPlan {
                app_server_coordination: 1,
                sqlite_fallback_eligible: 1,
                schema_skipped: 0,
            },
            summary: VisibilitySummary {
                candidates: 3,
                unchanged: 2,
                missing_index: 1,
                skipped: 4,
                blocked: 3,
                encrypted_content_risk: 1,
                active: 6,
                archived: 3,
            },
            readiness: VisibilityExecutionReadiness::AppServerUnavailable,
            consumer_state: VisibilityConsumerState::NoConsumers,
            can_execute: false,
            blockers: vec!["app_server_unavailable".to_owned()],
            reasons: vec![VisibilityReason {
                code: "provider_mismatch".to_owned(),
                count: 2,
            }],
        };

        log_session_visibility_preview(&store, &preview);

        let records = store.list(
            0,
            Some(IssueLogLevel::Info),
            Some("session_visibility.scan"),
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "session_visibility.scan_blocked");
        let details = records[0].details.as_deref().expect("scan details");
        for required in [
            "stage=scan",
            "target_mode=provider",
            "codex_version=codex-cli 0.150.1",
            "readiness=app_server_unavailable",
            "consumer_state=none",
            "schema=supported",
            "schema_variant=legacy",
            "candidates=3",
            "unchanged=2",
            "missing_index=1",
            "skipped=4",
            "blocked=3",
            "encrypted_content_risk=1",
            "active=6",
            "archived=3",
            "index_app_server_coordination=1",
            "index_sqlite_fallback_eligible=1",
            "index_schema_skipped=0",
            "error_codes=app_server_unavailable,provider_mismatch",
        ] {
            assert!(details.contains(required), "missing {required}: {details}");
        }
        let encoded = serde_json::to_string(&records[0]).expect("serialize issue log record");
        for sensitive in [
            "sensitive-provider-id",
            "sensitive-environment-revision",
            "private-title",
            "private-body",
            "C:\\private\\workspace",
            "11111111-1111-4111-8111-111111111111",
            "api-key-canary",
        ] {
            assert!(!encoded.contains(sensitive), "issue log leaked {sensitive}");
        }
    }

    #[test]
    fn session_visibility_failure_log_records_stable_stage_and_error_code() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());

        let result = finish_command::<(), _>(
            &store,
            "session_visibility.execute",
            Err(VisibilityFailure {
                message_id: "session_visibility.rescan_required",
                stage: "post_shutdown_target_recheck",
            }),
        );

        assert!(result.is_err());
        let records = store.list(
            0,
            Some(IssueLogLevel::Error),
            Some("session_visibility.execute"),
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "session_visibility.rescan_required");
        assert_eq!(
            records[0].details.as_deref(),
            Some(
                "category=session_visibility stage=post_shutdown_target_recheck error_code=session_visibility.rescan_required"
            )
        );
    }

    #[test]
    fn session_visibility_consumer_failure_log_states_that_no_write_or_recovery_started() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let visibility = SessionVisibilityApplication::with_recovery_root(
            directory.path().join("codex"),
            directory.path(),
        );
        let failure = VisibilityFailure {
            message_id: "session_visibility.cli_running",
            stage: "consumer_preflight",
        };

        log_session_visibility_execution_failure(
            &store,
            &visibility,
            &failure,
            None,
            VisibilityConsumerState::NoConsumers,
        );

        let records = store.list(
            0,
            Some(IssueLogLevel::Error),
            Some("session_visibility.execute"),
        );
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].details.as_deref(),
            Some(
                "stage=consumer_preflight; status=failed; error_code=session_visibility.cli_running; succeeded=0; retryable=0; verification_failed=0; schema_variant=missing; consumer_state=cli_running; writes_started=false; recovery_required=false"
            )
        );
    }

    #[test]
    fn session_visibility_partial_result_log_persists_redacted_stage_and_error_code() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let result = VisibilityExecutionResult {
            status: "partial",
            succeeded: 1,
            retryable: 2,
            encrypted_content_risk: 1,
            breakdown: VisibilityExecutionBreakdown {
                app_server_coordinated: 1,
                sqlite_fallback: 1,
                schema_skipped: 0,
                verification_failed: 1,
            },
            schema_variant: "codex_0_150_1".to_owned(),
            consumer_state: VisibilityConsumerState::NoConsumers,
            writes_started: true,
            recovery_required: true,
            block_codex_restart: false,
            message_id: "session_visibility.repair_partial",
            diagnostic_stage: "rollout_replace",
            error_code: "session_visibility.write_failed",
        };

        log_session_visibility_execution_result(&store, &result);

        let records = store.list(
            0,
            Some(IssueLogLevel::Warn),
            Some("session_visibility.execute"),
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "session_visibility.repair_partial");
        assert_eq!(
            records[0].details.as_deref(),
            Some(
                "stage=rollout_replace; status=partial; succeeded=1; retryable=2; encrypted_content_risk=1; index_app_server_coordinated=1; index_sqlite_fallback=1; index_schema_skipped=0; verification_failed=1; schema_variant=codex_0_150_1; consumer_state=none; writes_started=true; recovery_required=true; block_codex_restart=false; error_code=session_visibility.write_failed"
            )
        );
    }

    #[test]
    fn automatic_visibility_coordination_logs_early_failures_without_target_details() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let failure = VisibilityFailure {
            message_id: "session_visibility.environment_unavailable",
            stage: "target_snapshot",
        };

        log_visibility_coordination_failure(&store, &failure);

        let records = store.list(
            0,
            Some(IssueLogLevel::Error),
            Some("session_visibility.auto"),
        );
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].details.as_deref(),
            Some(
                "stage=target_snapshot; status=failed; error_codes=session_visibility.environment_unavailable"
            )
        );
        let encoded = serde_json::to_string(&records[0]).expect("serialize issue log record");
        assert!(!encoded.contains("sensitive-provider-id"));
        assert!(!encoded.contains("sensitive-environment-revision"));
    }

    #[test]
    fn production_mode_switch_seam_records_both_targets_and_never_rolls_back_on_failure() {
        const PROVIDER_ID: &str = "9f319739-f219-48ee-be35-22e08d5402d7";
        let directory = tempdir().expect("mode switch seam directory");
        let store = StateStore::new(StatePaths::from_root(directory.path().join("state")));
        assert!(store.bootstrap().is_ready());
        Connection::open(store.paths().database())
            .expect("open state database")
            .execute(
                "INSERT INTO providers (
                    id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint
                 ) VALUES (?1, 'Fixture Provider', 'https://fixture.example/v1',
                           'test-key-not-real', 'fixture-model', '1775606400',
                           'fixture-verification-fingerprint')",
                params![PROVIDER_ID],
            )
            .expect("insert provider fixture");
        let codex_home = directory.path().join(".codex");
        let environment = EnvironmentApplication::with_runtime_probes(
            store.clone(),
            &codex_home,
            Arc::new(LoggedOutProbe),
            Arc::new(StoppedConsumerScanner),
        );
        let visibility = SessionVisibilityApplication::with_recovery_root(
            &codex_home,
            directory.path().join("visibility-recovery"),
        )
        .with_pending_state(store.clone());
        let logs = IssueLogStore::new(directory.path().join("logs"));

        let provider = environment
            .apply_provider(PROVIDER_ID, true)
            .expect("switch to provider mode");
        let provider_target =
            record_mode_switch_pending_visibility(&environment, &visibility, &logs)
                .expect("run production provider follow-up seam");
        assert_eq!(provider_target.mode, VisibilityTargetMode::Provider);
        assert_eq!(provider_target.environment_revision, provider.revision);
        assert_eq!(
            store
                .pending_session_visibility()
                .expect("read provider pending state")
                .expect("provider pending state")
                .target_mode,
            PendingSessionVisibilityTargetMode::Provider,
        );

        let openai = environment
            .switch_to_openai_login(true, &provider.revision)
            .expect("switch to OpenAI login mode");
        let openai_target = record_mode_switch_pending_visibility(&environment, &visibility, &logs)
            .expect("run production OpenAI follow-up seam");
        assert_eq!(openai_target.mode, VisibilityTargetMode::OpenaiLogin);
        assert_eq!(openai_target.environment_revision, openai.revision);
        assert_eq!(
            store
                .pending_session_visibility()
                .expect("read OpenAI pending state")
                .expect("OpenAI pending state")
                .target_mode,
            PendingSessionVisibilityTargetMode::OpenaiLogin,
        );

        let switched_back = environment
            .apply_provider(PROVIDER_ID, true)
            .expect("switch back before follow-up failure");
        let unavailable_visibility = SessionVisibilityApplication::with_recovery_root(
            &codex_home,
            directory.path().join("unavailable-visibility-recovery"),
        );
        record_mode_switch_pending_visibility(&environment, &unavailable_visibility, &logs)
            .expect_err("persisting the independent follow-up state fails");
        let after_failure = environment
            .inspect_for_session_visibility()
            .expect("inspect mode after follow-up failure");
        assert_eq!(after_failure.mode, Some(AuthenticationMode::Provider));
        assert_eq!(after_failure.provider_id.as_deref(), Some(PROVIDER_ID));
        assert_eq!(after_failure.revision, switched_back.revision);
    }

    #[test]
    fn every_successful_native_mode_switch_keeps_the_visibility_follow_up_hook() {
        let source = include_str!("commands.rs");
        let hook = [
            "record_and_coordinate_mode_",
            "switch(&app, &logs.store).await;",
        ]
        .concat();
        assert_eq!(
            source.matches(&hook).count(),
            3,
            "apply, force apply, and OpenAI login must all run the shared follow-up",
        );
        assert!(source.contains("record_mode_switch_pending_visibility("));
        assert!(source.contains("coordinate_pending_session_visibility("));
    }

    #[test]
    fn automatic_visibility_logs_cover_mode_switch_execution_deferral_and_status_events() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        log_visibility_pending_recorded(&store);
        log_visibility_coordination(
            &store,
            &VisibilityCoordinationOutcome {
                status: VisibilityCoordinationStatus::Complete,
                block_codex_restart: false,
                error_code: "none".to_owned(),
                execution: Some(VisibilityExecutionResult {
                    status: "complete",
                    succeeded: 3,
                    retryable: 0,
                    encrypted_content_risk: 0,
                    breakdown: VisibilityExecutionBreakdown {
                        app_server_coordinated: 1,
                        sqlite_fallback: 1,
                        schema_skipped: 0,
                        verification_failed: 0,
                    },
                    schema_variant: "codex_0_150_1".to_owned(),
                    consumer_state: VisibilityConsumerState::NoConsumers,
                    writes_started: true,
                    recovery_required: false,
                    block_codex_restart: false,
                    message_id: "session_visibility.repair_complete",
                    diagnostic_stage: "verify",
                    error_code: "none",
                }),
            },
        );
        log_visibility_coordination(
            &store,
            &VisibilityCoordinationOutcome {
                status: VisibilityCoordinationStatus::Deferred,
                block_codex_restart: false,
                error_code: "session_visibility.cli_running".to_owned(),
                execution: None,
            },
        );
        log_visibility_status_event_failure(
            &store,
            "runtime.event_unavailable",
            "runtime",
            "status_event",
        );

        let automatic = store.list(0, None, Some("session_visibility.auto"));
        assert_eq!(automatic.len(), 3);
        assert!(automatic.iter().any(|record| {
            record.message == "session_visibility.pending_recorded"
                && record.details.as_deref()
                    == Some("stage=mode_switch; status=pending; error_codes=none")
        }));
        assert!(automatic.iter().any(|record| {
            record.details.as_deref()
                == Some(
                    "stage=verify; status=complete; succeeded=3; retryable=0; encrypted_content_risk=0; index_app_server_coordinated=1; index_sqlite_fallback=1; index_schema_skipped=0; verification_failed=0; schema_variant=codex_0_150_1; consumer_state=none; writes_started=true; recovery_required=false; block_codex_restart=false; error_code=none",
                )
        }));
        assert!(automatic.iter().any(|record| {
            record.details.as_deref()
                == Some(
                    "stage=consumer_recheck; status=deferred; succeeded=0; retryable=0; verification_failed=0; schema_variant=unknown; consumer_state=cli_running; writes_started=false; recovery_required=false; block_codex_restart=false; error_code=session_visibility.cli_running",
                )
        }));
        let status_events = store.list(
            0,
            Some(IssueLogLevel::Error),
            Some("session_visibility.status_event"),
        );
        assert_eq!(status_events.len(), 1);
        assert_eq!(
            status_events[0].details.as_deref(),
            Some("category=runtime stage=status_event")
        );
        let encoded = serde_json::to_string(&(automatic, status_events))
            .expect("serialize automatic visibility logs");
        for sensitive in [
            "sensitive-provider-id",
            "sensitive-environment-revision",
            "C:\\private\\rollout.jsonl",
            "api-key-canary",
        ] {
            assert!(!encoded.contains(sensitive), "issue log leaked {sensitive}");
        }
    }

    #[test]
    fn ordinary_visibility_coordination_failure_does_not_block_desktop_restart() {
        let directory = tempdir().expect("visibility recovery directory");
        let visibility = SessionVisibilityApplication::with_recovery_root(
            directory.path().join("codex-home"),
            directory.path(),
        );

        ensure_coordination_allows_restart(
            Err(VisibilityFailure {
                message_id: "session_visibility.rescan_required",
                stage: "consumer_recheck",
            }),
            &visibility,
        )
        .expect("a determinate coordination failure must not block restart");
    }

    #[test]
    fn indeterminate_visibility_recovery_blocks_desktop_start_and_restart() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        std::fs::write(
            directory.path().join("session-visibility-recovery.json"),
            b"not-json",
        )
        .expect("write indeterminate recovery manifest");
        let visibility = SessionVisibilityApplication::with_recovery_root(
            directory.path().join("codex-home"),
            directory.path(),
        );

        let failure = ensure_session_visibility_restart_allowed(&visibility, &store)
            .expect_err("indeterminate recovery blocks desktop lifecycle");

        assert_eq!(
            failure.message_id,
            "session_visibility.recovery_indeterminate"
        );
        let records = store.list(
            0,
            Some(IssueLogLevel::Error),
            Some("desktop.session_visibility_gate"),
        );
        assert_eq!(records.len(), 1);
        assert!(
            records[0]
                .details
                .as_deref()
                .is_some_and(|details| details.contains("stage=recovery"))
        );
    }

    #[test]
    fn final_update_check_failure_is_written_to_issue_log_without_network_details() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let snapshot = failed_update_snapshot(UpdateFailureCategory::DownloadFailed);

        log_update_check_failure(&store, &snapshot);

        let records = store.list(0, Some(IssueLogLevel::Error), Some("update.check"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event, "update.check");
        assert_eq!(
            records[0].details.as_deref(),
            Some("category=download_failed target_version=1.1.0")
        );
        assert!(
            !records[0]
                .details
                .as_deref()
                .unwrap_or_default()
                .contains("example.invalid")
        );
    }

    #[test]
    fn update_install_failure_is_written_to_issue_log() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let snapshot = failed_update_snapshot(UpdateFailureCategory::CheckFailed);
        let failure = UpdateInstallFailure {
            category: UpdateInstallFailureCategory::LaunchFailed,
            message_id: "update.install_launch_failed",
        };

        log_update_install_failure(&store, &failure, &snapshot);

        let records = store.list(0, Some(IssueLogLevel::Error), Some("update.install"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "update.install_launch_failed");
        assert_eq!(
            records[0].details.as_deref(),
            Some("category=launch_failed target_version=1.1.0")
        );
    }

    #[test]
    fn provider_deletion_failure_is_written_without_sensitive_context() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let failure = DeleteProviderFailure {
            category: "wsl_verification",
            message_id: "wsl.command_failed",
            lifecycle_outcome: None,
            lifecycle_results: Vec::new(),
        };

        let result: Result<(), DeleteProviderFailure> = Err(failure);
        assert!(finish_command(&store, "provider.delete", result).is_err());

        let records = store.list(0, Some(IssueLogLevel::Error), Some("provider.delete"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "wsl.command_failed");
        assert_eq!(
            records[0].details.as_deref(),
            Some("category=wsl_verification")
        );
        let encoded = serde_json::to_string(&records[0]).expect("serialize issue log record");
        assert!(!encoded.contains("api_key"));
        assert!(!encoded.contains("base_url"));
        assert!(!encoded.contains("provider_id"));
    }

    #[test]
    fn wsl_reclaim_audit_records_each_non_sensitive_controlled_rebuild_phase() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());

        for phase in [
            WslReclaimAuditPhase::Revalidated,
            WslReclaimAuditPhase::SafeApplyCheck,
            WslReclaimAuditPhase::RebuildRequired,
            WslReclaimAuditPhase::UserConfirmed,
            WslReclaimAuditPhase::Prepared,
            WslReclaimAuditPhase::ArtifactsReplaced,
            WslReclaimAuditPhase::StateCommitted,
            WslReclaimAuditPhase::Succeeded,
        ] {
            log_wsl_reclaim_phase(&store, phase);
        }

        let records = store.list(0, Some(IssueLogLevel::Info), Some("wsl.reclaim_provider"));
        assert_eq!(records.len(), 8);
        assert_eq!(
            records[0].details.as_deref(),
            Some("phase=revalidated status=succeeded")
        );
        assert_eq!(
            records[7].details.as_deref(),
            Some("phase=result status=succeeded")
        );
        let encoded = serde_json::to_string(&records).expect("serialize reclaim audit");
        assert!(!encoded.contains("provider_id"));
        assert!(!encoded.contains("environment_id"));
        assert!(!encoded.contains("api_key"));
    }

    #[test]
    fn wsl_reclaim_revalidation_audit_requires_context_success_and_no_address_suggestion() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let provider = ProviderSummary {
            id: "provider-id".to_owned(),
            name: "Sensitive Provider".to_owned(),
            base_url: "https://provider.example/v1".to_owned(),
            default_model: "model-a".to_owned(),
            verified_at_epoch_seconds: 1,
            is_current: false,
            recommendation_id: None,
            has_recommendation_update: false,
            recommendation_template_base_url: None,
        };
        let success = Ok(ProviderRevalidationResult {
            provider: provider.clone(),
            validation_receipt: None,
        });
        audit_provider_revalidation_result(&store, None, &success);
        let suggestion = Ok(ProviderRevalidationResult {
            provider,
            validation_receipt: Some(ProviderValidationReceipt {
                validation_id: "validation-id".to_owned(),
                requested_base_url: "https://requested.example/v1".to_owned(),
                normalized_base_url: "https://normalized.example/v1".to_owned(),
                default_model: "model-a".to_owned(),
                combination_fingerprint: "fingerprint".to_owned(),
                verified_at_epoch_seconds: 1,
            }),
        });
        audit_provider_revalidation_result(
            &store,
            Some(ProviderRevalidationAuditContext::WslReclaim),
            &suggestion,
        );
        let failure = Err(ProviderFailure::new(
            ProviderFailureCategory::StateUnavailable,
            "provider.state_unavailable",
        ));
        audit_provider_revalidation_result(
            &store,
            Some(ProviderRevalidationAuditContext::WslReclaim),
            &failure,
        );
        assert!(
            store
                .list(0, Some(IssueLogLevel::Info), Some("wsl.reclaim_provider"))
                .is_empty()
        );

        audit_provider_revalidation_result(
            &store,
            Some(ProviderRevalidationAuditContext::WslReclaim),
            &success,
        );
        let records = store.list(0, Some(IssueLogLevel::Info), Some("wsl.reclaim_provider"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "wsl.reclaim_revalidated");
        assert_eq!(
            records[0].details.as_deref(),
            Some("phase=revalidated status=succeeded")
        );
        let encoded = serde_json::to_string(&records).expect("serialize revalidation audit");
        assert!(!encoded.contains("provider-id"));
        assert!(!encoded.contains("Sensitive Provider"));
        assert!(!encoded.contains("provider.example"));
    }

    #[test]
    fn wsl_inventory_log_distinguishes_safe_states_without_environment_identity() {
        let environment = WslEnvironmentSummary {
            environment_id: "{11111111-1111-1111-1111-111111111111}".to_owned(),
            display_name: "Sensitive Ubuntu Name".to_owned(),
            command_name: Some("Sensitive Ubuntu Name".to_owned()),
            default_uid: Some(1000),
            running: true,
            availability: WslAvailability::Manageable,
            current_provider: None,
            actual_provider_id: None,
            configuration_state: WslConfigurationState::Conflict,
            requires_attention: true,
            pending_restart: false,
            revision: "secret-revision".to_owned(),
            message_id: Some("wsl.schema_unknown".to_owned()),
            reclaim_preview: Some(WslReclaimPreview {
                scope: WslReclaimScope::PreserveUnrelatedToml,
                full_config_backup: true,
                auth_json_unchanged: true,
                temporarily_starts_distribution: false,
            }),
        };

        let details = wsl_inventory_details(2, &[environment]);

        assert!(details.contains("provider_count=2"));
        assert!(details.contains("environment_count=1"));
        assert!(details.contains("unknown_schema_count=1"));
        assert!(!details.contains("Sensitive Ubuntu Name"));
        assert!(!details.contains("11111111"));
        assert!(!details.contains("secret-revision"));
    }

    #[test]
    fn desktop_restart_termination_timeout_log_records_phase_and_counts_without_process_identity() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let failure = DesktopFailure {
            category: DesktopFailureCategory::TerminationTimedOut,
            message_id: "desktop.termination_timed_out",
        };
        let observed = DesktopSnapshot {
            status: ConsumerStatus::Running,
            action: DesktopAction::Restart,
            message_id: "desktop.ready_to_restart",
            roots: vec![ConsumerIdentity {
                role: ConsumerRole::Desktop,
                pid: 42_042,
                started_at_epoch_millis: 8_000,
            }],
        };

        let result = finish_command_with_desktop_restart(&store, Err(failure), 1, Some(&observed));

        assert!(result.is_err());
        let records = store.list(0, Some(IssueLogLevel::Error), Some("desktop.restart"));
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].details.as_deref(),
            Some(
                "category=termination_timed_out phase=termination_observation expected_root_count=1 observed_status=running observed_root_count=1"
            )
        );
        let encoded = serde_json::to_string(&records[0]).expect("serialize issue log record");
        assert!(!encoded.contains("42042"));
        assert!(!encoded.contains("8000"));
    }

    #[test]
    fn desktop_restart_success_log_distinguishes_confirmed_tree_termination() {
        let directory = tempdir().expect("issue log directory");
        let store = IssueLogStore::new(directory.path());
        let snapshot = DesktopSnapshot {
            status: ConsumerStatus::Running,
            action: DesktopAction::Restart,
            message_id: "desktop.restarted_after_termination",
            roots: vec![ConsumerIdentity {
                role: ConsumerRole::Desktop,
                pid: 42_042,
                started_at_epoch_millis: 8_000,
            }],
        };

        let result = finish_command_with_desktop_restart(&store, Ok(snapshot), 1, None);

        assert!(result.is_ok());
        let records = store.list(0, Some(IssueLogLevel::Info), Some("desktop.restart"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "desktop.restarted_after_termination");
        assert_eq!(
            records[0].details.as_deref(),
            Some(
                "phase=completed expected_root_count=1 observed_status=running observed_root_count=1"
            )
        );
        let encoded = serde_json::to_string(&records[0]).expect("serialize issue log record");
        assert!(!encoded.contains("42042"));
        assert!(!encoded.contains("8000"));
    }

    #[test]
    fn every_fallible_tauri_command_is_connected_to_issue_logging() {
        let source = include_str!("commands.rs");
        let manually_logged = [
            "get_startup_snapshot",
            "refresh_startup_snapshot",
            "install_update",
            "enter_session_management",
            "leave_session_management",
            "cancel_session_request",
            "execute_session_visibility",
        ];

        for block in source.split("#[tauri::command]").skip(1) {
            let Some(signature_end) = block.find('{') else {
                continue;
            };
            let signature = &block[..signature_end];
            if !signature.contains("-> Result<") {
                continue;
            }
            let function_name = signature
                .split("fn ")
                .nth(1)
                .and_then(|tail| tail.split(['(', '\n']).next())
                .expect("fallible Tauri command name")
                .trim();
            assert!(
                signature.contains("IssueLogRuntime"),
                "fallible Tauri command {function_name} has no issue log state"
            );
            assert!(
                block.contains("finish_command") || manually_logged.contains(&function_name),
                "fallible Tauri command {function_name} does not record its failure"
            );
        }
    }
}
