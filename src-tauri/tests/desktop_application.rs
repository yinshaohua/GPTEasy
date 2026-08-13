use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpteasy_lib::consumer::{
    ConsumerRole, ConsumerScan, ConsumerScanner, ConsumerStatus, DesktopAction, DesktopActivator,
    DesktopApplication, DesktopBoundaryError, DesktopClock, DesktopFailureCategory, DesktopPackage,
    DesktopPackageDiscovery, DesktopProcessController, DesktopRestartStatus, FixtureProcess,
    ProcessAccess, classify_fixture_for_packages,
};

#[derive(Debug)]
struct FixtureDiscovery {
    packages: Vec<DesktopPackage>,
}

impl DesktopPackageDiscovery for FixtureDiscovery {
    fn discover(&self) -> Result<Vec<DesktopPackage>, DesktopBoundaryError> {
        Ok(self.packages.clone())
    }
}

#[derive(Debug)]
struct FixtureScanner {
    scans: Mutex<VecDeque<ConsumerScan>>,
    package_scans: Mutex<Vec<Vec<PathBuf>>>,
}

impl ConsumerScanner for FixtureScanner {
    fn scan(&self) -> ConsumerScan {
        let mut scans = self.scans.lock().expect("scan fixture lock");
        if scans.len() > 1 {
            scans.pop_front().expect("fixture scan")
        } else {
            scans.front().cloned().expect("fixture scan")
        }
    }

    fn scan_for_packages(&self, packages: &[DesktopPackage]) -> ConsumerScan {
        self.package_scans
            .lock()
            .expect("package scan fixture lock")
            .push(
                packages
                    .iter()
                    .map(|package| package.install_location.clone())
                    .collect(),
            );
        self.scan()
    }
}

#[derive(Debug)]
struct FixtureActivator {
    result: Result<(), DesktopBoundaryError>,
    aumids: Mutex<Vec<String>>,
}

#[derive(Debug, Default)]
struct FixtureProcessController {
    graceful_requests: Mutex<Vec<Vec<gpteasy_lib::consumer::ConsumerIdentity>>>,
    forced_requests: Mutex<Vec<Vec<gpteasy_lib::consumer::ConsumerIdentity>>>,
}

impl DesktopProcessController for FixtureProcessController {
    fn request_close(
        &self,
        roots: &[gpteasy_lib::consumer::ConsumerIdentity],
    ) -> Result<(), DesktopBoundaryError> {
        self.graceful_requests
            .lock()
            .expect("graceful requests fixture lock")
            .push(roots.to_vec());
        Ok(())
    }

    fn force_terminate(
        &self,
        roots: &[gpteasy_lib::consumer::ConsumerIdentity],
    ) -> Result<(), DesktopBoundaryError> {
        self.forced_requests
            .lock()
            .expect("forced requests fixture lock")
            .push(roots.to_vec());
        Ok(())
    }
}

#[derive(Debug)]
struct FixtureClock(u64);

impl DesktopClock for FixtureClock {
    fn now_epoch_millis(&self) -> u64 {
        self.0
    }
}

impl DesktopActivator for FixtureActivator {
    fn activate(&self, aumid: &str) -> Result<(), DesktopBoundaryError> {
        self.aumids
            .lock()
            .expect("activation fixture lock")
            .push(aumid.to_owned());
        self.result
    }
}

fn package(name: &str, family: &str, application_id: &str) -> DesktopPackage {
    DesktopPackage {
        name: name.to_owned(),
        family_name: family.to_owned(),
        application_id: application_id.to_owned(),
        install_location: PathBuf::from(format!(
            r"C:\Program Files\WindowsApps\{name}_1.2.3_x64__publisher"
        )),
    }
}

