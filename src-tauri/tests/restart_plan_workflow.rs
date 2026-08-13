use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpteasy_lib::codex::LoginStatus;
use gpteasy_lib::consumer::{
    ConsumerIdentity, ConsumerRole, ConsumerScan, ConsumerScanner, ConsumerStatus,
    DesktopActivator, DesktopApplication, DesktopBoundaryError, DesktopClock, DesktopPackage,
    DesktopPackageDiscovery, DesktopProcessController,
};
use gpteasy_lib::environment::{
    EnvironmentApplication, EnvironmentFailurePoint, EnvironmentFaultInjector, OpenAiLoginProbe,
    RestartDecision, RestartPlanStatus,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use rusqlite::{Connection, params};
use tempfile::TempDir;

const PROVIDER_ID: &str = "9f319739-f219-48ee-be35-22e08d5402d7";

struct StoppedConsumers;

impl ConsumerScanner for StoppedConsumers {
    fn scan(&self) -> ConsumerScan {
        ConsumerScan {
            desktop: ConsumerStatus::Stopped,
            cli: ConsumerStatus::Stopped,
            identities: Vec::new(),
            desktop_roots: Vec::new(),
        }
    }
}

struct NoPackages;

impl DesktopPackageDiscovery for NoPackages {
    fn discover(&self) -> Result<Vec<DesktopPackage>, DesktopBoundaryError> {
        Ok(Vec::new())
    }
}

struct NoActivation;

impl DesktopActivator for NoActivation {
    fn activate(&self, _: &str) -> Result<(), DesktopBoundaryError> {
        Err(DesktopBoundaryError)
    }
}

struct NoProcessControl;

impl DesktopProcessController for NoProcessControl {
    fn request_close(
        &self,
        _: &[gpteasy_lib::consumer::ConsumerIdentity],
    ) -> Result<(), DesktopBoundaryError> {
        Ok(())
    }

    fn force_terminate(
        &self,
        _: &[gpteasy_lib::consumer::ConsumerIdentity],
    ) -> Result<(), DesktopBoundaryError> {
        Ok(())
    }
}

struct FixedClock;

impl DesktopClock for FixedClock {
    fn now_epoch_millis(&self) -> u64 {
        8_500
    }
}

#[derive(Clone)]
struct MutableScan(Arc<Mutex<ConsumerScan>>);

impl ConsumerScanner for MutableScan {
    fn scan(&self) -> ConsumerScan {
        self.0.lock().expect("scan lock").clone()
    }
}

struct SequenceScanner {
    scans: Mutex<VecDeque<ConsumerScan>>,
}

impl ConsumerScanner for SequenceScanner {
    fn scan(&self) -> ConsumerScan {
        let mut scans = self.scans.lock().expect("sequence lock");
        if scans.len() > 1 {
            scans.pop_front().expect("sequence scan")
        } else {
            scans.front().cloned().expect("sequence scan")
        }
    }
}

struct PackageDiscovery;

impl DesktopPackageDiscovery for PackageDiscovery {
    fn discover(&self) -> Result<Vec<DesktopPackage>, DesktopBoundaryError> {
        Ok(vec![DesktopPackage {
            name: "OpenAI.Codex".to_owned(),
            family_name: "OpenAI.Codex_2p2nqsd0c76g0".to_owned(),
            application_id: "App".to_owned(),
            install_location: PathBuf::from(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__publisher",
            ),
        }])
    }
}

struct RecordingController {
    close_requests: Mutex<Vec<Vec<ConsumerIdentity>>>,
}

impl DesktopProcessController for RecordingController {
    fn request_close(&self, roots: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
        self.close_requests
            .lock()
            .expect("close requests lock")
            .push(roots.to_vec());
        Ok(())
    }

    fn force_terminate(&self, _: &[ConsumerIdentity]) -> Result<(), DesktopBoundaryError> {
        Ok(())
    }
}

struct UpdatingActivator {
    environment_scan: MutableScan,
    fail: bool,
    keep_cli: bool,
}

impl DesktopActivator for UpdatingActivator {
    fn activate(&self, _: &str) -> Result<(), DesktopBoundaryError> {
        if !self.fail {
            *self.environment_scan.0.lock().expect("scan lock") =
                running_desktop(self.keep_cli, 84, 9_000);
            Ok(())
        } else {
            Err(DesktopBoundaryError)
        }
    }
}

fn running_desktop(cli: bool, pid: u32, started_at_epoch_millis: u64) -> ConsumerScan {
    let desktop = ConsumerIdentity {
        role: ConsumerRole::Desktop,
        pid,
        started_at_epoch_millis,
    };
    let mut identities = vec![desktop.clone()];
    if cli {
        identities.push(ConsumerIdentity {
            role: ConsumerRole::Cli,
            pid: 55,
            started_at_epoch_millis: 7_000,
        });
    }
    ConsumerScan {
        desktop: ConsumerStatus::Running,
        cli: if cli {
            ConsumerStatus::Running
        } else {
            ConsumerStatus::Stopped
        },
        identities,
        desktop_roots: vec![desktop],
    }
}

fn stopped_scan() -> ConsumerScan {
    ConsumerScan {
        desktop: ConsumerStatus::Stopped,
        cli: ConsumerStatus::Stopped,
        identities: Vec::new(),
        desktop_roots: Vec::new(),
    }
}

fn desktop_fixture(
    environment_scan: MutableScan,
    initial: ConsumerScan,
    activation: Result<(), DesktopBoundaryError>,
) -> (DesktopApplication, Arc<RecordingController>) {
    let keep_cli = initial.cli == ConsumerStatus::Running;
    let controller = Arc::new(RecordingController {
        close_requests: Mutex::new(Vec::new()),
    });
    let new_desktop = running_desktop(false, 84, 9_000);
    let desktop = DesktopApplication::with_boundaries(
        Arc::new(PackageDiscovery),
        Arc::new(SequenceScanner {
            scans: Mutex::new(vec![initial, stopped_scan(), new_desktop].into()),
        }),
        Arc::new(UpdatingActivator {
            environment_scan,
            fail: activation.is_err(),
            keep_cli,
        }),
        controller.clone(),
        Arc::new(FixedClock),
        1,
        Duration::ZERO,
    );
    (desktop, controller)
}

struct FailsBeforeConfigWrite;

impl EnvironmentFaultInjector for FailsBeforeConfigWrite {
    fn fails_at(&self, point: EnvironmentFailurePoint) -> bool {
        point == EnvironmentFailurePoint::BeforeConfigReplace
    }
}

struct LoggedIn;

impl OpenAiLoginProbe for LoggedIn {
    fn status(&self) -> LoginStatus {
        LoginStatus::LoggedIn
    }
}

fn fixture() -> (
    TempDir,
    StateStore,
    EnvironmentApplication,
    DesktopApplication,
) {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, 'Fixture Provider', 'https://fixture.example/v1',
                       'test-key-not-real', 'fixture-model', '1775606400', 'fixture-fingerprint')",
            params![PROVIDER_ID],
        )
        .expect("insert provider");
    let environment = EnvironmentApplication::with_consumer_scanner(
        store.clone(),
        temp.path().join(".codex"),
        Arc::new(StoppedConsumers),
    );
    let desktop = DesktopApplication::with_boundaries(
        Arc::new(NoPackages),
        Arc::new(StoppedConsumers),
        Arc::new(NoActivation),
        Arc::new(NoProcessControl),
        Arc::new(FixedClock),
        1,
        Duration::ZERO,
    );
    (temp, store, environment, desktop)
}

