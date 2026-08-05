use crate::{
    appserver::{read_effective_config, EffectiveConfig},
    validation::{
        combination_fingerprint, mock_verified_provider, validate_live, ProviderInput,
        ValidationEvidence, VerifiedProvider,
    },
};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use sysinfo::{ProcessesToUpdate, System};
use toml_edit::DocumentMut;

const START: &str = "# >>> GPTEasy managed provider >>>";
const END: &str = "# <<< GPTEasy managed provider <<<";
const ID_PREFIX: &str = "# GPTEasy provider-id:";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ProcessState {
    pub desktop: bool,
    pub cli: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Injection {
    None,
    CrashAfterPrepared,
    ConfigFailBeforeReplace,
    CrashAfterConfigReplace,
    CrashAfterStateCommit,
    ExternalEditAfterPrepared,
    RestartFailure,
}

impl Injection {
    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "none" => Self::None,
            "crash_after_prepared" => Self::CrashAfterPrepared,
            "config_fail_before_replace" => Self::ConfigFailBeforeReplace,
            "crash_after_config_replace" => Self::CrashAfterConfigReplace,
            "crash_after_state_commit" => Self::CrashAfterStateCommit,
            "external_edit_after_prepared" => Self::ExternalEditAfterPrepared,
            "restart_failure" => Self::RestartFailure,
            _ => bail!("unsupported injection {value}"),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ScenarioPaths {
    pub root: PathBuf,
    pub db: PathBuf,
    pub codex_home: PathBuf,
    pub config: PathBuf,
    pub workspace: PathBuf,
    pub log: PathBuf,
}

#[derive(Debug)]
struct PreparedConfig {
    original: Vec<u8>,
    rendered: Vec<u8>,
    old_hash: String,
    new_hash: String,
    backup: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub current_provider: Option<String>,
    pub config_hash: String,
    pub phase: Option<String>,
    pub operation_count: i64,
    pub restart_attempts: i64,
    pub last_error: Option<String>,
    pub provider_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reconciliation {
    pub state: String,
    pub provider_id: Option<String>,
    pub user_model: Option<String>,
    pub effective: EffectiveConfig,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSummary {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub role: String,
    pub relaunch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessScan {
    pub counts: BTreeMap<String, usize>,
    pub processes: Vec<ProcessSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineReport {
    pub decision: String,
    pub injection: Injection,
    pub validation: ValidationEvidence,
    pub snapshot: Snapshot,
    pub effective: Option<EffectiveConfig>,
    pub reconciliation: Option<Reconciliation>,
    pub events: Vec<Value>,
    pub process_scan: Option<ProcessScan>,
    pub workspace: String,
}

#[derive(Debug, Clone, Serialize)]
struct CaseResult {
    name: String,
    passed: bool,
    evidence: Value,
}

pub fn create_scenario(session: &Path, name: &str) -> Result<ScenarioPaths> {
    let requested_root = session.join(name);
    fs::create_dir_all(&requested_root)?;
    let root = dunce::canonicalize(&requested_root)?;
    let codex_home = root.join("codex-home");
    let workspace = root.join("workspace");
    fs::create_dir_all(&codex_home)?;
    fs::create_dir_all(&workspace)?;
    let paths = ScenarioPaths {
        db: root.join("state.db"),
        config: codex_home.join("config.toml"),
        log: root.join("events.jsonl"),
        root,
        codex_home,
        workspace,
    };
    let trusted = paths.workspace.to_string_lossy().replace('\'', "''");
    let initial = format!(
        "model = \"old-model\"\n\
         model_provider = \"legacy\"\n\
         custom_flag = true\n\n\
         [model_providers.legacy]\n\
         name = \"Legacy\"\n\
         base_url = \"https://legacy.example/v1\"\n\
         wire_api = \"responses\"\n\
         experimental_bearer_token = \"old-fake-secret\"\n\n\
         [projects.'{trusted}']\n\
         trust_level = \"trusted\"\n"
    );
    fs::write(&paths.config, initial)?;
    let conn = open_db(&paths.db)?;
    initialize_schema(&conn)?;
    conn.execute(
        "INSERT INTO providers(id, name, base_url, model, combination_fingerprint, validation_state)
         VALUES ('provider-old', 'Old Provider', 'https://legacy.example/v1', 'old-model',
                 'old-fingerprint', 'validated')",
        [],
    )?;
    conn.execute(
        "INSERT INTO environments(id, current_provider_id, status)
         VALUES ('native', 'provider-old', 'managed_current')",
        [],
    )?;
    Ok(paths)
}

pub fn run_pipeline(
    paths: &ScenarioPaths,
    decision: &str,
    verified: Option<&VerifiedProvider>,
    processes: ProcessState,
    injection: Injection,
) -> Result<PipelineReport> {
    if !matches!(decision, "immediate" | "later" | "cancel") {
        bail!("unsupported decision {decision}");
    }
    if decision == "cancel" {
        append_event(&paths.log, "cancelled", json!({}))?;
        let fallback = verified.cloned().unwrap_or_else(mock_verified_provider);
        return Ok(PipelineReport {
            decision: decision.to_string(),
            injection,
            validation: fallback.evidence,
            snapshot: snapshot(paths)?,
            effective: None,
            reconciliation: None,
            events: read_events(&paths.log)?,
            process_scan: None,
            workspace: paths.root.to_string_lossy().to_string(),
        });
    }
    let verified = verified.context("provider validation failed before switch preparation")?;
    ensure_validation_binding(verified)?;
    let execution = execute_switch(paths, decision, verified, processes, injection);
    let recoverable = matches!(
        injection,
        Injection::CrashAfterPrepared
            | Injection::CrashAfterConfigReplace
            | Injection::CrashAfterStateCommit
            | Injection::ExternalEditAfterPrepared
            | Injection::ConfigFailBeforeReplace
    );
    if let Err(error) = execution {
        if !recoverable {
            return Err(error);
        }
        append_event(
            &paths.log,
            "simulated_interruption",
            json!({"injection": injection, "error": error.to_string()}),
        )?;
        recover(paths, true)?;
    }
    let snap = snapshot(paths)?;
    let effective = if snap.current_provider.is_some() && snap.phase.as_deref() != Some("needs_attention")
    {
        read_effective_config(&paths.codex_home, &paths.workspace).ok()
    } else {
        None
    };
    let reconciliation = effective
        .as_ref()
        .and_then(|value| reconcile(paths, value.clone()).ok());
    Ok(PipelineReport {
        decision: decision.to_string(),
        injection,
        validation: verified.evidence.clone(),
        snapshot: snap,
        effective,
        reconciliation,
        events: read_events(&paths.log)?,
        process_scan: None,
        workspace: paths.root.to_string_lossy().to_string(),
    })
}

pub fn run_live_pipeline(
    session: &Path,
    secret: ProviderInput,
    decision: &str,
) -> Result<PipelineReport> {
    let verified = validate_live(secret)?;
    let paths = create_scenario(session, "live-provider")?;
    let scan = scan_codex_processes();
    let processes = ProcessState {
        desktop: scan.counts.get("desktop_root").copied().unwrap_or(0) > 0,
        cli: scan.counts.get("cli").copied().unwrap_or(0) > 0,
    };
    let mut report = run_pipeline(
        &paths,
        decision,
        Some(&verified),
        processes,
        Injection::None,
    )?;
    report.process_scan = Some(scan);
    Ok(report)
}

fn ensure_validation_binding(verified: &VerifiedProvider) -> Result<()> {
    if !verified.evidence.ok || verified.evidence.category != "validated" {
        bail!("provider is not validated");
    }
    let actual = combination_fingerprint(&verified.input);
    if actual != verified.evidence.combination_fingerprint {
        bail!("validated provider combination changed after validation");
    }
    Ok(())
}

fn execute_switch(
    paths: &ScenarioPaths,
    decision: &str,
    verified: &VerifiedProvider,
    processes: ProcessState,
    injection: Injection,
) -> Result<()> {
    let prepared = prepare_config(&paths.config, &verified.input)?;
    let old_provider = current_provider(paths)?
        .context("environment has no current provider before switch")?;
    let operation_id = format!(
        "switch-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let mut conn = open_db(&paths.db)?;
    {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO providers(id, name, base_url, model, combination_fingerprint, validation_state)
             VALUES (?1, ?2, ?3, ?4, ?5, 'validated')
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, base_url=excluded.base_url, model=excluded.model,
               combination_fingerprint=excluded.combination_fingerprint,
               validation_state='validated'",
            params![
                verified.input.id,
                verified.input.name,
                verified.input.base_url,
                verified.input.model,
                verified.evidence.combination_fingerprint
            ],
        )?;
        tx.execute(
            "INSERT INTO switch_operations(
                id, environment_id, old_provider_id, new_provider_id,
                old_hash, new_hash, backup_path, decision,
                desktop_present, cli_present, phase, restart_attempts
             ) VALUES (?1, 'native', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'prepared', 0)",
            params![
                operation_id,
                old_provider,
                verified.input.id,
                prepared.old_hash,
                prepared.new_hash,
                prepared.backup.to_string_lossy(),
                decision,
                processes.desktop,
                processes.cli
            ],
        )?;
        tx.commit()?;
    }
    append_event(
        &paths.log,
        "prepared",
        json!({
            "operation_id": operation_id,
            "decision": decision,
            "provider_id": verified.input.id,
            "combination_fingerprint": verified.evidence.combination_fingerprint
        }),
    )?;

    if injection == Injection::CrashAfterPrepared {
        bail!("simulated crash after prepared");
    }
    if injection == Injection::ExternalEditAfterPrepared {
        fs::write(
            &paths.config,
            b"# external editor\nmodel = \"external-model\"\nmodel_provider = \"external\"\n",
        )?;
        bail!("simulated external edit after prepared");
    }
    apply_prepared(
        &paths.config,
        &prepared,
        injection == Injection::ConfigFailBeforeReplace,
    )?;
    append_event(
        &paths.log,
        "config_replaced",
        json!({"operation_id": operation_id, "new_hash": prepared.new_hash}),
    )?;
    if injection == Injection::CrashAfterConfigReplace {
        bail!("simulated crash after config replace");
    }

    commit_new_state(&mut conn, &operation_id, &verified.input.id)?;
    append_event(
        &paths.log,
        "state_committed",
        json!({"operation_id": operation_id, "provider_id": verified.input.id}),
    )?;
    if injection == Injection::CrashAfterStateCommit {
        bail!("simulated crash after state commit");
    }
    finalize_restart(
        &mut conn,
        &operation_id,
        decision,
        processes,
        injection != Injection::RestartFailure,
        &paths.log,
    )?;
    Ok(())
}

pub fn recover(paths: &ScenarioPaths, desktop_restart_succeeds: bool) -> Result<()> {
    let mut conn = open_db(&paths.db)?;
    let pending = {
        let mut statement = conn.prepare(
            "SELECT id, old_provider_id, new_provider_id, old_hash, new_hash,
                    decision, desktop_present, cli_present, phase
             FROM switch_operations
             WHERE phase NOT IN ('completed', 'rolled_back', 'needs_attention', 'pending_restart')
             ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    ProcessState {
                        desktop: row.get::<_, bool>(6)?,
                        cli: row.get::<_, bool>(7)?,
                    },
                    row.get::<_, String>(8)?,
                ))
            })?;
        let collected = rows.collect::<Result<Vec<_>, _>>()?;
        collected
    };

    for (id, old_provider, new_provider, old_hash, new_hash, decision, processes, phase) in pending
    {
        let current_hash = hash_bytes(&fs::read(&paths.config)?);
        if current_hash == old_hash {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE environments SET current_provider_id = ?1, status='managed_current'
                 WHERE id='native'",
                params![old_provider],
            )?;
            tx.execute(
                "UPDATE switch_operations SET phase='rolled_back',
                 last_error='recovery observed old config hash' WHERE id=?1",
                params![id],
            )?;
            tx.commit()?;
            append_event(
                &paths.log,
                "recovery_rolled_back",
                json!({"operation_id": id, "previous_phase": phase}),
            )?;
            continue;
        }
        if current_hash == new_hash {
            commit_new_state(&mut conn, &id, &new_provider)?;
            finalize_restart(
                &mut conn,
                &id,
                &decision,
                processes,
                desktop_restart_succeeds,
                &paths.log,
            )?;
            append_event(
                &paths.log,
                "recovery_completed_new_state",
                json!({"operation_id": id, "previous_phase": phase}),
            )?;
            continue;
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE environments SET current_provider_id=NULL, status='needs_attention'
             WHERE id='native'",
            [],
        )?;
        tx.execute(
            "UPDATE switch_operations SET phase='needs_attention',
             last_error='config hash matches neither old nor new' WHERE id=?1",
            params![id],
        )?;
        tx.commit()?;
        append_event(
            &paths.log,
            "recovery_external_config",
            json!({"operation_id": id, "previous_phase": phase}),
        )?;
    }
    Ok(())
}

fn commit_new_state(conn: &mut Connection, operation_id: &str, provider: &str) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE environments SET current_provider_id=?1, status='managed_current' WHERE id='native'",
        params![provider],
    )?;
    tx.execute(
        "UPDATE switch_operations SET phase='state_committed' WHERE id=?1",
        params![operation_id],
    )?;
    tx.commit()?;
    Ok(())
}

