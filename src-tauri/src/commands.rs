use std::process::Command;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::consumer::{DesktopApplication, DesktopFailure, DesktopSnapshot};
use crate::environment::{
    EnvironmentApplication, EnvironmentFailure, EnvironmentFailureCategory, EnvironmentSnapshot,
};
use crate::provider::{
    DAYWAY_WEBSITE, DiscoveryInput, ModelDiscovery, ProviderApiKey, ProviderApplication,
    ProviderFailure, ProviderFailureCategory, ProviderRevalidationResult, ProviderSummary,
    ProviderUpdateDiscoveryInput, ProviderUpdateValidationInput, ProviderValidationInput,
    ProviderValidationReceipt, ProviderValidationStage,
};
use crate::startup::{StartupCoordinator, StartupSnapshot};
use crate::tray;

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

impl ProviderRuntime {
    pub(crate) fn new(application: ProviderApplication) -> Self {
        Self { application }
    }

    pub(crate) fn list(&self) -> Result<Vec<ProviderSummary>, ProviderFailure> {
        self.application.list_providers()
    }
}

impl EnvironmentRuntime {
    pub(crate) fn new(application: EnvironmentApplication) -> Self {
        Self { application }
    }

    pub(crate) fn inspect(&self) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        self.application.inspect()
    }

    pub(crate) fn switch_provider(
        &self,
        provider_id: &str,
        expected_revision: &str,
    ) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
        self.application
            .apply_provider_at_revision(provider_id, true, expected_revision)
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
pub(crate) async fn get_environment_snapshot(
    state: State<'_, EnvironmentRuntime>,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let application = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || application.inspect())
        .await
        .map_err(|_| environment_task_failed())?
}

#[tauri::command]
pub(crate) async fn get_desktop_snapshot(
    state: State<'_, DesktopRuntime>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    let application = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || application.inspect())
        .await
        .map_err(|_| DesktopFailure {
            category: crate::consumer::DesktopFailureCategory::ActionUnavailable,
            message_id: "desktop.state_unavailable",
        })
}

#[tauri::command]
pub(crate) async fn start_desktop_application(
    state: State<'_, DesktopRuntime>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    let application = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || application.start())
        .await
        .map_err(|_| DesktopFailure {
            category: crate::consumer::DesktopFailureCategory::ActionUnavailable,
            message_id: "desktop.state_unavailable",
        })?
}

#[tauri::command]
pub(crate) async fn apply_environment_provider(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    provider_id: String,
    confirm_switch_risk: bool,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.apply_provider_at_revision(
            &provider_id,
            confirm_switch_risk,
            &expected_revision,
        )
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
    confirm_switch: bool,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let application = state.application.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        application.switch_to_openai_login(confirm_switch, &expected_revision)
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
    confirm_consumer_risk: bool,
) -> Result<ProviderSummary, ProviderFailure> {
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let provider_state = task_app.state::<ProviderRuntime>();
        let environment_state = task_app.state::<EnvironmentRuntime>();
        provider_state.application.save_and_apply_provider_update(
            &environment_state.application,
            &validation_id,
            &provider_id,
            &name,
            confirm_consumer_risk,
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
            Ok(applied.provider)
        }
        Err(failure) => Err(failure),
    }
}

#[tauri::command]
pub(crate) fn delete_provider(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    provider_id: String,
) -> Result<(), ProviderFailure> {
    let result = state.application.delete_provider(&provider_id);
    refresh_tray_after(&app, result)
}

#[tauri::command]
pub(crate) fn reorder_providers(
    app: AppHandle,
    state: State<'_, ProviderRuntime>,
    provider_ids: Vec<String>,
) -> Result<Vec<ProviderSummary>, ProviderFailure> {
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