fn fixture_process(
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

fn scan(status: ConsumerStatus, roots: &[(u32, u64)]) -> ConsumerScan {
    let desktop_roots = roots
        .iter()
        .map(
            |(pid, started_at_epoch_millis)| gpteasy_lib::consumer::ConsumerIdentity {
                role: ConsumerRole::Desktop,
                pid: *pid,
                started_at_epoch_millis: *started_at_epoch_millis,
            },
        )
        .collect::<Vec<_>>();
    ConsumerScan {
        desktop: status,
        cli: ConsumerStatus::Stopped,
        identities: desktop_roots.clone(),
        desktop_roots,
    }
}

fn scan_with_cli(status: ConsumerStatus, roots: &[(u32, u64)], cli: (u32, u64)) -> ConsumerScan {
    let mut result = scan(status, roots);
    result.cli = ConsumerStatus::Running;
    result
        .identities
        .push(gpteasy_lib::consumer::ConsumerIdentity {
            role: ConsumerRole::Cli,
            pid: cli.0,
            started_at_epoch_millis: cli.1,
        });
    result
}

fn application(
    packages: Vec<DesktopPackage>,
    scans: Vec<ConsumerScan>,
    activation: Result<(), DesktopBoundaryError>,
) -> (DesktopApplication, Arc<FixtureActivator>) {
    let (application, activator, _) = application_with_controller(packages, scans, activation);
    (application, activator)
}

fn application_with_controller(
    packages: Vec<DesktopPackage>,
    scans: Vec<ConsumerScan>,
    activation: Result<(), DesktopBoundaryError>,
) -> (
    DesktopApplication,
    Arc<FixtureActivator>,
    Arc<FixtureProcessController>,
) {
    let activator = Arc::new(FixtureActivator {
        result: activation,
        aumids: Mutex::new(Vec::new()),
    });
    let process_controller = Arc::new(FixtureProcessController::default());
    let application = DesktopApplication::with_boundaries(
        Arc::new(FixtureDiscovery { packages }),
        Arc::new(FixtureScanner {
            scans: Mutex::new(scans.into()),
            package_scans: Mutex::new(Vec::new()),
        }),
        activator.clone(),
        process_controller.clone(),
        Arc::new(FixtureClock(8_500)),
        3,
        Duration::ZERO,
    );
    (application, activator, process_controller)
}

#[test]
fn missing_installation_disables_start_with_a_stable_reason() {
    let (application, _) =
        application(Vec::new(), vec![scan(ConsumerStatus::Stopped, &[])], Ok(()));

    let snapshot = application.inspect();

    assert_eq!(snapshot.status, ConsumerStatus::Stopped);
    assert_eq!(snapshot.action, DesktopAction::Unavailable);
    assert_eq!(snapshot.message_id, "desktop.not_installed");
}

#[test]
fn multiple_official_package_candidates_disable_ambiguous_activation() {
    let (application, _) = application(
        vec![
            package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App"),
            package("OpenAI.ChatGPT", "OpenAI.ChatGPT_2p2nqsd0c76g0", "Desktop"),
        ],
        vec![scan(ConsumerStatus::Stopped, &[])],
        Ok(()),
    );

    let snapshot = application.inspect();

    assert_eq!(snapshot.action, DesktopAction::Unavailable);
    assert_eq!(snapshot.message_id, "desktop.ambiguous_installation");
}

#[test]
fn multiple_desktop_entries_in_one_package_are_not_reported_as_multiple_packages() {
    let (application, _) = application(
        vec![
            package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "ChatGPT"),
            package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "Codex"),
        ],
        vec![scan(ConsumerStatus::Stopped, &[])],
        Ok(()),
    );

    let snapshot = application.inspect();

    assert_eq!(snapshot.action, DesktopAction::Unavailable);
    assert_eq!(snapshot.message_id, "desktop.discovery_failed");
}

#[test]
fn same_named_package_from_another_publisher_is_not_treated_as_openai() {
    let (application, activator) = application(
        vec![package(
            "OpenAI.Codex",
            "OpenAI.Codex_untrustedpublisher",
            "App",
        )],
        vec![scan(ConsumerStatus::Stopped, &[])],
        Ok(()),
    );

    let snapshot = application.inspect();

    assert_eq!(snapshot.action, DesktopAction::Unavailable);
    assert_eq!(snapshot.message_id, "desktop.discovery_failed");
    assert!(
        activator
            .aumids
            .lock()
            .expect("activation fixture lock")
            .is_empty()
    );
}

#[test]
fn start_uses_the_discovered_aumid_and_waits_for_a_new_trusted_root() {
    let (application, activator) = application(
        vec![package(
            "OpenAI.Codex",
            "OpenAI.Codex_2p2nqsd0c76g0",
            "CodexDesktop",
        )],
        vec![
            scan(ConsumerStatus::Stopped, &[]),
            scan(ConsumerStatus::Stopped, &[]),
            scan(ConsumerStatus::Running, &[(420, 9_000)]),
        ],
        Ok(()),
    );

    let result = application.start().expect("trusted desktop launch");

    assert_eq!(result.status, ConsumerStatus::Running);
    assert_eq!(result.action, DesktopAction::Restart);
    assert_eq!(result.message_id, "desktop.running");
    assert_eq!(
        *activator.aumids.lock().expect("activation fixture lock"),
        vec!["OpenAI.Codex_2p2nqsd0c76g0!CodexDesktop"]
    );
}

