use serde_json::{json, Value};
use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use toml_edit::DocumentMut;

const START: &str = "# >>> GPTEasy managed provider >>>";
const END: &str = "# <<< GPTEasy managed provider <<<";
const ID_PREFIX: &str = "# GPTEasy provider-id:";

#[derive(Clone)]
struct ProviderRecord {
    id: &'static str,
    model: &'static str,
    base_url: &'static str,
}

#[derive(Debug, Clone)]
struct ParsedUserConfig {
    provider_id: Option<String>,
    model: String,
    provider_key: String,
    base_url: String,
}

#[derive(Debug, Clone)]
struct EffectiveConfig {
    model: Option<String>,
    provider: Option<String>,
    model_origin: Option<String>,
    provider_origin: Option<String>,
    layers: Vec<String>,
}

#[derive(Debug)]
struct Reconciliation {
    state: &'static str,
    provider_id: Option<String>,
    reason: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => {
            let output = PathBuf::from(args.next().ok_or("run requires output directory")?);
            let summary = run_matrix(&output)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            if summary["passed"] != summary["total"] {
                std::process::exit(1);
            }
        }
        _ => return Err("usage: external-config-reconciliation run OUTPUT_DIR".into()),
    }
    Ok(())
}

fn run_matrix(output: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    fs::create_dir_all(output)?;
    let session = output.join(format!(
        "session-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(&session)?;
    let codex_exe = PathBuf::from(
        env::var("GPTEASY_CODEX_EXE")
            .map_err(|_| "GPTEASY_CODEX_EXE is required; run through run.ps1")?,
    );
    let codex_version = command_output(&codex_exe, &["--version"])?;
    let catalog = catalog();
    let mut results = Vec::new();

    let user_only_root = session.join("user-only");
    let user_only = prepare_app_server_scenario(&user_only_root, None)?;
    let user_only_effective = read_effective_config(&codex_exe, &user_only.0, &user_only.1, None)?;
    let user_only_text = fs::read_to_string(user_only.0.join("config.toml"))?;
    let user_only_state = reconcile(
        parse_user_config(&user_only_text),
        &user_only_effective,
        &catalog,
    );
    results.push(case(
        "managed-user-config-is-current",
        user_only_state.state == "managed_current"
            && user_only_effective.model.as_deref() == Some("user-model")
            && user_only_effective.provider.as_deref() == Some("gpteasy")
            && user_only_effective.model_origin.as_deref() == Some("user")
            && user_only_effective.provider_origin.as_deref() == Some("user"),
        reconciliation_json(&user_only_state, &user_only_effective),
    ));

    let project_root = session.join("project-override");
    let project = prepare_app_server_scenario(
        &project_root,
        Some(
            r#"model = "project-model"
model_provider = "project-provider"

[model_providers.project-provider]
name = "Project Provider"
base_url = "https://project.example/v1"
wire_api = "responses"
experimental_bearer_token = "fake-project-key"
"#,
        ),
    )?;
    let project_effective = read_effective_config(&codex_exe, &project.0, &project.1, None)?;
    let project_text = fs::read_to_string(project.0.join("config.toml"))?;
    let project_state = reconcile(
        parse_user_config(&project_text),
        &project_effective,
        &catalog,
    );
    results.push(case(
        "project-model-override-is-visible-with-origin",
        project_state.state == "managed_overridden"
            && project_effective.model.as_deref() == Some("project-model")
            && project_effective.model_origin.as_deref() == Some("project")
            && project_effective.provider.as_deref() == Some("gpteasy")
            && project_effective.provider_origin.as_deref() == Some("user"),
        reconciliation_json(&project_state, &project_effective),
    ));

    let session_root = session.join("session-override");
    let session_scenario = prepare_app_server_scenario(&session_root, None)?;
    let session_effective = read_effective_config(
        &codex_exe,
        &session_scenario.0,
        &session_scenario.1,
        Some("session-model"),
    )?;
    let session_text = fs::read_to_string(session_scenario.0.join("config.toml"))?;
    let session_state = reconcile(
        parse_user_config(&session_text),
        &session_effective,
        &catalog,
    );
    results.push(case(
        "session-flag-override-is-visible-with-origin",
        session_state.state == "managed_overridden"
            && session_effective.model.as_deref() == Some("session-model")
            && session_effective.model_origin.as_deref() == Some("sessionFlags"),
        reconciliation_json(&session_state, &session_effective),
    ));

    let drifted = managed_config("provider-user", "user-model", "https://drifted.example/v1");
    let drifted_state = reconcile(parse_user_config(&drifted), &user_only_effective, &catalog);
    results.push(case(
        "known-id-with-field-drift-requires-revalidation",
        drifted_state.state == "managed_drifted"
            && drifted_state.provider_id.as_deref() == Some("provider-user"),
        reconciliation_json(&drifted_state, &user_only_effective),
    ));

    let unknown_id = managed_config("provider-deleted", "user-model", "https://user.example/v1");
    let unknown_id_state = reconcile(
        parse_user_config(&unknown_id),
        &user_only_effective,
        &catalog,
    );
    results.push(case(
        "unknown-managed-id-is-not-name-matched",
        unknown_id_state.state == "external_unknown_id",
        reconciliation_json(&unknown_id_state, &user_only_effective),
    ));

    let legacy_unique = external_config("unique-model", "https://unique.example/v1");
    let legacy_unique_effective = EffectiveConfig {
        model: Some("unique-model".to_string()),
        provider: Some("external".to_string()),
        model_origin: Some("user".to_string()),
        provider_origin: Some("user".to_string()),
        layers: vec!["user".to_string()],
    };
    let legacy_unique_state = reconcile(
        parse_user_config(&legacy_unique),
        &legacy_unique_effective,
        &catalog,
    );
    results.push(case(
        "legacy-config-can-have-one-conservative-match",
        legacy_unique_state.state == "legacy_unique_match"
            && legacy_unique_state.provider_id.as_deref() == Some("provider-unique"),
        reconciliation_json(&legacy_unique_state, &legacy_unique_effective),
    ));

    let ambiguous = external_config("shared-model", "https://shared.example/v1");
    let ambiguous_effective = EffectiveConfig {
        model: Some("shared-model".to_string()),
        provider: Some("external".to_string()),
        model_origin: Some("user".to_string()),
        provider_origin: Some("user".to_string()),
        layers: vec!["user".to_string()],
    };
    let ambiguous_state = reconcile(
        parse_user_config(&ambiguous),
        &ambiguous_effective,
        &catalog,
    );
    results.push(case(
        "ambiguous-address-and-model-remain-external",
        ambiguous_state.state == "external_ambiguous",
        reconciliation_json(&ambiguous_state, &ambiguous_effective),
    ));

    let unmatched = external_config("unknown-model", "https://unknown.example/v1");
    let unmatched_state = reconcile(
        parse_user_config(&unmatched),
        &ambiguous_effective,
        &catalog,
    );
    results.push(case(
        "unmatched-config-remains-external",
        unmatched_state.state == "external_unmatched",
        reconciliation_json(&unmatched_state, &ambiguous_effective),
    ));

    let damaged = format!(
        "{START}\n{ID_PREFIX} provider-user\nmodel = \"user-model\"\nmodel_provider = \"gpteasy\"\n"
    );
    let damaged_state = reconcile(parse_user_config(&damaged), &user_only_effective, &catalog);
    results.push(case(
        "damaged-managed-markers-need-attention",
        damaged_state.state == "needs_attention",
        reconciliation_json(&damaged_state, &user_only_effective),
    ));

    let duplicate_id = managed_config_with_duplicate_id();
    let duplicate_id_state = reconcile(
        parse_user_config(&duplicate_id),
        &user_only_effective,
        &catalog,
    );
    results.push(case(
        "duplicate-provider-id-comments-need-attention",
        duplicate_id_state.state == "needs_attention",
        reconciliation_json(&duplicate_id_state, &user_only_effective),
    ));

    let passed = results
        .iter()
        .filter(|entry| entry["passed"] == true)
        .count();
    let summary = json!({
        "codex_version": codex_version.trim(),
        "passed": passed,
        "total": results.len(),
        "results": results
    });
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    Ok(summary)
}

fn prepare_app_server_scenario(
    root: &Path,
    project_config: Option<&str>,
) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let codex_home = root.join("codex-home");
    let workspace = root.join("workspace");
    fs::create_dir_all(&codex_home)?;
    fs::create_dir_all(&workspace)?;
    let trusted = workspace.to_string_lossy().replace('\'', "''");
    let config = format!(
        "{}\n[projects.'{}']\ntrust_level = \"trusted\"\n",
        managed_config("provider-user", "user-model", "https://user.example/v1"),
        trusted
    );
    fs::write(codex_home.join("config.toml"), config)?;
    if let Some(project_config) = project_config {
        let dot_codex = workspace.join(".codex");
        fs::create_dir_all(&dot_codex)?;
        fs::write(dot_codex.join("config.toml"), project_config)?;
    }
    Ok((codex_home, workspace))
}

fn read_effective_config(
    codex_exe: &Path,
    codex_home: &Path,
    cwd: &Path,
    session_model: Option<&str>,
) -> Result<EffectiveConfig, Box<dyn std::error::Error>> {
    let mut command = Command::new(codex_exe);
    if let Some(model) = session_model {
        command.arg("-c").arg(format!("model=\"{model}\""));
    }
    command
        .arg("app-server")
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().ok_or("app-server stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("app-server stdout unavailable")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    send(
        &mut stdin,
        json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {"name": "gpteasy-spike-008", "version": "0.1.0"},
                "capabilities": {"experimentalApi": true}
            }
        }),
    )?;
    wait_response(&rx, 1)?;
    send(&mut stdin, json!({"method": "initialized", "params": {}}))?;
    send(
        &mut stdin,
        json!({
            "id": 2,
            "method": "config/read",
            "params": {"cwd": cwd, "includeLayers": true}
        }),
    )?;
    let response = wait_response(&rx, 2)?;
    stop_child(&mut child);
    let result = response
        .get("result")
        .ok_or_else(|| format!("config/read failed: {response}"))?;
    let config = result.get("config").and_then(Value::as_object);
    let model = config
        .and_then(|value| value.get("model"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let provider = config
        .and_then(|value| value.get("model_provider"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let origins = result.get("origins").and_then(Value::as_object);
    let model_origin = origins
        .and_then(|value| value.get("model"))
        .and_then(origin_type);
    let provider_origin = origins
        .and_then(|value| value.get("model_provider"))
        .and_then(origin_type);
    let layers = result
        .get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|layer| {
            layer
                .get("name")
                .and_then(|name| name.get("type"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    Ok(EffectiveConfig {
        model,
        provider,
        model_origin,
        provider_origin,
        layers,
    })
}

fn send(stdin: &mut ChildStdin, value: Value) -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer(&mut *stdin, &value)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

fn wait_response(receiver: &Receiver<Value>, id: i64) -> Result<Value, Box<dyn std::error::Error>> {
    let deadline = Duration::from_secs(20);
    loop {
        let value = receiver.recv_timeout(deadline)?;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(value);
        }
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn origin_type(value: &Value) -> Option<String> {
    value
        .get("name")
        .and_then(|name| name.get("type"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn parse_user_config(value: &str) -> Result<ParsedUserConfig, String> {
    let (starts, ends) = marker_ranges(value);
    let provider_id = match (starts.as_slice(), ends.as_slice()) {
        ([], []) => None,
        ([(start, _)], [(_, end)]) if start < end => {
            let ids = value[*start..*end]
                .lines()
                .filter_map(|line| line.trim().strip_prefix(ID_PREFIX))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>();
            match ids.as_slice() {
                [id] => Some((*id).to_string()),
                [] => None,
                _ => return Err("managed block has duplicate provider-id comments".to_string()),
            }
        }
        _ => return Err("managed block markers are missing, duplicated, or reversed".to_string()),
    };
    let doc = value
        .parse::<DocumentMut>()
        .map_err(|error| format!("invalid TOML: {error}"))?;
    let model = doc
        .get("model")
        .and_then(|item| item.as_str())
        .ok_or("model is missing")?
        .to_string();
    let provider_key = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .ok_or("model_provider is missing")?
        .to_string();
    let base_url = doc
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|table| table.get(&provider_key))
        .and_then(|item| item.as_table())
        .and_then(|table| table.get("base_url"))
        .and_then(|item| item.as_str())
        .ok_or("active provider base_url is missing")?
        .to_string();
    Ok(ParsedUserConfig {
        provider_id,
        model,
        provider_key,
        base_url,
    })
}

fn reconcile(
    parsed: Result<ParsedUserConfig, String>,
    effective: &EffectiveConfig,
    catalog: &[ProviderRecord],
) -> Reconciliation {
    let parsed = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            return Reconciliation {
                state: "needs_attention",
                provider_id: None,
                reason: error,
            }
        }
    };
    if let Some(provider_id) = parsed.provider_id.as_deref() {
        let Some(record) = catalog.iter().find(|record| record.id == provider_id) else {
            return Reconciliation {
                state: "external_unknown_id",
                provider_id: Some(provider_id.to_string()),
                reason: "managed provider-id is not present in the provider catalog".to_string(),
            };
        };
        if parsed.model != record.model || parsed.base_url != record.base_url {
            return Reconciliation {
                state: "managed_drifted",
                provider_id: Some(record.id.to_string()),
                reason: "managed provider fields differ from the last validated catalog record"
                    .to_string(),
            };
        }
        let effective_differs = effective.model.as_deref() != Some(parsed.model.as_str())
            || effective.provider.as_deref() != Some(parsed.provider_key.as_str())
            || effective.model_origin.as_deref() != Some("user")
            || effective.provider_origin.as_deref() != Some("user");
        if effective_differs {
            return Reconciliation {
                state: "managed_overridden",
                provider_id: Some(record.id.to_string()),
                reason: "effective Codex config is overridden by a non-user layer".to_string(),
            };
        }
        return Reconciliation {
            state: "managed_current",
            provider_id: Some(record.id.to_string()),
            reason: "user config and effective config match the validated provider".to_string(),
        };
    }

    let matches = catalog
        .iter()
        .filter(|record| record.model == parsed.model && record.base_url == parsed.base_url)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Reconciliation {
            state: "legacy_unique_match",
            provider_id: Some(record.id.to_string()),
            reason: "legacy config has one exact address and model match".to_string(),
        },
        [] => Reconciliation {
            state: "external_unmatched",
            provider_id: None,
            reason: "config has no immutable id and no exact catalog match".to_string(),
        },
        _ => Reconciliation {
            state: "external_ambiguous",
            provider_id: None,
            reason: "config has no immutable id and multiple catalog matches".to_string(),
        },
    }
}

fn managed_config(provider_id: &str, model: &str, base_url: &str) -> String {
    format!(
        "{START}\n\
         {ID_PREFIX} {provider_id}\n\
         model = \"{model}\"\n\
         model_provider = \"gpteasy\"\n\
         model_providers.gpteasy.name = \"User Provider\"\n\
         model_providers.gpteasy.base_url = \"{base_url}\"\n\
         model_providers.gpteasy.wire_api = \"responses\"\n\
         model_providers.gpteasy.experimental_bearer_token = \"fake-user-key\"\n\
         {END}\n"
    )
}

fn managed_config_with_duplicate_id() -> String {
    format!(
        "{START}\n\
         {ID_PREFIX} provider-user\n\
         {ID_PREFIX} provider-other\n\
         model = \"user-model\"\n\
         model_provider = \"gpteasy\"\n\
         model_providers.gpteasy.name = \"User Provider\"\n\
         model_providers.gpteasy.base_url = \"https://user.example/v1\"\n\
         model_providers.gpteasy.wire_api = \"responses\"\n\
         {END}\n"
    )
}

fn external_config(model: &str, base_url: &str) -> String {
    format!(
        "model = \"{model}\"\n\
         model_provider = \"external\"\n\
         \n\
         [model_providers.external]\n\
         name = \"External\"\n\
         base_url = \"{base_url}\"\n\
         wire_api = \"responses\"\n"
    )
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

fn catalog() -> Vec<ProviderRecord> {
    vec![
        ProviderRecord {
            id: "provider-user",
            model: "user-model",
            base_url: "https://user.example/v1",
        },
        ProviderRecord {
            id: "provider-unique",
            model: "unique-model",
            base_url: "https://unique.example/v1",
        },
        ProviderRecord {
            id: "provider-shared-a",
            model: "shared-model",
            base_url: "https://shared.example/v1",
        },
        ProviderRecord {
            id: "provider-shared-b",
            model: "shared-model",
            base_url: "https://shared.example/v1",
        },
    ]
}

fn command_output(path: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new(path).args(args).output()?;
    if !output.status.success() {
        return Err(format!("command failed with {}", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn reconciliation_json(reconciliation: &Reconciliation, effective: &EffectiveConfig) -> Value {
    json!({
        "state": reconciliation.state,
        "provider_id": reconciliation.provider_id,
        "reason": reconciliation.reason,
        "effective_model": effective.model,
        "effective_provider": effective.provider,
        "model_origin": effective.model_origin,
        "provider_origin": effective.provider_origin,
        "layers": effective.layers
    })
}

fn case(name: &str, passed: bool, details: Value) -> Value {
    json!({"name": name, "passed": passed, "details": details})
}
