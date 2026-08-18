use std::process::Command;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

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
    UpdateActivityGate, UpdateCoordinator, UpdateInstallFailure, UpdateInstallFailureCategory,
    UpdateSnapshot, UpdateState,
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

pub(crate) struct WslRuntime {
    application: WslApplication,
}

pub(crate) struct SessionRuntime {
    application: SessionApplication,
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
        let hidden = app
            .get_webview_window("main")
            .and_then(|window| window.is_visible().ok())
            .map(|visible| !visible)
            .unwrap_or(true);
        if !hidden {
            return;
        }
        let Ok(mut notified) = self.notified_version.lock() else {
            return;
        };
        if notified.as_deref() == Some(version) {
            return;
        }
        if app
            .notification()
            .builder()
            .title("GPTEasy 有待安装更新")
            .body(format!(
                "版本 {version} 已下载并通过签名验证。打开设置查看。"
            ))
            .extra("open_settings", true)
            .show()
            .is_ok()
        {
            *notified = Some(version.to_owned());
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

#[tauri::command]
pub(crate) fn get_startup_snapshot(
    state: State<'_, StartupRuntime>,
) -> Result<StartupSnapshot, CommandFailure> {
    state.inspect()
}

#[tauri::command]
pub(crate) fn get_update_snapshot(state: State<'_, UpdateRuntime>) -> UpdateSnapshot {
    state.coordinator.snapshot()
}

#[tauri::command]
pub(crate) fn install_update(
    app: AppHandle,
    state: State<'_, UpdateRuntime>,
) -> Result<UpdateSnapshot, UpdateInstallFailure> {
    let Some(_guard) = state.activity.try_begin_install() else {
        return Err(UpdateInstallFailure {
            category: UpdateInstallFailureCategory::Busy,
            message_id: "update.busy",
        });
    };
    let snapshot = state.coordinator.confirm_install()?;
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        let _ = app.emit("update-install-started", snapshot.clone());
        app.exit(0);
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    let _ = app;
    Ok(snapshot)
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub(crate) async fn perform_update_check(app: &AppHandle) -> UpdateSnapshot {
    let runtime = app.state::<UpdateRuntime>();
    let coordinator = runtime.coordinator.clone();
    let event_app = app.clone();
    let snapshot = coordinator
        .check_and_download(move |snapshot| {
            let _ = event_app.emit("update-progress", snapshot);
        })
        .await;
    runtime.notify_if_hidden(app, &snapshot);
    let _ = app.emit("update-progress", snapshot.clone());
    snapshot
}

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
pub(crate) async fn perform_update_check(app: &AppHandle) -> UpdateSnapshot {
    app.state::<UpdateRuntime>().coordinator.snapshot()
}

#[tauri::command]
pub(crate) async fn check_for_updates(app: AppHandle) -> UpdateSnapshot {
    perform_update_check(&app).await
}

#[tauri::command]
pub(crate) fn open_update_manual_download() -> Result<(), CommandFailure> {
    use std::process::Command;
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe")
        .arg(crate::update::MANUAL_DOWNLOAD_URL)
        .spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open")
        .arg(crate::update::MANUAL_DOWNLOAD_URL)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open")
        .arg(crate::update::MANUAL_DOWNLOAD_URL)
        .spawn();
    result.map(|_| ()).map_err(|_| CommandFailure {
        message_id: "update.manual_download_failed",
    })
}

#[tauri::command]
pub(crate) fn refresh_startup_snapshot(
    state: State<'_, StartupRuntime>,
) -> Result<StartupSnapshot, CommandFailure> {
    state.inspect()
}

#[tauri::command]
pub(crate) fn list_providers(
    state: State<'_, ProviderRuntime>,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    state.application.list_providers()
}

#[tauri::command]
pub(crate) async fn enter_session_management(
    state: State<'_, SessionRuntime>,
    lease_id: String,
) -> Result<SessionAvailability, CommandFailure> {
    Ok(state.application.enter(&lease_id).await)
}

#[tauri::command]
pub(crate) async fn leave_session_management(
    state: State<'_, SessionRuntime>,
    lease_id: String,
) -> Result<(), CommandFailure> {
    state.application.leave(&lease_id).await;
    Ok(())
}

#[tauri::command]
pub(crate) async fn list_sessions(
    state: State<'_, SessionRuntime>,
    query: SessionQuery,
) -> Result<SessionListPage, SessionFailure> {
    state.application.list(query).await
}

#[tauri::command]
pub(crate) async fn cancel_session_request(
    state: State<'_, SessionRuntime>,
    request_id: String,
) -> Result<bool, CommandFailure> {
    Ok(state.application.cancel_list_request(&request_id).await)
}

#[tauri::command]
pub(crate) async fn read_session(
    state: State<'_, SessionRuntime>,
    session_id: String,
) -> Result<SessionDetail, SessionFailure> {
    state.application.read(&session_id).await
}

#[tauri::command]
pub(crate) async fn archive_sessions(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    session_ids: Vec<String>,
) -> Result<Vec<SessionMutationResult>, CommandFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("会话修改");
    Ok(state.application.archive(session_ids).await)
}

#[tauri::command]
pub(crate) async fn unarchive_sessions(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    session_ids: Vec<String>,
) -> Result<Vec<SessionMutationResult>, CommandFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("会话修改");
    Ok(state.application.unarchive(session_ids).await)
}

#[tauri::command]
pub(crate) async fn delete_session(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    session_id: String,
) -> Result<SessionMutationResult, CommandFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("会话修改");
    Ok(state.application.delete(&session_id).await)
}

