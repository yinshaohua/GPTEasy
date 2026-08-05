use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, WindowEvent,
};

static EXPLICIT_EXIT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub executable: Option<String>,
    pub role: String,
    pub confidence: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub processes: Vec<ProcessInfo>,
    pub counts: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct RestartAction {
    pub pid: u32,
    pub role: String,
    pub action: String,
    pub relaunch: Option<String>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct RestartPlan {
    pub decision: String,
    pub write_configuration: bool,
    pub pending_restart: bool,
    pub actions: Vec<RestartAction>,
    pub warnings: Vec<String>,
}

pub fn scan() -> ScanReport {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let desktop_roots = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let name = process.name().to_string_lossy().to_ascii_lowercase();
            let executable = process
                .exe()
                .map(|path| path.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy().to_string())
                .collect::<Vec<_>>();
            is_desktop_root(&name, &executable, &command).then_some(pid.as_u32())
        })
        .collect::<HashSet<_>>();

    let mut relevant = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let name = process.name().to_string_lossy().to_string();
            let lower_name = name.to_ascii_lowercase();
            let executable_path = process.exe().map(Path::to_path_buf);
            let executable = executable_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string());
            let lower_executable = executable
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let parent_pid = process.parent().map(Pid::as_u32);
            let classification = classify_process(
                pid.as_u32(),
                &lower_name,
                &lower_executable,
                parent_pid,
                &desktop_roots,
            )?;
            Some(ProcessInfo {
                pid: pid.as_u32(),
                parent_pid,
                name,
                executable,
                role: classification.0.to_string(),
                confidence: classification.1.to_string(),
                reason: classification.2.to_string(),
            })
        })
        .collect::<Vec<_>>();
    relevant.sort_by_key(|process| (process.role.clone(), process.pid));

    let mut counts = BTreeMap::from([
        ("desktop_root".to_string(), 0),
        ("desktop_codex_child".to_string(), 0),
        ("cli".to_string(), 0),
        ("legacy_or_other_host".to_string(), 0),
    ]);
    for process in &relevant {
        *counts.entry(process.role.clone()).or_default() += 1;
    }
    ScanReport {
        processes: relevant,
        counts,
    }
}

fn is_desktop_root(name: &str, executable: &str, command: &[String]) -> bool {
    let desktop_name = matches!(
        name,
        "chatgpt.exe" | "chatgpt" | "codex.app" | "codex" | "codex.exe"
    );
    let packaged_windows =
        executable.contains("windowsapps") && executable.contains("openai.codex_");
    let mac_bundle = executable.contains(".app/contents/macos/")
        && (executable.contains("/codex.app/") || executable.contains("/chatgpt.app/"));
    let bundled_resource = executable.contains("\\resources\\codex")
        || executable.contains("/contents/resources/codex");
    let electron_helper = command
        .iter()
        .skip(1)
        .any(|argument| argument.starts_with("--type="));
    desktop_name && !bundled_resource && !electron_helper && (packaged_windows || mac_bundle)
}

fn classify_process<'a>(
    pid: u32,
    name: &str,
    executable: &str,
    parent_pid: Option<u32>,
    desktop_roots: &HashSet<u32>,
) -> Option<(&'a str, &'a str, &'a str)> {
    if desktop_roots.contains(&pid) {
        return Some((
            "desktop_root",
            "high",
            "packaged OpenAI Codex/ChatGPT desktop executable",
        ));
    }
    let is_codex = name == "codex" || name == "codex.exe";
    if is_codex
        && (parent_pid.is_some_and(|parent| desktop_roots.contains(&parent))
            || executable.contains("openai.codex_")
            || executable.contains(".app/contents/resources/codex"))
    {
        return Some((
            "desktop_codex_child",
            "high",
            "Codex executable is bundled by or parented to the desktop host",
        ));
    }
    if is_codex {
        return Some((
            "cli",
            "medium",
            "Codex executable is not attached to a recognized desktop host",
        ));
    }
    if (name == "chatgpt.exe" || name == "chatgpt")
        && parent_pid.is_some_and(|parent| desktop_roots.contains(&parent))
    {
        return None;
    }
    if name == "chatgpt.exe" || name == "chatgpt" || name == "codex++" || name == "codex++.exe"
    {
        return Some((
            "legacy_or_other_host",
            "medium",
            "ChatGPT/Codex-like host is outside the recognized packaged application",
        ));
    }
    None
}

