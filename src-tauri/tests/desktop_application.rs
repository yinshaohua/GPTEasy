use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpteasy_lib::consumer::{
    ConsumerRole, ConsumerScan, ConsumerScanner, ConsumerStatus, DesktopAction, DesktopActivator,
    DesktopApplication, DesktopBoundaryError, DesktopClock, DesktopFailureCategory, DesktopPackage,
    DesktopPackageDiscovery,
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

fn application(
    packages: Vec<DesktopPackage>,
    scans: Vec<ConsumerScan>,
    activation: Result<(), DesktopBoundaryError>,
) -> (DesktopApplication, Arc<FixtureActivator>) {
    let activator = Arc::new(FixtureActivator {
        result: activation,
        aumids: Mutex::new(Vec::new()),
    });
    let application = DesktopApplication::with_boundaries(
        Arc::new(FixtureDiscovery { packages }),
        Arc::new(FixtureScanner {
            scans: Mutex::new(scans.into()),
            package_scans: Mutex::new(Vec::new()),
        }),
        activator.clone(),
        Arc::new(FixtureClock(8_500)),
        3,
        Duration::ZERO,
    );
    (application, activator)
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
    assert_eq!(result.action, DesktopAction::Unavailable);
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
