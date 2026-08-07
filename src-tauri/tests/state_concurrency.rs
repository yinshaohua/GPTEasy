use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use gpteasy_lib::configure_builder;
use serde_json::{json, Value};
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY},
    webview::InvokeRequest,
    WebviewUrl, WebviewWindowBuilder,
};
use tempfile::tempdir;

const CHILD_MODE_ENV: &str = "GPTEASY_STATE_CONCURRENCY_TEST_CHILD";
const CHILD_ROOT_ENV: &str = "GPTEASY_STATE_CONCURRENCY_TEST_ROOT";
const CHILD_ROLE_ENV: &str = "GPTEASY_STATE_CONCURRENCY_TEST_ROLE";
const CHILD_READY_ENV: &str = "GPTEASY_STATE_CONCURRENCY_TEST_READY";
const OPENED_PREFIX: &str = "GPTEASY_STATE_COORDINATOR_OPENED=";
const LOCK_FILENAME: &str = "state.lock";
const OWNER_FILENAME: &str = "state-lock-owner.json";
const OWNER_SCHEMA: &str = "gpteasy.state-coordinator-owner.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    File(Vec<u8>),
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
    fn visit(root: &Path, current: &Path, entries: &mut BTreeMap<PathBuf, SnapshotEntry>) {
        let mut children = fs::read_dir(current)
            .expect("read state snapshot directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect state snapshot directory");
        children.sort_by_key(|entry| entry.file_name());
        for entry in children {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let file_type = entry.file_type().expect("read snapshot file type");
            if file_type.is_dir() {
                entries.insert(relative, SnapshotEntry::Directory);
                visit(root, &path, entries);
            } else if file_type.is_file() {
                entries.insert(
                    relative,
                    SnapshotEntry::File(fs::read(&path).expect("read snapshot file")),
                );
            } else {
                panic!("state root contains an unexpected non-file entry");
            }
        }
    }

    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn local_invoke_url() -> &'static str {
    if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
}

fn invoke_bootstrap(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
) -> Result<Value, Value> {
    get_ipc_response(
        webview,
        InvokeRequest {
            cmd: "bootstrap_state".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: local_invoke_url().parse().expect("parse local invoke URL"),
            body: InvokeBody::Json(json!({})),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    )
    .map(|response| response.deserialize::<Value>().expect("bootstrap response"))
}

#[allow(deprecated)] // Tauri mock build defers production setup until one test iteration.
fn run_child(role: &str) {
    if env::var_os(CHILD_MODE_ENV).is_none() {
        return;
    }
    let root = PathBuf::from(env::var_os(CHILD_ROOT_ENV).expect("child state root"));
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = root.to_string_lossy().into_owned();
    let built = configure_builder(mock_builder()).build(context);

    if role == "contend" {
        assert!(
            built.is_err(),
            "second production composition open unexpectedly succeeded"
        );
        println!("{OPENED_PREFIX}busy");
        return;
    }

    let mut app = built.expect("open state through production composition");
    app.run_iteration(|_, _| {});
    let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .expect("build state concurrency mock webview");
    let response = invoke_bootstrap(&webview).expect("invoke registered bootstrap_state command");
    assert_eq!(response["schema_version"], 1);

    if role == "hold" {
        let ready_path = PathBuf::from(env::var_os(CHILD_READY_ENV).expect("holder ready path"));
        fs::write(ready_path, std::process::id().to_string()).expect("publish holder readiness");
        loop {
            thread::sleep(Duration::from_secs(30));
        }
    }

    println!("{OPENED_PREFIX}ready");
}

fn child_command(root: &Path, role: &str) -> Command {
    let mut command = Command::new(env::current_exe().expect("integration test executable"));
    command
        .args([
            OsString::from("--exact"),
            OsString::from("state_coordinator_child_process"),
            OsString::from("--nocapture"),
            OsString::from("--test-threads=1"),
        ])
        .env(CHILD_MODE_ENV, "1")
        .env(CHILD_ROOT_ENV, root.as_os_str())
        .env(CHILD_ROLE_ENV, role);
    command
}

struct HolderGuard {
    child: Child,
}

impl Drop for HolderGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn spawn_holder(root: &Path) -> (HolderGuard, u32) {
    let ready_path = root
        .parent()
        .expect("state root has a test parent")
        .join("holder-ready.txt");
    let mut child = child_command(root, "hold")
        .env(CHILD_READY_ENV, ready_path.as_os_str())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn state lock holder");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if ready_path.exists() {
            let pid = fs::read_to_string(&ready_path)
                .expect("read holder readiness")
                .trim()
                .parse::<u32>()
                .expect("parse holder PID");
            assert_eq!(pid, child.id());
            return (HolderGuard { child }, pid);
        }
        assert!(
            child.try_wait().expect("poll holder process").is_none(),
            "holder exited before acquiring coordinator"
        );
        assert!(Instant::now() < deadline, "holder readiness timed out");
        thread::sleep(Duration::from_millis(25));
    }
}

