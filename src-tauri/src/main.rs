#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
compile_error!("GPTEasy v0.1 only supports Windows x64");

fn main() {
    gpteasy_lib::run();
}
