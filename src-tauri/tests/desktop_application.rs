use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpteasy_lib::consumer::{
    ConsumerIdentity, ConsumerRole, ConsumerScan, ConsumerScanner, ConsumerStatus,
};
use gpteasy_lib::desktop::{
    DesktopAction, DesktopActivator, DesktopApplication, DesktopBoundaryError, DesktopClock,
    DesktopFailureCategory, DesktopPackage, DesktopPackageDiscovery, DesktopProcessController,
};

struct FixtureDiscovery(Vec<DesktopPackage>);

impl DesktopPackageDiscovery for FixtureDiscovery {
    fn discover(&self) -> Result<Vec<DesktopPackage>, DesktopBoundaryError> {
        Ok(self.0.clone())
    }
}

struct FixtureScanner(Mutex<VecDeque<ConsumerScan>>);

impl ConsumerScanner for FixtureScanner {
    fn scan(&self) -> ConsumerScan {
        self.0
            .lock()
            .expect("scanner fixture lock")
            .pop_front()
            .unwrap_or_else(ConsumerScan::unknown)
    }

    fn scan_for_install_locations(&self, _install_locations: &[PathBuf]) -> ConsumerScan {
        self.scan()
    }
}

#[derive(Default)]
struct FixtureActivator {
    aumids: Mutex<Vec<String>>,
    fail: bool,
}

