use std::path::PathBuf;

use gpteasy_lib::shared_daemon::{
    SharedDaemonBackend, SharedDaemonCandidate, SharedDaemonCapability, SharedDaemonEntrySource,
    SharedDaemonEnvironmentMatch, SharedDaemonFixture, SharedDaemonIdentityEvidence,
    SharedDaemonProbe, SharedDaemonProcessKind, SharedDaemonReport, SharedDaemonStatus,
    SharedDaemonVersionVisibility, detect_shared_daemon,
};

const HOME: &str = r"C:\Users\alice\.codex";
const EXECUTABLE: &str =
    r"C:\Users\alice\.codex\packages\app-server-daemon\releases\26.9\codex-path.exe";

fn candidate(source: SharedDaemonEntrySource, path: &str) -> SharedDaemonCandidate {
    SharedDaemonCandidate {
        source,
        path: PathBuf::from(path),
        exists: true,
        native_identity_verified: true,
    }
}

fn evidence(kind: SharedDaemonProcessKind, pid: u32, started: u64) -> SharedDaemonIdentityEvidence {
    SharedDaemonIdentityEvidence {
        pid,
        started_at_epoch_millis: started,
        executable: PathBuf::from(EXECUTABLE),
        user: "alice".to_owned(),
        codex_home: PathBuf::from(HOME),
        backend: SharedDaemonBackend::Pid,
        socket_control_owner: "alice-default-codex-home".to_owned(),
        kind,
    }
}

fn running_fixture() -> SharedDaemonFixture {
    let process = evidence(SharedDaemonProcessKind::SharedDaemon, 41, 900);
    let report = SharedDaemonReport {
        pid: Some(41),
        started_at_epoch_millis: Some(900),
        executable: Some(PathBuf::from(EXECUTABLE)),
        user: Some("alice".to_owned()),
        codex_home: Some(PathBuf::from(HOME)),
        backend: SharedDaemonBackend::Pid,
        socket_control_owner: Some("alice-default-codex-home".to_owned()),
        version: Some("codex 0.157.1".to_owned()),
    };
    SharedDaemonFixture {
        current_user: "alice".to_owned(),
        default_codex_home: PathBuf::from(HOME),
        candidates: vec![candidate(SharedDaemonEntrySource::Standalone, EXECUTABLE)],
        probes: vec![(
            PathBuf::from(EXECUTABLE),
            SharedDaemonProbe::Running(report),
        )],
        reported_processes: vec![process.clone()],
        actual_processes: vec![process],
        owned_processes: Vec::new(),
    }
}

#[test]
fn verifies_running_daemon_against_report_and_process_identity() {
    let snapshot = detect_shared_daemon(&running_fixture());

    assert_eq!(snapshot.status, SharedDaemonStatus::Running);
    assert_eq!(snapshot.capability, SharedDaemonCapability::Supported);
    assert_eq!(snapshot.entry_source, SharedDaemonEntrySource::Standalone);
    assert_eq!(snapshot.version.as_deref(), Some("codex 0.157.1"));
    assert_eq!(
        snapshot.version_visibility,
        SharedDaemonVersionVisibility::Visible
    );
    assert_eq!(
        snapshot.environment_match,
        SharedDaemonEnvironmentMatch::Match
    );
    assert_eq!(
        snapshot.identity.as_ref().map(|identity| identity.pid),
        Some(41)
    );
}

#[test]
fn evaluates_all_candidates_instead_of_accepting_a_shadowing_first_entry() {
    let mut fixture = running_fixture();
    let npm = r"C:\Users\alice\AppData\Roaming\npm\node_modules\@openai\codex-win32-x64\codex.exe";
    fixture
        .candidates
        .insert(0, candidate(SharedDaemonEntrySource::Npm, npm));
    fixture.probes.insert(
        0,
        (
            PathBuf::from(npm),
            SharedDaemonProbe::Unknown { reason: "shadowed" },
        ),
    );

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Running);
    assert_eq!(snapshot.entry_source, SharedDaemonEntrySource::Standalone);
}

#[test]
fn rejects_npm_shim_and_desktop_bundled_entries_without_claiming_a_service() {
    let mut fixture = running_fixture();
    fixture.candidates = vec![
        candidate(
            SharedDaemonEntrySource::DesktopBundled,
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3\resources\codex\codex.exe",
        ),
        SharedDaemonCandidate {
            source: SharedDaemonEntrySource::Npm,
            path: PathBuf::from(r"C:\Users\alice\AppData\Roaming\npm\codex.cmd"),
            exists: true,
            native_identity_verified: false,
        },
    ];
    fixture.probes.clear();

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Unknown);
    assert_eq!(snapshot.reason, "missing_trusted_entry");
}

