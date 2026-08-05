use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use toml_edit::DocumentMut;

const START: &str = "# >>> GPTEasy managed provider >>>";
const END: &str = "# <<< GPTEasy managed provider <<<";

#[derive(Clone, Copy)]
struct Provider<'a> {
    id: &'a str,
    model: &'a str,
    name: &'a str,
    base_url: &'a str,
    bearer_token: &'a str,
}

#[derive(Clone, Copy)]
struct Processes {
    desktop: bool,
    cli: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Injection {
    None,
    CrashAfterPrepared,
    ConfigFailBeforeReplace,
    CrashAfterConfigReplace,
    CrashAfterStateCommit,
    ExternalEditAfterPrepared,
    RestartFailure,
}

struct ScenarioPaths {
    db: PathBuf,
    config: PathBuf,
    log: PathBuf,
}

struct PreparedConfig {
    original: Vec<u8>,
    rendered: Vec<u8>,
    old_hash: String,
    new_hash: String,
    backup: PathBuf,
}

#[derive(Debug)]
struct Snapshot {
    current_provider: Option<String>,
    config_hash: String,
    phase: Option<String>,
    operation_count: i64,
    restart_attempts: i64,
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
        _ => return Err("usage: provider-switch-saga run OUTPUT_DIR".into()),
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
    let mut results = Vec::new();

    let happy = create_scenario(&session, "happy-immediate")?;
    execute_switch(
        &happy,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: false,
        },
        Injection::None,
    )?;
    let happy_snapshot = snapshot(&happy)?;
    results.push(case(
        "immediate-desktop-switch-completes",
        is_new_and(&happy_snapshot, "completed") && happy_snapshot.restart_attempts == 1,
        snapshot_json(&happy_snapshot),
    ));