fn run_child_output(root: &Path, role: &str) -> Output {
    child_command(root, role)
        .output()
        .expect("run state coordinator child")
}

fn sanitized_output(output: &Output, private_root: &Path) -> (String, String) {
    let stdout = String::from_utf8(output.stdout.clone()).expect("child stdout UTF-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("child stderr UTF-8");
    let private_root = private_root.to_string_lossy();
    assert!(!stdout.contains(private_root.as_ref()));
    assert!(!stderr.contains(private_root.as_ref()));
    if let Some(user_profile) = env::var_os("USERPROFILE") {
        let user_profile = user_profile.to_string_lossy();
        assert!(!stdout.contains(user_profile.as_ref()));
        assert!(!stderr.contains(user_profile.as_ref()));
    }
    (stdout, stderr)
}

#[test]
fn state_coordinator_child_process() {
    let role = env::var(CHILD_ROLE_ENV).unwrap_or_else(|_| "none".to_owned());
    run_child(&role);
}

#[test]
fn os_lock_serializes_writers_releases_after_crash_and_ignores_stale_metadata() {
    let temp = tempdir().expect("create state concurrency temp directory");
    let state_root = temp.path().join("private-current-user-state");
    fs::create_dir(&state_root).expect("create fixed test state root");

    let (mut holder, holder_pid) = spawn_holder(&state_root);
    let before_contention = snapshot(&state_root);
    assert!(before_contention.contains_key(Path::new(LOCK_FILENAME)));
    assert!(before_contention.contains_key(Path::new(OWNER_FILENAME)));

    let started = Instant::now();
    let contender = run_child_output(&state_root, "contend");
    let elapsed = started.elapsed();
    let (stdout, stderr) = sanitized_output(&contender, temp.path());
    assert!(
        contender.status.success(),
        "contender did not report StateBusy\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains(&format!("{OPENED_PREFIX}busy")));
    assert!(
        elapsed < Duration::from_secs(3),
        "busy timeout was not bounded"
    );
    assert_eq!(
        snapshot(&state_root),
        before_contention,
        "failed contender mutated DB, WAL, lock, owner, or recovery artifacts"
    );

    holder
        .child
        .kill()
        .expect("terminate state lock holder process");
    holder
        .child
        .wait()
        .expect("reap terminated state lock holder process");

    let stale_owner = json!({
        "schema": OWNER_SCHEMA,
        "pid": holder_pid,
        "process_start_token": "stale-owner-token",
        "run_id_digest": "0".repeat(64)
    });
    fs::write(
        state_root.join(OWNER_FILENAME),
        serde_json::to_vec(&stale_owner).unwrap(),
    )
    .expect("write stale diagnostic owner metadata");

    let reopened = run_child_output(&state_root, "open");
    let (stdout, stderr) = sanitized_output(&reopened, temp.path());
    assert!(
        reopened.status.success(),
        "OS did not release lock after holder crash\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains(&format!("{OPENED_PREFIX}ready")));

    let owner: Value = serde_json::from_slice(
        &fs::read(state_root.join(OWNER_FILENAME)).expect("read replacement owner metadata"),
    )
    .expect("parse replacement owner metadata");
    assert_eq!(
        owner
            .as_object()
            .expect("owner metadata object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["pid", "process_start_token", "run_id_digest", "schema"]
    );
    assert_eq!(owner["schema"], OWNER_SCHEMA);
    assert_ne!(owner["pid"], holder_pid);
    assert_ne!(owner["process_start_token"], "stale-owner-token");
    assert_eq!(owner["run_id_digest"].as_str().unwrap().len(), 64);
}