fn finalize_restart(
    conn: &mut Connection,
    operation_id: &str,
    decision: &str,
    processes: ProcessState,
    desktop_restart_succeeds: bool,
    log: &Path,
) -> Result<()> {
    let (phase, last_error, attempts) = if decision == "later" {
        if processes.desktop || processes.cli {
            ("pending_restart", None, 0)
        } else {
            ("completed", None, 0)
        }
    } else {
        let attempts = i64::from(processes.desktop);
        if processes.desktop && !desktop_restart_succeeds {
            (
                "pending_restart",
                Some("desktop restart failed; new configuration remains selected"),
                attempts,
            )
        } else if processes.cli {
            (
                "pending_restart",
                Some("Codex CLI requires manual restart in the original terminal"),
                attempts,
            )
        } else {
            ("completed", None, attempts)
        }
    };
    conn.execute(
        "UPDATE switch_operations SET phase=?1, last_error=?2,
         restart_attempts=restart_attempts+?3 WHERE id=?4",
        params![phase, last_error, attempts, operation_id],
    )?;
    append_event(
        log,
        "restart_finalized",
        json!({
            "operation_id": operation_id,
            "phase": phase,
            "desktop_present": processes.desktop,
            "cli_present": processes.cli,
            "desktop_restart_succeeds": desktop_restart_succeeds,
            "real_processes_terminated": false
        }),
    )?;
    Ok(())
}