pub fn plan(decision: &str, processes: Vec<ProcessInfo>) -> Result<RestartPlan, String> {
    if !matches!(decision, "immediate" | "later" | "cancel") {
        return Err(format!("unsupported decision `{decision}`"));
    }
    if decision == "cancel" {
        return Ok(RestartPlan {
            decision: decision.to_string(),
            write_configuration: false,
            pending_restart: false,
            actions: Vec::new(),
            warnings: vec!["用户取消：不得写入配置。".to_string()],
        });
    }

    let has_relevant = processes
        .iter()
        .any(|process| process.role != "legacy_or_other_host");
    if decision == "later" {
        return Ok(RestartPlan {
            decision: decision.to_string(),
            write_configuration: true,
            pending_restart: has_relevant,
            actions: processes
                .into_iter()
                .filter(|process| process.role != "legacy_or_other_host")
                .map(|process| RestartAction {
                    pid: process.pid,
                    role: process.role,
                    action: "leave_running".to_string(),
                    relaunch: None,
                    reason: "配置写入后继续使用旧配置，进入待重启状态".to_string(),
                })
                .collect(),
            warnings: Vec::new(),
        });
    }

    let mut actions = Vec::new();
    let mut warnings = Vec::new();
    for process in processes {
        match process.role.as_str() {
            "desktop_root" => actions.push(RestartAction {
                pid: process.pid,
                role: process.role,
                action: "terminate_tree_then_activate_app".to_string(),
                relaunch: desktop_relaunch_method(process.executable.as_deref()),
                reason: "桌面主进程可以整体退出后通过应用激活机制重新启动".to_string(),
            }),
            "desktop_codex_child" => actions.push(RestartAction {
                pid: process.pid,
                role: process.role,
                action: "terminate_with_desktop_tree".to_string(),
                relaunch: None,
                reason: "由桌面主进程拥有，不应单独重启".to_string(),
            }),
            "cli" => {
                actions.push(RestartAction {
                    pid: process.pid,
                    role: process.role,
                    action: "manual_restart_required".to_string(),
                    relaunch: None,
                    reason: "无法可靠恢复原终端 TTY、cwd、stdin 和会话状态".to_string(),
                });
                warnings.push(format!(
                    "Codex CLI PID {} 需要用户在原终端退出并重新运行；GPTEasy 不应静默终止。",
                    process.pid
                ));
            }
            _ => {}
        }
    }
    Ok(RestartPlan {
        decision: decision.to_string(),
        write_configuration: true,
        pending_restart: actions
            .iter()
            .any(|action| action.action == "manual_restart_required"),
        actions,
        warnings,
    })
}

fn desktop_relaunch_method(executable: Option<&str>) -> Option<String> {
    let executable = executable?;
    let lower = executable.to_ascii_lowercase();
    if lower.contains("windowsapps") && lower.contains("openai.codex_") {
        return Some(
            "explorer.exe shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
        );
    }
    if lower.contains("/codex.app/") {
        return Some("open -a Codex".to_string());
    }
    if lower.contains("/chatgpt.app/") {
        return Some("open -a ChatGPT".to_string());
    }
    Some(executable.to_string())
}

#[tauri::command]
fn scan_processes() -> ScanReport {
    scan()
}

#[tauri::command]
fn build_restart_plan(
    decision: String,
    processes: Vec<ProcessInfo>,
) -> Result<RestartPlan, String> {
    plan(&decision, processes)
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示设置", true, None::<&str>)?;
    let scan = MenuItem::with_id(app, "scan", "重新扫描 Codex 进程", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "明确退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &scan, &quit])?;
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
            "scan" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("process-scan-requested", ());
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
        .invoke_handler(tauri::generate_handler![
            scan_processes,
            build_restart_plan
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
        .expect("failed to build Tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !EXPLICIT_EXIT.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}

pub fn fixture_cycle(root: &Path) -> Result<Value, String> {
    let before = scan();
    let root = root
        .canonicalize()
        .map_err(|error| format!("invalid fixture root: {error}"))?;
    let fixture_processes = before
        .processes
        .iter()
        .filter(|process| {
            process
                .executable
                .as_deref()
                .and_then(|path| PathBuf::from(path).canonicalize().ok())
                .is_some_and(|path| path.starts_with(&root))
        })
        .cloned()
        .collect::<Vec<_>>();
    if fixture_processes.is_empty() {
        return Err("no fixture processes found".to_string());
    }

    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    for process in fixture_processes
        .iter()
        .filter(|process| process.role == "desktop_codex_child" || process.role == "cli")
    {
        if let Some(target) = system.process(Pid::from_u32(process.pid)) {
            let _ = target.kill();
        }
    }
    for process in fixture_processes
        .iter()
        .filter(|process| process.role == "desktop_root")
    {
        if let Some(target) = system.process(Pid::from_u32(process.pid)) {
            let _ = target.kill();
        }
    }
    thread::sleep(Duration::from_millis(500));

    let desktop = root.join("WindowsApps/OpenAI.Codex_fixture/app/ChatGPT.exe");
    let child = root.join("WindowsApps/OpenAI.Codex_fixture/app/resources/codex.exe");
    Command::new(&desktop)
        .arg("desktop-root")
        .arg(&child)
        .spawn()
        .map_err(|error| format!("failed to relaunch fixture desktop: {error}"))?;
    thread::sleep(Duration::from_millis(800));

    let after = scan();
    let after_fixture = after
        .processes
        .iter()
        .filter(|process| {
            process
                .executable
                .as_deref()
                .and_then(|path| PathBuf::from(path).canonicalize().ok())
                .is_some_and(|path| path.starts_with(&root))
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "before": fixture_processes,
        "after": after_fixture,
        "desktop_relaunched": after_fixture.iter().any(|process| process.role == "desktop_root"),
        "desktop_child_relaunched": after_fixture.iter().any(|process| process.role == "desktop_codex_child"),
        "cli_relaunched": after_fixture.iter().any(|process| process.role == "cli")
    }))
}
