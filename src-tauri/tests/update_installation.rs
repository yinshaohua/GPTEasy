use std::fs;

use gpteasy_lib::update::{UpdateActivityGate, UpdateCoordinator, UpdateState};
use tempfile::TempDir;

#[test]
fn incomplete_update_state_is_independent_from_the_business_database() {
    let root = TempDir::new().expect("application data");
    fs::write(
        root.path().join("state.sqlite3"),
        b"unreadable business state",
    )
    .expect("business database fixture");
    let attempt = root.path().join("update-install-attempt.json");
    fs::write(&attempt, br#"{"target_version":"1.1.0"}"#).expect("install attempt");

    let coordinator = UpdateCoordinator::with_state_path("1.0.1", &attempt);

    let snapshot = coordinator.snapshot();
    assert_eq!(snapshot.state, UpdateState::Incomplete);
    assert_eq!(snapshot.available_version.as_deref(), Some("1.1.0"));
    assert_eq!(
        fs::read_to_string(attempt).expect("read install attempt"),
        r#"{"target_version":"1.1.0"}"#
    );
}

#[test]
fn install_gate_waits_for_every_write_and_then_rejects_new_writes() {
    let gate = UpdateActivityGate::default();
    let provider = gate.try_begin("供应商验证").expect("provider activity");
    let session = gate.try_begin("会话修改").expect("session activity");
    assert!(gate.try_begin_install().is_none());

    drop(provider);
    assert!(gate.try_begin_install().is_none());
    drop(session);

    let install = gate.try_begin_install().expect("install activity");
    install.commit_install();
    assert!(gate.try_begin("配置写入").is_none());
}
