mod appserver;
mod core;
mod validation;

pub use core::{
    run_live_pipeline, run_matrix, scan_codex_processes, write_live_summary, Injection,
    PipelineReport, ProcessScan,
};
pub use validation::{load_secret, mock_verified_provider};

use crate::core::{create_scenario, run_pipeline, ProcessState};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State, WindowEvent,
};

static EXPLICIT_EXIT: AtomicBool = AtomicBool::new(false);

struct UiState {
    latest_report: Arc<Mutex<Option<PathBuf>>>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            latest_report: Arc::new(Mutex::new(None)),
        }
    }
}

#[tauri::command]
fn scan_processes() -> ProcessScan {
    scan_codex_processes()
}

#[tauri::command]
async fn run_demo(
    decision: String,
    provider_source: String,
    injection: String,
    state: State<'_, UiState>,
) -> Result<PipelineReport, String> {
    let root = std::env::current_dir()
        .map_err(|error| error.to_string())?
        .join(".run/ui")
        .join(format!(
            "session-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos()
        ));
    let latest_report = state.latest_report.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<PipelineReport, String> {
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let paths = create_scenario(&root, "interactive").map_err(|error| error.to_string())?;
        let scan = scan_codex_processes();
        let processes = ProcessState {
            desktop: scan.counts.get("desktop_root").copied().unwrap_or(0) > 0,
            cli: scan.counts.get("cli").copied().unwrap_or(0) > 0,
        };
        let parsed_injection = Injection::parse(&injection).map_err(|error| error.to_string())?;
        let verified = match provider_source.as_str() {
            "mock" => mock_verified_provider(),
            "live" => {
                let secret_path =
                    find_project_secret(&std::env::current_dir().map_err(|e| e.to_string())?)
                        .ok_or_else(|| {
                            "未找到 .codex/skills/spike-findings-gpteasy/.secrets/provider.json".to_string()
                        })?;
                let input = load_secret(&secret_path).map_err(|error| error.to_string())?;
                validation::validate_live(input).map_err(|error| error.to_string())?
            }
            other => return Err(format!("unsupported provider source {other}")),
        };
        let mut report = run_pipeline(
            &paths,
            &decision,
            Some(&verified),
            processes,
            parsed_injection,
        )
        .map_err(|error| error.to_string())?;
        report.process_scan = Some(scan);
        let report_path = root.join("interactive-report.json");
        let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
        if bytes
            .windows(verified.input.api_key.len())
            .any(|window| window == verified.input.api_key.as_bytes())
        {
            return Err("API Key leaked into interactive report".to_string());
        }
        fs::write(&report_path, bytes).map_err(|error| error.to_string())?;
        *latest_report
            .lock()
            .map_err(|error| error.to_string())? = Some(report_path);
        Ok(report)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn export_latest_report(state: State<'_, UiState>) -> Result<serde_json::Value, String> {
    let path = state
        .latest_report
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "尚未生成报告".to_string())?;
    Ok(json!({
        "path": path,
        "bytes": fs::metadata(&path).map_err(|error| error.to_string())?.len()
    }))
}

fn find_project_secret(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let candidate = ancestor.join(".codex/skills/spike-findings-gpteasy/.secrets/provider.json");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示端到端实验", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "明确退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                EXPLICIT_EXIT.store(true, Ordering::SeqCst);
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .manage(UiState::default())
        .invoke_handler(tauri::generate_handler![
            scan_processes,
            run_demo,
            export_latest_report
        ])
        .setup(|app| {
            setup_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if !EXPLICIT_EXIT.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build desktop provider switch spike")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !EXPLICIT_EXIT.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::validation::{combination_fingerprint, mock_verified_provider};

    #[test]
    fn validated_combination_is_bound_to_key() {
        let verified = mock_verified_provider();
        let original = combination_fingerprint(&verified.input);
        let mut changed = verified.input.clone();
        changed.api_key.push_str("-changed");
        assert_ne!(original, combination_fingerprint(&changed));
    }
}