#[test]
fn activation_failure_returns_a_stable_failure_without_reporting_success() {
    let (application, _) = application(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![scan(ConsumerStatus::Stopped, &[])],
        Err(DesktopBoundaryError),
    );

    let failure = application.start().expect_err("activation must fail");

    assert_eq!(failure.category, DesktopFailureCategory::ActivationFailed);
    assert_eq!(failure.message_id, "desktop.activation_failed");
}

#[test]
fn pid_reuse_or_an_untrusted_third_party_process_cannot_prove_launch_success() {
    let previous = scan(ConsumerStatus::Stopped, &[]);
    let reused_pid_without_new_start = scan(ConsumerStatus::Running, &[(420, 8_000)]);
    let (application, _) = application(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            previous,
            reused_pid_without_new_start.clone(),
            reused_pid_without_new_start,
        ],
        Ok(()),
    );

    let failure = application.start().expect_err("no new trusted root");

    assert_eq!(failure.category, DesktopFailureCategory::LaunchNotObserved);
    assert_eq!(failure.message_id, "desktop.launch_not_observed");
}

#[test]
fn running_official_desktop_can_be_gracefully_closed_and_reactivated() {
    let old = (420, 8_000);
    let (application, activator, controller) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            scan(ConsumerStatus::Running, &[old]),
            scan(ConsumerStatus::Stopped, &[]),
            scan(ConsumerStatus::Running, &[(421, 9_000)]),
        ],
        Ok(()),
    );

    let result = application
        .restart(&scan(ConsumerStatus::Running, &[old]).desktop_roots)
        .expect("graceful restart");

    assert_eq!(result.status, DesktopRestartStatus::Restarted);
    assert_eq!(result.message_id, "desktop.restart_succeeded");
    assert!(result.desktop_identities.is_empty());
    assert_eq!(
        controller
            .graceful_requests
            .lock()
            .expect("graceful requests fixture lock")[0][0]
            .pid,
        old.0
    );
    assert!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")
            .is_empty()
    );
    assert_eq!(
        *activator.aumids.lock().expect("activation fixture lock"),
        vec!["OpenAI.Codex_2p2nqsd0c76g0!App"]
    );
}

#[test]
fn graceful_close_timeout_requires_a_second_confirmation_without_forcing() {
    let old = (420, 8_000);
    let still_running = scan(ConsumerStatus::Running, &[old]);
    let (application, activator, controller) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            still_running.clone(),
            still_running.clone(),
            still_running.clone(),
        ],
        Ok(()),
    );

    let result = application
        .restart(&scan(ConsumerStatus::Running, &[old]).desktop_roots)
        .expect("timeout is an explicit result");

    assert_eq!(result.status, DesktopRestartStatus::CloseTimedOut);
    assert_eq!(result.message_id, "desktop.close_timed_out");
    assert_eq!(result.desktop_identities[0].pid, old.0);
    assert!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")
            .is_empty()
    );
    assert!(
        activator
            .aumids
            .lock()
            .expect("activation fixture lock")
            .is_empty()
    );
}

#[test]
fn an_untrusted_scan_cannot_prove_that_the_old_desktop_tree_exited() {
    let old = (420, 8_000);
    let (application, activator, controller) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            scan(ConsumerStatus::Running, &[old]),
            scan(ConsumerStatus::Unknown, &[]),
            scan(ConsumerStatus::Unknown, &[]),
        ],
        Ok(()),
    );

    let result = application
        .restart(&scan(ConsumerStatus::Running, &[old]).desktop_roots)
        .expect("untrusted exit check is an explicit timeout");

    assert_eq!(result.status, DesktopRestartStatus::CloseTimedOut);
    assert!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")
            .is_empty()
    );
    assert!(
        activator
            .aumids
            .lock()
            .expect("activation fixture lock")
            .is_empty()
    );
}

#[test]
fn a_new_trusted_root_does_not_keep_the_old_tree_in_the_close_timeout() {
    let old = (420, 8_000);
    let (application, activator, _) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            scan(ConsumerStatus::Running, &[old]),
            scan(ConsumerStatus::Running, &[(421, 9_000)]),
            scan(ConsumerStatus::Running, &[(421, 9_000)]),
        ],
        Ok(()),
    );

    let result = application
        .restart(&scan(ConsumerStatus::Running, &[old]).desktop_roots)
        .expect("old tree exited before activation");

    assert_eq!(result.status, DesktopRestartStatus::Restarted);
    assert_eq!(
        *activator.aumids.lock().expect("activation fixture lock"),
        vec!["OpenAI.Codex_2p2nqsd0c76g0!App"]
    );
}

