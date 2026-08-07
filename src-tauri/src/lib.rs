use std::ffi::OsString;

use tauri::{Manager, Runtime};

pub mod commands;
pub mod domain;
pub mod path_smoke;
pub mod state;
pub mod state_smoke;

const STATE_SMOKE_COMMAND: &str = "phase1-state-smoke";

enum CliAction {
    Desktop,
    PathSmoke(String),
    StateSmoke {
        mode: state_smoke::StateSmokeMode,
        run_id: String,
    },
}

pub fn configure_builder<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .setup(|app| {
            let state_root = app.path().app_local_data_dir()?;
            let store = state::StateStore::open(&state_root)?;
            app.manage(store);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::update_app_settings,
            commands::bootstrap_state,
            commands::replace_state_snapshot,
            commands::bootstrap_state_snapshot
        ])
}

fn run_desktop() {
    configure_builder(tauri::Builder::default())
        .run(tauri::generate_context!())
        .expect("failed to run GPTEasy");
}

fn run_phase1_path_smoke(run_id: String) -> Result<(), &'static str> {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .map_err(|_| "failed to initialize GPTEasy for phase1-path-smoke")?;
    let report = path_smoke::run_path_smoke(app.handle(), &run_id)
        .map_err(|_| "phase1-path-smoke failed")?;
    let json = serde_json::to_string(&report)
        .map_err(|_| "failed to serialize phase1-path-smoke report")?;

    println!("{json}");
    app.cleanup_before_exit();
    Ok(())
}

fn run_phase1_state_smoke(
    mode: state_smoke::StateSmokeMode,
    run_id: String,
) -> Result<(), &'static str> {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .map_err(|_| "failed to initialize GPTEasy for phase1-state-smoke")?;
    let report = state_smoke::run_state_smoke(app.handle(), mode, &run_id)
        .map_err(|_| "phase1-state-smoke failed")?;
    let json = serde_json::to_string(&report)
        .map_err(|_| "failed to serialize phase1-state-smoke report")?;

    println!("{json}");
    app.cleanup_before_exit();
    Ok(())
}

fn exit_cli_error(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

fn parse_cli_action<I>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(CliAction::Desktop),
        [command, run_id] if command == path_smoke::PATH_SMOKE_COMMAND => {
            path_smoke::parse_cli_args(args)
                .map_err(|error| error.to_string())?
                .map(CliAction::PathSmoke)
                .ok_or_else(|| "phase1-path-smoke requires an opaque run ID".to_owned())
        }
        [command, mode, run_id] if command == STATE_SMOKE_COMMAND => {
            let mode = mode
                .to_str()
                .ok_or_else(|| "phase1-state-smoke arguments must be UTF-8".to_owned())?;
            let run_id = run_id
                .to_str()
                .ok_or_else(|| "phase1-state-smoke arguments must be UTF-8".to_owned())?;
            let mode =
                state_smoke::StateSmokeMode::parse(mode).map_err(|error| error.to_string())?;
            state_smoke::validate_run_id(run_id).map_err(|error| error.to_string())?;
            Ok(CliAction::StateSmoke {
                mode,
                run_id: run_id.to_owned(),
            })
        }
        _ => Err("invalid GPTEasy command arguments".to_owned()),
    }
}

fn run_with_args<I>(args: I)
where
    I: IntoIterator<Item = OsString>,
{
    match parse_cli_action(args) {
        Ok(CliAction::Desktop) => run_desktop(),
        Ok(CliAction::PathSmoke(run_id)) => {
            if let Err(message) = run_phase1_path_smoke(run_id) {
                exit_cli_error(message);
            }
        }
        Ok(CliAction::StateSmoke { mode, run_id }) => {
            if let Err(message) = run_phase1_state_smoke(mode, run_id) {
                exit_cli_error(message);
            }
        }
        Err(error) => exit_cli_error(&error),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    run_with_args(std::env::args_os().skip(1));
}
