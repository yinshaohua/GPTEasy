use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::provider::{
    DiscoveryInput, ModelDiscovery, ProviderApiKey, ProviderApplication, ProviderFailure,
    ProviderFailureCategory, ProviderSummary, ProviderUpdateDiscoveryInput,
    ProviderUpdateValidationInput, ProviderValidationInput, ProviderValidationReceipt,
    ProviderValidationStage,
};
use crate::startup::{StartupCoordinator, StartupSnapshot};

pub(crate) struct StartupRuntime {
    coordinator: Mutex<StartupCoordinator>,
}

pub(crate) struct ProviderRuntime {
    application: ProviderApplication,
}

impl ProviderRuntime {
    pub(crate) fn new(application: ProviderApplication) -> Self {
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
    state: State<'_, ProviderRuntime>,
    validation_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    state
        .application
        .save_verified_provider(&validation_id, &name)
}

#[tauri::command]
pub(crate) fn rename_provider(
    state: State<'_, ProviderRuntime>,
    provider_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    state.application.rename_provider(&provider_id, &name)
}

#[tauri::command]
pub(crate) fn save_provider_update(
    state: State<'_, ProviderRuntime>,
    validation_id: String,
    provider_id: String,
    name: String,
) -> Result<ProviderSummary, ProviderFailure> {
    state
        .application
        .save_provider_update(&validation_id, &provider_id, &name)
}

#[tauri::command]
pub(crate) fn delete_provider(
    state: State<'_, ProviderRuntime>,
    provider_id: String,
) -> Result<(), ProviderFailure> {
    state.application.delete_provider(&provider_id)
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
