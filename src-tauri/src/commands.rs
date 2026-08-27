use std::process::Command;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

use crate::desktop::{DesktopApplication, DesktopFailure, DesktopFailureCategory, DesktopSnapshot};
use crate::diagnostics::{IssueLogLevel, IssueLogRecord, IssueLogStore};
use crate::environment::{
    EnvironmentApplication, EnvironmentFailure, EnvironmentFailureCategory, EnvironmentSnapshot,
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
use crate::startup::{StartupCoordinator, StartupSnapshot};
use crate::tray;
use crate::update::{
    UpdateActivityGate, UpdateCoordinator, UpdateFailureCategory, UpdateInstallFailure,
    UpdateInstallFailureCategory, UpdateSnapshot, UpdateState,
};
use crate::wsl::{
    WslApplication, WslApplyResult, WslDeletionAuditError, WslEnvironmentSummary, WslFailure,
    WslLifecycleOutcome, WslLifecycleResult, WslRefreshResult,
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
}

pub(crate) struct IssueLogRuntime {
    pub(crate) store: IssueLogStore,
}

impl IssueLogRuntime {
    pub(crate) fn new(store: IssueLogStore) -> Self {
        Self { store }
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
}

impl SessionRuntime {
    pub(crate) fn new(application: SessionApplication) -> Self {
        Self { application }
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
            Some(format!("category={}", failure.category_name())),
        );
    }
    result
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
    state: State<'_, DesktopRuntime>,
    logs: State<'_, IssueLogRuntime>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || application.start())
        .await
        .map_err(|_| desktop_task_failed())?;
    finish_command(&logs.store, "desktop.start", result)
}

#[tauri::command]
pub(crate) async fn restart_desktop_application(
    state: State<'_, DesktopRuntime>,
    logs: State<'_, IssueLogRuntime>,
    expected_roots: Vec<crate::consumer::ConsumerIdentity>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    let application = state.application.clone();
    let expected_root_count = expected_roots.len();
    let (result, observed_after_failure) = tauri::async_runtime::spawn_blocking(move || {
        let result = application.restart(&expected_roots);
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
    let prefix = format!("{}/v", crate::update::GITCODE_RELEASES_URL);
    let Some(version) = url.strip_prefix(&prefix) else {
        return false;
    };
    let parts = version.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod update_release_notes_tests {
    use super::is_valid_update_release_notes_url;

    #[test]
    fn accepts_the_expected_gitcode_release_url() {
        assert!(is_valid_update_release_notes_url(
            "https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag/v1.1.4"
        ));
    }

    #[test]
    fn rejects_untrusted_or_malformed_release_urls() {
        for url in [
            "https://example.com/releases/tag/v1.1.4",
            "https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag/v1.1",
            "https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag/v1.1.4/notes",
            "https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag/v1.1.4?next=evil",
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
    let result = tauri::async_runtime::spawn_blocking(move || application.list())
        .await
        .map_err(|_| {
            WslFailure::new(
                crate::wsl::WslFailureCategory::StateUnavailable,
                "wsl.state_unavailable",
            )
        })
        .and_then(|result| result);
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
        DeleteProviderFailure, IssueLogLevel, IssueLogStore, UpdateFailureCategory,
        UpdateInstallFailure, UpdateInstallFailureCategory, UpdateSnapshot, UpdateState,
        finish_command, finish_command_with_desktop_restart, log_update_check_failure,
        log_update_install_failure,
    };
    use crate::consumer::{ConsumerIdentity, ConsumerRole, ConsumerStatus};
    use crate::desktop::{DesktopAction, DesktopFailure, DesktopFailureCategory, DesktopSnapshot};
    use tempfile::tempdir;

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