#[test]
fn restart_does_not_report_success_without_a_new_trusted_root() {
    let old = (420, 8_000);
    let old_started_root = scan(ConsumerStatus::Running, &[(421, 8_200)]);
    let (application, _, _) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            scan(ConsumerStatus::Running, &[old]),
            scan(ConsumerStatus::Stopped, &[]),
            old_started_root.clone(),
            old_started_root.clone(),
            old_started_root,
        ],
        Ok(()),
    );

    let failure = application
        .restart(&scan(ConsumerStatus::Running, &[old]).desktop_roots)
        .expect_err("activation requires a newly started trusted root");

    assert_eq!(failure.category, DesktopFailureCategory::LaunchNotObserved);
    assert_eq!(failure.message_id, "desktop.launch_not_observed");
}

#[test]
fn graceful_restart_rejects_identity_changes_after_the_first_confirmation() {
    let expected = scan(ConsumerStatus::Running, &[(420, 8_000)]).desktop_roots;
    let (application, activator, controller) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![scan(ConsumerStatus::Running, &[(420, 8_700)])],
        Ok(()),
    );

    let failure = application
        .restart(&expected)
        .expect_err("changed PID identity must fail closed");

    assert_eq!(failure.category, DesktopFailureCategory::IdentityChanged);
    assert_eq!(failure.message_id, "desktop.identity_changed");
    assert!(
        controller
            .graceful_requests
            .lock()
            .expect("graceful requests fixture lock")
            .is_empty()
    );
    assert!(
        activator
            .aumids
            .lock()
            .expect("activation fixture lock")
            .is_empty()
    );
}

#[test]
fn confirmed_force_restart_rechecks_identity_then_terminates_and_reactivates() {
    let old = (420, 8_000);
    let expected = scan(ConsumerStatus::Running, &[old]).desktop_roots;
    let still_running = scan(ConsumerStatus::Running, &[old]);
    let (application, activator, controller) = application_with_controller(
        vec![package(
            "OpenAI.ChatGPT",
            "OpenAI.ChatGPT_2p2nqsd0c76g0",
            "App",
        )],
        vec![
            still_running.clone(),
            still_running.clone(),
            still_running.clone(),
            still_running.clone(),
            still_running,
            scan(ConsumerStatus::Running, &[old]),
            scan(ConsumerStatus::Stopped, &[]),
            scan(ConsumerStatus::Running, &[(522, 9_100)]),
        ],
        Ok(()),
    );

    let timed_out = application
        .restart(&expected)
        .expect("normal close timeout before force confirmation");
    let result = application
        .force_restart(
            timed_out
                .force_authorization
                .as_deref()
                .expect("force authorization"),
        )
        .expect("confirmed force restart");

    assert_eq!(result.status, DesktopRestartStatus::Restarted);
    assert_eq!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")[0],
        expected
    );
    assert_eq!(
        *activator.aumids.lock().expect("activation fixture lock"),
        vec!["OpenAI.ChatGPT_2p2nqsd0c76g0!App"]
    );
}

#[test]
fn force_restart_rejects_pid_reuse_before_any_termination() {
    let expected = scan(ConsumerStatus::Running, &[(420, 8_000)]).desktop_roots;
    let still_running = scan(ConsumerStatus::Running, &[(420, 8_000)]);
    let (application, activator, controller) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            still_running.clone(),
            still_running.clone(),
            still_running.clone(),
            still_running,
            scan(ConsumerStatus::Running, &[(420, 8_700)]),
        ],
        Ok(()),
    );

    let timed_out = application
        .restart(&expected)
        .expect("normal close timeout before PID reuse");
    let failure = application
        .force_restart(
            timed_out
                .force_authorization
                .as_deref()
                .expect("force authorization"),
        )
        .expect_err("reused PID must fail closed");

    assert_eq!(failure.category, DesktopFailureCategory::IdentityChanged);
    assert_eq!(failure.message_id, "desktop.identity_changed");
    assert!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")
            .is_empty()
    );
    assert!(
        activator
            .aumids
            .lock()
            .expect("activation fixture lock")
            .is_empty()
    );
}

