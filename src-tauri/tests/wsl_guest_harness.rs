#![cfg(all(windows, feature = "wsl-guest-harness"))]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use gpteasy_lib::provider::{
    LinuxShell, ProviderApplication, ProviderValidator, ValidationTimeouts,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use gpteasy_lib::wsl::{
    WslApplication, WslAvailability, WslConfigurationState, WslLifecycleOutcome,
};
use rusqlite::{Connection, params};
use tempfile::TempDir;

const GUEST_WRITER: &[u8] = include_bytes!("../src/wsl_guest_writer.sh");
const GUEST_LOCK: &[u8] = include_bytes!("../src/wsl_guest_lock.sh");
const GUEST_PRIVATE_READER: &str = include_str!("../src/wsl_guest_private_reader.sh");
const GUEST_CREDENTIAL_CLEANUP: &str = include_str!("../src/wsl_guest_credential_cleanup.sh");

struct GuestHome {
    distribution: String,
    path: String,
}

struct RunningGuest {
    distribution: String,
    keeper_pid: String,
}

struct WslWrapper {
    _temp: TempDir,
    program: OsString,
    previous_real_exe: Option<OsString>,
    previous_distribution: Option<OsString>,
    previous_guest_home: Option<OsString>,
    previous_state: Option<OsString>,
    previous_log: Option<OsString>,
    previous_stop_after: Option<OsString>,
    state: PathBuf,
    log: PathBuf,
}

impl WslWrapper {
    fn start(distribution: &str, guest_home: &str) -> Self {
        Self::start_with_lifecycle(distribution, guest_home, true, None)
    }

    fn start_with_lifecycle(
        distribution: &str,
        guest_home: &str,
        initially_running: bool,
        stop_after_running_lists: Option<usize>,
    ) -> Self {
        let real_exe = Command::new("where.exe")
            .arg("wsl.exe")
            .output()
            .expect("locate the real wsl.exe");
        assert!(real_exe.status.success(), "locate the real wsl.exe");
        let real_exe = String::from_utf8(real_exe.stdout)
            .expect("wsl.exe path is UTF-8")
            .lines()
            .next()
            .expect("one wsl.exe path")
            .trim()
            .to_owned();
        let temp = TempDir::new().expect("wrapper temp");
        let wrapper = temp.path().join("wsl.exe");
        let state = temp.path().join("lifecycle-state");
        let log = temp.path().join("wsl-invocations.log");
        fs::write(
            &state,
            if initially_running {
                "running:0\n"
            } else {
                "stopped\n"
            },
        )
        .expect("seed WSL harness lifecycle");
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/wsl_harness_wrapper.rs");
        let compiled = Command::new("rustc")
            .args(["--edition=2024"])
            .arg(&source)
            .arg("-o")
            .arg(&wrapper)
            .output()
            .expect("compile WSL harness wrapper");
        assert!(
            compiled.status.success(),
            "compile WSL harness wrapper: {}",
            String::from_utf8_lossy(&compiled.stderr),
        );

        let previous_real_exe = env::var_os("GPTEASY_WSL_HARNESS_REAL_EXE");
        let previous_distribution = env::var_os("GPTEASY_WSL_HARNESS_DISTRIBUTION");
        let previous_guest_home = env::var_os("GPTEASY_WSL_HARNESS_GUEST_HOME");
        let previous_state = env::var_os("GPTEASY_WSL_HARNESS_STATE");
        let previous_log = env::var_os("GPTEASY_WSL_HARNESS_LOG");
        let previous_stop_after = env::var_os("GPTEASY_WSL_HARNESS_STOP_AFTER_RUNNING_LISTS");
        // This ignored test is the only test in the process and restores every variable in Drop.
        unsafe {
            env::set_var("GPTEASY_WSL_HARNESS_REAL_EXE", real_exe);
            env::set_var("GPTEASY_WSL_HARNESS_DISTRIBUTION", distribution);
            env::set_var("GPTEASY_WSL_HARNESS_GUEST_HOME", guest_home);
            env::set_var("GPTEASY_WSL_HARNESS_STATE", &state);
            env::set_var("GPTEASY_WSL_HARNESS_LOG", &log);
            if let Some(stop_after) = stop_after_running_lists {
                env::set_var(
                    "GPTEASY_WSL_HARNESS_STOP_AFTER_RUNNING_LISTS",
                    stop_after.to_string(),
                );
            } else {
                env::remove_var("GPTEASY_WSL_HARNESS_STOP_AFTER_RUNNING_LISTS");
            }
        }
        let isolated = Command::new(&wrapper)
            .args(["--list", "--running", "--quiet"])
            .output()
            .expect("run isolated WSL list");
        assert!(isolated.status.success(), "run isolated WSL list");
        let isolated = String::from_utf8(isolated.stdout).expect("isolated WSL list is UTF-8");
        if initially_running {
            assert_eq!(isolated.trim(), distribution);
        } else {
            assert!(isolated.trim().is_empty());
        }
        Self {
            _temp: temp,
            program: wrapper.into_os_string(),
            previous_real_exe,
            previous_distribution,
            previous_guest_home,
            previous_state,
            previous_log,
            previous_stop_after,
            state,
            log,
        }
    }