impl DesktopActivator for FixtureActivator {
    fn activate(&self, aumid: &str) -> Result<(), DesktopBoundaryError> {
        self.aumids
            .lock()
            .expect("activator fixture lock")
            .push(aumid.to_owned());
        if self.fail {
            Err(DesktopBoundaryError)
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct FixtureController {
    requests: Mutex<Vec<Vec<ConsumerIdentity>>>,
    terminations: Mutex<Vec<Vec<ConsumerIdentity>>>,
    close_fails: AtomicBool,
    termination_fails: AtomicBool,
}

impl DesktopProcessController for FixtureController {
    fn request_close(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
        self.requests
            .lock()
            .expect("controller fixture lock")
            .push(roots.to_vec());
        if self.close_fails.load(Ordering::SeqCst) {
            Err(DesktopBoundaryError)
        } else {
            Ok(())
        }
    }

    fn terminate_tree(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
        self.terminations
            .lock()
            .expect("controller fixture lock")
            .push(roots.to_vec());
        if self.termination_fails.load(Ordering::SeqCst) {
            Err(DesktopBoundaryError)
        } else {
            Ok(())
        }
    }
}

struct FixtureClock(u64);

impl DesktopClock for FixtureClock {
    fn now_epoch_millis(&self) -> u64 {
        self.0
    }
}

fn package() -> DesktopPackage {
    DesktopPackage {
        name: "OpenAI.Codex".to_owned(),
        family_name: "OpenAI.Codex_2p2nqsd0c76g0".to_owned(),
        application_id: "App".to_owned(),
        install_location: PathBuf::from(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__2p2nqsd0c76g0",
        ),
    }
}

fn identity(role: ConsumerRole, pid: u32, started_at_epoch_millis: u64) -> ConsumerIdentity {
    ConsumerIdentity {
        role,
        pid,
        started_at_epoch_millis,
    }
}

fn scan(status: ConsumerStatus, roots: &[(u32, u64)], cli: Option<(u32, u64)>) -> ConsumerScan {
    let desktop_roots = roots
        .iter()
        .map(|(pid, started)| identity(ConsumerRole::Desktop, *pid, *started))
        .collect::<Vec<_>>();
    let mut identities = desktop_roots.clone();
    if let Some((pid, started)) = cli {
        identities.push(identity(ConsumerRole::Cli, pid, started));
    }
    ConsumerScan {
        desktop: status,
        cli: if cli.is_some() {
            ConsumerStatus::Running
        } else {
            ConsumerStatus::Stopped
        },
        identities,
        desktop_roots,
    }
}

fn application(
    packages: Vec<DesktopPackage>,
    scans: Vec<ConsumerScan>,
    activation_fails: bool,
) -> (
    DesktopApplication,
    Arc<FixtureActivator>,
    Arc<FixtureController>,
) {
    application_with_polling(packages, scans, activation_fails, 3, 3)
}

fn application_with_polling(
    packages: Vec<DesktopPackage>,
    scans: Vec<ConsumerScan>,
    activation_fails: bool,
    close_scan_attempts: usize,
    launch_scan_attempts: usize,
) -> (
    DesktopApplication,
    Arc<FixtureActivator>,
    Arc<FixtureController>,
) {
    let activator = Arc::new(FixtureActivator {
        fail: activation_fails,
        ..FixtureActivator::default()
    });
    let controller = Arc::new(FixtureController::default());
    (
        DesktopApplication::with_polling_boundaries(
            Arc::new(FixtureDiscovery(packages)),
            Arc::new(FixtureScanner(Mutex::new(scans.into()))),
            activator.clone(),
            controller.clone(),
            Arc::new(FixtureClock(9_000)),
            close_scan_attempts,
            launch_scan_attempts,
            Duration::ZERO,
        ),
        activator,
        controller,
    )
}

#[test]
fn stopped_trusted_desktop_exposes_start_action() {
    let (application, _, _) = application(
        vec![package()],
        vec![scan(ConsumerStatus::Stopped, &[], None)],
        false,
    );

    let snapshot = application.inspect();

    assert_eq!(snapshot.status, ConsumerStatus::Stopped);
    assert_eq!(snapshot.action, DesktopAction::Start);
}

#[test]
fn untrusted_publisher_package_is_rejected() {
    let mut package = package();
    package.family_name = "OpenAI.Codex_untrusted".to_owned();
    let (application, _, _) = application(vec![package], Vec::new(), false);

    let snapshot = application.inspect();

    assert_eq!(snapshot.action, DesktopAction::Unavailable);
    assert_eq!(snapshot.message_id, "desktop.discovery_failed");
}

#[test]
fn start_activates_only_the_discovered_package_and_observes_a_new_root() {
    let (application, activator, _) = application(
        vec![package()],
        vec![
            scan(ConsumerStatus::Stopped, &[], None),
            scan(ConsumerStatus::Running, &[(421, 9_100)], None),
        ],
        false,
    );

    let snapshot = application.start().expect("trusted desktop start");

    assert_eq!(snapshot.status, ConsumerStatus::Running);
    assert_eq!(snapshot.action, DesktopAction::Restart);
    assert_eq!(
        *activator.aumids.lock().expect("activator fixture lock"),
        vec!["OpenAI.Codex_2p2nqsd0c76g0!App"]
    );
}

#[test]
fn failed_activation_never_reports_success() {
    let (application, _, _) = application(
        vec![package()],
        vec![scan(ConsumerStatus::Stopped, &[], None)],
        true,
    );

    let failure = application.start().expect_err("activation must fail");

    assert_eq!(failure.category, DesktopFailureCategory::ActivationFailed);
    assert_eq!(failure.message_id, "desktop.activation_failed");
}

#[test]
fn restart_requests_normal_close_then_observes_a_new_root() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let (application, activator, controller) = application(
        vec![package()],
        vec![
            old,
            scan(ConsumerStatus::Stopped, &[], None),
            scan(ConsumerStatus::Running, &[(421, 9_100)], None),
        ],
        false,
    );

    let snapshot = application
        .restart(&expected_roots)
        .expect("graceful desktop restart");

    assert_eq!(snapshot.status, ConsumerStatus::Running);
    assert_eq!(snapshot.message_id, "desktop.restarted_after_normal_exit");
    assert_eq!(
        controller.requests.lock().expect("controller fixture lock")[0],
        expected_roots
    );
    assert_eq!(
        activator
            .aumids
            .lock()
            .expect("activator fixture lock")
            .len(),
        1
    );
    assert!(
        controller
            .terminations
            .lock()
            .expect("controller fixture lock")
            .is_empty()
    );
}

#[test]
fn restart_checkpoint_runs_after_the_old_roots_exit_and_before_activation() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let (application, activator, controller) = application(
        vec![package()],
        vec![
            old,
            scan(ConsumerStatus::Stopped, &[], None),
            scan(ConsumerStatus::Running, &[(421, 9_100)], None),
        ],
        false,
    );
    let checkpoint_ran = Arc::new(AtomicBool::new(false));
    let observed_checkpoint = checkpoint_ran.clone();
    let observed_activator = activator.clone();
    let observed_controller = controller.clone();

    application
        .restart_with_checkpoint(&expected_roots, move || {
            assert_eq!(
                observed_controller
                    .requests
                    .lock()
                    .expect("controller fixture lock")
                    .len(),
                1,
                "the trusted root has already received its close request",
            );
            assert!(
                observed_activator
                    .aumids
                    .lock()
                    .expect("activator fixture lock")
                    .is_empty(),
                "the replacement desktop must not be activated before coordination",
            );
            observed_checkpoint.store(true, Ordering::SeqCst);
            Ok(())
        })
        .expect("restart through checkpoint");

    assert!(checkpoint_ran.load(Ordering::SeqCst));
    assert_eq!(
        activator
            .aumids
            .lock()
            .expect("activator fixture lock")
            .len(),
        1
    );
}

#[test]
fn restart_checkpoint_can_block_activation_after_a_confirmed_exit() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let (application, activator, _) = application(
        vec![package()],
        vec![old, scan(ConsumerStatus::Stopped, &[], None)],
        false,
    );