    let cli = create_scenario(&session, "immediate-cli")?;
    execute_switch(
        &cli,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: true,
        },
        Injection::None,
    )?;
    let cli_snapshot = snapshot(&cli)?;
    results.push(case(
        "cli-keeps-switch-pending",
        is_new_and(&cli_snapshot, "pending_restart") && cli_snapshot.restart_attempts == 1,
        snapshot_json(&cli_snapshot),
    ));

    let later = create_scenario(&session, "later")?;
    execute_switch(
        &later,
        "later",
        true,
        Processes {
            desktop: true,
            cli: true,
        },
        Injection::None,
    )?;
    let later_snapshot = snapshot(&later)?;
    results.push(case(
        "later-writes-without-restart-attempt",
        is_new_and(&later_snapshot, "pending_restart") && later_snapshot.restart_attempts == 0,
        snapshot_json(&later_snapshot),
    ));

    let no_processes = create_scenario(&session, "no-processes")?;
    execute_switch(
        &no_processes,
        "later",
        true,
        Processes {
            desktop: false,
            cli: false,
        },
        Injection::None,
    )?;
    let no_processes_snapshot = snapshot(&no_processes)?;
    results.push(case(
        "no-processes-completes-without-restart",
        is_new_and(&no_processes_snapshot, "completed")
            && no_processes_snapshot.restart_attempts == 0,
        snapshot_json(&no_processes_snapshot),
    ));

    let cancel = create_scenario(&session, "cancel")?;
    execute_switch(
        &cancel,
        "cancel",
        true,
        Processes {
            desktop: true,
            cli: true,
        },
        Injection::None,
    )?;
    let cancel_snapshot = snapshot(&cancel)?;
    results.push(case(
        "cancel-does-not-create-operation-or-write",
        is_old_without_operation(&cancel_snapshot),
        snapshot_json(&cancel_snapshot),
    ));

    let validation = create_scenario(&session, "validation-failure")?;
    let validation_result = execute_switch(
        &validation,
        "immediate",
        false,
        Processes {
            desktop: true,
            cli: false,
        },
        Injection::None,
    );
    let validation_snapshot = snapshot(&validation)?;
    results.push(case(
        "validation-failure-retains-old-state",
        validation_result.is_err() && is_old_without_operation(&validation_snapshot),
        json!({
            "snapshot": snapshot_json(&validation_snapshot),
            "error": validation_result.err().map(|error| error.to_string())
        }),
    ));

    let prepared_crash = create_scenario(&session, "crash-after-prepared")?;
    let _ = execute_switch(
        &prepared_crash,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: false,
        },
        Injection::CrashAfterPrepared,
    );
    recover(&prepared_crash, true)?;
    let prepared_snapshot = snapshot(&prepared_crash)?;
    results.push(case(
        "recovery-rolls-back-prepared-with-old-file",
        is_old_and(&prepared_snapshot, "rolled_back"),
        snapshot_json(&prepared_snapshot),
    ));

    let config_failure = create_scenario(&session, "config-failure")?;
    let _ = execute_switch(
        &config_failure,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: false,
        },
        Injection::ConfigFailBeforeReplace,
    );
    recover(&config_failure, true)?;
    let config_failure_snapshot = snapshot(&config_failure)?;
    results.push(case(
        "recovery-rolls-back-failed-config-replace",
        is_old_and(&config_failure_snapshot, "rolled_back"),
        snapshot_json(&config_failure_snapshot),
    ));

    let config_crash = create_scenario(&session, "crash-after-config")?;
    let _ = execute_switch(
        &config_crash,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: false,
        },
        Injection::CrashAfterConfigReplace,
    );
    let before_recovery = snapshot(&config_crash)?;
    recover(&config_crash, true)?;
    let config_crash_snapshot = snapshot(&config_crash)?;
    results.push(case(
        "recovery-commits-state-when-new-config-is-present",
        before_recovery.current_provider.as_deref() == Some("provider-old")
            && before_recovery.phase.as_deref() == Some("prepared")
            && is_new_and(&config_crash_snapshot, "completed")
            && config_crash_snapshot.restart_attempts == 1,
        json!({
            "before_recovery": snapshot_json(&before_recovery),
            "after_recovery": snapshot_json(&config_crash_snapshot)
        }),
    ));

    let state_crash = create_scenario(&session, "crash-after-state")?;
    let _ = execute_switch(
        &state_crash,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: false,
        },
        Injection::CrashAfterStateCommit,
    );
    recover(&state_crash, true)?;
    let state_crash_snapshot = snapshot(&state_crash)?;
    results.push(case(
        "recovery-resumes-restart-after-state-commit",
        is_new_and(&state_crash_snapshot, "completed")
            && state_crash_snapshot.restart_attempts == 1,
        snapshot_json(&state_crash_snapshot),
    ));

    let state_crash_cli = create_scenario(&session, "crash-after-state-with-cli")?;
    let _ = execute_switch(
        &state_crash_cli,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: true,
        },
        Injection::CrashAfterStateCommit,
    );
    recover(&state_crash_cli, true)?;
    let state_crash_cli_snapshot = snapshot(&state_crash_cli)?;
    results.push(case(
        "recovery-preserves-cli-pending-restart",
        is_new_and(&state_crash_cli_snapshot, "pending_restart")
            && state_crash_cli_snapshot.restart_attempts == 1,
        snapshot_json(&state_crash_cli_snapshot),
    ));

    let restart_failure = create_scenario(&session, "restart-failure")?;
    execute_switch(
        &restart_failure,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: false,
        },
        Injection::RestartFailure,
    )?;
    let restart_failure_snapshot = snapshot(&restart_failure)?;
    results.push(case(
        "restart-failure-does-not-roll-back-config",
        is_new_and(&restart_failure_snapshot, "pending_restart")
            && restart_failure_snapshot.restart_attempts == 1,
        snapshot_json(&restart_failure_snapshot),
    ));

    let external = create_scenario(&session, "external-edit")?;
    let _ = execute_switch(
        &external,
        "immediate",
        true,
        Processes {
            desktop: true,
            cli: false,
        },
        Injection::ExternalEditAfterPrepared,
    );
    recover(&external, true)?;
    let external_snapshot = snapshot(&external)?;
    results.push(case(
        "unknown-file-hash-becomes-external-needs-attention",
        external_snapshot.current_provider.is_none()
            && external_snapshot.phase.as_deref() == Some("needs_attention")
            && external_snapshot.restart_attempts == 0,
        snapshot_json(&external_snapshot),
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

fn create_scenario(
    session: &Path,
    name: &str,
) -> Result<ScenarioPaths, Box<dyn std::error::Error>> {
    let root = session.join(name);
    fs::create_dir_all(&root)?;
    let paths = ScenarioPaths {
        db: root.join("state.db"),
        config: root.join("config.toml"),
        log: root.join("events.jsonl"),
    };
    let old = old_provider();
    let initial = format!(
        "{}custom_flag = true\n\n[projects.demo]\ntrust_level = \"trusted\"\n",
        render_block(&old, "\n")
    );
    fs::write(&paths.config, initial)?;
    let conn = open_db(&paths.db)?;
    initialize_schema(&conn)?;
    insert_provider(&conn, &old)?;
    insert_provider(&conn, &new_provider())?;
    conn.execute(
        "INSERT INTO environments(id, current_provider_id) VALUES ('native', ?1)",
        params![old.id],
    )?;
    Ok(paths)
}

fn execute_switch(
    paths: &ScenarioPaths,
    decision: &str,
    validation_ok: bool,
    processes: Processes,
    injection: Injection,
) -> Result<(), Box<dyn std::error::Error>> {
    if !matches!(decision, "immediate" | "later" | "cancel") {
        return Err(format!("unsupported decision `{decision}`").into());
    }
    if decision == "cancel" {
        append_event(&paths.log, "cancelled", json!({}))?;
        return Ok(());
    }
    if !validation_ok {
        append_event(&paths.log, "validation_failed", json!({}))?;
        return Err("provider validation failed before switch preparation".into());
    }

    let old = old_provider();
    let new = new_provider();
    let prepared = prepare_config(&paths.config, &new)?;
    let operation_id = format!(
        "switch-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let mut conn = open_db(&paths.db)?;
    {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO switch_operations(
                id, environment_id, old_provider_id, new_provider_id,
                old_hash, new_hash, backup_path, decision,
                desktop_present, cli_present, phase, restart_attempts
             ) VALUES (?1, 'native', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'prepared', 0)",
            params![
                operation_id,
                old.id,
                new.id,
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
        json!({"operation_id": operation_id, "decision": decision}),
    )?;

    if injection == Injection::CrashAfterPrepared {
        return Err("simulated crash after prepared".into());
    }
    if injection == Injection::ExternalEditAfterPrepared {
        fs::write(
            &paths.config,
            b"# external editor\nmodel = \"external-model\"\nmodel_provider = \"external\"\n",
        )?;
        return Err("simulated external edit after prepared".into());
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
        return Err("simulated crash after config replace".into());
    }

    commit_new_state(&mut conn, &operation_id, new.id)?;
    append_event(
        &paths.log,
        "state_committed",
        json!({"operation_id": operation_id, "provider_id": new.id}),
    )?;
    if injection == Injection::CrashAfterStateCommit {
        return Err("simulated crash after state commit".into());
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

fn recover(
    paths: &ScenarioPaths,
    desktop_restart_succeeds: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = open_db(&paths.db)?;
    let pending = {
        let mut statement = conn.prepare(
            "SELECT id, old_provider_id, new_provider_id, old_hash, new_hash,
                    decision, desktop_present, cli_present, phase
             FROM switch_operations
             WHERE phase NOT IN ('completed', 'rolled_back', 'needs_attention')
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
                Processes {
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
                "UPDATE environments SET current_provider_id = ?1 WHERE id = 'native'",
                params![old_provider],
            )?;
            tx.execute(
                "UPDATE switch_operations
                 SET phase = 'rolled_back', last_error = 'recovery observed old config hash'
                 WHERE id = ?1",
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
            "UPDATE environments SET current_provider_id = NULL WHERE id = 'native'",
            [],
        )?;
        tx.execute(
            "UPDATE switch_operations
             SET phase = 'needs_attention', last_error = 'config hash matches neither old nor new'
             WHERE id = ?1",
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

fn finalize_restart(
    conn: &mut Connection,
    operation_id: &str,
    decision: &str,
    processes: Processes,
    desktop_restart_succeeds: bool,
    log: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
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
                Some("desktop restart failed; configuration remains active after restart"),
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
        "UPDATE switch_operations
         SET phase = ?1, last_error = ?2, restart_attempts = restart_attempts + ?3
         WHERE id = ?4",
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
            "desktop_restart_succeeds": desktop_restart_succeeds
        }),
    )?;
    Ok(())
}

fn commit_new_state(
    conn: &mut Connection,
    operation_id: &str,
    new_provider: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE environments SET current_provider_id = ?1 WHERE id = 'native'",
        params![new_provider],
    )?;
    tx.execute(
        "UPDATE switch_operations SET phase = 'state_committed' WHERE id = ?1",
        params![operation_id],
    )?;
    tx.commit()?;
    Ok(())
}

fn prepare_config(
    path: &Path,
    provider: &Provider<'_>,
) -> Result<PreparedConfig, Box<dyn std::error::Error>> {
    let original = fs::read(path)?;
    let original_text = std::str::from_utf8(&original)?;
    let newline = if original_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let rendered = replace_managed_block(original_text, provider, newline)?;
    rendered.parse::<DocumentMut>()?;
    let backup = create_backup(path, &original)?;
    Ok(PreparedConfig {
        old_hash: hash_bytes(&original),
        new_hash: hash_bytes(rendered.as_bytes()),
        original,
        rendered: rendered.into_bytes(),
        backup,
    })
}

fn apply_prepared(
    path: &Path,
    prepared: &PreparedConfig,
    fail_before_replace: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = write_temp(path, &prepared.rendered)?;
    if fail_before_replace {
        let _ = fs::remove_file(&temp);
        return Err("injected failure before atomic replace".into());
    }
    if fs::read(path)? != prepared.original {
        let _ = fs::remove_file(&temp);
        return Err("configuration changed concurrently".into());
    }
    atomic_replace(path, &temp)?;
    Ok(())
}

fn replace_managed_block(
    original: &str,
    provider: &Provider<'_>,
    newline: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let (starts, ends) = marker_ranges(original);
    match (starts.as_slice(), ends.as_slice()) {
        ([(start, _)], [(_, end)]) if start < end => {
            let mut rendered = String::with_capacity(original.len());
            rendered.push_str(&original[..*start]);
            rendered.push_str(&render_block(provider, newline));
            rendered.push_str(&original[*end..]);
            Ok(rendered)
        }
        _ => Err("switch saga requires exactly one established managed block".into()),
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
        format!("model_providers.gpteasy.id = {}", string(provider.id)),
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

fn initialize_schema(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute_batch(
        "
        CREATE TABLE providers(
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            base_url TEXT NOT NULL,
            api_key TEXT NOT NULL,
            model TEXT NOT NULL,
            validation_state TEXT NOT NULL CHECK(validation_state = 'validated')
        );
        CREATE TABLE environments(
            id TEXT PRIMARY KEY,
            current_provider_id TEXT REFERENCES providers(id)
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

fn insert_provider(
    conn: &Connection,
    provider: &Provider<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute(
        "INSERT INTO providers(id, name, base_url, api_key, model, validation_state)
         VALUES (?1, ?2, ?3, ?4, ?5, 'validated')",
        params![
            provider.id,
            provider.name,
            provider.base_url,
            provider.bearer_token,
            provider.model
        ],
    )?;
    Ok(())
}

fn open_db(path: &Path) -> Result<Connection, Box<dyn std::error::Error>> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = FULL;
        PRAGMA busy_timeout = 5000;
        ",
    )?;
    Ok(conn)
}

fn snapshot(paths: &ScenarioPaths) -> Result<Snapshot, Box<dyn std::error::Error>> {
    let conn = open_db(&paths.db)?;
    let current_provider = conn.query_row(
        "SELECT current_provider_id FROM environments WHERE id = 'native'",
        [],
        |row| row.get::<_, Option<String>>(0),
    )?;
    let operation_count = conn.query_row("SELECT COUNT(*) FROM switch_operations", [], |row| {
        row.get(0)
    })?;
    let latest = conn
        .query_row(
            "SELECT phase, restart_attempts
             FROM switch_operations ORDER BY rowid DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    Ok(Snapshot {
        current_provider,
        config_hash: hash_bytes(&fs::read(&paths.config)?),
        phase: latest.as_ref().map(|entry| entry.0.clone()),
        operation_count,
        restart_attempts: latest.map_or(0, |entry| entry.1),
    })
}

fn is_new_and(snapshot: &Snapshot, phase: &str) -> bool {
    snapshot.current_provider.as_deref() == Some("provider-new")
        && snapshot.phase.as_deref() == Some(phase)
}

fn is_old_and(snapshot: &Snapshot, phase: &str) -> bool {
    snapshot.current_provider.as_deref() == Some("provider-old")
        && snapshot.phase.as_deref() == Some(phase)
}

fn is_old_without_operation(snapshot: &Snapshot) -> bool {
    snapshot.current_provider.as_deref() == Some("provider-old")
        && snapshot.operation_count == 0
        && snapshot.phase.is_none()
}

fn snapshot_json(snapshot: &Snapshot) -> Value {
    json!({
        "current_provider": snapshot.current_provider,
        "config_hash": snapshot.config_hash,
        "phase": snapshot.phase,
        "operation_count": snapshot.operation_count,
        "restart_attempts": snapshot.restart_attempts
    })
}

fn old_provider() -> Provider<'static> {
    Provider {
        id: "provider-old",
        model: "old-model",
        name: "Old Provider",
        base_url: "https://old.example/v1",
        bearer_token: "old-fake-secret",
    }
}

fn new_provider() -> Provider<'static> {
    Provider {
        id: "provider-new",
        model: "new-model",
        name: "New Provider",
        base_url: "https://new.example/v1",
        bearer_token: "new-fake-secret",
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn create_backup(path: &Path, bytes: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = path
        .parent()
        .ok_or("config path has no parent")?
        .join(".gpteasy-backups");
    fs::create_dir_all(&dir)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let backup = dir.join(format!("config-{stamp}.toml"));
    write_synced(&backup, bytes)?;
    Ok(backup)
}

fn write_temp(path: &Path, bytes: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("config path has no parent")?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp = parent.join(format!(".config.toml.gpteasy-{stamp}.tmp"));
    write_synced(&temp, bytes)?;
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temp, metadata.permissions())?;
    }
    Ok(temp)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
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
    if let Some(parent) = target.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn append_event(
    path: &Path,
    category: &str,
    details: Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let event = json!({
        "timestamp_unix": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "category": category,
        "details": details
    });
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, &event)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn case(name: &str, passed: bool, details: Value) -> Value {
    json!({"name": name, "passed": passed, "details": details})
}
