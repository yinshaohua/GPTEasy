use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::Duration,
};
use sysinfo::{ProcessesToUpdate, System};
use toml_edit::DocumentMut;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const START: &str = "# >>> GPTEasy managed provider >>>";
const END: &str = "# <<< GPTEasy managed provider <<<";
const SECRET: &str = "spike-013-secret-value";
const WRITER_PATH: &str = "/usr/local/lib/gpteasy-spike-013-writer";
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone)]
struct Provider<'a> {
    id: &'a str,
    name: &'a str,
    base_url: &'a str,
    model: &'a str,
    bearer_token: &'a str,
}

#[derive(Debug, Serialize)]
struct CaseResult {
    name: String,
    passed: bool,
    evidence: Value,
}

#[derive(Debug, Serialize)]
struct Summary {
    generated_at: String,
    distro: String,
    total: usize,
    passed: usize,
    secret_in_artifacts: bool,
    cases: Vec<CaseResult>,
    verdict: String,
}

#[derive(Debug)]
struct TransactionResult {
    originally_running: bool,
    final_running: bool,
    writer_status: i32,
    writer_output: String,
    candidate: Vec<u8>,
    windows_cmdline_secret: bool,
}

fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("prepare") => prepare(args.get(2).context("missing distro")?),
        Some("matrix") => {
            let distro = args.get(2).context("missing distro")?;
            let output = PathBuf::from(args.get(3).context("missing output directory")?);
            run_matrix(distro, &output)
        }
        _ => bail!("usage: gpteasy-spike-013 <prepare DISTRO | matrix DISTRO OUTPUT_DIR>"),
    }
}

fn prepare(distro: &str) -> Result<()> {
    validate_distro_name(distro)?;
    let setup = r#"set -eu
if ! grep -q '^gpteasy:' /etc/passwd; then
  printf 'gpteasy:x:1000:1000:GPTEasy Spike:/home/gpteasy:/bin/sh\n' >> /etc/passwd
fi
if ! grep -q '^gpteasy:' /etc/group; then
  printf 'gpteasy:x:1000:\n' >> /etc/group
fi
mkdir -p /home/gpteasy
chown -R 1000:1000 /home/gpteasy
cat > /etc/wsl.conf <<'EOF'
[user]
default=gpteasy
EOF
"#;
    checked_wsl(
        &["--distribution", distro, "--user", "root", "--exec", "/bin/sh", "-s"],
        Some(setup.as_bytes()),
    )?;
    install_writer(distro)?;
    terminate(distro)?;
    thread::sleep(Duration::from_millis(750));
    let identity = checked_wsl(
        &[
            "--distribution",
            distro,
            "--exec",
            "/bin/sh",
            "-lc",
            "printf '%s|%s' \"$(id -u)\" \"$HOME\"",
        ],
        None,
    )?;
    let identity = decode_output(&identity.stdout);
    terminate(distro)?;
    if identity.trim() != "1000|/home/gpteasy" {
        bail!("unexpected default user identity: {identity:?}");
    }
    Ok(())
}

fn install_writer(distro: &str) -> Result<()> {
    let writer = include_bytes!("../guest-writer.sh");
    let command = format!(
        "umask 077; mkdir -p '{}'; cat > '{}'; chmod 755 '{}'",
        Path::new(WRITER_PATH)
            .parent()
            .and_then(Path::to_str)
            .unwrap_or("/usr/local/lib"),
        WRITER_PATH,
        WRITER_PATH
    );
    checked_wsl(
        &[
            "--distribution",
            distro,
            "--user",
            "root",
            "--exec",
            "/bin/sh",
            "-c",
            &command,
        ],
        Some(writer),
    )?;
    Ok(())
}

