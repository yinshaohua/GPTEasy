use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveConfig {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub model_origin: Option<String>,
    pub provider_origin: Option<String>,
    pub layers: Vec<String>,
}

pub fn read_effective_config(codex_home: &Path, cwd: &Path) -> Result<EffectiveConfig> {
    let mut command = codex_app_server_command()?;
    command
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().context("spawn codex app-server")?;
    let mut stdin = child.stdin.take().context("app-server stdin unavailable")?;
    let stdout = child.stdout.take().context("app-server stdout unavailable")?;
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
                "clientInfo": {"name": "gpteasy-spike-012", "version": "0.1.0"},
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
        .ok_or_else(|| anyhow::anyhow!("config/read failed: {response}"))?;
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

fn codex_app_server_command() -> Result<Command> {
    #[cfg(windows)]
    {
        if let Some(path) = std::env::var_os("GPTEASY_CODEX_EXE").map(PathBuf::from) {
            if path.is_file() {
                let mut command = Command::new(path);
                command.arg("app-server");
                command.creation_flags(CREATE_NO_WINDOW);
                return Ok(command);
            }
        }
        let output = Command::new("where.exe")
            .arg("codex.cmd")
            .output()
            .context("locate codex.cmd")?;
        if !output.status.success() {
            bail!("codex.cmd was not found on PATH");
        }
        let command_path = PathBuf::from(
            String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .context("where.exe returned no codex.cmd path")?
            .trim()
            .to_string(),
        );
        let node_global = command_path
            .parent()
            .context("codex.cmd has no parent directory")?;
        let native_root = node_global.join("node_modules/@openai/codex/node_modules");
        let native = find_native_codex(&native_root)?
            .context("native codex.exe was not found under the Codex npm package")?;
        let mut command = Command::new(native);
        command.arg("app-server");
        command.creation_flags(CREATE_NO_WINDOW);
        Ok(command)
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new("codex");
        command.arg("app-server");
        Ok(command)
    }
}

#[cfg(windows)]
fn find_native_codex(root: &Path) -> Result<Option<PathBuf>> {
    if !root.exists() {
        return Ok(None);
    }
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"))
                && path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("\\vendor\\")
            {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn send(stdin: &mut ChildStdin, value: Value) -> Result<()> {
    serde_json::to_writer(&mut *stdin, &value)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

fn wait_response(receiver: &Receiver<Value>, id: i64) -> Result<Value> {
    loop {
        let value = receiver
            .recv_timeout(Duration::from_secs(20))
            .context("app-server response timeout")?;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            if value.get("error").is_some() {
                bail!("app-server returned error: {value}");
            }
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