#[tauri::command]
pub(crate) fn choose_session_export_destination(
    app: AppHandle,
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
    selected
        .into_path()
        .map(|path| Some(path.to_string_lossy().into_owned()))
        .map_err(|_| CommandFailure {
            message_id: "session.export_destination_invalid",
        })
}

#[tauri::command]
pub(crate) async fn export_session_markdown(
    app: AppHandle,
    state: State<'_, SessionRuntime>,
    detail: SessionDetail,
    destination: String,
) -> Result<(), SessionFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("会话导出");
    state
        .application
        .export_markdown(&detail, std::path::Path::new(&destination))
        .await
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
    let path = selected.into_path().map_err(|_| LinuxExportFailure {
        category: crate::provider::LinuxExportFailureCategory::UnsafeDestination,
        message_id: "linux_export.unsafe_destination",
    })?;
    Ok(Some(LinuxExportDestination {
        exists: path.exists(),
        path: path.to_string_lossy().into_owned(),
    }))
}

#[tauri::command]
pub(crate) fn export_linux_script(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    shell: LinuxShell,
    destination: String,
    confirm_overwrite: bool,
) -> Result<LinuxExportResult, LinuxExportFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("Linux 导出");
    state.application.export_linux_script(
        shell,
        std::path::Path::new(&destination),
        confirm_overwrite,
    )
}

#[tauri::command]
pub(crate) async fn get_environment_snapshot(
    state: State<'_, EnvironmentRuntime>,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let application = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || application.inspect())
        .await
        .map_err(|_| environment_task_failed())?
}

#[tauri::command]
pub(crate) async fn list_wsl_environments(
    state: State<'_, WslRuntime>,
) -> Result<Vec<WslEnvironmentSummary>, WslFailure> {
    let application = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || application.list())
        .await
        .map_err(|_| {
            WslFailure::new(
                crate::wsl::WslFailureCategory::StateUnavailable,
                "wsl.state_unavailable",
            )
        })?
}

#[tauri::command]
pub(crate) async fn apply_wsl_provider(
    app: AppHandle,
    state: State<'_, WslRuntime>,
    environment_id: String,
    provider_id: String,
    expected_revision: String,
    confirm: bool,
) -> Result<WslApplyResult, WslFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("WSL2 应用");
    let application = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        application.apply_provider(&environment_id, &provider_id, &expected_revision, confirm)
    })
    .await
    .map_err(|_| {
        WslFailure::new(
            crate::wsl::WslFailureCategory::StateUnavailable,
            "wsl.state_unavailable",
        )
    })?
}

