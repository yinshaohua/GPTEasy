use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .to_path_buf()
}

fn acceptance_contract() -> Value {
    serde_json::from_slice(
        &fs::read(repository_root().join("scripts/linux-wsl-acceptance-contract.json"))
            .expect("read Linux/WSL2 acceptance contract"),
    )
    .expect("parse Linux/WSL2 acceptance contract")
}

fn ids(contract: &Value, field: &str) -> HashSet<String> {
    contract[field]
        .as_array()
        .unwrap_or_else(|| panic!("{field} array"))
        .iter()
        .map(|item| {
            item["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{field} id"))
                .to_owned()
        })
        .collect()
}

#[test]
fn issue_35_contract_covers_every_public_acceptance_surface() {
    let contract = acceptance_contract();

    assert_eq!(contract["schemaVersion"], 1);
    assert_eq!(contract["issue"], 35);
    assert_eq!(
        ids(&contract, "shellMatrix"),
        HashSet::from([
            "gnu-bash-4.4".to_owned(),
            "gnu-bash-current".to_owned(),
            "zsh-5.9".to_owned(),
        ])
    );
    assert_eq!(
        ids(&contract, "automatedMatrix"),
        HashSet::from([
            "linux-export-generator".to_owned(),
            "linux-shell-public-behavior".to_owned(),
            "wsl-shared-protocol".to_owned(),
            "sqlite-schema-and-saga".to_owned(),
            "provider-deletion-and-credential-cleanup".to_owned(),
            "react-linux-wsl-workflows".to_owned(),
            "domain-and-interface-contract".to_owned(),
        ])
    );
    assert_eq!(
        ids(&contract, "realEnvironmentGates"),
        HashSet::from([
            "independent-gnu-linux".to_owned(),
            "wsl2-running-guest".to_owned(),
            "wsl2-stopped-guest".to_owned(),
        ])
    );

    let surfaces = contract["canaryScannedSurfaces"]
        .as_array()
        .expect("canary surfaces")
        .iter()
        .map(|value| value.as_str().expect("surface").to_owned())
        .collect::<HashSet<_>>();
    assert_eq!(
        surfaces,
        HashSet::from([
            "process_arguments".to_owned(),
            "standard_output".to_owned(),
            "standard_error".to_owned(),
            "frontend_dom".to_owned(),
            "notification".to_owned(),
            "error_details".to_owned(),
            "test_logs".to_owned(),
            "application_logs".to_owned(),
            "screenshot_assist".to_owned(),
            "acceptance_report".to_owned(),
        ])
    );
    assert_eq!(
        contract["canarySurfaceVerifiers"]["process_arguments"],
        json!([
            "__runner_process_arguments__",
            "linux-shell-public-behavior"
        ])
    );
    assert_eq!(
        contract["canarySurfaceVerifiers"]["application_logs"],
        json!(["__application_logs__"])
    );
    assert_eq!(
        contract["applicationLogSteps"],
        json!([
            "wsl-shared-protocol",
            "provider-deletion-and-credential-cleanup",
            "wsl2-running-guest",
            "wsl2-stopped-guest"
        ])
    );
}

#[test]
fn issue_31_real_acceptance_requires_real_codex_and_an_independent_linux_kernel() {
    let contract = acceptance_contract();

    assert_eq!(contract["realAcceptanceIssue"], 31);
    assert_eq!(contract["realCodex"]["minimumVersion"], "0.147.0");
    assert_eq!(
        contract["realCodex"]["verificationMethod"],
        "app-server config/read"
    );
    assert_eq!(
        contract["nativeLinux"]["rejectedKernelPatterns"],
        json!(["microsoft", "wsl"])
    );
    assert_eq!(
        contract["nativeLinux"]["requiredDistribution"],
        "Ubuntu GNU/Linux"
    );
    assert_eq!(contract["nativeLinux"]["requiredOsReleaseId"], "ubuntu");

    for gate in contract["realEnvironmentGates"]
        .as_array()
        .expect("real environment gates")
    {
        assert_eq!(gate["completionIssue"], 31);
        assert!(
            gate["requiredEvidence"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "real gate must declare completion evidence: {gate}"
        );
    }
}

#[test]
fn extracted_real_acceptance_source_can_bind_evidence_to_a_commit() {
    let runner =
        fs::read_to_string(repository_root().join("scripts/run-linux-wsl-acceptance-gate.ps1"))
            .expect("read Linux and WSL acceptance runner");

    assert!(runner.contains("[string]$SourceCommit"));
    assert!(runner.contains("SourceCommit must be a 40-character Git commit SHA."));
    assert!(runner.contains("gitCommit = $gitCommit"));
    assert!(runner.contains("--features', 'native-linux-acceptance"));
    assert!(runner.contains("@('127.0.0.1', 'localhost', '::1')"));
    assert!(runner.contains("Full WSL2 acceptance requires -ConfirmDisposableWsl"));
    assert!(runner.contains("__application_logs__"));

    let manifest = fs::read_to_string(repository_root().join("src-tauri/Cargo.toml"))
        .expect("read Cargo manifest");
    assert!(manifest.contains("native-linux-acceptance = []"));
}

#[test]
fn package_keeps_issue_28_gate_and_adds_parallel_issue_35_commands() {
    let package: Value = serde_json::from_slice(
        &fs::read(repository_root().join("package.json")).expect("read package.json"),
    )
    .expect("parse package.json");

    assert_eq!(
        package["scripts"]["acceptance"],
        "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-acceptance-gate.ps1"
    );
    assert_eq!(
        package["scripts"]["acceptance:linux-wsl"],
        "pwsh -NoProfile -File scripts/run-linux-wsl-acceptance-gate.ps1 -Mode Full"
    );
    assert_eq!(
        package["scripts"]["acceptance:linux-wsl:automated"],
        "pwsh -NoProfile -File scripts/run-linux-wsl-acceptance-gate.ps1 -Mode Automated"
    );
    assert_eq!(
        package["scripts"]["acceptance:all"],
        "pwsh -NoProfile -File scripts/run-all-acceptance-gates.ps1"
    );

    let combined_runner =
        fs::read_to_string(repository_root().join("scripts/run-all-acceptance-gates.ps1"))
            .expect("read combined acceptance runner");
    assert!(combined_runner.contains("[string]$CodexPath"));
    assert!(combined_runner.contains("$linuxArguments += @('-CodexPath', $CodexPath)"));
}

#[cfg(windows)]
#[test]
fn linux_wsl_contract_gate_confirms_domain_adr_ui_and_prd_alignment() {
    let root = repository_root();
    let output = Command::new("pwsh.exe")
        .args(["-NoProfile", "-File", "scripts/test-linux-wsl-contract.ps1"])
        .arg("-RepositoryRoot")
        .arg(&root)
        .current_dir(&root)
        .output()
        .expect("run Linux/WSL2 contract gate");

    assert!(
        output.status.success(),
        "contract gate failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse contract report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["issue"], 35);
    assert_eq!(report["contradictions"], json!([]));
    assert_eq!(report["checkedDocuments"].as_array().map(Vec::len), Some(7));
}
