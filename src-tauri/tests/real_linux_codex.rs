use std::env;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use gpteasy_lib::provider::{
    LinuxShell, ProviderApplication, ProviderValidator, ValidationTimeouts,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use rusqlite::{Connection, params};
use serde_json::Value;
use tempfile::TempDir;

const PROVIDER_ID: &str = "31313131-3131-4131-8131-313131313131";
const PROVIDER_NAME: &str = "GPTEasy Real Codex Acceptance";
const BASE_URL: &str = "https://acceptance.invalid/v1";
const MODEL: &str = "gpteasy-real-codex-model";

#[test]
#[ignore = "requires GPTEASY_RUN_REAL_CODEX_ACCEPTANCE=1 and a selected real Codex CLI"]
fn exported_provider_is_effective_in_real_codex_cli() {
    assert_eq!(
        env::var("GPTEASY_RUN_REAL_CODEX_ACCEPTANCE").as_deref(),
        Ok("1"),
        "set GPTEASY_RUN_REAL_CODEX_ACCEPTANCE=1 to confirm the isolated real Codex check",
    );
    let codex = env::var("GPTEASY_REAL_CODEX").expect("select a real Codex executable");
    let canary = env::var("GPTEASY_ACCEPTANCE_KEY_A")
        .expect("the acceptance runner must provide an API key canary");
    let fixture = RealCodexFixture::new(&canary);
    let snapshot = fixture.temp.path().join("gpteasy.sh");
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &snapshot, false)
        .expect("export real Codex acceptance snapshot");

    let shell = env::var("GPTEASY_TEST_BASH").unwrap_or_else(|_| "bash".to_owned());
    let mut command = acceptance_shell_command(&shell, &snapshot, &codex);
    assert_process_arguments_are_clean(&command, &canary);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start real Codex acceptance shell");
    child
        .stdin
        .take()
        .expect("real Codex acceptance stdin")
        .write_all(real_codex_harness().as_bytes())
        .expect("write real Codex acceptance harness");
    let output = child
        .wait_with_output()
        .expect("wait for real Codex acceptance shell");

    assert_public_output_is_clean(&output.stdout, &output.stderr, &canary);
    assert!(
        output.status.success(),
        "real Codex acceptance failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("real Codex app-server output is UTF-8");
    let response = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|message| message["id"] == 2)
        .unwrap_or_else(|| panic!("config/read response from real Codex app-server: {stdout}"));
    assert!(
        response.get("error").is_none(),
        "config/read failed: {response}"
    );
    let config = &response["result"]["config"];
    assert_eq!(config["model"], MODEL);
    assert_eq!(config["model_provider"], "gpteasy");
    assert_eq!(config["model_providers"]["gpteasy"]["name"], PROVIDER_NAME);
    assert_eq!(config["model_providers"]["gpteasy"]["base_url"], BASE_URL);
    assert_eq!(
        config["model_providers"]["gpteasy"]["wire_api"],
        "responses"
    );
}

fn real_codex_harness() -> &'static str {
    r#"set -euo pipefail
