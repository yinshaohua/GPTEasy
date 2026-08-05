use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
#[cfg(target_os = "macos")]
use std::process::Command;
use sysinfo::{ProcessesToUpdate, System};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, State, WindowEvent,
};

static EXPLICIT_EXIT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractEvent {
    pub at: String,
    pub category: String,
    pub detail: String,
}

#[derive(Default)]
struct EventLog(Mutex<Vec<ContractEvent>>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleEvidence {
    pub name: String,
    pub path: String,
    pub bundle_id: Option<String>,
    pub location: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessEvidence {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub executable: Option<String>,
    pub role: String,
    pub relaunch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractSnapshot {
    pub captured_at: String,
    pub os: String,
    pub arch: String,
    pub home: String,
    pub codex_config_path: String,
    pub codex_config_exists: bool,
    pub user_applications_path: String,
    pub user_applications_writable: bool,
    pub current_executable: Option<String>,
    pub current_app_scope: String,
    pub app_bundles: Vec<BundleEvidence>,
    pub processes: Vec<ProcessEvidence>,
    pub relaunch_candidates: Vec<String>,
    pub evidence_level: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatrixCase {
    pub name: String,
    pub expected_scope: String,
    pub actual_scope: String,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatrixSummary {
    pub total: usize,
    pub passed: usize,
    pub cases: Vec<MatrixCase>,
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn record(log: &EventLog, category: &str, detail: impl Into<String>) {
    if let Ok(mut events) = log.0.lock() {
        events.push(ContractEvent {
            at: now(),
            category: category.to_string(),
            detail: detail.into(),
        });
    }
}

fn current_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn classify_install_scope(executable: Option<&Path>, home: &Path) -> String {
    let Some(executable) = executable else {
        return "unknown".to_string();
    };
    let user_apps = home.join("Applications");
    if executable.starts_with(&user_apps) {
        return "current_user".to_string();
    }
    if executable.starts_with("/Applications") {
        return "system_applications".to_string();
    }
    if executable
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains(".app/contents/macos/")
    {
        return "other_app_bundle".to_string();
    }
    "unbundled".to_string()
}

fn location_for(path: &Path, home: &Path) -> String {
    if path.starts_with(home.join("Applications")) {
        "current_user".to_string()
    } else if path.starts_with("/Applications") {
        "system".to_string()
    } else {
        "other".to_string()
    }
}

fn read_bundle_id(app: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let plist = app.join("Contents/Info.plist");
        let output = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", "Print :CFBundleIdentifier"])
            .arg(plist)
            .output()
            .ok()?;
        output.status.success().then(|| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .to_string()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        None
    }
}

fn discover_bundles(home: &Path) -> Vec<BundleEvidence> {
    let candidates = [
        PathBuf::from("/Applications/Codex.app"),
        PathBuf::from("/Applications/ChatGPT.app"),
        home.join("Applications/Codex.app"),
        home.join("Applications/ChatGPT.app"),
    ];
    candidates
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| BundleEvidence {
            name: path
                .file_stem()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            bundle_id: read_bundle_id(&path),
            location: location_for(&path, home),
            path: path.to_string_lossy().to_string(),
        })
        .collect()
}

fn is_electron_helper(command: &[String]) -> bool {
    command
        .iter()
        .skip(1)
        .any(|part| part.starts_with("--type="))
}

fn mac_relaunch(executable: &str) -> Option<String> {
    let lower = executable.to_ascii_lowercase();
    if lower.contains("/codex.app/") {
        Some("open -a Codex".to_string())
    } else if lower.contains("/chatgpt.app/") {
        Some("open -a ChatGPT".to_string())
    } else {
        None
    }
}

fn discover_processes() -> Vec<ProcessEvidence> {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let desktop_roots = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let executable = process
                .exe()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default();
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy().to_string())
                .collect::<Vec<_>>();
            let lower = executable.to_ascii_lowercase();
            let is_bundle = lower.contains("/codex.app/contents/macos/")
                || lower.contains("/chatgpt.app/contents/macos/");
            let is_resource = lower.contains("/contents/resources/codex");
            (is_bundle && !is_resource && !is_electron_helper(&command)).then_some(pid.as_u32())
        })
        .collect::<HashSet<_>>();

    let mut results = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let name = process.name().to_string_lossy().to_string();
            let lower_name = name.to_ascii_lowercase();
            let executable = process
                .exe()
                .map(|path| path.to_string_lossy().to_string());
            let lower_executable = executable
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let parent_pid = process.parent().map(|value| value.as_u32());
            let (role, relaunch) = if desktop_roots.contains(&pid.as_u32()) {
                (
                    "desktop_root",
                    executable.as_deref().and_then(mac_relaunch),
                )
            } else if (lower_name == "codex" || lower_name == "codex.exe")
                && (parent_pid.is_some_and(|parent| desktop_roots.contains(&parent))
                    || lower_executable.contains("/contents/resources/codex"))
            {
                ("desktop_codex_child", None)
            } else if lower_name == "codex" || lower_name == "codex.exe" {
                ("cli", None)
            } else {
                return None;
            };
            Some(ProcessEvidence {
                pid: pid.as_u32(),
                parent_pid,
                name,
                executable,
                role: role.to_string(),
                relaunch,
            })
        })
        .collect::<Vec<_>>();
    results.sort_by_key(|item| (item.role.clone(), item.pid));
    results
}

