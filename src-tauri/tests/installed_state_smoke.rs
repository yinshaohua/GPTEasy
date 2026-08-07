use std::{
    collections::BTreeSet,
    env,
    ffi::OsString,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

const STATE_SMOKE_COMMAND: &str = "phase1-state-smoke";
const STATE_SMOKE_SCHEMA: &str = "gpteasy.phase1.state-smoke.v1";
const EXPECTED_STATE_DIGEST: &str =
    "3a634244209687ed670e40d2ba9a9dc5175e85c00d994708c43bcb17de61365f";
const ALPHA_KEY_CANARY: &str = "state-secret-canary-alpha-4D2F9C0E";
const BETA_KEY_CANARY: &str = "state-secret-canary-beta-7A1E3B8D";

fn production_binary() -> &'static str {
    env!("CARGO_BIN_EXE_gpteasy")
}

fn unique_run_id(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after Unix epoch")
        .as_nanos();
    format!("installed-{label}-{}-{nanos}", std::process::id())
}

fn run_smoke(mode: &str, run_id: &str) -> Output {
    Command::new(production_binary())
        .args([
            OsString::from(STATE_SMOKE_COMMAND),
            OsString::from(mode),
            OsString::from(run_id),
        ])
        .output()
        .expect("run production state smoke CLI")
}

fn output_text(output: &Output) -> (String, String) {
    (
        String::from_utf8(output.stdout.clone()).expect("state smoke stdout is UTF-8"),
        String::from_utf8(output.stderr.clone()).expect("state smoke stderr is UTF-8"),
    )
}

fn assert_sanitized(output: &Output) -> (String, String) {
    let (stdout, stderr) = output_text(output);
    for forbidden in [ALPHA_KEY_CANARY, BETA_KEY_CANARY] {
        assert!(!stdout.contains(forbidden));
        assert!(!stderr.contains(forbidden));
    }
    if let Some(user_profile) = env::var_os("USERPROFILE") {
        let user_profile = user_profile.to_string_lossy();
        assert!(!stdout.contains(user_profile.as_ref()));
        assert!(!stderr.contains(user_profile.as_ref()));
    }
    (stdout, stderr)
}

fn successful_report(mode: &str, run_id: &str) -> Value {
    let output = run_smoke(mode, run_id);
    let (stdout, stderr) = assert_sanitized(&output);
    assert!(
        output.status.success(),
        "{mode} failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    serde_json::from_str(stdout.trim()).expect("parse production state smoke report")
}

fn assert_state_report(report: &Value, mode: &str, run_id: &str) {
    let object = report.as_object().expect("state smoke report object");
    assert_eq!(
        object.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["counts", "mode", "run_id", "schema", "state_digest",])
    );
    assert_eq!(report["schema"], STATE_SMOKE_SCHEMA);
    assert_eq!(report["mode"], mode);
    assert_eq!(report["run_id"], run_id);
    assert_eq!(report["state_digest"], EXPECTED_STATE_DIGEST);
    assert_eq!(
        report["counts"],
        serde_json::json!({
            "providers": 2,
            "verifications": 2,
            "native_environments": 1,
            "wsl2_environments": 1,
            "current_provider_assignments": 2,
            "settings_fields": 7
        })
    );
}

#[test]
fn production_cli_seed_verify_and_explicit_cleanup_preserve_truthful_state() {
    let run_id = unique_run_id("lifecycle");
    assert!(run_id.len() <= 64);

    let seed = successful_report("seed", &run_id);
    assert_state_report(&seed, "seed", &run_id);

    let first_verify = successful_report("verify", &run_id);
    let second_verify = successful_report("verify", &run_id);
    assert_state_report(&first_verify, "verify", &run_id);
    assert_eq!(first_verify, second_verify, "verify must not mutate state");

    let cleanup = successful_report("cleanup", &run_id);
    assert_eq!(
        cleanup,
        serde_json::json!({
            "schema": STATE_SMOKE_SCHEMA,
            "mode": "cleanup",
            "run_id": run_id,
            "cleaned": true
        })
    );

    let missing = run_smoke("verify", cleanup["run_id"].as_str().unwrap());
    let (stdout, stderr) = assert_sanitized(&missing);
    assert!(!missing.status.success());
    assert!(stdout.trim().is_empty());
    assert_eq!(stderr.trim(), "phase1-state-smoke failed");
}

#[test]
fn cleanup_rejects_unmatched_markers_and_path_shaped_run_ids() {
    let live_run_id = unique_run_id("bounded");
    let absent_run_id = unique_run_id("absent");
    successful_report("seed", &live_run_id);

    let absent_cleanup = run_smoke("cleanup", &absent_run_id);
    let (stdout, stderr) = assert_sanitized(&absent_cleanup);
    assert!(!absent_cleanup.status.success());
    assert!(stdout.trim().is_empty());
    assert_eq!(stderr.trim(), "phase1-state-smoke failed");
    assert_state_report(
        &successful_report("verify", &live_run_id),
        "verify",
        &live_run_id,
    );

    for invalid_id in [
        "",
        ".",
        "..",
        "../escape",
        r"..\escape",
        "contains.dot",
        "contains_underscore",
        "包含Unicode",
        &"a".repeat(65),
    ] {
        let output = run_smoke("cleanup", invalid_id);
        let (stdout, _) = assert_sanitized(&output);
        assert!(
            !output.status.success(),
            "accepted invalid ID {invalid_id:?}"
        );
        assert!(stdout.trim().is_empty());
    }

    successful_report("cleanup", &live_run_id);
}
