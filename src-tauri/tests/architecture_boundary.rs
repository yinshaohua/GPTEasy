use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .to_path_buf()
}

#[test]
fn frontend_has_no_direct_resource_plugins() {
    let root = repository_root();
    let package: Value = serde_json::from_str(
        &fs::read_to_string(root.join("package.json")).expect("read package.json"),
    )
    .expect("parse package.json");
    let dependencies = package["dependencies"]
        .as_object()
        .expect("production dependencies")
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected = ["@tauri-apps/api", "lucide-react", "react", "react-dom"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(dependencies, expected);

    let capability: Value = serde_json::from_str(
        &fs::read_to_string(root.join("src-tauri/capabilities/default.json"))
            .expect("read capability"),
    )
    .expect("parse capability");
    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert_eq!(
        capability["permissions"],
        serde_json::json!(["core:default"])
    );
}

#[test]
fn bundle_is_windows_x64_current_user_nsis() {
    let root = repository_root();
    let config: Value = serde_json::from_str(
        &fs::read_to_string(root.join("src-tauri/tauri.conf.json")).expect("read Tauri config"),
    )
    .expect("parse Tauri config");

    assert_eq!(config["bundle"]["targets"], serde_json::json!(["nsis"]));
    assert_eq!(
        config["bundle"]["windows"]["nsis"]["installMode"],
        "currentUser"
    );
    assert_eq!(
        config["bundle"]["windows"]["nsis"]["installerHooks"],
        "windows/installer-hooks.nsh"
    );
    let installer_hooks = fs::read_to_string(root.join("src-tauri/windows/installer-hooks.nsh"))
        .expect("read NSIS installer hooks");
    assert!(installer_hooks.contains("${AtLeastBuild} 19045"));
    let manifest = fs::read_to_string(root.join("src-tauri/src/main.rs")).expect("read main.rs");
    assert!(manifest.contains("target_arch = \"x86_64\""));
    assert!(manifest.contains("target_os = \"windows\""));
}

#[test]
fn main_window_uses_the_compact_provider_management_size() {
    let root = repository_root();
    let config: Value = serde_json::from_str(
        &fs::read_to_string(root.join("src-tauri/tauri.conf.json")).expect("read Tauri config"),
    )
    .expect("parse Tauri config");
    let window = &config["app"]["windows"][0];

    assert_eq!(window["width"], 1120);
    assert_eq!(window["height"], 620);
    assert_eq!(window["minWidth"], 680);
    assert_eq!(window["minHeight"], 520);
    assert_eq!(window["resizable"], true);
}

#[test]
fn config_changes_are_independent_from_desktop_lifecycle_contracts() {
    let root = repository_root();
    let commands =
        fs::read_to_string(root.join("src-tauri/src/commands.rs")).expect("read Tauri commands");
    let environment_contract = fs::read_to_string(root.join("src/contracts/environment.ts"))
        .expect("read environment contract");
    let provider_contract =
        fs::read_to_string(root.join("src/contracts/provider.ts")).expect("read provider contract");
    let provider_page =
        fs::read_to_string(root.join("src/ProviderPage.tsx")).expect("read provider page");
    let assembly =
        fs::read_to_string(root.join("src-tauri/src/lib.rs")).expect("read Tauri assembly");

    for removed_contract in [
        "RestartDecision",
        "ConfigChangeResult",
        "restartDecision",
        "forceAuthorization",
        "forceExpectedRevision",
    ] {
        assert!(
            !environment_contract.contains(removed_contract),
            "environment contract still exposes {removed_contract}"
        );
        assert!(
            !provider_contract.contains(removed_contract),
            "provider contract still exposes {removed_contract}"
        );
    }

    assert!(!commands.contains("apply_provider_with_restart_plan"));
    assert!(!commands.contains("switch_to_openai_login_with_restart_plan"));
    assert!(!commands.contains("save_and_apply_provider_update_with_restart_plan"));
    assert!(!provider_page.contains("start_desktop_application"));
    assert!(!provider_page.contains("restart_desktop_application"));
    assert!(!provider_page.contains("force_restart_desktop_application"));
    assert!(!assembly.contains("force_complete_config_restart"));
}

#[test]
fn production_has_no_active_desktop_control_capability() {
    let root = repository_root();
    let commands =
        fs::read_to_string(root.join("src-tauri/src/commands.rs")).expect("read Tauri commands");
    let assembly =
        fs::read_to_string(root.join("src-tauri/src/lib.rs")).expect("read Tauri assembly");
    let consumer =
        fs::read_to_string(root.join("src-tauri/src/consumer.rs")).expect("read consumer module");

    for forbidden_command in [
        "get_desktop_snapshot",
        "start_desktop_application",
        "restart_desktop_application",
        "force_restart_desktop_application",
    ] {
        assert!(
            !commands.contains(forbidden_command),
            "Tauri commands still expose {forbidden_command}"
        );
        assert!(
            !assembly.contains(forbidden_command),
            "Tauri assembly still registers {forbidden_command}"
        );
    }

    assert!(
        !root.join("src/contracts/desktop.ts").exists(),
        "frontend desktop-control contract still exists"
    );
    for forbidden_capability in [
        "DesktopApplication",
        "DesktopActivator",
        "DesktopProcessController",
        "request_windows_close",
        "force_terminate_windows_processes",
        "TerminateProcess",
        "WM_CLOSE",
        "shell:AppsFolder",
    ] {
        assert!(
            !consumer.contains(forbidden_capability),
            "consumer production code still contains {forbidden_capability}"
        );
    }
}