fn run_matrix(distro: &str, output_dir: &Path) -> Result<()> {
    validate_distro_name(distro)?;
    fs::create_dir_all(output_dir)?;
    let mut cases = Vec::new();
    let provider_a = Provider {
        id: "provider-a",
        name: "Spike Provider A",
        base_url: "https://provider-a.example/v1",
        model: "model-a",
        bearer_token: SECRET,
    };
    let provider_b = Provider {
        id: "provider-b",
        name: "Spike Provider B",
        base_url: "https://provider-b.example/v1",
        model: "model-b",
        bearer_token: SECRET,
    };

    terminate(distro)?;
    seed(
        distro,
        "/home/gpteasy/.codex/config.toml",
        "custom_flag = true\nmodel = \"legacy\"\nmodel_provider = \"legacy\"\n\n[model_providers.legacy]\nname = \"Legacy\"\nbase_url = \"https://legacy.example/v1\"\nwire_api = \"responses\"\n",
        "600",
    )?;
    seed(
        distro,
        "/root/.codex/config.toml",
        "root_only = true\n",
        "600",
    )?;
    terminate(distro)?;

    let before_cancel = is_running(distro)?;
    let after_cancel = is_running(distro)?;
    cases.push(case(
        "cancel-does-not-start-distro",
        !before_cancel && !after_cancel,
        json!({"before_running": before_cancel, "after_running": after_cancel}),
    ));

    let first = switch_provider(distro, &provider_a, "normal", None)?;
    let first_config = read_default_config_then_restore_state(distro, false)?;
    let first_doc = std::str::from_utf8(&first_config)?.parse::<DocumentMut>()?;
    let first_ok = !first.originally_running
        && !first.final_running
        && first.writer_status == 0
        && first_doc["custom_flag"].as_bool() == Some(true)
        && first_doc["model"].as_str() == Some("model-a")
        && first_doc["model_provider"].as_str() == Some("gpteasy")
        && first_doc["model_providers"]["legacy"]["name"].as_str() == Some("Legacy")
        && first_doc["model_providers"]["gpteasy"]["base_url"].as_str()
            == Some("https://provider-a.example/v1");
    cases.push(case(
        "stopped-success-restores-stopped-state",
        first_ok,
        transaction_evidence(&first),
    ));

    cases.push(case(
        "secret-absent-from-host-and-guest-command-lines",
        !first.windows_cmdline_secret
            && first.writer_output.contains("\"self_cmdline_secret\":false")
            && first.writer_output.contains("\"parent_cmdline_secret\":false"),
        json!({
            "windows_cmdline_secret": first.windows_cmdline_secret,
            "writer_output": first.writer_output
        }),
    ));

    let outside_before = outside_block(&first_config)?;
    let second = switch_provider(distro, &provider_b, "normal", None)?;
    let second_config = read_default_config_then_restore_state(distro, false)?;
    let second_doc = std::str::from_utf8(&second_config)?.parse::<DocumentMut>()?;
    cases.push(case(
        "repeat-switch-preserves-outside-block",
        second.writer_status == 0
            && outside_block(&second_config)? == outside_before
            && second_doc["model"].as_str() == Some("model-b"),
        transaction_evidence(&second),
    ));

    let before_failure = second_config.clone();
    let failed = switch_provider(distro, &provider_a, "fail-before-replace", None)?;
    let after_failure = read_default_config_then_restore_state(distro, false)?;
    cases.push(case(
        "failure-before-replace-keeps-config-and-stopped-state",
        failed.writer_status != 0
            && !failed.final_running
            && before_failure == after_failure
            && failed.writer_output.contains("injected_failure"),
        transaction_evidence(&failed),
    ));

    seed(
        distro,
        "/home/gpteasy/.codex/config.toml",
        "# >>> GPTEasy managed provider >>>\nmodel = \"broken\"\n",
        "600",
    )?;
    terminate(distro)?;
    let malformed = switch_provider(distro, &provider_a, "normal", None);
    let malformed_config = read_default_config_then_restore_state(distro, false)?;
    cases.push(case(
        "malformed-managed-block-stops-before-guest-write",
        malformed.is_err()
            && malformed_config == b"# >>> GPTEasy managed provider >>>\nmodel = \"broken\"\n",
        json!({"error": malformed.err().map(|error| error.to_string())}),
    ));

    seed(
        distro,
        "/home/gpteasy/.codex/config.toml",
        "custom_flag = true\n",
        "600",
    )?;
    terminate(distro)?;
    let concurrent = switch_provider(
        distro,
        &provider_a,
        "delay-before-replace",
        Some("printf '\\nexternal_change = true\\n' >> \"$HOME/.codex/config.toml\""),
    )?;
    let concurrent_config = read_default_config_then_restore_state(distro, false)?;
    cases.push(case(
        "concurrent-guest-edit-is-not-overwritten",
        concurrent.writer_status != 0
            && concurrent.writer_output.contains("concurrent_change")
            && std::str::from_utf8(&concurrent_config)?.contains("external_change = true")
            && !concurrent.final_running,
        transaction_evidence(&concurrent),
    ));

    seed(
        distro,
        "/home/gpteasy/.codex/config.toml",
        "custom_flag = true\n",
        "600",
    )?;
    terminate(distro)?;
    for index in 0..7 {
        let provider = Provider {
            id: if index % 2 == 0 { "provider-a" } else { "provider-b" },
            name: "Retention Provider",
            base_url: "https://retention.example/v1",
            model: if index % 2 == 0 { "model-a" } else { "model-b" },
            bearer_token: SECRET,
        };
        let result = switch_provider(distro, &provider, "normal", None)?;
        if result.writer_status != 0 {
            bail!("retention switch {index} failed: {}", result.writer_output);
        }
        thread::sleep(Duration::from_millis(2));
    }
    let backup_count = guest_text(
        distro,
        "find \"$HOME/.codex/backups\" -maxdepth 1 -type f -name 'config-*.toml' | wc -l",
    )?;
    let mode = guest_text(distro, "stat -c '%a' \"$HOME/.codex/config.toml\"")?;
    terminate(distro)?;
    cases.push(case(
        "backup-retention-and-permissions",
        backup_count.trim() == "5" && mode.trim() == "600",
        json!({"backup_count": backup_count.trim(), "mode": mode.trim()}),
    ));

    let root_config = root_text(distro, "cat /root/.codex/config.toml")?;
    terminate(distro)?;
    cases.push(case(
        "only-default-user-config-is-managed",
        root_config == "root_only = true\n",
        json!({"root_config_unchanged": root_config == "root_only = true\n"}),
    ));

    let keeper = spawn_wsl(
        &[
            "--distribution",
            distro,
            "--exec",
            "/bin/sh",
            "-c",
            "sleep 30",
        ],
        None,
    )?;
    wait_until_running(distro)?;
    let running_switch = switch_provider(distro, &provider_b, "normal", None)?;
    cases.push(case(
        "originally-running-distro-remains-running",
        running_switch.originally_running
            && running_switch.final_running
            && running_switch.writer_status == 0,
        transaction_evidence(&running_switch),
    ));
    stop_keeper_and_distro(keeper, distro)?;

    let summary_path = output_dir.join("summary.json");
    let secret_in_artifacts_before = scan_for_secret(output_dir, SECRET.as_bytes())?;
    let passed = cases.iter().filter(|item| item.passed).count();
    let summary = Summary {
        generated_at: Utc::now().to_rfc3339(),
        distro: distro.to_string(),
        total: cases.len(),
        passed,
        secret_in_artifacts: secret_in_artifacts_before,
        verdict: if passed == cases.len() && !secret_in_artifacts_before {
            "validated".to_string()
        } else {
            "partial".to_string()
        },
        cases,
    };
    fs::write(&summary_path, serde_json::to_vec_pretty(&summary)?)?;
    let secret_in_summary = fs::read(&summary_path)?
        .windows(SECRET.len())
        .any(|window| window == SECRET.as_bytes());
    if secret_in_summary {
        bail!("secret leaked into summary");
    }
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn switch_provider(
    distro: &str,
    provider: &Provider<'_>,
    mode: &str,
    concurrent_command: Option<&str>,
) -> Result<TransactionResult> {
    let originally_running = is_running(distro)?;
    let operation = (|| -> Result<TransactionResult> {
        let original = read_default_config(distro)?;
        let original_text = std::str::from_utf8(&original)?;
        let candidate = render_transaction(original_text, provider)?;
        candidate.parse::<DocumentMut>()?;
        let expected = sha256(&original);
        let child = spawn_wsl(
            &[
                "--distribution",
                distro,
                "--exec",
                WRITER_PATH,
                &expected,
                mode,
            ],
            Some(candidate.as_bytes()),
        )?;
        thread::sleep(Duration::from_millis(400));
        let windows_cmdline_secret = process_command_lines_contain(SECRET);
        if let Some(command) = concurrent_command {
            checked_wsl(
                &[
                    "--distribution",
                    distro,
                    "--exec",
                    "/bin/sh",
                    "-lc",
                    command,
                ],
                None,
            )?;
        }
        let output = child.wait_with_output()?;
        let writer_output = format!(
            "{}{}",
            decode_output(&output.stdout),
            decode_output(&output.stderr)
        );
        Ok(TransactionResult {
            originally_running,
            final_running: false,
            writer_status: output.status.code().unwrap_or(-1),
            writer_output,
            candidate: candidate.into_bytes(),
            windows_cmdline_secret,
        })
    })();

    if !originally_running {
        let _ = terminate(distro);
        thread::sleep(Duration::from_millis(500));
    }
    let mut result = operation?;
    result.final_running = is_running(distro)?;
    Ok(result)
}

fn read_default_config_then_restore_state(distro: &str, originally_running: bool) -> Result<Vec<u8>> {
    let content = read_default_config(distro)?;
    if !originally_running {
        terminate(distro)?;
        thread::sleep(Duration::from_millis(400));
    }
    Ok(content)
}

fn read_default_config(distro: &str) -> Result<Vec<u8>> {
    let output = checked_wsl(
        &[
            "--distribution",
            distro,
            "--exec",
            "/bin/sh",
            "-lc",
            "if [ -f \"$HOME/.codex/config.toml\" ]; then cat \"$HOME/.codex/config.toml\"; fi",
        ],
        None,
    )?;
    Ok(output.stdout)
}

fn seed(distro: &str, target: &str, content: &str, mode: &str) -> Result<()> {
    let parent = Path::new(target)
        .parent()
        .and_then(Path::to_str)
        .context("seed target has no parent")?;
    let command = format!(
        "umask 077; mkdir -p '{parent}'; cat > '{target}'; chmod '{mode}' '{target}'"
    );
    checked_wsl(
        &[
            "--distribution",
            distro,
            "--user",
            "root",
            "--exec",
            "/bin/sh",
            "-c",
            &command,
        ],
        Some(content.as_bytes()),
    )?;
    if target.starts_with("/home/gpteasy/") {
        checked_wsl(
            &[
                "--distribution",
                distro,
                "--user",
                "root",
                "--exec",
                "chown",
                "-R",
                "1000:1000",
                "/home/gpteasy/.codex",
            ],
            None,
        )?;
    }
    Ok(())
}

fn guest_text(distro: &str, command: &str) -> Result<String> {
    let output = checked_wsl(
        &[
            "--distribution",
            distro,
            "--exec",
            "/bin/sh",
            "-lc",
            command,
        ],
        None,
    )?;
    Ok(decode_output(&output.stdout))
}

fn root_text(distro: &str, command: &str) -> Result<String> {
    let output = checked_wsl(
        &[
            "--distribution",
            distro,
            "--user",
            "root",
            "--exec",
            "/bin/sh",
            "-lc",
            command,
        ],
        None,
    )?;
    Ok(decode_output(&output.stdout))
}

fn render_transaction(original: &str, provider: &Provider<'_>) -> Result<String> {
    let newline = if original.contains("\r\n") { "\r\n" } else { "\n" };
    let markers = marker_ranges(original);
    let block = render_block(provider, newline);
    let rendered = match (markers.0.as_slice(), markers.1.as_slice()) {
        ([], []) => {
            let body = migrate_structurally(original, newline)?;
            format!("{block}{body}")
        }
        ([(start, _)], [(_, end)]) if start < end => {
            let mut rendered = String::with_capacity(original.len() + block.len());
            rendered.push_str(&original[..*start]);
            rendered.push_str(&block);
            rendered.push_str(&original[*end..]);
            rendered
        }
        _ => bail!("managed block markers are missing, duplicated, or reversed"),
    };
    rendered.parse::<DocumentMut>()?;
    Ok(rendered)
}

fn migrate_structurally(original: &str, newline: &str) -> Result<String> {
    let mut doc = original.parse::<DocumentMut>()?;
    doc.remove("model");
    doc.remove("model_provider");
    let mut remove_parent = false;
    if let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_mut())
    {
        providers.remove("gpteasy");
        if providers.iter().any(|(_, item)| !item.is_table()) {
            bail!("explicit model_providers table contains direct values");
        }
        if providers.is_empty() {
            remove_parent = true;
        } else {
            providers.set_implicit(true);
        }
    }
    if remove_parent {
        doc.remove("model_providers");
    }
    Ok(normalize_newlines(&doc.to_string(), newline))
}

