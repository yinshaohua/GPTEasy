use anyhow::{bail, Context, Result};
use gpteasy_spike_012_lib::{
    load_secret, run_live_pipeline, run_matrix, write_live_summary,
};
use std::{fs, path::PathBuf};

fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("matrix") => {
            let work = PathBuf::from(args.get(2).context("missing work root")?);
            let evidence = PathBuf::from(args.get(3).context("missing evidence root")?);
            let summary = run_matrix(&work, &evidence)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            if summary["passed"] != summary["total"] {
                std::process::exit(1);
            }
        }
        Some("live") => {
            let secret_path = PathBuf::from(args.get(2).context("missing secret path")?);
            let work = PathBuf::from(args.get(3).context("missing work root")?);
            let evidence = PathBuf::from(args.get(4).context("missing evidence root")?);
            fs::create_dir_all(&work)?;
            let input = load_secret(&secret_path)?;
            let key = input.api_key.clone();
            let report = run_live_pipeline(&work, input, "later")?;
            let summary = write_live_summary(&report, &evidence, key.as_bytes())?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        _ => bail!(
            "usage: spike-012-matrix <matrix WORK_ROOT EVIDENCE_ROOT | live SECRET WORK_ROOT EVIDENCE_ROOT>"
        ),
    }
    Ok(())
}
