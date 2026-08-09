use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::environment::{EnvironmentApplication, EnvironmentFailure, EnvironmentSnapshot};
use crate::provider::{
    DiscoveryInput, ModelDiscovery, ProviderApiKey, ProviderApplication, ProviderFailure,
    ProviderFailureCategory, ProviderSummary, ProviderUpdateDiscoveryInput,
    ProviderUpdateValidationInput, ProviderValidationInput, ProviderValidationReceipt,
    ProviderValidationStage,
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
pub(crate) fn get_environment_snapshot(
    state: State<'_, EnvironmentRuntime>,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    state.application.inspect()
}

#[tauri::command]
pub(crate) fn apply_environment_provider(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    provider_id: String,
    confirm_switch_risk: bool,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let result = state.application.apply_provider_at_revision(
        &provider_id,
        confirm_switch_risk,
        &expected_revision,
    );
    refresh_environment_tray_after(&app, result)
}

#[tauri::command]
pub(crate) fn restore_last_environment_config(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    confirm_restore: bool,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let result = state
        .application
        .restore_last_config(confirm_restore, &expected_revision);
    refresh_environment_tray_after(&app, result)
}

#[tauri::command]
pub(crate) fn switch_to_openai_login(
    app: AppHandle,
    state: State<'_, EnvironmentRuntime>,
    confirm_switch: bool,
    expected_revision: String,
) -> Result<EnvironmentSnapshot, EnvironmentFailure> {
    let result = state
        .application
        .switch_to_openai_login(confirm_switch, &expected_revision);
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
) -> Result<ProviderSummary, ProviderFailure> {
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
pub(crate) fn save_and_apply_provider_update(
    app: AppHandle,
    provider_state: State<'_, ProviderRuntime>,
    environment_state: State<'_, EnvironmentRuntime>,
    validation_id: String,
    provider_id: String,
    name: String,
    confirm_consumer_risk: bool,
) -> Result<ProviderSummary, ProviderFailure> {
    let result = provider_state.application.save_and_apply_provider_update(
        &environment_state.application,
        &validation_id,
        &provider_id,
        &name,
        confirm_consumer_risk,
    );
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
