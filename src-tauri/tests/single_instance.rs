#![cfg(target_os = "windows")]

use std::fs;
use std::sync::mpsc;
use std::time::Duration;

use gpteasy_lib::single_instance::{InstanceRole, acquire};
use tempfile::TempDir;

fn executable_fixture(root: &TempDir, relative_path: &str) -> std::path::PathBuf {
    let path = root.path().join(relative_path);
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
    fs::write(&path, b"fixture").expect("write executable fixture");
    path
}

#[test]
fn second_launch_of_the_same_executable_notifies_the_primary_instance() {
    let executable = std::env::current_exe().expect("current test executable");
    let primary = match acquire(&executable).expect("acquire primary instance") {
        InstanceRole::Primary(primary) => primary,
        InstanceRole::Secondary => panic!("first launch must become primary"),
    };
    let (activated_tx, activated_rx) = mpsc::channel();
    let _listener = primary
        .listen(move || {
            let _ = activated_tx.send(());
        })
        .expect("listen for secondary launches");

    assert!(matches!(
        acquire(&executable).expect("acquire secondary instance"),
        InstanceRole::Secondary
    ));
    activated_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("secondary launch must activate the primary instance");
}

#[test]
fn different_executable_paths_have_independent_instance_ownership() {
    let root = TempDir::new().expect("temp directory");
    let installed = executable_fixture(&root, "installed/gpteasy.exe");
    let development = executable_fixture(&root, "development/gpteasy.exe");

    let installed = acquire(&installed).expect("acquire installed instance");
    let development = acquire(&development).expect("acquire development instance");

    assert!(matches!(installed, InstanceRole::Primary(_)));
    assert!(matches!(development, InstanceRole::Primary(_)));
}