    let failure = application
        .restart_with_checkpoint(&expected_roots, || {
            Err(gpteasy_lib::desktop::DesktopFailure {
                category: DesktopFailureCategory::ActionUnavailable,
                message_id: "session_visibility.recovery_indeterminate",
            })
        })
        .expect_err("indeterminate coordination blocks activation");

    assert_eq!(
        failure.message_id,
        "session_visibility.recovery_indeterminate"
    );
    assert!(
        activator
            .aumids
            .lock()
            .expect("activator fixture lock")
            .is_empty()
    );
}

#[test]
fn restart_starts_when_the_expected_desktop_has_already_exited() {
    let expected_roots = scan(ConsumerStatus::Running, &[(420, 8_000)], None).desktop_roots;
    let (application, activator, controller) = application(
        vec![package()],
        vec![
            scan(ConsumerStatus::Stopped, &[], None),
            scan(ConsumerStatus::Running, &[(421, 9_100)], None),
        ],
        false,
    );

    let snapshot = application
        .restart(&expected_roots)
        .expect("an already stopped desktop should be started");

    assert_eq!(snapshot.status, ConsumerStatus::Running);
    assert!(
        controller
            .requests
            .lock()
            .expect("controller fixture lock")
            .is_empty()
    );
    assert_eq!(
        activator
            .aumids
            .lock()
            .expect("activator fixture lock")
            .len(),
        1
    );
}

#[test]
fn restart_keeps_waiting_after_the_previous_close_window() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let mut scans = vec![old.clone()];
    scans.extend(std::iter::repeat_n(old, 20));
    scans.push(scan(ConsumerStatus::Stopped, &[], None));
    scans.push(scan(ConsumerStatus::Running, &[(421, 9_100)], None));
    let (application, activator, _) =
        application_with_polling(vec![package()], scans, false, 60, 3);

    let snapshot = application
        .restart(&expected_roots)
        .expect("a slow graceful exit should still restart");

    assert_eq!(snapshot.status, ConsumerStatus::Running);
    assert_eq!(
        activator
            .aumids
            .lock()
            .expect("activator fixture lock")
            .len(),
        1
    );
}

#[test]
fn restart_terminates_the_same_trusted_tree_when_normal_close_does_not_exit() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let (application, activator, controller) = application(
        vec![package()],
        vec![
            old.clone(),
            old.clone(),
            old.clone(),
            old.clone(),
            old,
            scan(ConsumerStatus::Stopped, &[], None),
            scan(ConsumerStatus::Running, &[(421, 9_100)], None),
        ],
        false,
    );

    let snapshot = application
        .restart(&expected_roots)
        .expect("confirmed restart should terminate a tray-resident desktop");

    assert_eq!(snapshot.message_id, "desktop.restarted_after_termination");
    assert_eq!(
        *controller
            .terminations
            .lock()
            .expect("controller fixture lock"),
        vec![expected_roots]
    );
    assert_eq!(
        activator
            .aumids
            .lock()
            .expect("activator fixture lock")
            .len(),
        1
    );
}