snapshot=$1
real_codex=$2
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-real-codex.XXXXXX")
server_pid=
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf -- "$workspace"
}
trap cleanup EXIT
mkdir -m 700 -- "$workspace/codex" "$workspace/bin"
cp -- "$snapshot" "$workspace/gpteasy.sh"
chmod 600 "$workspace/gpteasy.sh"
printf '%s' '{"login":"unchanged"}' >"$workspace/codex/auth.json"
chmod 600 "$workspace/codex/auth.json"
auth_before=$(sha256sum "$workspace/codex/auth.json")
cat >"$workspace/bin/codex" <<'CODEX_WRAPPER'
#!/bin/sh
exec "$GPTEASY_REAL_CODEX_PATH" "$@"
CODEX_WRAPPER
chmod 700 "$workspace/bin/codex"
export GPTEASY_REAL_CODEX_PATH=$real_codex
export CODEX_HOME="$workspace/codex"
export PATH="$workspace/bin:$PATH"
cd -- "$workspace"
source "$workspace/gpteasy.sh"
gpteasy <<<"1" >/dev/null
[[ "$auth_before" == "$(sha256sum "$workspace/codex/auth.json")" ]]
coproc CODEX_SERVER { codex app-server --listen stdio://; }
server_pid=$CODEX_SERVER_PID
server_in=${CODEX_SERVER[1]}
server_out=${CODEX_SERVER[0]}
printf '%s\n' '{"method":"initialize","id":0,"params":{"clientInfo":{"name":"gpteasy_acceptance","title":"GPTEasy Acceptance","version":"1"}}}' >&$server_in
while IFS= read -r -u "$server_out" line; do
  printf '%s\n' "$line"
  [[ "$line" == *'"id":0'* ]] && break
done
printf '%s\n' '{"method":"initialized","params":{}}' >&$server_in
printf '%s\n' '{"method":"config/read","id":2,"params":{"includeLayers":true}}' >&$server_in
while IFS= read -r -u "$server_out" line; do
  printf '%s\n' "$line"
  [[ "$line" == *'"id":2'* ]] && break
done
[[ "$auth_before" == "$(sha256sum "$workspace/codex/auth.json")" ]]
"#
}

#[cfg(windows)]
fn acceptance_shell_command(shell: &str, snapshot: &Path, codex: &str) -> Command {
    let mut command = Command::new("wsl.exe");
    command.args([
        "--distribution",
        &wsl_test_distribution(),
        "--exec",
        shell,
        "-s",
        "--",
        &windows_path_for_wsl(snapshot),
        codex,
    ]);
    command
}

#[cfg(not(windows))]
fn acceptance_shell_command(shell: &str, snapshot: &Path, codex: &str) -> Command {
    let mut command = Command::new(shell);
    command.args([
        "-s",
        "--",
        snapshot.to_str().expect("UTF-8 snapshot path"),
        codex,
    ]);
    command
}

fn assert_process_arguments_are_clean(command: &Command, canary: &str) {
    assert!(!command.get_program().to_string_lossy().contains(canary));
    assert!(
        !command
            .get_args()
            .any(|argument| argument.to_string_lossy().contains(canary)),
        "API key canary leaked into real Codex process arguments",
    );
}

fn assert_public_output_is_clean(stdout: &[u8], stderr: &[u8], canary: &str) {
    for (label, output) in [("stdout", stdout), ("stderr", stderr)] {
        assert!(
            !contains_bytes(output, canary.as_bytes()),
            "API key canary leaked into real Codex {label}",
        );
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[cfg(windows)]
fn wsl_test_distribution() -> String {
    env::var("GPTEASY_TEST_WSL_DISTRIBUTION").expect("select a WSL2 distribution")
}

#[cfg(windows)]
fn windows_path_for_wsl(path: &Path) -> String {
    let windows_path = path.to_str().expect("UTF-8 test path").replace('\\', "/");
    let output = Command::new("wsl.exe")
        .args([
            "--distribution",
            &wsl_test_distribution(),
            "--exec",
            "wslpath",
            "-a",
            "-u",
            &windows_path,
        ])
        .output()
        .expect("translate real Codex snapshot path for WSL");
    assert!(output.status.success(), "translate snapshot path for WSL");
    String::from_utf8(output.stdout)
        .expect("WSL path is UTF-8")
        .trim()
        .to_owned()
}

struct RealCodexFixture {
    temp: TempDir,
    application: ProviderApplication,
}

impl RealCodexFixture {
    fn new(canary: &str) -> Self {
        let temp = TempDir::new().expect("real Codex fixture temp");
        let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
        assert!(store.bootstrap().is_ready());
        let connection = Connection::open(store.paths().database()).expect("open fixture state");
        connection
            .execute(
                "INSERT INTO providers (
                    id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint, sort_order
                 ) VALUES (?1, ?2, ?3, ?4, ?5, '1786800000', 'verified', 0)",
                params![PROVIDER_ID, PROVIDER_NAME, BASE_URL, canary, MODEL],
            )
            .expect("insert real Codex provider fixture");
        drop(connection);
        let application =
            ProviderApplication::new(store, ProviderValidator::new(ValidationTimeouts::default()));
        Self { temp, application }
    }
}
