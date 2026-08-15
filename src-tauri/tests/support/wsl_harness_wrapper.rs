use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::process::{Command, exit};

fn main() {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    let is_list = args.iter().any(|arg| arg == "--list");
    let distribution = required("GPTEASY_WSL_HARNESS_DISTRIBUTION");
    if is_list {
        writeln!(io::stdout(), "{}", distribution.to_string_lossy())
            .expect("write isolated Running set");
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
        }
    }

    let status = Command::new(required("GPTEASY_WSL_HARNESS_REAL_EXE"))
        .args(forwarded)
        .status()
        .expect("forward to the real wsl.exe");
    exit(status.code().unwrap_or(1));
}

fn required(name: &str) -> OsString {
    env::var_os(name).unwrap_or_else(|| panic!("missing {name}"))
}
