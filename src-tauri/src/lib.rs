pub mod codex;
mod commands;
pub mod consumer;
pub mod diagnostic_assistant;
pub mod diagnostic_report;
pub mod diagnostics;
pub mod environment;
pub mod provider;
pub mod session;
#[cfg(windows)]
pub mod single_instance;
pub mod startup;
pub mod state;
mod tray;
pub mod update;
pub mod wsl;

use codex::{CodexInspector, LoginStatusCommand};
use commands::{
    EnvironmentRuntime, IssueLogRuntime, ProviderRuntime, SessionRuntime, StartupRuntime,
    UpdateRuntime, WslRuntime, apply_environment_provider, apply_wsl_provider, archive_sessions,
    cancel_provider_request, cancel_session_request, check_for_updates,
    choose_issue_log_export_destination, choose_linux_export_destination,
    choose_session_export_destination, confirm_provider_validation_base_url, copy_issue_logs,
    copy_provider_api_key, delete_provider, delete_session, discard_provider_validation,
    discover_provider_models, discover_provider_models_for_update, enter_session_management,
    export_all_issue_logs, export_issue_logs, export_linux_script, export_session_markdown,
    get_environment_snapshot, get_issue_log_path, get_startup_snapshot, get_update_snapshot,
    install_update, leave_session_management, list_issue_logs, list_providers, list_sessions,
    list_wsl_environments, open_dayway_website, open_update_manual_download,
    open_update_release_notes, perform_update_check, read_session, refresh_startup_snapshot,
    refresh_wsl_environment, rename_provider, reorder_providers, restore_last_environment_config,
    revalidate_provider, reveal_provider_api_key, save_and_apply_provider_update,
    save_dayway_provider, save_provider_update, save_verified_provider, switch_to_openai_login,
    unarchive_sessions, validate_provider, validate_provider_update,
};
use diagnostic_report::{
    DiagnosticApplication, DiagnosticRuntime, analyze_diagnostic_report,
    choose_diagnostic_export_destination, export_diagnostic_report, get_diagnostic_report,
    repair_diagnostic_custom_provider,
};
use diagnostics::IssueLogStore;
use environment::EnvironmentApplication;
use provider::{ProviderApplication, ProviderValidator, ValidationTimeouts};
use session::SessionApplication;
#[cfg(windows)]
use single_instance::{InstanceRole, acquire};
use startup::StartupCoordinator;
use state::{StatePaths, StateStore};
use tauri::Manager;
use tray::LifecycleRuntime;
use update::UpdateCoordinator;
use wsl::WslApplication;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(windows)]
    let executable = std::env::current_exe().expect("GPTEasy executable path is unavailable");
    #[cfg(windows)]
    let primary_instance = match acquire(&executable).expect("GPTEasy single-instance setup failed")
    {
        InstanceRole::Primary(primary) => primary,
        InstanceRole::Secondary => return,
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let state_root = app.path().app_local_data_dir()?;
            let home = app.path().home_dir()?;
            let state_store = StateStore::new(StatePaths::from_root(state_root));
            app.manage(IssueLogRuntime::new(IssueLogStore::new(
                state_store.paths().root(),
            )));
            app.manage(LifecycleRuntime::new(state_store.clone()));
            app.manage(UpdateRuntime::new(UpdateCoordinator::with_state_path(
                env!("CARGO_PKG_VERSION"),
                state_store
                    .paths()
                    .root()
                    .join("update-install-attempt.json"),
            )));
            let codex_home = home.join(".codex");
            let environment = EnvironmentApplication::new(state_store.clone(), &codex_home);
            app.manage(DiagnosticRuntime::new(
                DiagnosticApplication::with_environment(
                    &codex_home,
                    std::env::var_os("CODEX_HOME").map(std::path::PathBuf::from),
                    environment.clone(),
                ),
            ));
            let coordinator = StartupCoordinator::new(
                state_store.clone(),
                CodexInspector::new(&codex_home, LoginStatusCommand::codex_default()),
            );
            app.manage(StartupRuntime::new(coordinator));
            let _ = environment.recover_pending();
            app.manage(EnvironmentRuntime::new(environment));
            let wsl = WslApplication::new(state_store.clone());
            let _ = wsl.recover_pending();
            app.manage(WslRuntime::new(wsl));
            app.manage(SessionRuntime::new(SessionApplication::new(
                state_store.clone(),
            )));
            app.manage(ProviderRuntime::new(ProviderApplication::new(
                state_store,
                ProviderValidator::new(ValidationTimeouts::default()),
            )));
            #[cfg(windows)]
            {
                // Windows toast activation re-enters the single-instance listener,
                // which only reveals settings and never starts installation.
                let activation_handle = app.app_handle().clone();
                app.manage(primary_instance.listen(move || {
                    let main_thread_handle = activation_handle.clone();
                    let _ = activation_handle.run_on_main_thread(move || {
                        tray::show_settings(&main_thread_handle);
                    });
                })?);
            }
            tray::setup(app)?;
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            start_update_monitor(app.app_handle().clone());
            Ok(())
        })
        .on_window_event(tray::handle_window_event)
        .invoke_handler(tauri::generate_handler![
            get_startup_snapshot,
            refresh_startup_snapshot,
            get_update_snapshot,
            check_for_updates,
            install_update,
            open_update_manual_download,
            open_update_release_notes,
            get_environment_snapshot,
            get_diagnostic_report,
            analyze_diagnostic_report,
            repair_diagnostic_custom_provider,
            choose_diagnostic_export_destination,
            export_diagnostic_report,
            get_issue_log_path,
            list_issue_logs,
            copy_issue_logs,
            choose_issue_log_export_destination,
            export_issue_logs,
            export_all_issue_logs,
            enter_session_management,
            leave_session_management,
            list_sessions,
            cancel_session_request,
            read_session,
            archive_sessions,
            unarchive_sessions,
            delete_session,
            choose_session_export_destination,
            export_session_markdown,
            list_wsl_environments,
            refresh_wsl_environment,
            apply_wsl_provider,
            apply_environment_provider,
            switch_to_openai_login,
            restore_last_environment_config,
            list_providers,
            choose_linux_export_destination,
            export_linux_script,
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

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn start_update_monitor(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut schedule = update::UpdateCheckSchedule::default();
        loop {
            let snapshot = perform_update_check(&app, false).await;
            tokio::time::sleep(schedule.next_delay(&snapshot)).await;
        }
    });
}
