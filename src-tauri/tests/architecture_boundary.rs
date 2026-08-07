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
