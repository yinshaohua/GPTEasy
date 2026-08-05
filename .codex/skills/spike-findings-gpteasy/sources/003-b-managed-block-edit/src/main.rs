use serde_json::{json, Value};
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
const BACKUP_LIMIT: usize = 5;

#[derive(Clone)]
struct Provider<'a> {
    model: &'a str,
    name: &'a str,
    base_url: &'a str,
    bearer_token: &'a str,
}

enum Injection<'a> {
    None,
    FailBeforeReplace,
    ConcurrentWrite(&'a [u8]),
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
        _ => return Err("usage: managed-block-edit run OUTPUT_DIR".into()),
    }
    Ok(())
}

fn run_matrix(output: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    fs::create_dir_all(output)?;
    let summary_path = output.join("summary.json");
    let session = output.join(format!(
        "session-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos()
    ));
    fs::create_dir_all(&session)?;
    let provider = Provider {
        model: "provider-model",
        name: "GPTEasy Test",
        base_url: "https://provider.example/v1",
        bearer_token: "fake-secret-not-real",
    };
    let mut results = Vec::new();

    let adopt_dir = session.join("adopt");
    fs::create_dir_all(&adopt_dir)?;
    let adopt_file = adopt_dir.join("config.toml");
    let adopt_original = "# user heading\ncustom_flag = true\n\n[projects.demo]\ntrust_level = \"trusted\"\n";
    fs::write(&adopt_file, adopt_original)?;
    replace_block(&adopt_file, &provider, Injection::None)?;
    let adopted = fs::read_to_string(&adopt_file)?;
    let adopted_doc = adopted.parse::<DocumentMut>()?;
    let adopt_ok = adopted.starts_with(START)
        && adopted.ends_with(adopt_original)
        && adopted_doc["model"].as_str() == Some("provider-model")
        && adopted_doc["model_providers"]["gpteasy"]["base_url"].as_str()
            == Some("https://provider.example/v1")
        && adopted_doc["projects"]["demo"]["trust_level"].as_str() == Some("trusted");
    results.push(case("safe-first-adoption-without-conflicts", adopt_ok, json!({})));

    let migration_dir = session.join("needs-migration");
    fs::create_dir_all(&migration_dir)?;
    let migration_file = migration_dir.join("config.toml");
    let migration_original =
        b"model = \"existing\"\nmodel_provider = \"existing-provider\"\ncustom_flag = true\n";
    fs::write(&migration_file, migration_original)?;
    let migration_result = replace_block(&migration_file, &provider, Injection::None);
    let migration_ok =
        migration_result.is_err() && fs::read(&migration_file)? == migration_original;
    results.push(case(
        "existing-managed-keys-require-migration",
        migration_ok,
        json!({"error": migration_result.err().map(|error| error.to_string())}),
    ));

    let exact_dir = session.join("exact-preservation");
    fs::create_dir_all(&exact_dir)?;
    let exact_file = exact_dir.join("config.toml");
    let prefix = "# before\ncustom_flag = true\n";
    let old_block = render_block(
        &Provider {
            model: "old",
            name: "Old",
            base_url: "https://old.example/v1",
            bearer_token: "old-fake",
        },
        "\n",
    );
    let suffix = "[projects.demo]\ntrust_level = \"trusted\"\n# after\n";
    let exact_original = format!("{prefix}{old_block}{suffix}");
    fs::write(&exact_file, &exact_original)?;
    replace_block(&exact_file, &provider, Injection::None)?;
    let exact_edited = fs::read_to_string(&exact_file)?;
    let exact_ok = exact_edited.starts_with(prefix)
        && exact_edited.ends_with(suffix)
        && exact_edited.contains("provider-model")
        && !exact_edited.contains("old-fake");
    results.push(case("outside-bytes-preserved", exact_ok, json!({})));

    for (name, content) in [
        (
            "missing-end-marker",
            format!("{START}\nmodel = \"x\"\ncustom_flag = true\n"),
        ),
        (
            "duplicate-start-marker",
            format!("{START}\n{START}\nmodel = \"x\"\n{END}\n"),
        ),
        (
            "reversed-markers",
            format!("{END}\nmodel = \"x\"\n{START}\n"),
        ),
    ] {
        let dir = session.join(name);
        fs::create_dir_all(&dir)?;
        let file = dir.join("config.toml");
        fs::write(&file, content.as_bytes())?;
        let result = replace_block(&file, &provider, Injection::None);
        let ok = result.is_err() && fs::read(&file)? == content.as_bytes();
        results.push(case(name, ok, json!({})));
    }

    let crlf_dir = session.join("crlf");
    fs::create_dir_all(&crlf_dir)?;
    let crlf_file = crlf_dir.join("config.toml");
    let crlf_original = "custom_flag = true\r\n\r\n[projects.demo]\r\ntrust_level = \"trusted\"\r\n";
    fs::write(&crlf_file, crlf_original)?;
    replace_block(&crlf_file, &provider, Injection::None)?;
    let crlf_edited = fs::read_to_string(&crlf_file)?;
    let crlf_ok = !crlf_edited.replace("\r\n", "").contains('\n');
    results.push(case("crlf-preserved", crlf_ok, json!({})));

    let fail_dir = session.join("fail-before-replace");
    fs::create_dir_all(&fail_dir)?;
    let fail_file = fail_dir.join("config.toml");
    let fail_original = b"custom_flag = true\n";
    fs::write(&fail_file, fail_original)?;
    let fail_result = replace_block(&fail_file, &provider, Injection::FailBeforeReplace);
    let fail_ok = fail_result.is_err()
        && fs::read(&fail_file)? == fail_original
        && backup_files(&fail_file)?.len() == 1;
    results.push(case("forced-failure-preserves-original", fail_ok, json!({})));

    let concurrent_dir = session.join("concurrent");
    fs::create_dir_all(&concurrent_dir)?;
    let concurrent_file = concurrent_dir.join("config.toml");
    fs::write(&concurrent_file, b"custom_flag = true\n")?;
    let external = b"custom_flag = false\n# external editor\n";
    let concurrent_result =
        replace_block(&concurrent_file, &provider, Injection::ConcurrentWrite(external));
    let concurrent_ok =
        concurrent_result.is_err() && fs::read(&concurrent_file)? == external;
    results.push(case("concurrent-change-aborts", concurrent_ok, json!({})));

    let retention_dir = session.join("retention");
    fs::create_dir_all(&retention_dir)?;
    let retention_file = retention_dir.join("config.toml");
    fs::write(&retention_file, b"custom_flag = true\n")?;
    for index in 0..7 {
        let model = format!("provider-model-{index}");
        let iteration = Provider {
            model: &model,
            ..provider.clone()
        };
        replace_block(&retention_file, &iteration, Injection::None)?;
    }
    let retention_ok = backup_files(&retention_file)?.len() == BACKUP_LIMIT;
    results.push(case("retain-latest-five-backups", retention_ok, json!({})));

    let restore_dir = session.join("restore");
    fs::create_dir_all(&restore_dir)?;
    let restore_file = restore_dir.join("config.toml");
    let restore_original = b"custom_flag = true\n";
    fs::write(&restore_file, restore_original)?;
    replace_block(&restore_file, &provider, Injection::None)?;
    restore_latest(&restore_file)?;
    let restore_ok = fs::read(&restore_file)? == restore_original;
    results.push(case("restore-latest-backup", restore_ok, json!({})));

    let passed = results
        .iter()
        .filter(|entry| entry["passed"] == true)
        .count();
    let summary = json!({"passed": passed, "total": results.len(), "results": results});
    fs::write(summary_path, serde_json::to_vec_pretty(&summary)?)?;
    Ok(summary)
}

