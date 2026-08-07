use std::{
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

const CHILD_MODE_ENV: &str = "GPTEASY_STATE_PERSISTENCE_TEST_CHILD";
const CHILD_ROOT_ENV: &str = "GPTEASY_STATE_PERSISTENCE_TEST_ROOT";
const CHILD_RUN_ID_ENV: &str = "GPTEASY_STATE_PERSISTENCE_TEST_RUN_ID";
const REPORT_PREFIX: &str = "GPTEASY_STATE_PERSISTENCE_REPORT=";
const ALPHA_KEY_CANARY: &str = "state-secret-canary-alpha-4D2F9C0E";
const BETA_KEY_CANARY: &str = "state-secret-canary-beta-7A1E3B8D";
const ALPHA_FINGERPRINT: &str =
    "89c467dc278dd6a92bd996d7735f9ba8610ad0d930f7d97f4b1e9935614ae416";
const BETA_FINGERPRINT: &str =
    "27851fe992655e033d9c3ea9e1066b89ce4c79297d110de9dae58ac3c7a725ac";
const EXPECTED_STATE_DIGEST: &str =
    "3a634244209687ed670e40d2ba9a9dc5175e85c00d994708c43bcb17de61365f";

fn full_state_input() -> Value {
    json!({
        "input": {
            "providers": [
                {
                    "id": "11111111-1111-4111-8111-111111111111",
                    "provider_kind": "built_in_recommended",
                    "built_in_key": "dayway",
                    "display_name": "DayWay",
                    "base_url": "https://dayway.site/v1",
                    "api_key": ALPHA_KEY_CANARY,
                    "default_model": "dayway-model-a",
                    "created_at": "2026-08-07T00:00:00.000Z",
                    "updated_at": "2026-08-07T00:01:00.000Z"
                },
                {
                    "id": "22222222-2222-4222-8222-222222222222",
                    "provider_kind": "custom",
                    "built_in_key": null,
                    "display_name": "Local Compatible",
                    "base_url": "http://127.0.0.1:4010/v1",
                    "api_key": BETA_KEY_CANARY,
                    "default_model": "local-model-b",
                    "created_at": "2026-08-07T00:02:00.000Z",
                    "updated_at": "2026-08-07T00:03:00.000Z"
                }
            ],
            "verifications": [
                {
                    "provider_id": "11111111-1111-4111-8111-111111111111",
                    "combination_fingerprint": ALPHA_FINGERPRINT,
                    "verified_at": "2026-08-07T00:04:00.000Z",
                    "contract_version": "gpteasy.provider-validation.v1"
                },
                {
                    "provider_id": "22222222-2222-4222-8222-222222222222",
                    "combination_fingerprint": BETA_FINGERPRINT,
                    "verified_at": "2026-08-07T00:05:00.000Z",
                    "contract_version": "gpteasy.provider-validation.v1"
                }
            ],
            "environments": [
                {
                    "id": "33333333-3333-4333-8333-333333333333",
                    "environment_kind": "native_codex",
                    "platform_identity": "native-current-user",
                    "display_name": "Native Codex",
                    "current_provider_id": "11111111-1111-4111-8111-111111111111",
                    "first_seen_at": "2026-08-07T00:06:00.000Z",
                    "last_seen_at": "2026-08-07T00:07:00.000Z"
                },
                {
                    "id": "44444444-4444-4444-8444-444444444444",
                    "environment_kind": "wsl2",
                    "platform_identity": "wsl-registration-a1b2c3d4",
                    "display_name": "Ubuntu-24.04",
                    "current_provider_id": "22222222-2222-4222-8222-222222222222",
                    "first_seen_at": "2026-08-07T00:08:00.000Z",
                    "last_seen_at": "2026-08-07T00:09:00.000Z"
                }
            ],
            "settings": {
                "locale": "zh-CN",
                "theme": "dark",
                "launch_at_login_desired": true,
                "close_to_tray_notice_seen": true,
                "onboarding_completed": true,
                "last_update_check_at": "2026-08-07T00:10:00.000Z",
                "updated_at": "2026-08-07T00:11:00.000Z"
            }
        }
    })
}

fn expected_public_snapshot() -> Value {
    json!({
        "schema_version": 1,
        "counts": {
            "providers": 2,
            "verified_providers": 2,
            "managed_environments": 2
        },
        "providers": [
            {
                "id": "11111111-1111-4111-8111-111111111111",
                "provider_kind": "built_in_recommended",
                "verification_status": "verified",
                "combination_fingerprint": ALPHA_FINGERPRINT
            },
            {
                "id": "22222222-2222-4222-8222-222222222222",
                "provider_kind": "custom",
                "verification_status": "verified",
                "combination_fingerprint": BETA_FINGERPRINT
            }
        ],
        "environments": [
            {
                "id": "33333333-3333-4333-8333-333333333333",
                "environment_kind": "native_codex",
                "current_provider_id": "11111111-1111-4111-8111-111111111111"
            },
            {
                "id": "44444444-4444-4444-8444-444444444444",
                "environment_kind": "wsl2",
                "current_provider_id": "22222222-2222-4222-8222-222222222222"
            }
        ],
        "settings": {
            "locale": "zh-CN",
            "theme": "dark",
            "launch_at_login_desired": true,
            "close_to_tray_notice_seen": true,
            "onboarding_completed": true,
            "last_update_check_at": "2026-08-07T00:10:00.000Z",
            "updated_at": "2026-08-07T00:11:00.000Z"
        },
        "state_digest": EXPECTED_STATE_DIGEST
    })
}

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
        .expect("spawn full state child process")
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
) -> Result<Value, Value> {
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
    .map(|response| {
        response
            .deserialize::<Value>()
            .expect("deserialize command response")
    })
}