#[test]
fn cancel_returns_before_any_configuration_side_effect() {
    let (temp, _, environment, desktop) = fixture();
    let preview = environment.inspect().expect("inspect environment");

    let result = environment
        .apply_provider_with_restart_plan(
            &desktop,
            PROVIDER_ID,
            RestartDecision::Cancel,
            &preview.revision,
        )
        .expect("cancel plan");

    assert!(result.cancelled);
    assert!(!temp.path().join(".codex").exists());
}

#[test]
fn immediate_restart_clears_pending_after_the_old_desktop_exits() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, 'Fixture Provider', 'https://fixture.example/v1',
                       'test-key-not-real', 'fixture-model', '1775606400', 'fixture-fingerprint')",
            params![PROVIDER_ID],
        )
        .expect("insert provider");
    let initial = running_desktop(false, 42, 7_000);
    let environment_scan = MutableScan(Arc::new(Mutex::new(initial.clone())));
    let environment = EnvironmentApplication::with_consumer_scanner(
        store,
        temp.path().join(".codex"),
        Arc::new(environment_scan.clone()),
    );
    let (desktop, controller) = desktop_fixture(environment_scan, initial, Ok(()));
    let preview = environment.inspect().expect("inspect environment");

    let result = environment
        .apply_provider_with_restart_plan(
            &desktop,
            PROVIDER_ID,
            RestartDecision::Immediate,
            &preview.revision,
        )
        .expect("apply and restart");

    assert_eq!(result.restart_status, RestartPlanStatus::Restarted);
    assert!(!result.environment.pending_restart);
    assert_eq!(
        controller
            .close_requests
            .lock()
            .expect("close requests")
            .len(),
        1
    );
}

