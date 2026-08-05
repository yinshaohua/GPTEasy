use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use toml_edit::DocumentMut;

const START: &str = "# >>> GPTEasy managed provider >>>";
const END: &str = "# <<< GPTEasy managed provider <<<";
const BACKUP_LIMIT: usize = 5;

#[derive(Debug, Deserialize)]
struct WindowsEvidence {
    probe_ok: bool,
    distributions: Vec<String>,
    running_before: Vec<String>,
    running_after: Vec<String>,
    running_set_unchanged: bool,
    commands_that_entered_a_distribution: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Running,
    Stopped,
}

#[derive(Debug, Clone)]
struct Distribution {
    name: String,
    state: State,
    default_user: String,
    codex_running: bool,
    infrastructure: bool,
    root: PathBuf,
}

#[derive(Debug, Serialize)]
struct DetectedEnvironment {
    name: String,
    state: State,
    default_user: String,
}

#[derive(Debug, Serialize)]
struct SwitchResult {
    name: String,
    changed: bool,
    pending_restart: bool,
    restored_original_state: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Injection {
    None,
    FailBeforeReplace,
}

struct Harness {
    distributions: BTreeMap<String, Distribution>,
    actions: Vec<String>,
}

#[derive(Clone)]
struct Provider<'a> {
    model: &'a str,
    base_url: &'a str,
    bearer_token: &'a str,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => {
            let output = PathBuf::from(args.next().ok_or("run requires output directory")?);
            let evidence = PathBuf::from(args.next().ok_or("run requires evidence path")?);
            let summary = run_matrix(&output, &evidence)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            if summary["passed"] != summary["total"] {
                std::process::exit(1);
            }
        }
        _ => return Err("usage: wsl2-environment-lifecycle run OUTPUT_DIR EVIDENCE".into()),
    }
    Ok(())
}

