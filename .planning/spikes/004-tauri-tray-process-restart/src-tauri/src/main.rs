use std::{env, path::PathBuf};

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--probe") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&gpteasy_spike_004_lib::scan()).unwrap()
            );
        }
        Some("--plan") => {
            let decision = args.next().unwrap_or_else(|| "later".to_string());
            let scan = gpteasy_spike_004_lib::scan();
            let plan = gpteasy_spike_004_lib::plan(&decision, scan.processes).unwrap();
            println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        }
        Some("--fixture-cycle") => {
            let root = PathBuf::from(args.next().expect("--fixture-cycle requires root"));
            match gpteasy_spike_004_lib::fixture_cycle(&root) {
                Ok(report) => println!("{}", serde_json::to_string_pretty(&report).unwrap()),
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
        }
        Some(other) => {
            eprintln!("unknown argument: {other}");
            std::process::exit(2);
        }
        None => gpteasy_spike_004_lib::run(),
    }
}
