use std::ffi::OsString;

pub mod path_smoke;

fn run_desktop() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run GPTEasy");
}

fn run_phase1_path_smoke(run_id: String) -> Result<(), &'static str> {
    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .map_err(|_| "failed to initialize GPTEasy for phase1-path-smoke")?;
    let report = path_smoke::run_path_smoke(app.handle(), &run_id)
        .map_err(|_| "phase1-path-smoke failed")?;
    let json = serde_json::to_string(&report)
        .map_err(|_| "failed to serialize phase1-path-smoke report")?;

    println!("{json}");
    app.cleanup_before_exit();
    Ok(())
}

fn exit_cli_error(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

fn run_with_args<I>(args: I)
where
    I: IntoIterator<Item = OsString>,
{
    match path_smoke::parse_cli_args(args) {
        Ok(None) => run_desktop(),
        Ok(Some(run_id)) => {
            if let Err(message) = run_phase1_path_smoke(run_id) {
                exit_cli_error(message);
            }
        }
        Err(error) => exit_cli_error(&error.to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    run_with_args(std::env::args_os().skip(1));
}
