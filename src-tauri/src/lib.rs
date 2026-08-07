pub mod codex;
mod commands;
pub mod startup;
pub mod state;

use codex::{CodexInspector, LoginStatusCommand};
use commands::{StartupRuntime, get_startup_snapshot, refresh_startup_snapshot};
use startup::StartupCoordinator;
use state::{StatePaths, StateStore};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let state_root = app.path().app_local_data_dir()?;
            let home = app.path().home_dir()?;
            let coordinator = StartupCoordinator::new(
                StateStore::new(StatePaths::from_root(state_root)),
                CodexInspector::new(home.join(".codex"), LoginStatusCommand::codex_default()),
            );
            app.manage(StartupRuntime::new(coordinator));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_startup_snapshot,
            refresh_startup_snapshot
        ])
        .run(tauri::generate_context!())
        .expect("GPTEasy runtime failed");
}