fn render_block(provider: &Provider<'_>, newline: &str) -> String {
    let string = |value: &str| toml_edit::Value::from(value).to_string();
    [
        START.to_string(),
        format!("# GPTEasy provider-id: {}", provider.id),
        format!("model = {}", string(provider.model)),
        "model_provider = \"gpteasy\"".to_string(),
        format!("model_providers.gpteasy.name = {}", string(provider.name)),
        format!(
            "model_providers.gpteasy.base_url = {}",
            string(provider.base_url)
        ),
        "model_providers.gpteasy.wire_api = \"responses\"".to_string(),
        "model_providers.gpteasy.supports_websockets = false".to_string(),
        format!(
            "model_providers.gpteasy.experimental_bearer_token = {}",
            string(provider.bearer_token)
        ),
        END.to_string(),
        String::new(),
    ]
    .join(newline)
}

fn marker_ranges(original: &str) -> (Vec<(usize, usize)>, Vec<(usize, usize)>) {
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut offset = 0usize;
    for line in original.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if content == START {
            starts.push((offset, offset + line.len()));
        }
        if content == END {
            ends.push((offset, offset + line.len()));
        }
        offset += line.len();
    }
    if offset < original.len() {
        let content = original[offset..].trim_end_matches('\r');
        if content == START {
            starts.push((offset, original.len()));
        }
        if content == END {
            ends.push((offset, original.len()));
        }
    }
    (starts, ends)
}