    fn program(&self) -> OsString {
        self.program.clone()
    }

    fn reports_running(&self) -> bool {
        fs::read_to_string(&self.state)
            .expect("read WSL harness lifecycle")
            .trim()
            != "stopped"
    }

    fn set_stopped(&self, stop_after_running_lists: Option<usize>) {
        fs::write(&self.state, "stopped\n").expect("reset WSL harness lifecycle");
        unsafe {
            if let Some(stop_after) = stop_after_running_lists {
                env::set_var(
                    "GPTEASY_WSL_HARNESS_STOP_AFTER_RUNNING_LISTS",
                    stop_after.to_string(),
                );
            } else {
                env::remove_var("GPTEASY_WSL_HARNESS_STOP_AFTER_RUNNING_LISTS");
            }
        }
    }

    fn invocation_log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for WslWrapper {
    fn drop(&mut self) {
        restore_environment(
            "GPTEASY_WSL_HARNESS_REAL_EXE",
            self.previous_real_exe.take(),
        );
        restore_environment(
            "GPTEASY_WSL_HARNESS_DISTRIBUTION",
            self.previous_distribution.take(),
        );
        restore_environment(
            "GPTEASY_WSL_HARNESS_GUEST_HOME",
            self.previous_guest_home.take(),
        );
        restore_environment("GPTEASY_WSL_HARNESS_STATE", self.previous_state.take());
        restore_environment("GPTEASY_WSL_HARNESS_LOG", self.previous_log.take());
        restore_environment(
            "GPTEASY_WSL_HARNESS_STOP_AFTER_RUNNING_LISTS",
            self.previous_stop_after.take(),
        );
    }
}

fn restore_environment(name: &str, value: Option<OsString>) {
    // See WslWrapper::start: the opt-in harness runs without concurrent tests.
    unsafe {
        if let Some(value) = value {
            env::set_var(name, value);
        } else {
            env::remove_var(name);
        }
    }
}

impl RunningGuest {
    fn start(distribution: &str) -> Self {
        let output = checked_output(
            distribution,
            &[
                "/bin/sh",
                "-c",
                "nohup sleep 120 </dev/null >/dev/null 2>&1 & printf '%s\\n' \"$!\"",
            ],
            &[],
        );
        let keeper_pid = String::from_utf8(output)
            .expect("keeper PID is UTF-8")
            .trim()
            .to_owned();
        assert!(
            !keeper_pid.is_empty() && keeper_pid.bytes().all(|byte| byte.is_ascii_digit()),
            "keeper PID must be numeric",
        );
        Self {
            distribution: distribution.to_owned(),
            keeper_pid,
        }
    }
}

impl Drop for RunningGuest {
    fn drop(&mut self) {
        let _ = wsl_output(
            &self.distribution,
            &["/bin/kill", "--", &self.keeper_pid],
            &[],
        );
    }
}

impl Drop for GuestHome {
    fn drop(&mut self) {
        if self.path.starts_with("/tmp/gpteasy-wsl-harness.") {
            let _ = wsl_output(
                &self.distribution,
                &["/bin/rm", "-rf", "--", &self.path],
                &[],
            );
        }
    }
}

fn wsl_output(distribution: &str, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new("wsl.exe")
        .args(["--distribution", distribution, "--exec"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start WSL guest harness command");
    if !stdin.is_empty() {
        child
            .stdin
            .take()
            .expect("guest stdin")
            .write_all(stdin)
            .expect("write guest stdin");
    }
    child
        .wait_with_output()
        .expect("wait for WSL guest harness")
}

fn checked_output(distribution: &str, args: &[&str], stdin: &[u8]) -> Vec<u8> {
    let output = wsl_output(distribution, args, stdin);
    assert!(output.status.success(), "WSL guest harness command failed");
    output.stdout
}

fn bundle(config: &[u8], credential: &[u8]) -> Vec<u8> {
    let mut result = format!(
        "GPTEASY_WSL_BUNDLE_V2\n{}\n{}\n",
        config.len(),
        credential.len()
    )
    .into_bytes();
    result.extend_from_slice(config);
    result.extend_from_slice(credential);
    result
}

fn assert_distribution_is_running(distribution: &str) {
    let output = Command::new("wsl.exe")
        .args(["--list", "--running", "--quiet"])
        .output()
        .expect("list running WSL distributions");
    assert!(output.status.success(), "list running WSL distributions");
    let units = output
        .stdout
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    let names = String::from_utf16_lossy(&units);
    assert!(
        names
            .lines()
            .map(|name| name.trim_matches(['\0', '\u{feff}', '\r']))
            .any(|name| name.eq_ignore_ascii_case(distribution)),
        "the selected WSL2 distribution must already be Running",
    );
}

fn bash_snapshot() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("snapshot temp");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("state database");
    insert_harness_providers(&connection);
    drop(connection);
    let destination = temp.path().join("gpteasy.sh");
    ProviderApplication::new(store, ProviderValidator::new(ValidationTimeouts::default()))
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export Bash snapshot");
    (temp, destination)
}

fn insert_harness_providers(connection: &Connection) {
    for (id, name, api_key, model, sort_order) in [
        (
            "22222222-2222-4222-8222-222222222222",
            "Harness",
            "wsl-harness-secret",
            "model-a",
            0,
        ),
        (
            "33333333-3333-4333-8333-333333333333",
            "Shell Harness",
            "shell-harness-secret",
            "model-b",
            1,
        ),
    ] {
        connection
            .execute(
                "INSERT INTO providers(
                    id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint, sort_order
                 ) VALUES (?1, ?2, 'https://provider.example/v1', ?3, ?4, '1', 'fingerprint', ?5)",
                params![id, name, api_key, model, sort_order],
            )
            .expect("insert harness provider");
    }
}

fn wsl_application_state() -> (TempDir, StateStore) {
    let temp = TempDir::new().expect("WSL application temp");
    let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
    assert!(store.bootstrap().is_ready());
    let connection = Connection::open(store.paths().database()).expect("state database");
    insert_harness_providers(&connection);
    (temp, store)
}

fn windows_path_for_wsl(distribution: &str, path: &Path) -> String {
    String::from_utf8(checked_output(
        distribution,
        &[
            "wslpath",
            "-a",
            "-u",
            path.to_str().expect("UTF-8 snapshot path"),
        ],
        &[],
    ))
    .expect("WSL path is UTF-8")
    .trim()
    .to_owned()
}

#[test]
#[ignore = "requires --features wsl-guest-harness, GPTEASY_RUN_WSL_GUEST_HARNESS=1, and an explicitly selected WSL2 distribution"]
fn running_guest_harness_preserves_auth_and_enforces_the_shared_desktop_lock() {
    assert_eq!(
        std::env::var("GPTEASY_RUN_WSL_GUEST_HARNESS").as_deref(),
        Ok("1"),
        "set GPTEASY_RUN_WSL_GUEST_HARNESS=1 to confirm the isolated WSL harness",
    );
    let distribution =
        std::env::var("GPTEASY_WSL_TEST_DISTRIBUTION").expect("select a WSL2 distribution");
    let _running_guest = RunningGuest::start(&distribution);
    assert_distribution_is_running(&distribution);
    let (_snapshot_temp, snapshot) = bash_snapshot();
    let snapshot = windows_path_for_wsl(&distribution, &snapshot);
    let home_path = String::from_utf8(checked_output(
        &distribution,
        &["/bin/sh", "-c", "mktemp -d /tmp/gpteasy-wsl-harness.XXXXXX"],
        &[],
    ))
    .expect("temporary HOME is UTF-8")
    .trim()
    .to_owned();
    assert!(home_path.starts_with("/tmp/gpteasy-wsl-harness."));
    let guest_home = GuestHome {
        distribution: distribution.clone(),
        path: home_path,
    };
    let home = guest_home.path.as_str();

    let token = "desktop-harness-token";
    let setup = r#"set -eu
home=$1
export HOME=$home
mkdir -p "$HOME/.codex"
printf '%s' '{"login":"unchanged"}' >"$HOME/.codex/auth.json"
chmod 600 "$HOME/.codex/auth.json"
cat >"$HOME/lock" && chmod 700 "$HOME/lock"
"#;
    checked_output(
        &distribution,
        &["/bin/sh", "-c", setup, "gpteasy", home],
        GUEST_LOCK,
    );
    checked_output(
        &distribution,
        &[
            "/usr/bin/env",
            &format!("HOME={home}"),
            "/bin/sh",
            &format!("{home}/lock"),
            "acquire",
            token,
            "switch",
        ],
        &[],
    );
    checked_output(
        &distribution,
        &[
            "/bin/sh",
            "-c",
            "cat >\"$1/writer\" && chmod 700 \"$1/writer\"",
            "gpteasy",
            home,
        ],
        GUEST_WRITER,
    );

    let wrong_token = wsl_output(
        &distribution,
        &[
            "/usr/bin/env",
            &format!("HOME={home}"),
            "/bin/sh",
            &format!("{home}/writer"),
            "wrong-token",
            "missing",
        ],
        &[],
    );
    assert!(!wrong_token.status.success());

    let provider_id = "22222222-2222-4222-8222-222222222222";
    let credential_relative =
        format!(".gpteasy-shell/credentials/desktop-harness/{provider_id}.token");
    let auth_script = format!("cat -- \"${{CODEX_HOME:-$HOME/.codex}}/{credential_relative}\"");
    let config = format!(
        "# >>> GPTEasy managed provider >>>\n\
# GPTEasy schema-version: 1\n\
# GPTEasy provider-id: {provider_id}\n\
# GPTEasy source-id: desktop-harness\n\
# GPTEasy credential-file: {credential_relative}\n\
model = \"model-a\"\n\
model_provider = \"gpteasy\"\n\
model_providers.gpteasy.name = \"Harness\"\n\
model_providers.gpteasy.base_url = \"https://provider.example/v1\"\n\
model_providers.gpteasy.wire_api = \"responses\"\n\
model_providers.gpteasy.supports_websockets = false\n\
model_providers.gpteasy.requires_openai_auth = false\n\
model_providers.gpteasy.auth.command = \"sh\"\n\
 model_providers.gpteasy.auth.args = [\"-c\", '{auth_script}']\n\
# <<< GPTEasy managed provider <<<\n"
    );
    let credential = b"wsl-harness-secret";
    let written = wsl_output(
        &distribution,
        &[
            "/usr/bin/env",
            &format!("HOME={home}"),
            "/bin/sh",
            &format!("{home}/writer"),
            token,
            "missing",
        ],
        &bundle(config.as_bytes(), credential),
    );
    assert!(
        written.status.success(),
        "guest writer rejected the valid bundle: status={:?}, response={}, error={}",
        written.status.code(),
        String::from_utf8_lossy(&written.stdout),
        String::from_utf8_lossy(&written.stderr)
    );

    let read_file = |relative: &str| {
        checked_output(
            &distribution,
            &[
                "/usr/bin/env",
                &format!("HOME={home}"),
                "/bin/sh",
                "-c",
                "cat -- \"$HOME/$1\"",
                "gpteasy",
                relative,
            ],
            &[],
        )
    };
    assert!(read_file(".codex/config.toml") == config.as_bytes());
    assert!(read_file(&format!(".codex/{credential_relative}")) == credential);
    assert!(read_file(".codex/auth.json") == br#"{"login":"unchanged"}"#);

    let lock_reference =
        ".gpteasy-shell/credentials/desktop-lock/44444444-4444-4444-8444-444444444444.token";
    let shell_restore_reference =
        ".gpteasy-shell/credentials/shell-restore/66666666-6666-4666-8666-666666666666.token";
    let desktop_backup_reference =
        ".gpteasy-shell/credentials/desktop-backup/77777777-7777-4777-8777-777777777777.token";
    let stale_credential =
        ".gpteasy-shell/credentials/desktop-stale/55555555-5555-4555-8555-555555555555.token";
    checked_output(
        &distribution,
        &[
            "/usr/bin/env",
            &format!("HOME={home}"),
            "/bin/sh",
            "-c",
            r#"set -eu
for relative in "$1" "$2" "$3" "$4"; do
  directory="$HOME/.codex/${relative%/*}"
  mkdir -m 700 -p "$directory"
  printf '%s' 'credential-canary' >"$HOME/.codex/$relative"
  chmod 600 "$HOME/.codex/$relative"
done
printf '%s\n' "$1" >>"$HOME/.codex/.gpteasy-shell/lock/active/references"
chmod 600 "$HOME/.codex/.gpteasy-shell/lock/active/references"
mkdir -p "$HOME/.codex/.gpteasy-shell/shell-restore/switch-1"
chmod 700 "$HOME/.codex/.gpteasy-shell/shell-restore" "$HOME/.codex/.gpteasy-shell/shell-restore/switch-1"
printf '# GPTEasy credential-file: %s\n' "$2" >"$HOME/.codex/.gpteasy-shell/shell-restore/switch-1/config.toml"
chmod 600 "$HOME/.codex/.gpteasy-shell/shell-restore/switch-1/config.toml"
printf '# GPTEasy credential-file: %s\n' "$3" >"$HOME/.codex/.gpteasy-shell/desktop-backups/config-1.toml"
chmod 600 "$HOME/.codex/.gpteasy-shell/desktop-backups/config-1.toml"
"#,
            "gpteasy",
            lock_reference,
            shell_restore_reference,
            desktop_backup_reference,
            stale_credential,
        ],
        &[],
    );
    checked_output(
        &distribution,
        &[
            "/usr/bin/env",
            &format!("HOME={home}"),
            "/bin/sh",
            "-c",
            "mv \"$HOME/.codex/config.toml\" \"$HOME/config.toml\" && ln -s \"$HOME/config.toml\" \"$HOME/.codex/config.toml\"",
        ],
        &[],
    );
    let cleaned = wsl_output(
        &distribution,
        &[
            "/usr/bin/env",
            &format!("HOME={home}"),
            "/bin/sh",
            "-c",
            GUEST_CREDENTIAL_CLEANUP,
            "gpteasy",
            token,
        ],
        &[],
    );
    assert!(
        cleaned.status.success(),
        "credential cleanup failed: status={:?}, response={}, error={}",
        cleaned.status.code(),
        String::from_utf8_lossy(&cleaned.stdout),
        String::from_utf8_lossy(&cleaned.stderr),
    );
    let credential_exists = |relative: &str| {
        wsl_output(
            &distribution,
            &[
                "/usr/bin/env",
                &format!("HOME={home}"),
                "/bin/sh",
                "-c",
                "test -f \"$HOME/.codex/$1\"",
                "gpteasy",
                relative,
            ],
            &[],
        )
        .status
        .success()
    };
    assert!(credential_exists(&credential_relative));
    assert!(credential_exists(lock_reference));
    assert!(credential_exists(shell_restore_reference));
    assert!(credential_exists(desktop_backup_reference));
    assert!(!credential_exists(stale_credential));
    assert!(read_file(".codex/auth.json") == br#"{"login":"unchanged"}"#);

    let read_private = || {
        wsl_output(
            &distribution,
            &[
                "/usr/bin/env",
                &format!("HOME={home}"),
                "/bin/sh",
                "-c",
                GUEST_PRIVATE_READER,
                "gpteasy",
                &credential_relative,
            ],
            &[],
        )
    };
    let private = read_private();
    assert!(private.status.success());
    assert_eq!(private.stdout, credential);

    let alter_credential = |command: &str| {
        checked_output(
            &distribution,
            &[
                "/usr/bin/env",
                &format!("HOME={home}"),
                "/bin/sh",
                "-c",
                command,
                "gpteasy",
                &credential_relative,
            ],
            &[],
        );
    };
    alter_credential(
        "target=\"$HOME/credential-target\"; file=\"$HOME/.codex/$1\"; mv \"$file\" \"$target\"; ln -s \"$target\" \"$file\"",
    );
    assert_eq!(read_private().status.code(), Some(43));
    alter_credential(
        "target=\"$HOME/credential-target\"; file=\"$HOME/.codex/$1\"; rm \"$file\"; ln \"$target\" \"$file\"",
    );
    assert_eq!(read_private().status.code(), Some(43));
    alter_credential(
        "target=\"$HOME/credential-target\"; file=\"$HOME/.codex/$1\"; rm \"$target\"; chmod 644 \"$file\"",
    );
    assert_eq!(read_private().status.code(), Some(43));
    alter_credential("chmod 600 \"$HOME/.codex/$1\"");
    assert!(read_private().status.success());
    alter_credential(
        "file=\"$HOME/.codex/$1\"; source=${file%/*}; mv \"$source\" \"$HOME/credential-source\"; ln -s \"$HOME/credential-source\" \"$source\"",
    );
    assert_eq!(read_private().status.code(), Some(43));
    alter_credential(
        "file=\"$HOME/.codex/$1\"; source=${file%/*}; rm \"$source\"; mv \"$HOME/credential-source\" \"$source\"",
    );
    assert!(read_private().status.success());
    assert!(read_file(".codex/auth.json") == br#"{"login":"unchanged"}"#);

    let install_snapshot = r#"set -eu
home=$1
snapshot=$2
export HOME=$home
mkdir -p "$HOME/bin"
cp -- "$snapshot" "$HOME/gpteasy.sh"
chmod 600 "$HOME/gpteasy.sh"
cat >"$HOME/bin/codex" <<'CODEX'
#!/bin/sh
printf '%s\n' 'codex-cli 0.147.0'
CODEX
chmod 700 "$HOME/bin/codex"
"#;
    checked_output(
        &distribution,
        &[
            "/bin/sh",
            "-c",
            install_snapshot,
            "gpteasy",
            home,
            &snapshot,
        ],
        &[],
    );
    let shell_home = format!("HOME={home}");
    let shell_codex_home = format!("CODEX_HOME={home}/.codex");
    let shell_path =
        format!("PATH={home}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin");
    let run_shell = |command: &str, stdin: &[u8]| {
        wsl_output(
            &distribution,
            &[
                "/usr/bin/env",
                &shell_home,
                &shell_codex_home,
                &shell_path,
                "/bin/bash",
                "-c",
                command,
            ],
            stdin,
        )
    };

    let (_application_temp, application_store) = wsl_application_state();
    let _wsl_wrapper = WslWrapper::start(&distribution, home);
    let application = WslApplication::with_wsl_program_for_harness(
        application_store.clone(),
        _wsl_wrapper.program(),
    );
    let busy = application
        .list()
        .expect("desktop observes its active lock")
        .into_iter()
        .find(|environment| {
            environment.running && environment.display_name.eq_ignore_ascii_case(&distribution)
        })
        .expect("selected WSL environment");
    assert_eq!(
        busy.configuration_state,
        WslConfigurationState::Busy,
        "unexpected isolated desktop state: {busy:?}",
    );

    let desktop_current = run_shell("source \"$HOME/gpteasy.sh\"; gpteasy current", &[]);
    assert!(
        desktop_current.status.success(),
        "shell current rejected desktop config: stdout={}, stderr={}",
        String::from_utf8_lossy(&desktop_current.stdout),
        String::from_utf8_lossy(&desktop_current.stderr),
    );
    assert!(String::from_utf8_lossy(&desktop_current.stdout).contains("Harness"));

    let blocked = run_shell("source \"$HOME/gpteasy.sh\"; gpteasy", b"2\n");
    assert!(
        !blocked.status.success(),
        "desktop lock must block shell switch"
    );
    assert!(!String::from_utf8_lossy(&blocked.stdout).contains("wsl-harness-secret"));
    assert!(!String::from_utf8_lossy(&blocked.stderr).contains("wsl-harness-secret"));

    let connection =
        Connection::open(application_store.paths().database()).expect("state database");
    connection
        .execute(
            "UPDATE wsl_environments SET refresh_lock_token = ?2 WHERE environment_id = ?1",
            params![busy.environment_id, token],
        )
        .expect("persist interrupted desktop lock token");
    drop(connection);
    let recovered_application =
        WslApplication::with_wsl_program_for_harness(application_store, _wsl_wrapper.program());
    let desktop_refreshed = recovered_application
        .list()
        .expect("recover persisted desktop lock and refresh")
        .into_iter()
        .find(|environment| {
            environment.running && environment.display_name.eq_ignore_ascii_case(&distribution)
        })
        .expect("selected WSL environment");
    assert_eq!(
        desktop_refreshed.configuration_state,
        WslConfigurationState::Current
    );
    assert_eq!(
        desktop_refreshed.actual_provider_id.as_deref(),
        Some(provider_id)
    );

    let switched = run_shell("source \"$HOME/gpteasy.sh\"; gpteasy", b"2\n");
    assert!(
        switched.status.success(),
        "shell switch failed after desktop lock recovery: {}",
        String::from_utf8_lossy(&switched.stderr),
    );
    let shell_config = read_file(".codex/config.toml");
    let shell_config_text = String::from_utf8(shell_config).expect("shell config is UTF-8");
    assert!(shell_config_text.contains("# GPTEasy schema-version: 1"));
    assert!(
        shell_config_text.contains("# GPTEasy provider-id: 33333333-3333-4333-8333-333333333333")
    );
    assert!(shell_config_text.contains("# GPTEasy source-id:"));
    assert!(!shell_config_text.contains("shell-harness-secret"));
    assert!(read_file(".codex/auth.json") == br#"{"login":"unchanged"}"#);
    let shell_refreshed = recovered_application
        .list()
        .expect("desktop refreshes the shell switch")
        .into_iter()
        .find(|environment| {
            environment.running && environment.display_name.eq_ignore_ascii_case(&distribution)
        })
        .expect("selected WSL environment");
    assert_eq!(
        shell_refreshed.configuration_state,
        WslConfigurationState::Current
    );
    assert_eq!(
        shell_refreshed.actual_provider_id.as_deref(),
        Some("33333333-3333-4333-8333-333333333333")
    );
    assert!(shell_refreshed.pending_restart);

    checked_output(
        &distribution,
        &[
            "/usr/bin/env",
            &shell_home,
            "/bin/sh",
            "-c",
            r##"awk '
$0 == "# GPTEasy schema-version: 1" { next }
index($0, "# GPTEasy source-id:") == 1 { next }
index($0, "# GPTEasy credential-file:") == 1 { next }
index($0, "model_providers.gpteasy.auth.") == 1 { next }
{ print }
' "$HOME/.codex/config.toml" >"$HOME/.codex/legacy.toml"
chmod 600 "$HOME/.codex/legacy.toml"
mv "$HOME/.codex/legacy.toml" "$HOME/.codex/config.toml""##,
        ],
        &[],
    );
    let legacy = run_shell("source \"$HOME/gpteasy.sh\"; gpteasy current", &[]);
    assert!(legacy.status.success());
    assert!(String::from_utf8_lossy(&legacy.stdout).contains("旧格式"));
    let migrated = run_shell("source \"$HOME/gpteasy.sh\"; gpteasy", b"1\n");
    assert!(migrated.status.success());
    let migrated_config = read_file(".codex/config.toml");
    let migrated_text = String::from_utf8(migrated_config).expect("migrated config is UTF-8");
    assert!(migrated_text.contains("# GPTEasy schema-version: 1"));
    assert!(migrated_text.contains("# GPTEasy provider-id: 22222222-2222-4222-8222-222222222222"));
    assert!(read_file(".codex/auth.json") == br#"{"login":"unchanged"}"#);
    let migrated_refreshed = recovered_application
        .list()
        .expect("desktop refreshes the migrated shell switch")
        .into_iter()
        .find(|environment| {
            environment.running && environment.display_name.eq_ignore_ascii_case(&distribution)
        })
        .expect("selected WSL environment");
    assert_eq!(
        migrated_refreshed.configuration_state,
        WslConfigurationState::Current
    );
    assert_eq!(
        migrated_refreshed.actual_provider_id.as_deref(),
        Some(provider_id)
    );
}

#[test]
#[ignore = "requires --features wsl-guest-harness, GPTEASY_RUN_WSL_GUEST_HARNESS=1, and an explicitly selected WSL2 distribution"]
fn stopped_guest_harness_authorizes_start_and_never_forces_termination() {
    assert_eq!(
        std::env::var("GPTEASY_RUN_WSL_GUEST_HARNESS").as_deref(),
        Ok("1"),
        "set GPTEASY_RUN_WSL_GUEST_HARNESS=1 to confirm the isolated WSL harness",
    );
    let distribution =
        std::env::var("GPTEASY_WSL_TEST_DISTRIBUTION").expect("select a WSL2 distribution");
    let _running_guest = RunningGuest::start(&distribution);
    let home_path = String::from_utf8(checked_output(
        &distribution,
        &["/bin/sh", "-c", "mktemp -d /tmp/gpteasy-wsl-harness.XXXXXX"],
        &[],
    ))
    .expect("temporary HOME is UTF-8")
    .trim()
    .to_owned();
    let guest_home = GuestHome {
        distribution: distribution.clone(),
        path: home_path,
    };
    let home = guest_home.path.as_str();
    checked_output(
        &distribution,
        &[
            "/bin/sh",
            "-c",
            r#"set -eu
home=$1
mkdir -p "$home/.codex" "$home/bin"
printf '%s\n' 'custom = true' >"$home/.codex/config.toml"
printf '%s' '{"login":"unchanged"}' >"$home/.codex/auth.json"
chmod 600 "$home/.codex/auth.json"
cat >"$home/bin/codex" <<'CODEX'
#!/bin/sh
printf '%s\n' 'codex-cli 0.147.0'
CODEX
chmod 700 "$home/bin/codex"
"#,
            "gpteasy",
            home,
        ],
        &[],
    );

    let (_application_temp, application_store) = wsl_application_state();
    let wrapper = WslWrapper::start_with_lifecycle(&distribution, home, false, Some(3));
    let application = WslApplication::with_wsl_program_and_timeout_for_harness(
        application_store.clone(),
        wrapper.program(),
        Duration::from_secs(1),
    );
    let stopped = application
        .list()
        .expect("side-effect free stopped probe")
        .into_iter()
        .find(|environment| {
            environment.availability == WslAvailability::Manageable
                && environment.display_name.eq_ignore_ascii_case(&distribution)
        })
        .expect("selected stopped WSL environment");
    assert!(!stopped.running);
    assert_eq!(stopped.availability, WslAvailability::Manageable);
    assert_eq!(stopped.configuration_state, WslConfigurationState::Unknown);

    let denied = application
        .apply_provider(
            &stopped.environment_id,
            "22222222-2222-4222-8222-222222222222",
            &stopped.revision,
            false,
        )
        .expect_err("explicit confirmation is required");
    assert_eq!(denied.message_id, "wsl.confirmation_required");
    assert!(!wrapper.reports_running());

    let applied = application
        .apply_provider(
            &stopped.environment_id,
            "22222222-2222-4222-8222-222222222222",
            &stopped.revision,
            true,
        )
        .unwrap_or_else(|failure| {
            panic!(
                "apply to temporarily started WSL environment: {failure:?}\n{}",
                wrapper.invocation_log()
            )
        });
    assert_eq!(
        applied.lifecycle_outcome,
        WslLifecycleOutcome::StoppedNaturally
    );
    assert!(!applied.environment.running);
    assert!(!wrapper.reports_running());

    wrapper.set_stopped(None);
    let running_application = WslApplication::with_wsl_program_and_timeout_for_harness(
        application_store,
        wrapper.program(),
        Duration::ZERO,
    );
    let stopped_again = running_application
        .list()
        .expect("observe stopped again")
        .into_iter()
        .find(|environment| {
            environment.availability == WslAvailability::Manageable
                && environment.display_name.eq_ignore_ascii_case(&distribution)
        })
        .expect("selected stopped WSL environment");
    let remained_running = running_application
        .apply_provider(
            &stopped_again.environment_id,
            "22222222-2222-4222-8222-222222222222",
            &stopped_again.revision,
            true,
        )
        .expect("apply while a user workload keeps WSL Running");
    assert_eq!(
        remained_running.lifecycle_outcome,
        WslLifecycleOutcome::StillRunning
    );
    assert!(remained_running.environment.running);

    wrapper.set_stopped(None);
    let blocked_without_authorization = running_application
        .audit_provider_deletion("22222222-2222-4222-8222-222222222222", false)
        .expect_err("stopped deletion audit needs authorization");
    assert_eq!(
        blocked_without_authorization.message_id,
        "wsl.delete_start_authorization_required"
    );
    let blocked_by_actual_config = running_application
        .audit_provider_deletion("22222222-2222-4222-8222-222222222222", true)
        .expect_err("actual guest config protects the provider");
    assert_eq!(
        blocked_by_actual_config.message_id,
        "provider.wsl_current_delete_forbidden"
    );
    assert_eq!(
        checked_output(
            &distribution,
            &[
                "/bin/sh",
                "-c",
                "cat -- \"$1/.codex/auth.json\"",
                "gpteasy",
                home
            ],
            &[],
        ),
        br#"{"login":"unchanged"}"#
    );
    assert!(!wrapper.invocation_log().contains("--terminate"));
}
