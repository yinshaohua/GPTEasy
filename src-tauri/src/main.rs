#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(
    not(all(target_os = "windows", target_arch = "x86_64")),
    not(feature = "native-linux-acceptance")
))]
compile_error!("GPTEasy v0.1 only supports Windows x64");

#[cfg(any(
    all(target_os = "windows", target_arch = "x86_64"),
    not(feature = "native-linux-acceptance")
))]
fn main() {
    gpteasy_lib::run();
}

#[cfg(all(
    not(all(target_os = "windows", target_arch = "x86_64")),
    feature = "native-linux-acceptance"
))]
fn main() {}