fn replace_block(
    path: &Path,
    provider: &Provider<'_>,
    injection: Injection<'_>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let original = fs::read(path)?;
    let original_text = std::str::from_utf8(&original)?;
    let newline = if original_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let block = render_block(provider, newline);
    let rendered = replace_or_insert(original_text, &block)?;
    rendered.parse::<DocumentMut>()?;

    let backup = create_backup(path, &original)?;
    prune_backups(path, BACKUP_LIMIT)?;
    let temp = write_temp(path, rendered.as_bytes())?;

    match injection {
        Injection::None => {}
        Injection::FailBeforeReplace => {
            let _ = fs::remove_file(&temp);
            return Err("injected failure before atomic replace".into());
        }
        Injection::ConcurrentWrite(bytes) => fs::write(path, bytes)?,
    }
    if fs::read(path)? != original {
        let _ = fs::remove_file(&temp);
        return Err("configuration changed concurrently".into());
    }
    atomic_replace(path, &temp)?;
    Ok(backup)
}

fn replace_or_insert(original: &str, block: &str) -> Result<String, Box<dyn std::error::Error>> {
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

    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => {
            let doc = original.parse::<DocumentMut>()?;
            let has_model = doc.get("model").is_some();
            let has_provider = doc.get("model_provider").is_some();
            let has_managed_table = doc
                .get("model_providers")
                .and_then(|item| item.as_table())
                .is_some_and(|table| table.contains_key("gpteasy"));
            if has_model || has_provider || has_managed_table {
                return Err("existing managed keys require structural migration".into());
            }
            Ok(format!("{block}{original}"))
        }
        ([(start, _)], [(_, end)]) if start < end => {
            let mut rendered = String::with_capacity(original.len() + block.len());
            rendered.push_str(&original[..*start]);
            rendered.push_str(block);
            rendered.push_str(&original[*end..]);
            Ok(rendered)
        }
        _ => Err("managed block markers are missing, duplicated, or reversed".into()),
    }
}

fn render_block(provider: &Provider<'_>, newline: &str) -> String {
    let string = |value: &str| toml_edit::Value::from(value).to_string();
    [
        START.to_string(),
        format!("model = {}", string(provider.model)),
        "model_provider = \"gpteasy\"".to_string(),
        format!(
            "model_providers.gpteasy.name = {}",
            string(provider.name)
        ),
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

fn create_backup(path: &Path, bytes: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = backup_dir(path)?;
    fs::create_dir_all(&dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos();
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
    Ok(path.parent().ok_or("config path has no parent")?.join(".gpteasy-backups"))
}

fn write_temp(path: &Path, bytes: &[u8]) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("config path has no parent")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos();
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
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH},
    };
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
            REPLACEFILE_WRITE_THROUGH,
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

fn case(name: &str, passed: bool, details: Value) -> Value {
    json!({"name": name, "passed": passed, "details": details})
}
