use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::process::{Command, exit};

fn main() {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if let Some(log) = env::var_os("GPTEASY_WSL_HARNESS_LOG") {
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .expect("open WSL harness log");
        writeln!(
            log,
            "{}",
            args.iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .expect("write WSL harness log");
    }
    let is_list = args.iter().any(|arg| arg == "--list");
    let distribution = required("GPTEASY_WSL_HARNESS_DISTRIBUTION");
    if is_list {
        if !args.iter().any(|arg| arg == "--running") || harness_reports_running() {
            writeln!(io::stdout(), "{}", distribution.to_string_lossy())
                .expect("write isolated WSL set");
        }
        return;
    }

    let selected = args
        .windows(2)
        .find(|pair| pair[0] == "--distribution")
        .is_some_and(|pair| {
            pair[1]
                .to_string_lossy()
                .eq_ignore_ascii_case(&distribution.to_string_lossy())
        });
    let guest_home = required("GPTEASY_WSL_HARNESS_GUEST_HOME");
    if selected {
        mark_started();
    }
    let mut forwarded = Vec::with_capacity(args.len() + 2);
    for arg in args {
        let is_exec = arg == "--exec";
        forwarded.push(arg);
        if selected && is_exec {
            forwarded.push(OsString::from("/usr/bin/env"));
            forwarded.push(OsString::from(format!(
                "HOME={}",
                guest_home.to_string_lossy()
            )));
            forwarded.push(OsString::from(format!(
                "PATH={}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                guest_home.to_string_lossy()
            )));
        }
    }

    let status = Command::new(required("GPTEASY_WSL_HARNESS_REAL_EXE"))
        .args(forwarded)
        .status()
        .expect("forward to the real wsl.exe");
    exit(status.code().unwrap_or(1));
}

fn harness_reports_running() -> bool {
    let Some(path) = env::var_os("GPTEASY_WSL_HARNESS_STATE") else {
        return true;
    };
    let state = fs::read_to_string(&path).unwrap_or_else(|_| "running:0".to_owned());
    let state = state.trim();
    if state == "stopped" {
        return false;
    }
    let count = state
        .strip_prefix("running:")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let stop_after = env::var("GPTEASY_WSL_HARNESS_STOP_AFTER_RUNNING_LISTS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    if stop_after.is_some_and(|limit| count >= limit) {
        fs::write(path, "stopped\n").expect("record natural stop");
        return false;
    }
    fs::write(path, format!("running:{}\n", count + 1)).expect("record Running probe");
    true
}

fn mark_started() {
    let Some(path) = env::var_os("GPTEASY_WSL_HARNESS_STATE") else {
        return;
    };
    let state = fs::read_to_string(&path).unwrap_or_else(|_| "running:0".to_owned());
    if state.trim() == "stopped" {
        fs::write(path, "running:0\n").expect("record temporary start");
    }
}

fn required(name: &str) -> OsString {
    env::var_os(name).unwrap_or_else(|| panic!("missing {name}"))
}