#[test]
fn does_not_turn_pid_reuse_or_creation_time_mismatch_into_running() {
    let mut fixture = running_fixture();
    fixture.actual_processes[0].started_at_epoch_millis = 901;

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Unknown);
    assert_eq!(snapshot.reason, "probe_or_identity_unknown");
}

#[test]
fn excludes_gpteeasy_owned_app_server_by_exact_identity() {
    let mut fixture = running_fixture();
    fixture.owned_processes = fixture.reported_processes.clone();

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Unknown);
}

#[test]
fn rejects_other_users_and_custom_homes_as_unknown() {
    let mut fixture = running_fixture();
    fixture.reported_processes[0].user = "bob".to_owned();

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Unknown);
    assert_eq!(
        snapshot.environment_match,
        SharedDaemonEnvironmentMatch::Unknown
    );
}

#[test]
fn rejects_custom_home_even_when_the_process_identity_is_otherwise_exact() {
    let mut fixture = running_fixture();
    fixture.probes = vec![(
        PathBuf::from(EXECUTABLE),
        SharedDaemonProbe::Running(SharedDaemonReport {
            codex_home: Some(PathBuf::from(r"C:\Users\alice\other-codex")),
            ..match_running_report()
        }),
    )];

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Unknown);
}

#[test]
fn rejects_interactive_cli_and_other_app_server_processes() {
    for kind in [
        SharedDaemonProcessKind::InteractiveCli,
        SharedDaemonProcessKind::OtherAppServer,
        SharedDaemonProcessKind::DesktopBundled,
    ] {
        let mut fixture = running_fixture();
        fixture.actual_processes[0].kind = kind;

        let snapshot = detect_shared_daemon(&fixture);

        assert_eq!(snapshot.status, SharedDaemonStatus::Unknown);
    }
}

#[test]
fn missing_identity_fields_are_unknown_and_public_snapshot_is_redacted() {
    let mut fixture = running_fixture();
    fixture.probes = vec![(
        PathBuf::from(EXECUTABLE),
        SharedDaemonProbe::Running(SharedDaemonReport {
            socket_control_owner: None,
            ..match_running_report()
        }),
    )];
    let unknown = detect_shared_daemon(&fixture);
    assert_eq!(unknown.status, SharedDaemonStatus::Unknown);

    let snapshot = detect_shared_daemon(&running_fixture());
    let serialized = serde_json::to_string(&snapshot).expect("serialize snapshot");
    assert!(serialized.contains("executableIdentity"));
    assert!(!serialized.contains(EXECUTABLE));
    assert!(!serialized.contains("alice-default-codex-home"));
}

#[test]
fn reports_confirmed_stopped_only_from_a_valid_read_only_probe() {
    let mut fixture = running_fixture();
    fixture.probes = vec![(
        PathBuf::from(EXECUTABLE),
        SharedDaemonProbe::Stopped {
            version: Some("codex 0.157.1".to_owned()),
        },
    )];
    fixture.reported_processes.clear();
    fixture.actual_processes.clear();

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Stopped);
    assert_eq!(snapshot.capability, SharedDaemonCapability::Supported);
}

#[test]
fn keeps_unknown_probe_failures_unknown_instead_of_stopped() {
    let mut fixture = running_fixture();
    fixture.probes = vec![(
        PathBuf::from(EXECUTABLE),
        SharedDaemonProbe::Unknown {
            reason: "malformed_json",
        },
    )];

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Unknown);
    assert_eq!(snapshot.reason, "probe_or_identity_unknown");
}

#[test]
fn exposes_unknown_backend_without_treating_it_as_stopped() {
    let mut fixture = running_fixture();
    fixture.probes = vec![(
        PathBuf::from(EXECUTABLE),
        SharedDaemonProbe::Running(SharedDaemonReport {
            backend: SharedDaemonBackend::Unknown,
            ..match_running_report()
        }),
    )];

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Unknown);
}

#[test]
fn exposes_an_explicitly_unsupported_management_contract() {
    let mut fixture = running_fixture();
    fixture.probes = vec![(
        PathBuf::from(EXECUTABLE),
        SharedDaemonProbe::Unsupported {
            reason: "daemon_status_unsupported",
        },
    )];

    let snapshot = detect_shared_daemon(&fixture);

    assert_eq!(snapshot.status, SharedDaemonStatus::Unsupported);
    assert_eq!(snapshot.capability, SharedDaemonCapability::Unsupported);
    assert_eq!(snapshot.reason, "daemon_status_unsupported");
}

fn match_running_report() -> SharedDaemonReport {
    SharedDaemonReport {
        pid: Some(41),
        started_at_epoch_millis: Some(900),
        executable: Some(PathBuf::from(EXECUTABLE)),
        user: Some("alice".to_owned()),
        codex_home: Some(PathBuf::from(HOME)),
        backend: SharedDaemonBackend::Pid,
        socket_control_owner: Some("alice-default-codex-home".to_owned()),
        version: None,
    }
}