#[test]
fn force_restart_never_passes_an_independent_cli_to_the_process_controller() {
    let current = scan_with_cli(ConsumerStatus::Running, &[(420, 8_000)], (900, 7_000));
    let expected = current.desktop_roots.clone();
    let (application, _, controller) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            current,
            scan_with_cli(ConsumerStatus::Stopped, &[], (900, 7_000)),
            scan_with_cli(ConsumerStatus::Running, &[(421, 9_000)], (900, 7_000)),
        ],
        Ok(()),
    );

    let timed_out = application
        .restart(&expected)
        .expect("normal close timeout with independent CLI");
    application
        .force_restart(
            timed_out
                .force_authorization
                .as_deref()
                .expect("force authorization"),
        )
        .expect("desktop restart with independent CLI");

    assert_eq!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")[0],
        expected
    );
}

#[test]
fn force_restart_never_passes_a_similarly_named_third_party_process_to_the_controller() {
    let package = package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App");
    let install_locations = [package.install_location.clone()];
    let third_party = fixture_process(
        900,
        1,
        7_000,
        "Codex++.exe",
        r"C:\Users\example\AppData\Local\CodexPlusPlus\Codex++.exe",
    );
    let current = classify_fixture_for_packages(
        &[
            fixture_process(
                420,
                1,
                8_000,
                "Codex.exe",
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__publisher\Codex.exe",
            ),
            fixture_process(
                421,
                420,
                8_100,
                "codex.exe",
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__publisher\resources\codex\codex.exe",
            ),
            third_party.clone(),
        ],
        &install_locations,
    );
    let stopped = classify_fixture_for_packages(&[third_party.clone()], &install_locations);
    let started = classify_fixture_for_packages(
        &[
            fixture_process(
                422,
                1,
                9_000,
                "Codex.exe",
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__publisher\Codex.exe",
            ),
            third_party,
        ],
        &install_locations,
    );
    let expected_roots = current.desktop_roots.clone();
    let (application, _, controller) = application_with_controller(
        vec![package],
        vec![
            current.clone(),
            current.clone(),
            current.clone(),
            current.clone(),
            current,
            stopped,
            started,
        ],
        Ok(()),
    );

    let timed_out = application
        .restart(&expected_roots)
        .expect("normal close timeout with a third-party process");
    let expected_tree = timed_out.desktop_identities.clone();
    application
        .force_restart(
            timed_out
                .force_authorization
                .as_deref()
                .expect("force authorization"),
        )
        .expect("desktop restart excludes the third-party process");

    assert_eq!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")[0],
        expected_tree
    );
    assert!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")[0]
            .iter()
            .all(|identity| identity.pid != 900)
    );
}

#[test]
fn force_restart_cannot_be_called_without_a_normal_close_timeout() {
    let (application, activator, controller) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![scan(ConsumerStatus::Running, &[(420, 8_000)])],
        Ok(()),
    );

    let failure = application
        .force_restart("not-issued")
        .expect_err("force restart requires a backend authorization");

    assert_eq!(failure.category, DesktopFailureCategory::ActionUnavailable);
    assert_eq!(failure.message_id, "desktop.force_not_authorized");
    assert!(
        controller
            .forced_requests
            .lock()
            .expect("forced requests fixture lock")
            .is_empty()
    );
    assert!(
        activator
            .aumids
            .lock()
            .expect("activation fixture lock")
            .is_empty()
    );
}

#[test]
fn force_authorization_is_consumed_before_process_control() {
    let old = (420, 8_000);
    let expected = scan(ConsumerStatus::Running, &[old]).desktop_roots;
    let still_running = scan(ConsumerStatus::Running, &[old]);
    let (application, _, _) = application_with_controller(
        vec![package("OpenAI.Codex", "OpenAI.Codex_2p2nqsd0c76g0", "App")],
        vec![
            still_running.clone(),
            still_running.clone(),
            still_running.clone(),
            still_running.clone(),
            still_running,
            scan(ConsumerStatus::Stopped, &[]),
            scan(ConsumerStatus::Running, &[(421, 9_000)]),
        ],
        Ok(()),
    );
    let timed_out = application
        .restart(&expected)
        .expect("normal close timeout before force confirmation");
    let authorization = timed_out.force_authorization.expect("force authorization");

    application
        .force_restart(&authorization)
        .expect("first confirmed force restart");
    let replay = application
        .force_restart(&authorization)
        .expect_err("force authorization is one-time");

    assert_eq!(replay.message_id, "desktop.force_not_authorized");
}
