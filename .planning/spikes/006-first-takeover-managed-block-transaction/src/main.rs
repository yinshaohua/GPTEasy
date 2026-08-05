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
        _ => return Err("usage: first-takeover-managed-block-transaction run OUTPUT_DIR".into()),
    }
    Ok(())
}

fn run_matrix(output: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    fs::create_dir_all(output)?;
    let summary_path = output.join("summary.json");
    let session = output.join(format!(
        "session-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    fs::create_dir_all(&session)?;
    let provider = Provider {
        model: "provider-model",
        name: "GPTEasy Test",
        base_url: "https://provider.example/v1",
        bearer_token: "fake-secret-not-real",
    };
    let mut results = Vec::new();

    let migrate_dir = session.join("migrate-existing");
    fs::create_dir_all(&migrate_dir)?;
    let migrate_file = migrate_dir.join("config.toml");
    let migrate_original = concat!(
        "# user heading\n",
        "model = \"old-model\" # GPTEasy may replace this\n",
        "model_provider = \"legacy\"\n",
        "custom_flag = true\n",
        "\n",
        "[model_providers.legacy]\n",
        "name = \"Legacy\"\n",
        "base_url = \"https://legacy.example/v1\"\n",
        "wire_api = \"responses\"\n",
        "\n",
        "[model_providers.gpteasy]\n",
        "name = \"Stale managed provider\"\n",
        "base_url = \"https://stale.example/v1\"\n",
        "wire_api = \"responses\"\n",
        "\n",
        "[projects.demo]\n",
        "trust_level = \"trusted\"\n",
    )
    .replace('\n', "\r\n");
    fs::write(&migrate_file, migrate_original.as_bytes())?;
    apply_provider(&migrate_file, &provider, Injection::None)?;
    let migrated = fs::read_to_string(&migrate_file)?;
    let migrated_doc = migrated.parse::<DocumentMut>()?;
    let migrate_ok = count_exact_line(&migrated, START) == 1
        && count_exact_line(&migrated, END) == 1
        && migrated_doc["model"].as_str() == Some("provider-model")
        && migrated_doc["model_provider"].as_str() == Some("gpteasy")
        && migrated_doc["model_providers"]["gpteasy"]["base_url"].as_str()
            == Some("https://provider.example/v1")
        && migrated_doc["model_providers"]["legacy"]["name"].as_str() == Some("Legacy")
        && migrated_doc["custom_flag"].as_bool() == Some(true)
        && migrated_doc["projects"]["demo"]["trust_level"].as_str() == Some("trusted")
        && !migrated.contains("Stale managed provider")
        && !migrated.replace("\r\n", "").contains('\n')
        && backup_files(&migrate_file)?.len() == 1;
    results.push(case(
        "structural-migration-establishes-one-block",
        migrate_ok,
        json!({"length": migrated.len()}),
    ));

    let explicit_parent_dir = session.join("explicit-parent");
    fs::create_dir_all(&explicit_parent_dir)?;
    let explicit_parent_file = explicit_parent_dir.join("config.toml");
    let explicit_parent_original = concat!(
        "model = \"old\"\n",
        "model_provider = \"legacy\"\n",
        "\n",
        "[model_providers]\n",
        "\n",
        "[model_providers.legacy]\n",
        "name = \"Legacy\"\n",
        "base_url = \"https://legacy.example/v1\"\n",
        "wire_api = \"responses\"\n",
    );
    fs::write(&explicit_parent_file, explicit_parent_original)?;
    apply_provider(&explicit_parent_file, &provider, Injection::None)?;
    let explicit_parent = fs::read_to_string(&explicit_parent_file)?;
    let explicit_parent_doc = explicit_parent.parse::<DocumentMut>()?;
    let explicit_parent_ok = !explicit_parent.contains("\n[model_providers]\n")
        && explicit_parent_doc["model_providers"]["legacy"]["name"].as_str() == Some("Legacy")
        && explicit_parent_doc["model_providers"]["gpteasy"]["name"].as_str()
            == Some("GPTEasy Test");
    results.push(case(
        "empty-explicit-parent-converted-to-implicit",
        explicit_parent_ok,
        json!({}),
    ));

    let unsupported_parent_dir = session.join("unsupported-parent-value");
    fs::create_dir_all(&unsupported_parent_dir)?;
    let unsupported_parent_file = unsupported_parent_dir.join("config.toml");
    let unsupported_parent_original = concat!(
        "model = \"old\"\n",
        "\n",
        "[model_providers]\n",
        "custom = \"unknown-shape\"\n",
        "\n",
        "[model_providers.legacy]\n",
        "name = \"Legacy\"\n",
    );
    fs::write(&unsupported_parent_file, unsupported_parent_original)?;
    let unsupported_parent_result =
        apply_provider(&unsupported_parent_file, &provider, Injection::None);
    let unsupported_parent_ok = unsupported_parent_result.is_err()
        && fs::read_to_string(&unsupported_parent_file)? == unsupported_parent_original
        && backup_files(&unsupported_parent_file)?.is_empty();
    results.push(case(
        "explicit-parent-values-stop-before-write",
        unsupported_parent_ok,
        json!({"error": unsupported_parent_result.err().map(|error| error.to_string())}),
    ));

    let clean_dir = session.join("clean-adoption");
    fs::create_dir_all(&clean_dir)?;
    let clean_file = clean_dir.join("config.toml");
    let clean_original =
        "# user heading\ncustom_flag = true\n\n[projects.demo]\ntrust_level = \"trusted\"\n";
    fs::write(&clean_file, clean_original)?;
    apply_provider(&clean_file, &provider, Injection::None)?;
    let clean_adopted = fs::read_to_string(&clean_file)?;
    let clean_doc = clean_adopted.parse::<DocumentMut>()?;
    let clean_ok = clean_adopted.starts_with(START)
        && clean_doc["custom_flag"].as_bool() == Some(true)
        && clean_doc["projects"]["demo"]["trust_level"].as_str() == Some("trusted");
    results.push(case("clean-first-adoption", clean_ok, json!({})));

    let preserve_dir = session.join("subsequent-preservation");
    fs::create_dir_all(&preserve_dir)?;
    let preserve_file = preserve_dir.join("config.toml");
    let preserve_original =
        "# before\ncustom_flag = true\n\n[projects.demo]\ntrust_level = \"trusted\"\n# after\n";
    fs::write(&preserve_file, preserve_original)?;
    apply_provider(&preserve_file, &provider, Injection::None)?;
    let first = fs::read_to_string(&preserve_file)?;
    let first_outside = outside_block(&first)?;
    let next_provider = Provider {
        model: "provider-model-next",
        name: "GPTEasy Next",
        base_url: "https://next.example/v1",
        bearer_token: "another-fake-secret",
    };
    apply_provider(&preserve_file, &next_provider, Injection::None)?;
    let second = fs::read_to_string(&preserve_file)?;
    let preserve_ok = outside_block(&second)? == first_outside
        && second.contains("provider-model-next")
        && !second.contains("fake-secret-not-real")
        && count_exact_line(&second, START) == 1;
    results.push(case(
        "subsequent-switch-preserves-outside-bytes",
        preserve_ok,
        json!({"outside_length": first_outside.len()}),
    ));

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
        let result = apply_provider(&file, &provider, Injection::None);
        let ok = result.is_err()
            && fs::read(&file)? == content.as_bytes()
            && backup_files(&file)?.is_empty();
        results.push(case(name, ok, json!({})));
    }

    let duplicate_outside_dir = session.join("duplicate-outside");
    fs::create_dir_all(&duplicate_outside_dir)?;
    let duplicate_outside_file = duplicate_outside_dir.join("config.toml");
    let duplicate_outside_original =
        format!("{}model = \"duplicate\"\n", render_block(&provider, "\n"));
    fs::write(&duplicate_outside_file, &duplicate_outside_original)?;
    let duplicate_outside_result =
        apply_provider(&duplicate_outside_file, &provider, Injection::None);
    let duplicate_outside_ok = duplicate_outside_result.is_err()
        && fs::read_to_string(&duplicate_outside_file)? == duplicate_outside_original
        && backup_files(&duplicate_outside_file)?.is_empty();
    results.push(case(
        "duplicate-managed-key-outside-block-rejected",
        duplicate_outside_ok,
        json!({"error": duplicate_outside_result.err().map(|error| error.to_string())}),
    ));

    let fail_dir = session.join("fail-before-replace");
    fs::create_dir_all(&fail_dir)?;
    let fail_file = fail_dir.join("config.toml");
    let fail_original = b"model = \"old\"\ncustom_flag = true\n";
    fs::write(&fail_file, fail_original)?;
    let fail_result = apply_provider(&fail_file, &provider, Injection::FailBeforeReplace);
    let fail_ok = fail_result.is_err()
        && fs::read(&fail_file)? == fail_original
        && backup_files(&fail_file)?.len() == 1;
    results.push(case(
        "forced-failure-preserves-original",
        fail_ok,
        json!({}),
    ));

    let concurrent_dir = session.join("concurrent");
    fs::create_dir_all(&concurrent_dir)?;
    let concurrent_file = concurrent_dir.join("config.toml");
    fs::write(&concurrent_file, b"model = \"old\"\ncustom_flag = true\n")?;
    let external = b"model = \"external\"\n# external editor\n";
    let concurrent_result = apply_provider(
        &concurrent_file,
        &provider,
        Injection::ConcurrentWrite(external),
    );
    let concurrent_ok = concurrent_result.is_err() && fs::read(&concurrent_file)? == external;
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
        apply_provider(&retention_file, &iteration, Injection::None)?;
    }
    let retention_ok = backup_files(&retention_file)?.len() == BACKUP_LIMIT;
    results.push(case("retain-latest-five-backups", retention_ok, json!({})));

    let restore_dir = session.join("restore");
    fs::create_dir_all(&restore_dir)?;
    let restore_file = restore_dir.join("config.toml");
    let restore_original = b"model = \"old\"\ncustom_flag = true\n";
    fs::write(&restore_file, restore_original)?;
    apply_provider(&restore_file, &provider, Injection::None)?;
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

fn apply_provider(
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
    let rendered = render_transaction(original_text, provider, newline)?;
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

fn render_transaction(
    original: &str,
    provider: &Provider<'_>,
    newline: &str,
) -> Result<String, Box<dyn std::error::Error>> {
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
        _ => return Err("managed block markers are missing, duplicated, or reversed".into()),
    };
    rendered.parse::<DocumentMut>()?;
    Ok(rendered)
}

fn migrate_structurally(
    original: &str,
    newline: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut doc = original.parse::<DocumentMut>()?;
    doc.remove("model");
    doc.remove("model_provider");

    let mut remove_parent = false;
    if let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_mut())
    {
        providers.remove("gpteasy");
        let unsupported_direct_value = providers.iter().any(|(_, item)| !item.is_table());
        if unsupported_direct_value {
            return Err(
                "explicit model_providers table contains direct values; refusing lossy migration"
                    .into(),
            );
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

fn render_block(provider: &Provider<'_>, newline: &str) -> String {
    let string = |value: &str| toml_edit::Value::from(value).to_string();
    [
        START.to_string(),
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

fn outside_block(value: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let markers = marker_ranges(value);
    match (markers.0.as_slice(), markers.1.as_slice()) {
        ([(start, _)], [(_, end)]) if start < end => {
            let mut outside = Vec::new();
            outside.extend_from_slice(value[..*start].as_bytes());
            outside.extend_from_slice(value[*end..].as_bytes());
            Ok(outside)
        }
        _ => Err("expected exactly one valid managed block".into()),
    }
}

fn count_exact_line(value: &str, expected: &str) -> usize {
    value
        .lines()
        .filter(|line| line.trim_end_matches('\r') == expected)
        .count()
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
