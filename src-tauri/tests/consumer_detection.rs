use std::path::PathBuf;

use gpteasy_lib::consumer::{
    ConsumerRole, ConsumerScanner, ConsumerStatus, FixtureProcess, ProcessAccess,
    WindowsConsumerScanner, classify_fixture, classify_fixture_for_packages,
};

fn process(
    pid: u32,
    parent_pid: u32,
    started_at_epoch_millis: u64,
    name: &str,
    executable: &str,
) -> FixtureProcess {
    FixtureProcess {
        pid,
        parent_pid,
        started_at_epoch_millis,
        name: name.to_owned(),
        executable: PathBuf::from(executable),
        access: ProcessAccess::Available,
        electron_helper: false,
    }
}

#[test]
fn recognizes_desktop_root_and_its_bundled_codex_child_from_process_evidence() {
    let scan = classify_fixture(&[
        process(
            100,
            1,
            1_000,
            "Codex.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__2p2nqsd0c76g0\Codex.exe",
        ),
        process(
            101,
            100,
            1_001,
            "codex.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__2p2nqsd0c76g0\resources\codex\codex.exe",
        ),
    ]);

    assert_eq!(scan.desktop, ConsumerStatus::Running);
    assert_eq!(scan.cli, ConsumerStatus::Stopped);
    assert_eq!(scan.identities.len(), 2);
    assert_eq!(scan.identities[0].role, ConsumerRole::Desktop);
    assert_eq!(scan.identities[0].pid, 100);
    assert_eq!(scan.identities[1].role, ConsumerRole::Desktop);
    assert_eq!(scan.identities[1].pid, 101);
    assert_eq!(scan.desktop_roots.len(), 1);
    assert_eq!(scan.desktop_roots[0].pid, 100);
}

#[test]
fn recognizes_standalone_codex_cli_without_mistaking_the_bundled_child_for_cli() {
    let scan = classify_fixture(&[
        process(
            100,
            1,
            1_000,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__2p2nqsd0c76g0\ChatGPT.exe",
        ),
        process(
            101,
            100,
            1_001,
            "codex.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__2p2nqsd0c76g0\resources\codex\codex.exe",
        ),
        process(
            202,
            77,
            2_000,
            "codex.exe",
            r"C:\Users\example\AppData\Roaming\npm\node_modules\@openai\codex\vendor\codex.exe",
        ),
    ]);

    assert_eq!(scan.desktop, ConsumerStatus::Running);
    assert_eq!(scan.cli, ConsumerStatus::Running);
    assert_eq!(
        scan.identities
            .iter()
            .find(|identity| identity.role == ConsumerRole::Cli)
            .map(|identity| identity.pid),
        Some(202)
    );
}

#[test]
fn codex_name_without_a_trusted_install_path_is_unknown() {
    let scan = classify_fixture(&[process(
        202,
        77,
        2_000,
        "codex.exe",
        r"C:\tools\unrelated\codex.exe",
    )]);

    assert_eq!(scan.desktop, ConsumerStatus::Stopped);
    assert_eq!(scan.cli, ConsumerStatus::Unknown);
    assert!(scan.identities.is_empty());
}

#[test]
fn desktop_root_must_belong_to_a_dynamically_discovered_install_location() {
    let scan = classify_fixture_for_packages(
        &[process(
            100,
            1,
            1_000,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_forged\ChatGPT.exe",
        )],
        &[PathBuf::from(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_real",
        )],
    );

    assert_eq!(scan.desktop, ConsumerStatus::Stopped);
    assert!(scan.desktop_roots.is_empty());
}

#[test]
fn orphaned_bundled_codex_child_is_unknown_and_not_a_cli() {
    let scan = classify_fixture(&[process(
        101,
        100,
        1_001,
        "codex.exe",
        r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__2p2nqsd0c76g0\resources\codex\codex.exe",
    )]);

    assert_eq!(scan.desktop, ConsumerStatus::Unknown);
    assert_eq!(scan.cli, ConsumerStatus::Stopped);
    assert!(scan.identities.is_empty());
}

#[test]
fn electron_helper_is_not_classified_as_a_desktop_root() {
    let mut helper = process(
        100,
        1,
        1_000,
        "Codex.exe",
        r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__2p2nqsd0c76g0\Codex.exe",
    );
    helper.electron_helper = true;

    let scan = classify_fixture(&[helper]);

    assert_eq!(scan.desktop, ConsumerStatus::Stopped);
    assert_eq!(scan.cli, ConsumerStatus::Stopped);
    assert!(scan.identities.is_empty());
}

#[test]
fn renderer_gpu_network_and_crashpad_helpers_are_never_controllable_desktop_roots() {
    let helpers = ["renderer", "gpu-process", "utility", "crashpad-handler"]
        .into_iter()
        .enumerate()
        .map(|(index, _helper_type)| {
            let mut helper = process(
                110 + index as u32,
                100,
                1_100 + index as u64,
                "ChatGPT.exe",
                r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_1.2.3_x64__publisher\ChatGPT.exe",
            );
            helper.electron_helper = true;
            helper
        })
        .collect::<Vec<_>>();

    let scan = classify_fixture(&helpers);

    assert_eq!(scan.desktop, ConsumerStatus::Stopped);
    assert!(scan.desktop_roots.is_empty());
}