fn prepare_config(path: &Path, provider: &ProviderInput) -> Result<PreparedConfig> {
    let original = fs::read(path)?;
    let original_text = std::str::from_utf8(&original)?;
    let rendered = render_transaction(original_text, provider)?;
    rendered.parse::<DocumentMut>()?;
    let backup = create_backup(path, &original)?;
    prune_backups(path, 5)?;
    Ok(PreparedConfig {
        old_hash: hash_bytes(&original),
        new_hash: hash_bytes(rendered.as_bytes()),
        original,
        rendered: rendered.into_bytes(),
        backup,
    })
}

fn apply_prepared(path: &Path, prepared: &PreparedConfig, fail_before_replace: bool) -> Result<()> {
    let temp = write_temp(path, &prepared.rendered)?;
    if fail_before_replace {
        let _ = fs::remove_file(&temp);
        bail!("injected failure before atomic replace");
    }
    if fs::read(path)? != prepared.original {
        let _ = fs::remove_file(&temp);
        bail!("configuration changed concurrently");
    }
    atomic_replace(path, &temp)?;
    Ok(())
}

fn render_transaction(original: &str, provider: &ProviderInput) -> Result<String> {
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

fn render_block(provider: &ProviderInput, newline: &str) -> String {
    let string = |value: &str| toml_edit::Value::from(value).to_string();
    [
        START.to_string(),
        format!("{ID_PREFIX} {}", provider.id),
        format!("model = {}", string(&provider.model)),
        "model_provider = \"gpteasy\"".to_string(),
        format!("model_providers.gpteasy.name = {}", string(&provider.name)),
        format!(
            "model_providers.gpteasy.base_url = {}",
            string(&provider.base_url)
        ),
        "model_providers.gpteasy.wire_api = \"responses\"".to_string(),
        "model_providers.gpteasy.supports_websockets = false".to_string(),
        format!(
            "model_providers.gpteasy.experimental_bearer_token = {}",
            string(&provider.api_key)
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

fn normalize_newlines(value: &str, newline: &str) -> String {
    if newline == "\r\n" {
        value.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        value.replace("\r\n", "\n")
    }
}

pub fn snapshot(paths: &ScenarioPaths) -> Result<Snapshot> {
    let conn = open_db(&paths.db)?;
    let current_provider = conn.query_row(
        "SELECT current_provider_id FROM environments WHERE id='native'",
        [],
        |row| row.get::<_, Option<String>>(0),
    )?;
    let operation_count = conn.query_row("SELECT COUNT(*) FROM switch_operations", [], |row| {
        row.get(0)
    })?;
    let provider_count = conn.query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))?;
    let latest = conn
        .query_row(
            "SELECT phase, restart_attempts, last_error
             FROM switch_operations ORDER BY rowid DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(Snapshot {
        current_provider,
        config_hash: hash_bytes(&fs::read(&paths.config)?),
        phase: latest.as_ref().map(|value| value.0.clone()),
        operation_count,
        restart_attempts: latest.as_ref().map_or(0, |value| value.1),
        last_error: latest.and_then(|value| value.2),
        provider_count,
    })
}

fn current_provider(paths: &ScenarioPaths) -> Result<Option<String>> {
    let conn = open_db(&paths.db)?;
    Ok(conn.query_row(
        "SELECT current_provider_id FROM environments WHERE id='native'",
        [],
        |row| row.get(0),
    )?)
}

pub fn reconcile(paths: &ScenarioPaths, effective: EffectiveConfig) -> Result<Reconciliation> {
    let user = parse_user_config(&fs::read_to_string(&paths.config)?)?;
    let conn = open_db(&paths.db)?;
    let current: Option<(String, String)> = conn
        .query_row(
            "SELECT p.id, p.model
             FROM environments e LEFT JOIN providers p ON p.id=e.current_provider_id
             WHERE e.id='native'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (state, explanation) = match current {
        None => (
            "external_unmatched",
            "database has no current managed provider",
        ),
        Some((id, _model)) if user.provider_id.as_deref() != Some(id.as_str()) => (
            "external_unknown_id",
            "managed block provider-id differs from database",
        ),
        Some((_id, model)) if user.model != model => (
            "managed_drifted",
            "managed block model differs from validated provider record",
        ),
        Some((_id, _model))
            if effective.model.as_deref() != Some(user.model.as_str())
                || effective.provider.as_deref() != Some("gpteasy") =>
        {
            (
                "managed_overridden",
                "user managed block is correct but a higher-priority layer overrides it",
            )
        }
        Some(_) => (
            "managed_current",
            "database, user managed block, and effective config agree",
        ),
    };
    Ok(Reconciliation {
        state: state.to_string(),
        provider_id: user.provider_id,
        user_model: Some(user.model),
        effective,
        explanation: explanation.to_string(),
    })
}

struct ParsedUserConfig {
    provider_id: Option<String>,
    model: String,
}

fn parse_user_config(value: &str) -> Result<ParsedUserConfig> {
    let (starts, ends) = marker_ranges(value);
    let provider_id = match (starts.as_slice(), ends.as_slice()) {
        ([(start, _)], [(_, end)]) if start < end => {
            let ids = value[*start..*end]
                .lines()
                .filter_map(|line| line.trim().strip_prefix(ID_PREFIX))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>();
            match ids.as_slice() {
                [id] => Some((*id).to_string()),
                _ => bail!("managed block provider-id is missing or duplicated"),
            }
        }
        _ => bail!("managed block markers are missing, duplicated, or reversed"),
    };
    let doc = value.parse::<DocumentMut>()?;
    let model = doc
        .get("model")
        .and_then(|item| item.as_str())
        .context("model is missing")?
        .to_string();
    Ok(ParsedUserConfig { provider_id, model })
}

pub fn scan_codex_processes() -> ProcessScan {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let desktop_roots = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let executable = process
                .exe()
                .map(|path| path.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let command = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy().to_string())
                .collect::<Vec<_>>();
            let packaged_windows =
                executable.contains("windowsapps") && executable.contains("openai.codex_");
            let mac_bundle = executable.contains(".app/contents/macos/")
                && (executable.contains("/codex.app/") || executable.contains("/chatgpt.app/"));
            let bundled = executable.contains("\\resources\\codex")
                || executable.contains("/contents/resources/codex");
            let helper = command
                .iter()
                .skip(1)
                .any(|argument| argument.starts_with("--type="));
            ((packaged_windows || mac_bundle) && !bundled && !helper).then_some(pid.as_u32())
        })
        .collect::<HashSet<_>>();
    let mut processes = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let name = process.name().to_string_lossy().to_string();
            let lower_name = name.to_ascii_lowercase();
            let executable = process
                .exe()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default();
            let lower_executable = executable.to_ascii_lowercase();
            let parent_pid = process.parent().map(|value| value.as_u32());
            let (role, relaunch) = if desktop_roots.contains(&pid.as_u32()) {
                let relaunch = if lower_executable.contains("windowsapps") {
                    Some(
                        "explorer.exe shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App"
                            .to_string(),
                    )
                } else if lower_executable.contains("/codex.app/") {
                    Some("open -a Codex".to_string())
                } else if lower_executable.contains("/chatgpt.app/") {
                    Some("open -a ChatGPT".to_string())
                } else {
                    None
                };
                ("desktop_root", relaunch)
            } else if (lower_name == "codex" || lower_name == "codex.exe")
                && (parent_pid.is_some_and(|parent| desktop_roots.contains(&parent))
                    || lower_executable.contains("\\resources\\codex")
                    || lower_executable.contains("/contents/resources/codex"))
            {
                ("desktop_codex_child", None)
            } else if lower_name == "codex" || lower_name == "codex.exe" {
                ("cli", None)
            } else {
                return None;
            };
            Some(ProcessSummary {
                pid: pid.as_u32(),
                parent_pid,
                name,
                role: role.to_string(),
                relaunch,
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by_key(|value| (value.role.clone(), value.pid));
    let mut counts = BTreeMap::new();
    for process in &processes {
        *counts.entry(process.role.clone()).or_insert(0) += 1;
    }
    ProcessScan { counts, processes }
}

pub fn run_matrix(work_root: &Path, evidence_root: &Path) -> Result<Value> {
    fs::create_dir_all(work_root)?;
    fs::create_dir_all(evidence_root)?;
    let session = work_root.join(format!(
        "session-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(&session)?;
    let verified = mock_verified_provider();
    let mut results = Vec::new();

    let cancel = create_scenario(&session, "cancel")?;
    let before_cancel = snapshot(&cancel)?;
    let cancel_report = run_pipeline(
        &cancel,
        "cancel",
        Some(&verified),
        ProcessState {
            desktop: true,
            cli: true,
        },
        Injection::None,
    )?;
    results.push(case(
        "cancel-before-write",
        cancel_report.snapshot.operation_count == 0
            && cancel_report.snapshot.config_hash == before_cancel.config_hash,
        json!(cancel_report.snapshot),
    ));

    let unvalidated = create_scenario(&session, "unvalidated")?;
    let unvalidated_result = run_pipeline(
        &unvalidated,
        "immediate",
        None,
        ProcessState {
            desktop: true,
            cli: false,
        },
        Injection::None,
    );
    results.push(case(
        "unvalidated-provider-cannot-enter-saga",
        unvalidated_result.is_err() && snapshot(&unvalidated)?.operation_count == 0,
        json!({"error": unvalidated_result.err().map(|error| error.to_string())}),
    ));

    let takeover = create_scenario(&session, "first-takeover")?;
    let takeover_report = run_pipeline(
        &takeover,
        "later",
        Some(&verified),
        ProcessState {
            desktop: true,
            cli: true,
        },
        Injection::None,
    )?;
    let takeover_config = fs::read_to_string(&takeover.config)?;
    let takeover_doc = takeover_config.parse::<DocumentMut>()?;
    results.push(case(
        "validation-to-first-takeover-to-pending-restart",
        takeover_report.snapshot.current_provider.as_deref() == Some("provider-new")
            && takeover_report.snapshot.phase.as_deref() == Some("pending_restart")
            && takeover_doc["custom_flag"].as_bool() == Some(true)
            && takeover_doc["model_providers"]["legacy"]["name"].as_str() == Some("Legacy")
            && takeover_report.reconciliation.as_ref().map(|value| value.state.as_str())
                == Some("managed_current"),
        json!({
            "snapshot": takeover_report.snapshot,
            "reconciliation": takeover_report.reconciliation
        }),
    ));

    let immediate = create_scenario(&session, "immediate-desktop")?;
    let immediate_report = run_pipeline(
        &immediate,
        "immediate",
        Some(&verified),
        ProcessState {
            desktop: true,
            cli: false,
        },
        Injection::None,
    )?;
    results.push(case(
        "immediate-desktop-completes-with-dry-run-restart-plan",
        immediate_report.snapshot.phase.as_deref() == Some("completed")
            && immediate_report.snapshot.restart_attempts == 1
            && immediate_report
                .events
                .iter()
                .any(|event| event["details"]["real_processes_terminated"] == false),
        json!(immediate_report.snapshot),
    ));

    let cli = create_scenario(&session, "immediate-cli")?;
    let cli_report = run_pipeline(
        &cli,
        "immediate",
        Some(&verified),
        ProcessState {
            desktop: true,
            cli: true,
        },
        Injection::None,
    )?;
    results.push(case(
        "cli-remains-manual-pending-restart",
        cli_report.snapshot.phase.as_deref() == Some("pending_restart")
            && cli_report
                .snapshot
                .last_error
                .as_deref()
                .is_some_and(|value| value.contains("manual restart")),
        json!(cli_report.snapshot),
    ));

    let restart_failure = create_scenario(&session, "restart-failure")?;
    let restart_failure_report = run_pipeline(
        &restart_failure,
        "immediate",
        Some(&verified),
        ProcessState {
            desktop: true,
            cli: false,
        },
        Injection::RestartFailure,
    )?;
    results.push(case(
        "restart-failure-does-not-roll-back-config",
        restart_failure_report.snapshot.current_provider.as_deref() == Some("provider-new")
            && restart_failure_report.snapshot.phase.as_deref() == Some("pending_restart"),
        json!(restart_failure_report.snapshot),
    ));

    for (name, injection, expected_phase, expected_provider) in [
        (
            "crash-after-prepared-recovers-old",
            Injection::CrashAfterPrepared,
            "rolled_back",
            Some("provider-old"),
        ),
        (
            "crash-after-config-recovers-new",
            Injection::CrashAfterConfigReplace,
            "completed",
            Some("provider-new"),
        ),
        (
            "crash-after-state-resumes-finalize",
            Injection::CrashAfterStateCommit,
            "completed",
            Some("provider-new"),
        ),
        (
            "external-edit-becomes-needs-attention",
            Injection::ExternalEditAfterPrepared,
            "needs_attention",
            None,
        ),
    ] {
        let paths = create_scenario(&session, name)?;
        let report = run_pipeline(
            &paths,
            "immediate",
            Some(&verified),
            ProcessState {
                desktop: true,
                cli: false,
            },
            injection,
        )?;
        results.push(case(
            name,
            report.snapshot.phase.as_deref() == Some(expected_phase)
                && report.snapshot.current_provider.as_deref() == expected_provider,
            json!(report.snapshot),
        ));
    }

    let config_failure = create_scenario(&session, "config-failure")?;
    let config_failure_before = snapshot(&config_failure)?;
    let config_failure_report = run_pipeline(
        &config_failure,
        "immediate",
        Some(&verified),
        ProcessState {
            desktop: true,
            cli: false,
        },
        Injection::ConfigFailBeforeReplace,
    )?;
    results.push(case(
        "config-failure-recovers-old",
        config_failure_report.snapshot.phase.as_deref() == Some("rolled_back")
            && config_failure_report.snapshot.current_provider.as_deref() == Some("provider-old")
            && config_failure_report.snapshot.config_hash == config_failure_before.config_hash,
        json!(config_failure_report.snapshot),
    ));

    let override_paths = create_scenario(&session, "project-override")?;
    let _ = run_pipeline(
        &override_paths,
        "later",
        Some(&verified),
        ProcessState {
            desktop: false,
            cli: false,
        },
        Injection::None,
    )?;
    let project_codex = override_paths.workspace.join(".codex");
    fs::create_dir_all(&project_codex)?;
    fs::write(project_codex.join("config.toml"), "model = \"project-model\"\n")?;
    let effective = read_effective_config(&override_paths.codex_home, &override_paths.workspace)?;
    let reconciliation = reconcile(&override_paths, effective)?;
    results.push(case(
        "project-layer-is-reported-not-overwritten",
        reconciliation.state == "managed_overridden"
            && reconciliation.effective.model.as_deref() == Some("project-model"),
        json!(reconciliation),
    ));

    let mutated = {
        let mut input = verified.input.clone();
        input.api_key.push_str("-changed");
        input
    };
    results.push(case(
        "validation-binding-detects-credential-change",
        combination_fingerprint(&mutated) != verified.evidence.combination_fingerprint,
        json!({"fingerprints_differ": true}),
    ));

    let real_scan = scan_codex_processes();
    let real_process_paths = create_scenario(&session, "real-process-handoff")?;
    let real_process_report = run_pipeline(
        &real_process_paths,
        "later",
        Some(&verified),
        ProcessState {
            desktop: real_scan.counts.get("desktop_root").copied().unwrap_or(0) > 0,
            cli: real_scan.counts.get("cli").copied().unwrap_or(0) > 0,
        },
        Injection::None,
    )?;
    let real_processes_present = real_scan
        .counts
        .get("desktop_root")
        .copied()
        .unwrap_or(0)
        + real_scan.counts.get("cli").copied().unwrap_or(0)
        > 0;
    results.push(case(
        "real-process-scan-feeds-safe-restart-boundary",
        !real_processes_present
            || real_process_report.snapshot.phase.as_deref() == Some("pending_restart"),
        json!({
            "counts": real_scan.counts,
            "phase": real_process_report.snapshot.phase,
            "real_processes_terminated": false
        }),
    ));

    let sensitive_files = session
        .read_dir()?
        .filter_map(Result::ok)
        .flat_map(|entry| {
            let root = entry.path();
            [root.join("state.db"), root.join("events.jsonl")]
        })
        .collect::<Vec<_>>();
    let secret_leaked = files_contain(&sensitive_files, verified.input.api_key.as_bytes())?;
    results.push(case(
        "database-and-event-logs-do-not-contain-api-key",
        !secret_leaked,
        json!({"secret_leaked": secret_leaked}),
    ));

    let passed = results.iter().filter(|value| value.passed).count();
    let summary = json!({
        "generated_at": Utc::now().to_rfc3339(),
        "passed": passed,
        "total": results.len(),
        "results": results,
        "real_process_scan": scan_codex_processes()
    });
    fs::write(
        evidence_root.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    Ok(summary)
}

pub fn write_live_summary(
    report: &PipelineReport,
    evidence_root: &Path,
    secret: &[u8],
) -> Result<Value> {
    fs::create_dir_all(evidence_root)?;
    let summary = json!({
        "generated_at": Utc::now().to_rfc3339(),
        "validation": report.validation,
        "snapshot": report.snapshot,
        "effective": report.effective,
        "reconciliation": report.reconciliation,
        "event_count": report.events.len()
    });
    let bytes = serde_json::to_vec_pretty(&summary)?;
    if bytes.windows(secret.len()).any(|window| window == secret) {
        bail!("live API key leaked into evidence summary");
    }
    fs::write(evidence_root.join("live-summary.json"), &bytes)?;
    Ok(summary)
}

fn files_contain(paths: &[PathBuf], needle: &[u8]) -> Result<bool> {
    for path in paths {
        if path.exists() {
            let bytes = fs::read(path)?;
            if bytes.windows(needle.len()).any(|window| window == needle) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn case(name: &str, passed: bool, evidence: Value) -> CaseResult {
    CaseResult {
        name: name.to_string(),
        passed,
        evidence,
    }
}

fn initialize_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE providers(
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            base_url TEXT NOT NULL,
            model TEXT NOT NULL,
            combination_fingerprint TEXT NOT NULL,
            validation_state TEXT NOT NULL CHECK(validation_state='validated')
        );
        CREATE TABLE environments(
            id TEXT PRIMARY KEY,
            current_provider_id TEXT REFERENCES providers(id),
            status TEXT NOT NULL
        );
        CREATE TABLE switch_operations(
            id TEXT PRIMARY KEY,
            environment_id TEXT NOT NULL REFERENCES environments(id),
            old_provider_id TEXT NOT NULL REFERENCES providers(id),
            new_provider_id TEXT NOT NULL REFERENCES providers(id),
            old_hash TEXT NOT NULL,
            new_hash TEXT NOT NULL,
            backup_path TEXT NOT NULL,
            decision TEXT NOT NULL,
            desktop_present INTEGER NOT NULL,
            cli_present INTEGER NOT NULL,
            phase TEXT NOT NULL,
            restart_attempts INTEGER NOT NULL DEFAULT 0,
            last_error TEXT
        );
        ",
    )?;
    Ok(())
}

fn open_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "
        PRAGMA foreign_keys=ON;
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=FULL;
        PRAGMA busy_timeout=5000;
        ",
    )?;
    Ok(conn)
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn create_backup(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let dir = path
        .parent()
        .context("config path has no parent")?
        .join(".gpteasy-backups");
    fs::create_dir_all(&dir)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let backup = dir.join(format!("config-{stamp}.toml"));
    write_synced(&backup, bytes)?;
    Ok(backup)
}

fn prune_backups(path: &Path, keep: usize) -> Result<()> {
    let dir = path
        .parent()
        .context("config path has no parent")?
        .join(".gpteasy-backups");
    let mut backups = fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("config-") && name.ends_with(".toml"))
        })
        .collect::<Vec<_>>();
    backups.sort();
    let remove_count = backups.len().saturating_sub(keep);
    for stale in backups.into_iter().take(remove_count) {
        fs::remove_file(stale)?;
    }
    Ok(())
}

fn write_temp(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let parent = path.parent().context("config path has no parent")?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp = parent.join(format!(".config.toml.gpteasy-{stamp}.tmp"));
    write_synced(&temp, bytes)?;
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temp, metadata.permissions())?;
    }
    Ok(temp)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn atomic_replace(target: &Path, replacement: &Path) -> Result<()> {
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
        bail!("ReplaceFileW failed with Win32 error {code}");
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(target: &Path, replacement: &Path) -> Result<()> {
    fs::rename(replacement, target)?;
    if let Some(parent) = target.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn append_event(path: &Path, category: &str, details: Value) -> Result<()> {
    let event = json!({
        "at": Utc::now().to_rfc3339(),
        "category": category,
        "details": details
    });
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, &event)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn read_events(path: &Path) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
}