fn can_write_directory(path: &Path) -> bool {
    if fs::create_dir_all(path).is_err() {
        return false;
    }
    let marker = path.join(format!(".gpteasy-spike-017-{}", std::process::id()));
    match fs::write(&marker, b"probe") {
        Ok(()) => {
            let _ = fs::remove_file(marker);
            true
        }
        Err(_) => false,
    }
}

pub fn collect_snapshot() -> ContractSnapshot {
    let home = current_home();
    let config = home.join(".codex/config.toml");
    let user_apps = home.join("Applications");
    let executable = std::env::current_exe().ok();
    let scope = classify_install_scope(executable.as_deref(), &home);
    let bundles = discover_bundles(&home);
    let processes = discover_processes();
    let mut relaunch_candidates = processes
        .iter()
        .filter_map(|process| process.relaunch.clone())
        .collect::<Vec<_>>();
    for bundle in &bundles {
        let candidate = format!("open -a {}", bundle.name);
        if !relaunch_candidates.contains(&candidate) {
            relaunch_candidates.push(candidate);
        }
    }
    let is_macos = cfg!(target_os = "macos");
    ContractSnapshot {
        captured_at: now(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        home: home.to_string_lossy().to_string(),
        codex_config_path: config.to_string_lossy().to_string(),
        codex_config_exists: config.exists(),
        user_applications_path: user_apps.to_string_lossy().to_string(),
        user_applications_writable: can_write_directory(&user_apps),
        current_executable: executable
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
        current_app_scope: scope,
        app_bundles: bundles,
        processes,
        relaunch_candidates,
        evidence_level: if is_macos {
            "native_host_probe".to_string()
        } else {
            "non_macos_development_host".to_string()
        },
        limitations: if is_macos {
            vec![
                "托盘可见性和关闭窗口后的视觉体验仍需人工确认".to_string(),
                "真实 Codex 终止和重新激活不会由探针自动执行".to_string(),
                "签名、公证和 updater 替换需要发布凭据及两版本产物".to_string(),
            ]
        } else {
            vec![
                "当前执行环境不是 macOS，不能授予真实宿主验证结论".to_string(),
                "交叉编译或 fixture 不能替代 LaunchServices、APFS 和 WindowServer 实测".to_string(),
            ]
        },
    }
}

pub fn fixture_matrix() -> MatrixSummary {
    let home = Path::new("/Users/alice");
    let cases = [
        (
            "user-app",
            Some(Path::new(
                "/Users/alice/Applications/GPTEasy.app/Contents/MacOS/GPTEasy",
            )),
            "current_user",
        ),
        (
            "system-app",
            Some(Path::new(
                "/Applications/GPTEasy.app/Contents/MacOS/GPTEasy",
            )),
            "system_applications",
        ),
        (
            "other-bundle",
            Some(Path::new(
                "/Volumes/Test/GPTEasy.app/Contents/MacOS/GPTEasy",
            )),
            "other_app_bundle",
        ),
        ("unbundled", Some(Path::new("/tmp/gpteasy")), "unbundled"),
        ("missing", None, "unknown"),
    ];
    let cases = cases
        .into_iter()
        .map(|(name, executable, expected)| {
            let actual = classify_install_scope(executable, home);
            MatrixCase {
                name: name.to_string(),
                expected_scope: expected.to_string(),
                passed: actual == expected,
                actual_scope: actual,
            }
        })
        .collect::<Vec<_>>();
    MatrixSummary {
        total: cases.len(),
        passed: cases.iter().filter(|case| case.passed).count(),
        cases,
    }
}

#[tauri::command]
fn get_contract_snapshot(log: State<'_, EventLog>) -> ContractSnapshot {
    let snapshot = collect_snapshot();
    record(
        &log,
        "snapshot",
        format!(
            "os={} arch={} bundles={} processes={}",
            snapshot.os,
            snapshot.arch,
            snapshot.app_bundles.len(),
            snapshot.processes.len()
        ),
    );
    snapshot
}

#[tauri::command]
fn write_update_canary(app: AppHandle, value: String, log: State<'_, EventLog>) -> Result<String, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("update-canary.txt");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&path, value.as_bytes()).map_err(|error| error.to_string())?;
    record(&log, "canary", "persistent update canary written");
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn read_update_canary(app: AppHandle, log: State<'_, EventLog>) -> Result<Option<String>, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("update-canary.txt");
    let value = if path.exists() {
        Some(fs::read_to_string(&path).map_err(|error| error.to_string())?)
    } else {
        None
    };
    record(
        &log,
        "canary",
        if value.is_some() {
            "persistent update canary found"
        } else {
            "persistent update canary missing"
        },
    );
    Ok(value)
}

#[tauri::command]
fn export_evidence(app: AppHandle, log: State<'_, EventLog>) -> Result<String, String> {
    record(&log, "export", "evidence export requested");
    let events = log
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    let snapshot = collect_snapshot();
    let output = serde_json::json!({
        "snapshot": snapshot,
        "events": events,
        "summary": {
            "event_count": events.len(),
            "categories": events.iter().fold(BTreeMap::<String, usize>::new(), |mut counts, event| {
                *counts.entry(event.category.clone()).or_default() += 1;
                counts
            })
        }
    });
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("macos-contract-evidence.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&output).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示契约探针", true, None::<&str>)?;
    let export = MenuItem::with_id(app, "export", "导出证据", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "明确退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &export, &quit])?;
    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                record(&app.state::<EventLog>(), "tray", "show selected");
            }
            "export" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                record(&app.state::<EventLog>(), "tray", "export selected");
            }
            "quit" => {
                EXPLICIT_EXIT.store(true, Ordering::SeqCst);
                record(&app.state::<EventLog>(), "lifecycle", "explicit exit selected");
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
        .manage(EventLog::default())
        .invoke_handler(tauri::generate_handler![
            get_contract_snapshot,
            write_update_canary,
            read_update_canary,
            export_evidence
        ])
        .setup(|app| {
            record(&app.state::<EventLog>(), "lifecycle", "application started");
            setup_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if !EXPLICIT_EXIT.load(Ordering::SeqCst) {
                    api.prevent_close();
                    let _ = window.hide();
                    record(
                        &window.app_handle().state::<EventLog>(),
                        "lifecycle",
                        "window close prevented and hidden",
                    );
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build macOS contract harness")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !EXPLICIT_EXIT.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    record(
                        &app.state::<EventLog>(),
                        "lifecycle",
                        "implicit exit prevented",
                    );
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_scope_matrix_passes() {
        let summary = fixture_matrix();
        assert_eq!(summary.total, 5);
        assert_eq!(summary.passed, summary.total);
    }

    #[test]
    fn mac_relaunch_only_accepts_known_bundles() {
        assert_eq!(
            mac_relaunch("/Applications/Codex.app/Contents/MacOS/Codex"),
            Some("open -a Codex".to_string())
        );
        assert_eq!(
            mac_relaunch("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"),
            Some("open -a ChatGPT".to_string())
        );
        assert_eq!(mac_relaunch("/tmp/codex"), None);
    }
}
