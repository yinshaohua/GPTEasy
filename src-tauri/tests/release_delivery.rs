use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const REQUIRED_UAT_CHECKS: &[&str] = &[
    "release_tree",
    "install_current_user",
    "application_launch",
    "real_provider_validation",
    "provider_save_and_switch",
    "pending_restart",
    "cli_new_process_read",
    "desktop_new_process_read",
    "restore_last_config",
    "external_config_takeover",
    "managed_conflict",
    "openai_login_mode",
    "provider_combination_applied",
    "provider_combination_match",
    "tray_residency",
    "overwrite_install",
    "overwrite_launch",
    "uninstall",
    "data_retention",
    "credential_leak_scan",
];

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .to_path_buf()
}

fn current_commit(root: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .expect("read current commit");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("commit is UTF-8")
        .trim()
        .to_owned()
}

fn unsigned_uat_fixture() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let root = repository_root();
    let temp = TempDir::new().expect("release fixture");
    let installer = temp.path().join("GPTEasy_0.1.0_x64-setup.exe");
    fs::copy(
        std::env::current_exe().expect("current test executable"),
        &installer,
    )
    .expect("copy unsigned PE fixture");
    let bytes = fs::read(&installer).expect("read fixture artifact");
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let checks = REQUIRED_UAT_CHECKS
        .iter()
        .map(|id| json!({ "id": id, "passed": true }))
        .collect::<Vec<_>>();
    let candidate_manifest = json!({
        "schemaVersion": 1,
        "issue": 11,
        "gitCommit": current_commit(&root),
        "platform": "windows-x64-current-user",
        "verification": {
            "frontendCheck": "passed",
            "frontendTests": "passed",
            "rustTests": "passed",
            "acceptanceGate": "passed",
            "releaseTree": "passed"
        },
        "artifact": {
            "path": "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/GPTEasy_0.1.0_x64-setup.exe",
            "sha256": sha256,
            "size": bytes.len(),
            "authenticodeStatus": "NotSigned"
        }
    });
    let candidate_manifest_path = temp.path().join("manifest.json");
    let candidate_manifest_bytes =
        serde_json::to_vec_pretty(&candidate_manifest).expect("serialize candidate manifest");
    fs::write(&candidate_manifest_path, &candidate_manifest_bytes)
        .expect("write candidate manifest");
    let candidate_manifest_sha256 = format!("{:x}", Sha256::digest(&candidate_manifest_bytes));
    let evidence = json!({
        "schemaVersion": 1,
        "issue": 11,
        "evidenceOrigin": "synthetic-test",
        "completedAtUtc": "2026-08-10T00:00:00Z",
        "gitCommit": current_commit(&root),
        "candidateManifestSha256": candidate_manifest_sha256,
        "platform": { "os": "windows", "architecture": "x64", "build": 19045 },
        "codexCliVersion": "codex-cli 0.147.0",
        "desktopCodexVersion": "26.803.5235.0",
        "providerCombinationFingerprint": "a".repeat(64),
        "artifact": {
            "fileName": installer.file_name().expect("file name").to_string_lossy(),
            "sha256": sha256,
            "size": bytes.len(),
            "authenticodeStatus": "NotSigned"
        },
        "checks": checks
    });
    let evidence_path = temp.path().join("evidence.json");
    fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).expect("serialize evidence"),
    )
    .expect("write evidence");
    (temp, evidence_path, installer, candidate_manifest_path)
}

fn run_readiness_gate(
    mode: &str,
    evidence: &Path,
    installer: &Path,
    candidate_manifest: &Path,
) -> std::process::Output {
    let root = repository_root();
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "scripts/test-release-readiness.ps1",
            "-Mode",
            mode,
            "-EvidencePath",
        ])
        .arg(evidence)
        .arg("-InstallerPath")
        .arg(installer)
        .arg("-CandidateManifestPath")
        .arg(candidate_manifest)
        .arg("-RepositoryRoot")
        .arg(&root)
        .current_dir(&root);
    command.output().expect("run release readiness gate")
}

#[test]
fn release_tree_gate_accepts_the_active_repository() {
    let root = repository_root();
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "scripts/test-release-tree.ps1",
            "-RepositoryRoot",
        ])
        .arg(&root)
        .current_dir(&root)
        .output()
        .expect("run release tree gate");

    assert!(
        output.status.success(),
        "release tree gate failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse release tree report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["legacySourceEntries"], serde_json::json!([]));
    assert_eq!(report["activeRoadmapEntries"], serde_json::json!([]));
    assert_eq!(report["archiveProgressReferences"], serde_json::json!([]));
}

#[test]
fn windows_uat_refuses_to_run_without_disposable_environment_confirmation() {
    let root = repository_root();
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "scripts/run-windows-uat.ps1",
        ])
        .current_dir(&root)
        .output()
        .expect("run Windows UAT gate");

    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("ConfirmDisposableEnvironment"),
        "UAT gate did not explain its safety refusal: {combined}"
    );
}

#[test]
fn acceptance_readiness_rejects_synthetic_evidence_without_rejecting_unsigned_artifact() {
    let (_temp, evidence, installer, manifest) = unsigned_uat_fixture();
    let output = run_readiness_gate("Acceptance", &evidence, &installer, &manifest);
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse readiness failure");
    let errors = report["errors"].as_array().expect("errors array");
    assert!(errors.iter().any(|error| {
        error
            .as_str()
            .is_some_and(|value| value.contains("Synthetic"))
    }));
    assert!(!errors.iter().any(|error| {
        error
            .as_str()
            .is_some_and(|value| value.contains("Authenticode"))
    }));
    assert_eq!(report["passed"], false);
    assert_eq!(report["mode"], "Acceptance");
    assert_eq!(report["authenticodeStatus"], "NotSigned");
}

#[test]
fn formal_release_readiness_rejects_unsigned_installer() {
    let (_temp, evidence, installer, manifest) = unsigned_uat_fixture();
    let output = run_readiness_gate("Release", &evidence, &installer, &manifest);
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse readiness failure");
    assert_eq!(report["passed"], false);
    assert!(
        report["errors"]
            .as_array()
            .expect("errors array")
            .iter()
            .any(|error| error
                .as_str()
                .is_some_and(|value| value.contains("Authenticode")))
    );
}

#[test]
fn release_readiness_rejects_a_changed_candidate_manifest() {
    let (_temp, evidence, installer, manifest) = unsigned_uat_fixture();
    let mut changed: Value = serde_json::from_slice(&fs::read(&manifest).expect("read manifest"))
        .expect("parse manifest");
    changed["artifact"]["sha256"] = json!("b".repeat(64));
    fs::write(
        &manifest,
        serde_json::to_vec_pretty(&changed).expect("serialize changed manifest"),
    )
    .expect("change manifest");

    let output = run_readiness_gate("Acceptance", &evidence, &installer, &manifest);
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse readiness failure");
    assert!(
        report["errors"]
            .as_array()
            .expect("errors array")
            .iter()
            .any(|error| error
                .as_str()
                .is_some_and(|value| value.contains("candidate manifest")))
    );
}
