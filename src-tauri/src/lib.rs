pub mod codex;
mod commands;
pub mod consumer;
pub mod environment;
pub mod provider;
pub mod single_instance;
pub mod startup;
pub mod state;
mod tray;

use codex::{CodexInspector, LoginStatusCommand};
use commands::{
    DesktopRuntime, EnvironmentRuntime, ProviderRuntime, StartupRuntime,
    apply_environment_provider, cancel_provider_request, confirm_provider_validation_base_url,
    copy_provider_api_key, delete_provider, discard_provider_validation, discover_provider_models,
    discover_provider_models_for_update, force_restart_desktop_application, get_desktop_snapshot,
    get_environment_snapshot, get_startup_snapshot, list_providers, open_dayway_website,
    refresh_startup_snapshot, rename_provider, reorder_providers, restart_desktop_application,
    restore_last_environment_config, revalidate_provider, reveal_provider_api_key,
    save_and_apply_provider_update, save_dayway_provider, save_provider_update,
    save_verified_provider, start_desktop_application, switch_to_openai_login, validate_provider,
    validate_provider_update,
};
use consumer::DesktopApplication;
use environment::EnvironmentApplication;
use provider::{ProviderApplication, ProviderValidator, ValidationTimeouts};
use single_instance::{InstanceRole, acquire};
use startup::StartupCoordinator;
use state::{StatePaths, StateStore};
use tauri::Manager;
use tray::LifecycleRuntime;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let executable = std::env::current_exe().expect("GPTEasy executable path is unavailable");
    let primary_instance = match acquire(&executable).expect("GPTEasy single-instance setup failed")
    {
        InstanceRole::Primary(primary) => primary,
        InstanceRole::Secondary => return,
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let state_root = app.path().app_local_data_dir()?;
            let home = app.path().home_dir()?;
            let state_store = StateStore::new(StatePaths::from_root(state_root));
            app.manage(LifecycleRuntime::new(state_store.clone()));
            let codex_home = home.join(".codex");
            let coordinator = StartupCoordinator::new(
                state_store.clone(),
                CodexInspector::new(&codex_home, LoginStatusCommand::codex_default()),
            );
            app.manage(StartupRuntime::new(coordinator));
            let environment = EnvironmentApplication::new(state_store.clone(), codex_home);
            let _ = environment.recover_pending();
            app.manage(EnvironmentRuntime::new(environment));
            app.manage(DesktopRuntime::new(DesktopApplication::new()));
            app.manage(ProviderRuntime::new(ProviderApplication::new(
                state_store,
                ProviderValidator::new(ValidationTimeouts::default()),
            )));
            let activation_handle = app.app_handle().clone();
            app.manage(primary_instance.listen(move || {
                let main_thread_handle = activation_handle.clone();
                let _ = activation_handle.run_on_main_thread(move || {
                    tray::show_settings(&main_thread_handle);
                });
            })?);
            tray::setup(app)?;
            Ok(())
        })
        .on_window_event(tray::handle_window_event)
        .invoke_handler(tauri::generate_handler![
            get_startup_snapshot,
            refresh_startup_snapshot,
            get_environment_snapshot,
            get_desktop_snapshot,
            start_desktop_application,
            restart_desktop_application,
            force_restart_desktop_application,
            apply_environment_provider,
            switch_to_openai_login,
            restore_last_environment_config,
            list_providers,
            discover_provider_models,
            discover_provider_models_for_update,
            validate_provider,
            validate_provider_update,
            revalidate_provider,
            cancel_provider_request,
            confirm_provider_validation_base_url,
            save_verified_provider,
            save_dayway_provider,
            open_dayway_website,
            rename_provider,
            save_provider_update,
            save_and_apply_provider_update,
            delete_provider,
            reorder_providers,
            reveal_provider_api_key,
            copy_provider_api_key,
            discard_provider_validation
        ])
        .run(tauri::generate_context!())
        .expect("GPTEasy runtime failed");
}