#[test]
fn restart_termination_failure_does_not_reactivate() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let (application, activator, controller) = application(
        vec![package()],
        vec![old.clone(), old.clone(), old.clone(), old.clone(), old],
        false,
    );
    controller.termination_fails.store(true, Ordering::SeqCst);

    let failure = application
        .restart(&expected_roots)
        .expect_err("failed termination must stop the restart");

    assert_eq!(failure.category, DesktopFailureCategory::TerminationFailed);
    assert_eq!(failure.message_id, "desktop.termination_failed");
    assert!(
        activator
            .aumids
            .lock()
            .expect("activator fixture lock")
            .is_empty()
    );
}

#[test]
fn restart_termination_timeout_does_not_reactivate() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let (application, activator, _) = application(
        vec![package()],
        vec![
            old.clone(),
            old.clone(),
            old.clone(),
            old.clone(),
            old.clone(),
            old.clone(),
            old.clone(),
            old,
        ],
        false,
    );

    let failure = application
        .restart(&expected_roots)
        .expect_err("a root that survives termination must fail");

    assert_eq!(
        failure.category,
        DesktopFailureCategory::TerminationTimedOut
    );
    assert_eq!(failure.message_id, "desktop.termination_timed_out");
    assert!(
        activator
            .aumids
            .lock()
            .expect("activator fixture lock")
            .is_empty()
    );
}

#[test]
fn restart_still_terminates_when_the_normal_close_request_is_unavailable() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let (application, _, controller) = application(
        vec![package()],
        vec![
            old.clone(),
            old,
            scan(ConsumerStatus::Stopped, &[], None),
            scan(ConsumerStatus::Running, &[(421, 9_100)], None),
        ],
        false,
    );
    controller.close_fails.store(true, Ordering::SeqCst);

    let snapshot = application
        .restart(&expected_roots)
        .expect("confirmed termination must not depend on a close window");

    assert_eq!(snapshot.message_id, "desktop.restarted_after_termination");
    assert_eq!(
        controller
            .terminations
            .lock()
            .expect("controller fixture lock")[0],
        expected_roots
    );
}

#[test]
fn restart_rejects_identity_changes_before_requesting_close() {
    let expected_roots = scan(ConsumerStatus::Running, &[(420, 8_000)], None).desktop_roots;
    let (application, _, controller) = application(
        vec![package()],
        vec![scan(ConsumerStatus::Running, &[(420, 8_700)], None)],
        false,
    );

    let failure = application
        .restart(&expected_roots)
        .expect_err("changed process identity must fail closed");

    assert_eq!(failure.category, DesktopFailureCategory::IdentityChanged);
    assert!(
        controller
            .requests
            .lock()
            .expect("controller fixture lock")
            .is_empty()
    );
}

#[test]
fn restart_rejects_identity_changes_before_termination() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], None);
    let expected_roots = old.desktop_roots.clone();
    let (application, _, controller) = application(
        vec![package()],
        vec![
            old.clone(),
            old.clone(),
            old.clone(),
            old,
            scan(ConsumerStatus::Running, &[(421, 9_100)], None),
        ],
        false,
    );

    let failure = application
        .restart(&expected_roots)
        .expect_err("changed identity must never be terminated");

    assert_eq!(failure.category, DesktopFailureCategory::IdentityChanged);
    assert!(
        controller
            .terminations
            .lock()
            .expect("controller fixture lock")
            .is_empty()
    );
}

#[test]
fn restart_never_passes_independent_cli_to_process_controller() {
    let old = scan(ConsumerStatus::Running, &[(420, 8_000)], Some((900, 7_000)));
    let expected_roots = old.desktop_roots.clone();
    let (application, _, controller) = application(
        vec![package()],
        vec![
            old.clone(),
            old.clone(),
            old.clone(),
            old.clone(),
            old,
            scan(ConsumerStatus::Stopped, &[], Some((900, 7_000))),
            scan(ConsumerStatus::Running, &[(421, 9_100)], Some((900, 7_000))),
        ],
        false,
    );

    application
        .restart(&expected_roots)
        .expect("desktop restart with independent CLI");

    let requests = controller.requests.lock().expect("controller fixture lock");
    assert_eq!(requests[0], expected_roots);
    assert!(
        requests[0]
            .iter()
            .all(|item| item.role == ConsumerRole::Desktop)
    );
    let terminations = controller
        .terminations
        .lock()
        .expect("controller fixture lock");
    assert_eq!(terminations[0], expected_roots);
    assert!(
        terminations[0]
            .iter()
            .all(|item| item.role == ConsumerRole::Desktop)
    );
}
