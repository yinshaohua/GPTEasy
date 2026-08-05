use serde_json::{json, Value};
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use toml_edit::{value, DocumentMut, Item, Table};

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
        _ => return Err("usage: toml-structural-edit run OUTPUT_DIR".into()),
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

    let preserve_dir = session.join("preserve");
    fs::create_dir_all(&preserve_dir)?;
    let preserve_file = preserve_dir.join("config.toml");
    let original_crlf = concat!(
        "# user heading\r\n",
        "model = \"old-model\" # keep inline\r\n",
        "model_provider = \"old-provider\"\r\n",
        "custom_flag = true\r\n",
        "\r\n",
        "[projects.'C:\\\\src\\\\demo']\r\n",
        "trust_level = \"trusted\"\r\n",
        "\r\n",
        "[model_providers.old-provider]\r\n",
        "name = \"Old\"\r\n",
        "base_url = \"https://old.example/v1\"\r\n",
        "wire_api = \"responses\"\r\n"
    );
    fs::write(&preserve_file, original_crlf.as_bytes())?;
    switch_provider(&preserve_file, &provider, Injection::None)?;
    let edited = fs::read_to_string(&preserve_file)?;
    let parsed = edited.parse::<DocumentMut>()?;
    let preserve_ok = edited.contains("# user heading\r\n")
        && edited.contains("custom_flag = true\r\n")
        && edited.contains("[projects.'C:\\\\src\\\\demo']\r\n")
        && !edited.replace("\r\n", "").contains('\n')
        && parsed["model"].as_str() == Some("provider-model")
        && parsed["model_provider"].as_str() == Some("gpteasy")
        && parsed["model_providers"]["old-provider"]["name"].as_str() == Some("Old")
        && parsed["model_providers"]["gpteasy"]["experimental_bearer_token"].as_str()
            == Some("fake-secret-not-real");
    results.push(case("preserve-comments-unknown-crlf", preserve_ok, json!({})));

    let malformed_dir = session.join("malformed");
    fs::create_dir_all(&malformed_dir)?;
    let malformed_file = malformed_dir.join("config.toml");
    let malformed = b"model = \"unterminated\n";
    fs::write(&malformed_file, malformed)?;
    let malformed_result = switch_provider(&malformed_file, &provider, Injection::None);
    let malformed_ok = malformed_result.is_err() && fs::read(&malformed_file)? == malformed;
    results.push(case("malformed-no-write", malformed_ok, json!({})));

    let fail_dir = session.join("fail-before-replace");
    fs::create_dir_all(&fail_dir)?;
    let fail_file = fail_dir.join("config.toml");
    let fail_original = b"custom_flag = true\n";
    fs::write(&fail_file, fail_original)?;
    let fail_result = switch_provider(&fail_file, &provider, Injection::FailBeforeReplace);
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
        switch_provider(&concurrent_file, &provider, Injection::ConcurrentWrite(external));
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
        switch_provider(&retention_file, &iteration, Injection::None)?;
    }
    let backups = backup_files(&retention_file)?;
    let retention_ok = backups.len() == BACKUP_LIMIT;
    results.push(case(
        "retain-latest-five-backups",
        retention_ok,
        json!({"backups": backups.len()}),
    ));

    let restore_dir = session.join("restore");
    fs::create_dir_all(&restore_dir)?;
    let restore_file = restore_dir.join("config.toml");
    let restore_original = b"model = \"before\"\ncustom_flag = true\n";
    fs::write(&restore_file, restore_original)?;
    switch_provider(&restore_file, &provider, Injection::None)?;
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

fn switch_provider(
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
    let mut doc = original_text.parse::<DocumentMut>()?;
    doc["model"] = value(provider.model);
    doc["model_provider"] = value("gpteasy");

    if !doc.contains_key("model_providers") || !doc["model_providers"].is_table() {
        doc["model_providers"] = Item::Table(Table::new());
    }
    let providers = doc["model_providers"]
        .as_table_mut()
        .ok_or("model_providers is not a table")?;
    if !providers.contains_key("gpteasy") || !providers["gpteasy"].is_table() {
        providers["gpteasy"] = Item::Table(Table::new());
    }
    let managed = providers["gpteasy"]
        .as_table_mut()
        .ok_or("gpteasy provider is not a table")?;
    managed["name"] = value(provider.name);
    managed["base_url"] = value(provider.base_url);
    managed["wire_api"] = value("responses");
    managed["supports_websockets"] = value(false);
    managed["experimental_bearer_token"] = value(provider.bearer_token);

    let rendered = normalize_newlines(&doc.to_string(), newline);
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