#[test]
fn immediate_restart_keeps_pending_when_cli_remains_in_its_terminal() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, 'Fixture Provider', 'https://fixture.example/v1',
                       'test-key-not-real', 'fixture-model', '1775606400', 'fixture-fingerprint')",
            params![PROVIDER_ID],
        )
        .expect("insert provider");
    let initial = running_desktop(true, 42, 7_000);
    let environment_scan = MutableScan(Arc::new(Mutex::new(initial.clone())));
    let environment = EnvironmentApplication::with_consumer_scanner(
        store,
        temp.path().join(".codex"),
        Arc::new(environment_scan.clone()),
    );
    let (desktop, _) = desktop_fixture(environment_scan, initial, Ok(()));
    let preview = environment.inspect().expect("inspect environment");

    let result = environment
        .apply_provider_with_restart_plan(
            &desktop,
            PROVIDER_ID,
            RestartDecision::Immediate,
            &preview.revision,
        )
        .expect("apply and restart desktop");

    assert_eq!(result.restart_status, RestartPlanStatus::Restarted);
    assert!(result.environment.pending_restart);
    assert_eq!(result.environment.consumers.cli, ConsumerStatus::Running);
}

#[test]
fn configuration_failure_never_requests_desktop_close() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, 'Fixture Provider', 'https://fixture.example/v1',
                       'test-key-not-real', 'fixture-model', '1775606400', 'fixture-fingerprint')",
            params![PROVIDER_ID],
        )
        .expect("insert provider");
    let initial = running_desktop(false, 42, 7_000);
    let environment_scan = MutableScan(Arc::new(Mutex::new(initial.clone())));
    let environment = EnvironmentApplication::with_runtime_dependencies(
        store,
        temp.path().join(".codex"),
        Arc::new(FailsBeforeConfigWrite),
        Arc::new(gpteasy_lib::codex::LoginStatusCommand::new(
            "codex-command-that-does-not-exist",
            std::iter::empty::<&str>(),
        )),
        Arc::new(environment_scan.clone()),
    );
    let (desktop, controller) = desktop_fixture(environment_scan, initial, Ok(()));
    let preview = environment.inspect().expect("inspect environment");

    environment
        .apply_provider_with_restart_plan(
            &desktop,
            PROVIDER_ID,
            RestartDecision::Immediate,
            &preview.revision,
        )
        .expect_err("configuration write must fail");

    assert!(
        controller
            .close_requests
            .lock()
            .expect("close requests")
            .is_empty()
    );
    assert!(
        environment
            .inspect()
            .expect("inspect old state")
            .current_provider
            .is_none()
    );
}

#[test]
fn activation_failure_keeps_the_new_configuration_and_pending_restart() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("open state database");
    connection
        .execute(
            "INSERT INTO providers (
                id, name, base_url, api_key, default_model, verified_at,
                verification_fingerprint
             ) VALUES (?1, 'Fixture Provider', 'https://fixture.example/v1',
                       'test-key-not-real', 'fixture-model', '1775606400', 'fixture-fingerprint')",
            params![PROVIDER_ID],
        )
        .expect("insert provider");
    let initial = running_desktop(false, 42, 7_000);
    let environment_scan = MutableScan(Arc::new(Mutex::new(initial.clone())));
    let environment = EnvironmentApplication::with_consumer_scanner(
        store,
        temp.path().join(".codex"),
        Arc::new(environment_scan.clone()),
    );
    let (desktop, _) = desktop_fixture(environment_scan, initial, Err(DesktopBoundaryError));
    let preview = environment.inspect().expect("inspect environment");

    let result = environment
        .apply_provider_with_restart_plan(
            &desktop,
            PROVIDER_ID,
            RestartDecision::Immediate,
            &preview.revision,
        )
        .expect("configuration remains committed");

    assert_eq!(result.restart_status, RestartPlanStatus::RestartFailed);
    assert_eq!(
        result
            .environment
            .current_provider
            .expect("current provider")
            .id,
        PROVIDER_ID
    );
    assert!(result.environment.pending_restart);
    assert_eq!(result.restart_message_id, Some("desktop.activation_failed"));
}

#[test]
fn openai_login_switch_uses_the_same_deferred_restart_plan() {
    let temp = TempDir::new().expect("temp dir");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let initial = running_desktop(true, 42, 7_000);
    let environment_scan = MutableScan(Arc::new(Mutex::new(initial)));
    let environment = EnvironmentApplication::with_runtime_probes(
        store,
        temp.path().join(".codex"),
        Arc::new(LoggedIn),
        Arc::new(environment_scan),
    );
    let desktop = DesktopApplication::with_boundaries(
        Arc::new(NoPackages),
        Arc::new(StoppedConsumers),
        Arc::new(NoActivation),
        Arc::new(NoProcessControl),
        Arc::new(FixedClock),
        1,
        Duration::ZERO,
    );
    let preview = environment.inspect().expect("inspect environment");

    let result = environment
        .switch_to_openai_login_with_restart_plan(
            &desktop,
            RestartDecision::Later,
            &preview.revision,
        )
        .expect("switch to OpenAI login");

    assert_eq!(result.restart_status, RestartPlanStatus::Deferred);
    assert_eq!(
        result.environment.mode,
        Some(gpteasy_lib::environment::AuthenticationMode::OpenaiLogin)
    );
    assert!(result.environment.pending_restart);
    assert_eq!(result.environment.consumers.cli, ConsumerStatus::Running);
}