#[tauri::command]
pub(crate) async fn refresh_wsl_environment(
    app: AppHandle,
    state: State<'_, WslRuntime>,
    environment_id: String,
    expected_revision: String,
    authorize_start: bool,
) -> Result<WslRefreshResult, WslFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("WSL2 环境协调");
    let application = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        application.refresh_environment(&environment_id, &expected_revision, authorize_start)
    })
    .await
    .map_err(|_| {
        WslFailure::new(
            crate::wsl::WslFailureCategory::StateUnavailable,
            "wsl.state_unavailable",
        )
    })?
}

#[tauri::command]
pub(crate) async fn apply_environment_provider(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    provider_id: String,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("配置写入");
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.apply_provider_at_revision(&provider_id, true, &expected_revision)
    })
    .await
    .map_err(|_| environment_task_failed())?;
    refresh_environment_tray_after(&app, result)
}

#[tauri::command]
pub(crate) async fn restore_last_environment_config(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    confirm_restore: bool,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("配置恢复");
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.restore_last_config(confirm_restore, &expected_revision)
    })
    .await
    .map_err(|_| environment_task_failed())?;
    refresh_environment_tray_after(&app, result)
}

#[tauri::command]
pub(crate) async fn switch_to_openai_login(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("配置写入");
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.switch_to_openai_login(true, &expected_revision)
    })
    .await
    .map_err(|_| environment_task_failed())?;
    refresh_environment_tray_after(&app, result)
}

#[tauri::command]
pub(crate) async fn discover_provider_models(
    state: State<'_, ProviderRuntime>,
    request_id: String,
    input: DiscoveryInput,
) -> Result<ModelDiscovery, ProviderFailure> {
    state.application.discover_models(request_id, input).await
}

#[tauri::command]
pub(crate) async fn discover_provider_models_for_update(
    state: State<'_, ProviderRuntime>,
    request_id: String,
    input: ProviderUpdateDiscoveryInput,
) -> Result<ModelDiscovery, ProviderFailure> {
    state
        .application
        .discover_models_for_update(request_id, input)
        .await
}

#[tauri::command]
pub(crate) async fn validate_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    request_id: String,
    input: ProviderValidationInput,
) -> Result<ProviderValidationReceipt, ProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商验证");
    let progress_request_id = request_id.clone();
    state
        .application
        .validate_provider_with_progress(request_id, input, move |stage| {
            let _ = app.emit(
                "provider-validation-progress",
                ProviderValidationProgress {
                    request_id: progress_request_id.clone(),
                    stage,
                },
            );
        })
        .await
}

#[tauri::command]
pub(crate) async fn validate_provider_update(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    request_id: String,
    input: ProviderUpdateValidationInput,
) -> Result<ProviderValidationReceipt, ProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商验证");
    let progress_request_id = request_id.clone();
    state
        .application
        .validate_provider_update_with_progress(request_id, input, move |stage| {
            let _ = app.emit(
                "provider-validation-progress",
                ProviderValidationProgress {
                    request_id: progress_request_id.clone(),
                    stage,
                },
            );
        })
        .await
}

#[tauri::command]
pub(crate) async fn revalidate_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    request_id: String,
    provider_id: String,
) -> Result<ProviderRevalidationResult, ProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商验证");
    let progress_request_id = request_id.clone();
    state
        .application
        .revalidate_provider_with_progress(request_id, provider_id, move |stage| {
            let _ = app.emit(
                "provider-validation-progress",
                ProviderValidationProgress {
                    request_id: progress_request_id.clone(),
                    stage,
                },
            );
        })
        .await
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
    validation_id: String,
    base_url: String,
) -> Result<(), ProviderFailure> {
    state
        .application
        .confirm_validation_base_url(&validation_id, &base_url)
}