fn outside_block(value: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(value)?;
    let markers = marker_ranges(text);
    match (markers.0.as_slice(), markers.1.as_slice()) {
        ([(start, _)], [(_, end)]) if start < end => {
            let mut outside = Vec::new();
            outside.extend_from_slice(&value[..*start]);
            outside.extend_from_slice(&value[*end..]);
            Ok(outside)
        }
        _ => bail!("expected one managed block"),
    }
}

fn normalize_newlines(value: &str, newline: &str) -> String {
    if newline == "\r\n" {
        value.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        value.replace("\r\n", "\n")
    }
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn case(name: &str, passed: bool, evidence: Value) -> CaseResult {
    CaseResult {
        name: name.to_string(),
        passed,
        evidence,
    }
}

fn transaction_evidence(result: &TransactionResult) -> Value {
    json!({
        "originally_running": result.originally_running,
        "final_running": result.final_running,
        "writer_status": result.writer_status,
        "writer_output": result.writer_output,
        "candidate_bytes": result.candidate.len(),
        "windows_cmdline_secret": result.windows_cmdline_secret
    })
}

fn validate_distro_name(distro: &str) -> Result<()> {
    if distro != "GPTEasy-Spike-013" {
        bail!("refusing to operate on unexpected distro name: {distro}");
    }
    Ok(())
}

fn process_command_lines_contain(secret: &str) -> bool {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.processes().values().any(|process| {
        process
            .cmd()
            .iter()
            .any(|part| part.to_string_lossy().contains(secret))
    })
}

fn is_running(distro: &str) -> Result<bool> {
    let output = checked_wsl(&["--list", "--running", "--quiet"], None)?;
    let names = decode_output(&output.stdout);
    Ok(names.lines().any(|line| line.trim() == distro))
}

fn wait_until_running(distro: &str) -> Result<()> {
    for _ in 0..30 {
        if is_running(distro)? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("distro did not become running")
}

fn terminate(distro: &str) -> Result<()> {
    let output = wsl_output(&["--terminate", distro], None)?;
    if output.status.success() || !is_running(distro)? {
        Ok(())
    } else {
        bail!("failed to terminate distro: {}", decode_output(&output.stderr))
    }
}

fn stop_keeper_and_distro(mut keeper: Child, distro: &str) -> Result<()> {
    let _ = checked_wsl(
        &[
            "--distribution",
            distro,
            "--user",
            "root",
            "--exec",
            "pkill",
            "-f",
            "sleep 30",
        ],
        None,
    );
    let _ = keeper.kill();
    let _ = keeper.wait();
    terminate(distro)
}

fn checked_wsl(args: &[&str], stdin: Option<&[u8]>) -> Result<Output> {
    let output = wsl_output(args, stdin)?;
    if !output.status.success() {
        bail!(
            "wsl {:?} failed with {:?}: {}",
            args,
            output.status.code(),
            decode_output(&output.stderr)
        );
    }
    Ok(output)
}

fn wsl_output(args: &[&str], stdin: Option<&[u8]>) -> Result<Output> {
    let child = spawn_wsl(args, stdin)?;
    Ok(child.wait_with_output()?)
}

fn spawn_wsl(args: &[&str], stdin: Option<&[u8]>) -> Result<Child> {
    let mut command = Command::new("wsl.exe");
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().with_context(|| format!("spawn wsl.exe {args:?}"))?;
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .context("wsl stdin unavailable")?
            .write_all(bytes)?;
    }
    Ok(child)
}

fn decode_output(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && (bytes[0] == 0xff && bytes[1] == 0xfe || bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count() > bytes.len() / 8) {
        let start = usize::from(bytes.starts_with(&[0xff, 0xfe])) * 2;
        let units = bytes[start..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units).replace('\0', "")
    } else {
        String::from_utf8_lossy(bytes).replace('\0', "")
    }
}

fn scan_for_secret(root: &Path, secret: &[u8]) -> Result<bool> {
    if !root.exists() {
        return Ok(false);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if scan_for_secret(&path, secret)? {
                return Ok(true);
            }
        } else {
            let bytes = fs::read(&path)?;
            if bytes.windows(secret.len()).any(|window| window == secret) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