#[test]
fn bundled_codex_uses_the_full_ancestor_chain_but_codex_plus_plus_is_excluded() {
    let mut renderer = process(
        101,
        100,
        1_001,
        "ChatGPT.exe",
        r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_1.2.3_x64__publisher\ChatGPT.exe",
    );
    renderer.electron_helper = true;
    let scan = classify_fixture(&[
        process(
            100,
            1,
            1_000,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_1.2.3_x64__publisher\ChatGPT.exe",
        ),
        renderer,
        process(
            102,
            101,
            1_002,
            "codex.exe",
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_1.2.3_x64__publisher\resources\codex\codex.exe",
        ),
        process(
            500,
            1,
            5_000,
            "Codex++.exe",
            r"C:\Users\example\AppData\Local\CodexPlusPlus\Codex++.exe",
        ),
    ]);

    assert_eq!(scan.desktop, ConsumerStatus::Running);
    assert_eq!(scan.cli, ConsumerStatus::Stopped);
    assert_eq!(scan.desktop_roots.len(), 1);
    assert_eq!(scan.desktop_roots[0].pid, 100);
    assert!(scan.identities.iter().any(|identity| identity.pid == 102));
    assert!(scan.identities.iter().all(|identity| identity.pid != 500));
}

#[test]
fn access_limited_candidate_is_unknown_instead_of_being_reported_as_stopped() {
    let mut candidate = process(202, 77, 2_000, "codex.exe", r"C:\unknown\codex.exe");
    candidate.access = ProcessAccess::Denied;

    let scan = classify_fixture(&[candidate]);

    assert_eq!(scan.desktop, ConsumerStatus::Unknown);
    assert_eq!(scan.cli, ConsumerStatus::Unknown);
}

#[test]
fn another_users_codex_process_is_outside_the_managed_environment() {
    let mut candidate = process(
        202,
        77,
        2_000,
        "codex.exe",
        r"C:\Users\other\node_modules\@openai\codex\vendor\codex.exe",
    );
    candidate.access = ProcessAccess::OtherUser;

    let scan = classify_fixture(&[candidate]);

    assert_eq!(scan.desktop, ConsumerStatus::Stopped);
    assert_eq!(scan.cli, ConsumerStatus::Stopped);
    assert!(scan.identities.is_empty());
}

#[test]
fn access_limited_candidate_keeps_the_role_unknown_even_when_another_process_is_visible() {
    let visible = process(
        202,
        77,
        2_000,
        "codex.exe",
        r"C:\Users\example\AppData\Roaming\npm\node_modules\@openai\codex\vendor\codex.exe",
    );
    let mut hidden = process(203, 77, 2_001, "codex.exe", r"C:\unknown\codex.exe");
    hidden.access = ProcessAccess::Denied;

    let scan = classify_fixture(&[visible, hidden]);

    assert_eq!(scan.cli, ConsumerStatus::Unknown);
    assert!(!scan.is_trustworthy());
}

#[test]
fn pending_identity_uses_start_time_so_pid_reuse_does_not_keep_restart_pending() {
    let before = classify_fixture(&[process(
        202,
        77,
        2_000,
        "codex.exe",
        r"C:\Users\example\AppData\Roaming\npm\node_modules\@openai\codex\vendor\codex.exe",
    )]);
    let after = classify_fixture(&[process(
        202,
        77,
        3_000,
        "codex.exe",
        r"C:\Users\example\AppData\Roaming\npm\node_modules\@openai\codex\vendor\codex.exe",
    )]);

    assert!(before.has_live_identity_from(&before.identities));
    assert!(!after.has_live_identity_from(&before.identities));
}

#[cfg(windows)]
#[test]
fn windows_scanner_recognizes_a_real_process_from_a_trusted_cli_path() {
    use std::fs;
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant};

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let temp = tempfile::TempDir::new().expect("temp directory");
    let fixture_dir = temp
        .path()
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("vendor");
    fs::create_dir_all(&fixture_dir).expect("create trusted fixture path");
    let executable = fixture_dir.join("codex.exe");
    let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
    fs::copy(
        PathBuf::from(system_root).join("System32").join("cmd.exe"),
        &executable,
    )
    .expect("copy process fixture");
    let child = Command::new(&executable)
        .args(["/d", "/c", "ping.exe -n 30 127.0.0.1 >nul"])
        .spawn()
        .expect("start process fixture");
    let pid = child.id();
    let _guard = ChildGuard(child);
    let scanner = WindowsConsumerScanner::new();
    let deadline = Instant::now() + Duration::from_secs(3);

    loop {
        let scan = scanner.scan();
        if scan
            .identities
            .iter()
            .any(|identity| identity.role == ConsumerRole::Cli && identity.pid == pid)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "scanner did not identify fixture PID {pid}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}
