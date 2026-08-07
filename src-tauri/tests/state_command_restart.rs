use std::{
    collections::BTreeSet,
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output},
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

const CHILD_MODE_ENV: &str = "GPTEASY_STATE_COMMAND_TEST_CHILD";
const CHILD_ROOT_ENV: &str = "GPTEASY_STATE_COMMAND_TEST_ROOT";
const CHILD_RUN_ID_ENV: &str = "GPTEASY_STATE_COMMAND_TEST_RUN_ID";
const REPORT_PREFIX: &str = "GPTEASY_STATE_COMMAND_REPORT=";

fn spawn_child(test_name: &str, root: &Path, run_id: &str) -> Output {
    Command::new(env::current_exe().expect("resolve integration test executable"))
        .args([
            OsString::from("--exact"),
            OsString::from(test_name),
            OsString::from("--nocapture"),
            OsString::from("--test-threads=1"),
        ])
        .env(CHILD_MODE_ENV, "1")
        .env(CHILD_ROOT_ENV, root.as_os_str())
        .env(CHILD_RUN_ID_ENV, run_id)
        .output()
        .expect("spawn state command child process")
}

fn report_from(output: &Output, private_root: &Path) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("child stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("child stderr is UTF-8");
    assert!(
        output.status.success(),
        "child failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let private_root = private_root.to_string_lossy();
    assert!(!stdout.contains(private_root.as_ref()));
    assert!(!stderr.contains(private_root.as_ref()));
    if let Some(user_profile) = env::var_os("USERPROFILE") {
        let user_profile = user_profile.to_string_lossy();
        assert!(!stdout.contains(user_profile.as_ref()));
        assert!(!stderr.contains(user_profile.as_ref()));
    }

    let json = stdout
        .lines()
        .find_map(|line| {
            line.find(REPORT_PREFIX)
                .map(|index| &line[index + REPORT_PREFIX.len()..])
        })
        .expect("child emitted state command report");
    serde_json::from_str(json).expect("parse state command report")
}

fn local_invoke_url() -> &'static str {
    if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
}

fn invoke_command(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    command: &str,
    body: Value,
) -> Value {
    get_ipc_response(
        webview,
        InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: local_invoke_url().parse().expect("parse local invoke URL"),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    )
    .unwrap_or_else(|error| panic!("{command} IPC failed: {error}"))
    .deserialize::<Value>()
    .expect("deserialize command response")
}

fn run_child(command: &str, body: Value) {
    if env::var_os(CHILD_MODE_ENV).is_none() {
        return;
    }

    let root = PathBuf::from(env::var_os(CHILD_ROOT_ENV).expect("child test root"));
    let run_id = env::var(CHILD_RUN_ID_ENV).expect("child run ID");
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = root.to_string_lossy().into_owned();
    let app = configure_builder(mock_builder())
        .build(context)
        .expect("build child mock app through production composition");
    let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .expect("build child mock webview");
    let response = invoke_command(&webview, command, body);

    println!(
        "{REPORT_PREFIX}{}",
        json!({
            "command": command,
            "response": response,
            "run_id": run_id,
        })
    );
}

#[test]
fn state_command_write_child_process() {
    run_child(
        "update_app_settings",
        json!({ "input": { "theme": "dark" } }),
    );
}

#[test]
fn state_command_read_child_process() {
    run_child("bootstrap_state", json!({}));
}

#[test]
fn registered_commands_round_trip_settings_across_os_processes() {
    let temp = tempdir().expect("create integration temp directory");
    let app_root = temp.path().join("private-user-root");
    let run_id = "state-command-restart-20260807";

    let write = report_from(
        &spawn_child("state_command_write_child_process", &app_root, run_id),
        temp.path(),
    );
    let read = report_from(
        &spawn_child("state_command_read_child_process", &app_root, run_id),
        temp.path(),
    );

    assert_eq!(write["run_id"], run_id);
    assert_eq!(write["command"], "update_app_settings");
    assert_eq!(write["response"]["settings"]["theme"], "dark");
    assert_eq!(read["run_id"], run_id);
    assert_eq!(read["command"], "bootstrap_state");
    assert_eq!(read["response"]["settings"]["theme"], "dark");
    assert_eq!(read["response"]["schema_version"], 1);

    let public_keys = read["response"]
        .as_object()
        .expect("bootstrap response object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(public_keys, BTreeSet::from(["schema_version", "settings"]));

    let public_json = read["response"].to_string().to_ascii_lowercase();
    for forbidden in [
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "private-user-root",
    ] {
        assert!(
            !public_json.contains(forbidden),
            "bootstrap response leaked forbidden field {forbidden}"
        );
    }
}