#[allow(deprecated)] // Tauri mock build defers production setup until this one test iteration.
fn run_child(command: &str) {
    if env::var_os(CHILD_MODE_ENV).is_none() {
        return;
    }

    let root = PathBuf::from(env::var_os(CHILD_ROOT_ENV).expect("child test root"));
    let run_id = env::var(CHILD_RUN_ID_ENV).expect("child run ID");
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = root.to_string_lossy().into_owned();
    let mut app = configure_builder(mock_builder())
        .build(context)
        .expect("build child mock app through production composition");
    app.run_iteration(|_, _| {});
    let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .expect("build child mock webview");

    let response = if command == "replace_state_snapshot" {
        let mut invalid = full_state_input();
        invalid["input"]["providers"][0]["id"] = json!("DayWay-display-name-is-not-an-id");
        let rejection = invoke_command(&webview, command, invalid)
            .expect_err("non-UUID provider identity must be rejected");
        assert_eq!(rejection, json!({ "code": "invalid_state_input" }));
        invoke_command(&webview, command, full_state_input())
            .expect("replace complete state through registered command")
    } else {
        invoke_command(&webview, command, json!({}))
            .expect("bootstrap complete state through registered command")
    };

    println!(
        "{REPORT_PREFIX}{}",
        json!({
            "command": command,
            "response": response,
            "run_id": run_id,
        })
    );
}

fn report_from(output: &Output, private_root: &Path) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("child stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("child stderr is UTF-8");
    assert!(
        output.status.success(),
        "child failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    for forbidden in [
        ALPHA_KEY_CANARY,
        BETA_KEY_CANARY,
        private_root.to_string_lossy().as_ref(),
    ] {
        assert!(!stdout.contains(forbidden));
        assert!(!stderr.contains(forbidden));
    }
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
        .expect("child emitted full state report");
    serde_json::from_str(json).expect("parse full state report")
}

#[test]
fn full_state_write_child_process() {
    run_child("replace_state_snapshot");
}

#[test]
fn full_state_read_child_process() {
    run_child("bootstrap_state_snapshot");
}

#[test]
fn complete_state_round_trips_across_processes_without_secret_output() {
    let temp = tempdir().expect("create integration temp directory");
    let app_root = temp.path().join("private-full-state-root");
    let run_id = "full-state-persistence-20260807";

    let write = report_from(
        &spawn_child("full_state_write_child_process", &app_root, run_id),
        temp.path(),
    );
    let read = report_from(
        &spawn_child("full_state_read_child_process", &app_root, run_id),
        temp.path(),
    );

    assert_eq!(write["run_id"], run_id);
    assert_eq!(write["command"], "replace_state_snapshot");
    assert_eq!(read["run_id"], run_id);
    assert_eq!(read["command"], "bootstrap_state_snapshot");
    assert_eq!(write["response"], expected_public_snapshot());
    assert_eq!(read["response"], expected_public_snapshot());

    let public_json = read["response"].to_string();
    assert!(!public_json.contains(ALPHA_KEY_CANARY));
    assert!(!public_json.contains(BETA_KEY_CANARY));
    for forbidden_field in ["api_key", "base_url", "default_model", "platform_identity"] {
        assert!(!public_json.contains(forbidden_field));
    }
}