fn run_matrix(output: &Path, evidence_path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    fs::create_dir_all(output)?;
    let evidence: WindowsEvidence = serde_json::from_slice(&fs::read(evidence_path)?)?;
    let session = output.join(format!(
        "session-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(&session)?;
    let provider = Provider {
        model: "provider-model",
        base_url: "https://provider.example/v1",
        bearer_token: "fake-secret-not-real",
    };
    let mut results = Vec::new();

    results.push(case(
        "real-detection-does-not-start-or-enter-distros",
        evidence.probe_ok
            && evidence.running_set_unchanged
            && evidence.running_before == evidence.running_after
            && evidence.commands_that_entered_a_distribution == 0,
        json!({
            "distributions": evidence.distributions,
            "running_before": evidence.running_before,
            "running_after": evidence.running_after
        }),
    ));

    let detection = create_harness(&session.join("detection"))?;
    let detected = detection.detect();
    let detection_ok = detected.len() == 3
        && detected.iter().all(|entry| entry.name != "docker-desktop")
        && detection.actions.is_empty()
        && detection.state("Ubuntu") == Some(State::Running)
        && detection.state("Debian") == Some(State::Stopped);
    results.push(case(
        "fixture-detection-is-side-effect-free",
        detection_ok,
        json!({"detected": detected, "actions": detection.actions}),
    ));

    let mut running = create_harness(&session.join("running-switch"))?;
    let ubuntu_other_before = running.read_user_config("Ubuntu", "other")?;
    let running_result = running.switch("Ubuntu", "apply", &provider, Injection::None)?;
    let running_ok = running_result.changed
        && !running_result.pending_restart
        && running_result.restored_original_state
        && running.state("Ubuntu") == Some(State::Running)
        && running.read_user_config("Ubuntu", "other")? == ubuntu_other_before
        && running
            .read_user_config("Ubuntu", "yin")?
            .contains("provider-model")
        && !running
            .actions
            .iter()
            .any(|action| action.contains("start:Ubuntu"))
        && !running
            .actions
            .iter()
            .any(|action| action.contains("terminate:Ubuntu"));
    results.push(case(
        "running-distro-switches-only-default-user",
        running_ok,
        json!({"result": running_result, "actions": running.actions}),
    ));

    let mut stopped = create_harness(&session.join("stopped-switch"))?;
    let stopped_result = stopped.switch("Debian", "apply", &provider, Injection::None)?;
    let stopped_ok = stopped_result.changed
        && stopped_result.restored_original_state
        && stopped.state("Debian") == Some(State::Stopped)
        && stopped.actions == vec!["start:Debian", "write:Debian:alice", "terminate:Debian"];
    results.push(case(
        "stopped-distro-starts-temporarily-and-restores",
        stopped_ok,
        json!({"result": stopped_result, "actions": stopped.actions}),
    ));

    let mut failed = create_harness(&session.join("stopped-failure"))?;
    let failed_original = failed.read_user_config("Debian", "alice")?;
    let failed_result = failed.switch("Debian", "apply", &provider, Injection::FailBeforeReplace);
    let failed_ok = failed_result.is_err()
        && failed.state("Debian") == Some(State::Stopped)
        && failed.read_user_config("Debian", "alice")? == failed_original
        && failed.actions == vec!["start:Debian", "terminate:Debian"];
    results.push(case(
        "failed-write-still-restores-stopped-state",
        failed_ok,
        json!({
            "error": failed_result.err().map(|error| error.to_string()),
            "actions": failed.actions
        }),
    ));

    let mut pending = create_harness(&session.join("pending-restart"))?;
    let pending_result = pending.switch("Fedora", "apply", &provider, Injection::None)?;
    let pending_ok = pending_result.pending_restart
        && pending.state("Fedora") == Some(State::Running)
        && !pending
            .actions
            .iter()
            .any(|action| action.starts_with("kill:"));
    results.push(case(
        "running-codex-becomes-pending-without-kill",
        pending_ok,
        json!({"result": pending_result, "actions": pending.actions}),
    ));

    let mut cancel = create_harness(&session.join("cancel"))?;
    let cancel_before = cancel.read_user_config("Debian", "alice")?;
    let cancel_result = cancel.switch("Debian", "cancel", &provider, Injection::None)?;
    let cancel_ok = !cancel_result.changed
        && cancel.actions.is_empty()
        && cancel.state("Debian") == Some(State::Stopped)
        && cancel.read_user_config("Debian", "alice")? == cancel_before;
    results.push(case(
        "cancel-does-not-start-or-write",
        cancel_ok,
        json!({"result": cancel_result, "actions": cancel.actions}),
    ));

    let mut batch = create_harness(&session.join("batch"))?;
    let batch_results =
        batch.batch_switch(&["Ubuntu", "Debian", "Fedora"], &provider, Injection::None);
    let batch_ok = batch_results.iter().all(Result::is_ok)
        && batch.state("Ubuntu") == Some(State::Running)
        && batch.state("Debian") == Some(State::Stopped)
        && batch.state("Fedora") == Some(State::Running)
        && batch
            .actions
            .iter()
            .filter(|action| *action == "start:Debian")
            .count()
            == 1
        && batch
            .actions
            .iter()
            .filter(|action| *action == "terminate:Debian")
            .count()
            == 1;
    results.push(case(
        "batch-switch-restores-each-original-state",
        batch_ok,
        json!({"actions": batch.actions}),
    ));

    let mut retention = create_harness(&session.join("retention"))?;
    for index in 0..7 {
        let model = format!("provider-model-{index}");
        let iteration = Provider {
            model: &model,
            ..provider.clone()
        };
        retention.switch("Ubuntu", "apply", &iteration, Injection::None)?;
    }
    let retention_path = retention.config_path("Ubuntu", "yin")?;
    let retention_ok = backup_files(&retention_path)?.len() == BACKUP_LIMIT;
    results.push(case(
        "wsl-environment-retains-latest-five-backups",
        retention_ok,
        json!({"backup_count": backup_files(&retention_path)?.len()}),
    ));

    let mut restore = create_harness(&session.join("restore"))?;
    let restore_path = restore.config_path("Ubuntu", "yin")?;
    let restore_original = fs::read(&restore_path)?;
    restore.switch("Ubuntu", "apply", &provider, Injection::None)?;
    restore_latest(&restore_path)?;
    let restore_ok = fs::read(&restore_path)? == restore_original;
    results.push(case(
        "wsl-environment-restores-latest-backup",
        restore_ok,
        json!({}),
    ));

    let mut damaged = create_harness(&session.join("damaged"))?;
    let damaged_path = damaged.config_path("Debian", "alice")?;
    fs::write(
        &damaged_path,
        format!("{START}\nmodel = \"broken\"\n").as_bytes(),
    )?;
    let damaged_result = damaged.switch("Debian", "apply", &provider, Injection::None);
    let damaged_ok = damaged_result.is_err()
        && damaged.state("Debian") == Some(State::Stopped)
        && damaged.actions == vec!["start:Debian", "terminate:Debian"];
    results.push(case(
        "damaged-block-stops-and-restores-state",
        damaged_ok,
        json!({
            "error": damaged_result.err().map(|error| error.to_string()),
            "actions": damaged.actions
        }),
    ));

    let passed = results
        .iter()
        .filter(|entry| entry["passed"] == true)
        .count();
    let summary = json!({"passed": passed, "total": results.len(), "results": results});
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    Ok(summary)
}

impl Harness {
    fn detect(&self) -> Vec<DetectedEnvironment> {
        self.distributions
            .values()
            .filter(|distribution| !distribution.infrastructure)
            .map(|distribution| DetectedEnvironment {
                name: distribution.name.clone(),
                state: distribution.state,
                default_user: distribution.default_user.clone(),
            })
            .collect()
    }

    fn state(&self, name: &str) -> Option<State> {
        self.distributions
            .get(name)
            .map(|distribution| distribution.state)
    }

    fn config_path(
        &self,
        distribution: &str,
        user: &str,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let distribution = self
            .distributions
            .get(distribution)
            .ok_or("distribution not found")?;
        Ok(distribution
            .root
            .join("home")
            .join(user)
            .join(".codex")
            .join("config.toml"))
    }

    fn read_user_config(
        &self,
        distribution: &str,
        user: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        Ok(fs::read_to_string(self.config_path(distribution, user)?)?)
    }

    fn switch(
        &mut self,
        name: &str,
        decision: &str,
        provider: &Provider<'_>,
        injection: Injection,
    ) -> Result<SwitchResult, Box<dyn std::error::Error>> {
        if decision == "cancel" {
            return Ok(SwitchResult {
                name: name.to_string(),
                changed: false,
                pending_restart: false,
                restored_original_state: true,
            });
        }
        if decision != "apply" {
            return Err(format!("unsupported decision `{decision}`").into());
        }
        let distribution = self
            .distributions
            .get(name)
            .ok_or("distribution not found")?
            .clone();
        if distribution.infrastructure {
            return Err("infrastructure distribution is not manageable".into());
        }
        let originally_stopped = distribution.state == State::Stopped;
        if originally_stopped {
            self.actions.push(format!("start:{name}"));
            self.distributions.get_mut(name).unwrap().state = State::Running;
        }

        let path = self.config_path(name, &distribution.default_user)?;
        let write_result = apply_provider(&path, provider, injection);
        if write_result.is_ok() {
            self.actions
                .push(format!("write:{name}:{}", distribution.default_user));
        }
        if originally_stopped {
            self.actions.push(format!("terminate:{name}"));
            self.distributions.get_mut(name).unwrap().state = State::Stopped;
        }
        write_result?;
        Ok(SwitchResult {
            name: name.to_string(),
            changed: true,
            pending_restart: distribution.codex_running,
            restored_original_state: self.state(name) == Some(distribution.state),
        })
    }

    fn batch_switch(
        &mut self,
        names: &[&str],
        provider: &Provider<'_>,
        injection: Injection,
    ) -> Vec<Result<SwitchResult, String>> {
        names
            .iter()
            .map(|name| {
                self.switch(name, "apply", provider, injection)
                    .map_err(|error| error.to_string())
            })
            .collect()
    }
}

fn create_harness(root: &Path) -> Result<Harness, Box<dyn std::error::Error>> {
    fs::create_dir_all(root)?;
    let definitions = [
        ("Ubuntu", State::Running, "yin", false, false),
        ("Debian", State::Stopped, "alice", false, false),
        ("Fedora", State::Running, "fedora", true, false),
        ("docker-desktop", State::Stopped, "root", false, true),
    ];
    let mut distributions = BTreeMap::new();
    for (name, state, default_user, codex_running, infrastructure) in definitions {
        let distribution_root = root.join(name);
        for user in [default_user, "other"] {
            let config = distribution_root
                .join("home")
                .join(user)
                .join(".codex")
                .join("config.toml");
            fs::create_dir_all(config.parent().unwrap())?;
            fs::write(
                &config,
                format!("custom_user = \"{user}\"\n\n[projects.demo]\ntrust_level = \"trusted\"\n"),
            )?;
        }
        distributions.insert(
            name.to_string(),
            Distribution {
                name: name.to_string(),
                state,
                default_user: default_user.to_string(),
                codex_running,
                infrastructure,
                root: distribution_root,
            },
        );
    }
    Ok(Harness {
        distributions,
        actions: Vec::new(),
    })
}

fn apply_provider(
    path: &Path,
    provider: &Provider<'_>,
    injection: Injection,
) -> Result<(), Box<dyn std::error::Error>> {
    let original = fs::read(path)?;
    let original_text = std::str::from_utf8(&original)?;
    let newline = if original_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let rendered = render_transaction(original_text, provider, newline)?;
    rendered.parse::<DocumentMut>()?;
    create_backup(path, &original)?;
    prune_backups(path, BACKUP_LIMIT)?;
    let temp = write_temp(path, rendered.as_bytes())?;
    if injection == Injection::FailBeforeReplace {
        let _ = fs::remove_file(&temp);
        return Err("injected failure before atomic replace".into());
    }
    if fs::read(path)? != original {
        let _ = fs::remove_file(&temp);
        return Err("configuration changed concurrently".into());
    }
    atomic_replace(path, &temp)?;
    Ok(())
}

fn render_transaction(
    original: &str,
    provider: &Provider<'_>,
    newline: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let (starts, ends) = marker_ranges(original);
    let block = render_block(provider, newline);
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => {
            let mut doc = original.parse::<DocumentMut>()?;
            doc.remove("model");
            doc.remove("model_provider");
            if let Some(providers) = doc
                .get_mut("model_providers")
                .and_then(|item| item.as_table_mut())
            {
                providers.remove("gpteasy");
                if providers.iter().any(|(_, item)| !item.is_table()) {
                    return Err("unsupported model_providers parent values".into());
                }
                providers.set_implicit(true);
            }
            Ok(format!(
                "{block}{}",
                normalize_newlines(&doc.to_string(), newline)
            ))
        }
        ([(start, _)], [(_, end)]) if start < end => Ok(format!(
            "{}{}{}",
            &original[..*start],
            block,
            &original[*end..]
        )),
        _ => Err("managed block markers are missing, duplicated, or reversed".into()),
    }
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
    (starts, ends)
}

fn render_block(provider: &Provider<'_>, newline: &str) -> String {
    let string = |value: &str| toml_edit::Value::from(value).to_string();
    [
        START.to_string(),
        format!("model = {}", string(provider.model)),
        "model_provider = \"gpteasy\"".to_string(),
        "model_providers.gpteasy.name = \"GPTEasy WSL Fixture\"".to_string(),
        format!(
            "model_providers.gpteasy.base_url = {}",
            string(provider.base_url)
        ),
        "model_providers.gpteasy.wire_api = \"responses\"".to_string(),
        format!(
            "model_providers.gpteasy.experimental_bearer_token = {}",
            string(provider.bearer_token)
        ),
        END.to_string(),
        String::new(),
    ]
    .join(newline)
}

fn create_backup(path: &Path, bytes: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = backup_dir(path)?;
    fs::create_dir_all(&dir)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let backup = dir.join(format!("config-{stamp}.toml"));
    write_synced(&backup, bytes)?;
    Ok(backup)
}

fn restore_latest(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let latest = backup_files(path)?
        .pop()
        .ok_or("no backup available for restore")?;
    let bytes = fs::read(latest)?;
    let temp = write_temp(path, &bytes)?;
    atomic_replace(path, &temp)?;
    Ok(())
}

fn prune_backups(path: &Path, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let backups = backup_files(path)?;
    let remove_count = backups.len().saturating_sub(limit);
    for old in backups.into_iter().take(remove_count) {
        fs::remove_file(old)?;
    }
    Ok(())
}

fn backup_files(path: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let dir = backup_dir(path)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| entry.is_file())
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn backup_dir(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(path
        .parent()
        .ok_or("config path has no parent")?
        .join(".gpteasy-backups"))
}

fn write_temp(path: &Path, bytes: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("config path has no parent")?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp = parent.join(format!(".config.toml.gpteasy-{stamp}.tmp"));
    write_synced(&temp, bytes)?;
    Ok(temp)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn atomic_replace(target: &Path, replacement: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{Foundation::GetLastError, Storage::FileSystem::ReplaceFileW};
    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement_wide = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            replacement_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        let code = unsafe { GetLastError() };
        let _ = fs::remove_file(replacement);
        return Err(format!("ReplaceFileW failed with Win32 error {code}").into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(target: &Path, replacement: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::rename(replacement, target)?;
    Ok(())
}

fn normalize_newlines(value: &str, newline: &str) -> String {
    let lf = value.replace("\r\n", "\n");
    if newline == "\r\n" {
        lf.replace('\n', "\r\n")
    } else {
        lf
    }
}

fn case(name: &str, passed: bool, details: Value) -> Value {
    json!({"name": name, "passed": passed, "details": details})
}