#[tauri::command]
pub(crate) fn save_verified_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    validation_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入");
    let result = state
        .application
        .save_verified_provider(&validation_id, &name);
    refresh_tray_after(&app, result)
}

#[tauri::command]
pub(crate) fn save_dayway_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    validation_id: String,
    confirm_name_conflict: bool,
) -> Result<ProviderSummary, ProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入");
    let result = state
        .application
        .save_dayway_provider_with_name_conflict_confirmation(
            &validation_id,
            confirm_name_conflict,
        );
    refresh_tray_after(&app, result)
}

#[tauri::command]
pub(crate) fn open_dayway_website() -> Result<(), ProviderFailure> {
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe").arg(DAYWAY_WEBSITE).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(DAYWAY_WEBSITE).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(DAYWAY_WEBSITE).spawn();
    result.map(|_| ()).map_err(|_| {
        ProviderFailure::new(
            ProviderFailureCategory::StateUnavailable,
            "provider.website_open_failed",
        )
    })
}

#[tauri::command]
pub(crate) fn rename_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    provider_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入");
    let result = state.application.rename_provider(&provider_id, &name);
    refresh_tray_after(&app, result)
}

#[tauri::command]
pub(crate) fn save_provider_update(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    validation_id: String,
    provider_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入");
    let result = state
        .application
        .save_provider_update(&validation_id, &provider_id, &name);
    refresh_tray_after(&app, result)
}

#[tauri::command]
pub(crate) async fn save_and_apply_provider_update(
    app: AppHandle,
    validation_id: String,
    provider_id: String,
    name: String,
) -> Result<AppliedProviderUpdate, ProviderFailure> {
    let _activity = app.state::<UpdateRuntime>().activity.try_begin("配置写入");
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
    })?;
    match result {
        Ok(applied) => {
            let _ = tray::refresh_with_snapshot(&app, &applied.environment);
            Ok(applied)
        }
        Err(failure) => Err(failure),
    }
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
    provider_id: String,
    authorize_stopped_wsl: bool,
) -> Result<DeleteProviderResult, DeleteProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入");
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
    })?;
    let result = match result {
        Ok((audit, ())) => Ok(DeleteProviderResult {
            lifecycle_results: audit.lifecycle_results,
        }),
        Err(WslDeletionAuditError::Verification(failure)) => Err(DeleteProviderFailure {
            category: "wsl_verification",
            message_id: failure.message_id,
            lifecycle_outcome: failure.lifecycle_outcome,
            lifecycle_results: Vec::new(),
        }),
        Err(WslDeletionAuditError::Deletion {
            failure,
            lifecycle_results,
        }) => Err(DeleteProviderFailure {
            category: "provider",
            message_id: failure.message_id,
            lifecycle_outcome: None,
            lifecycle_results,
        }),
    };
    refresh_tray_after(&app, result)
}

#[tauri::command]
pub(crate) fn reorder_providers(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    provider_ids: Vec<String>,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
    let _activity = app
        .state::<UpdateRuntime>()
        .activity
        .try_begin("供应商目录写入");
    let result = state.application.reorder_providers(&provider_ids);
    refresh_tray_after(&app, result)
}

#[tauri::command]
pub(crate) fn reveal_provider_api_key(
    state: State<'_, ProviderRuntime>,
    provider_id: String,
) -> Result<ProviderApiKey, ProviderFailure> {
    state.application.reveal_provider_api_key(&provider_id)
}

#[tauri::command]
pub(crate) fn copy_provider_api_key(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    provider_id: String,
) -> Result<(), ProviderFailure> {
    let api_key = state.application.reveal_provider_api_key(&provider_id)?;
    app.clipboard().write_text(api_key.expose()).map_err(|_| {
        ProviderFailure::new(
            ProviderFailureCategory::ClipboardUnavailable,
            "provider.clipboard_unavailable",
        )
    })
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
