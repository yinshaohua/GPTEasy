pub mod codex;
mod commands;
pub mod provider;
pub mod startup;
pub mod state;

use codex::{CodexInspector, LoginStatusCommand};
use commands::{
    ProviderRuntime, StartupRuntime, cancel_provider_request, discard_provider_validation,
    discover_provider_models, get_startup_snapshot, list_providers, refresh_startup_snapshot,
    save_verified_provider, validate_provider,
};
use provider::{ProviderApplication, ProviderValidator, ValidationTimeouts};
use startup::StartupCoordinator;
use state::{StatePaths, StateStore};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let state_root = app.path().app_local_data_dir()?;
            let home = app.path().home_dir()?;
            let state_store = StateStore::new(StatePaths::from_root(state_root));
            let coordinator = StartupCoordinator::new(
                state_store.clone(),
                CodexInspector::new(home.join(".codex"), LoginStatusCommand::codex_default()),
            );
            app.manage(StartupRuntime::new(coordinator));
            app.manage(ProviderRuntime::new(ProviderApplication::new(
                state_store,
                ProviderValidator::new(ValidationTimeouts::default()),
            )));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_startup_snapshot,
            refresh_startup_snapshot,
            list_providers,
            discover_provider_models,
            validate_provider,
            cancel_provider_request,
            save_verified_provider,
            discard_provider_validation
        ])
        .run(tauri::generate_context!())
        .expect("GPTEasy runtime failed");
}
