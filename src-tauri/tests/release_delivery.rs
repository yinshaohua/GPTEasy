use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .to_path_buf()
}

fn windows_release_contract() -> Value {
    serde_json::from_slice(
        &fs::read(repository_root().join("scripts/windows-release-contract.json"))
            .expect("read Windows release contract"),
    )
    .expect("parse Windows release contract")
}

fn required_uat_checks(contract: &Value) -> Vec<&str> {
    contract["requiredUatChecks"]
        .as_array()
        .expect("required UAT checks")
        .iter()
        .map(|check| check["id"].as_str().expect("check id"))
        .collect()
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
    let installer = temp.path().join("GPTEasy_1.0.0_x64-setup.exe");
    fs::copy(
        std::env::current_exe().expect("current test executable"),
        &installer,
    )
    .expect("copy unsigned PE fixture");
    let bytes = fs::read(&installer).expect("read fixture artifact");
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let contract = windows_release_contract();
    let checks = required_uat_checks(&contract)
        .into_iter()
        .map(|id| json!({ "id": id, "passed": true }))
        .collect::<Vec<_>>();
    let candidate_manifest = json!({
        "schemaVersion": 1,
        "issue": 28,
        "gitCommit": current_commit(&root),
        "platform": "windows-x64-current-user",
        "verification": {
            "frontendCheck": "passed",
            "frontendTests": "passed",
            "layoutTests": "passed",
            "rustTests": "passed",
            "acceptanceGate": "passed",
            "releaseTree": "passed",
            "releaseContract": "passed",
            "updateTrustRoot": "passed",
            "updaterSignature": "passed"
        },
        "artifact": {
            "path": "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/GPTEasy_1.0.0_x64-setup.exe",
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
        "issue": 28,
        "evidenceOrigin": "synthetic-test",
        "completedAtUtc": "2026-08-10T00:00:00Z",
        "gitCommit": current_commit(&root),
        "candidateManifestSha256": candidate_manifest_sha256,
        "platform": { "os": "windows", "architecture": "x64", "build": 19045 },
        "codexCliVersion": "codex-cli 0.147.0",
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

fn run_maintainer_release_gate(
    installer: &Path,
    candidate_manifest: &Path,
    confirm_acceptance: bool,
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
            "Release",
            "-InstallerPath",
        ])
        .arg(installer)
        .arg("-CandidateManifestPath")
        .arg(candidate_manifest)
        .arg("-RepositoryRoot")
        .arg(&root);
    if confirm_acceptance {
        command.arg("-ConfirmMaintainerAcceptance");
    }
    command
        .current_dir(&root)
        .output()
        .expect("run maintainer release gate")
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
    assert_eq!(
        report["legacyDomesticDistributionEntries"],
        serde_json::json!([])
    );
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
fn windows_uat_uses_the_current_user_shortcut_instead_of_the_shell_cache() {
    let root = repository_root();
    let script = fs::read_to_string(root.join("scripts/run-windows-uat.ps1"))
        .expect("read Windows UAT runner");

    assert!(script.contains(
        r"$startMenuShortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\GPTEasy.lnk'"
    ));
    assert_eq!(
        script
            .matches("Test-Path -LiteralPath $startMenuShortcut")
            .count(),
        3,
        "preflight, installation, and uninstallation must inspect the current-user shortcut"
    );
    assert!(
        !script.contains("Get-StartApps"),
        "Get-StartApps is backed by an asynchronous shell cache"
    );
}

#[test]
fn windows_uat_operator_prompts_are_in_simplified_chinese() {
    let script_path = repository_root().join("scripts/run-windows-uat.ps1");
    let script_bytes = fs::read(&script_path).expect("read Windows UAT runner bytes");
    assert!(
        script_bytes.starts_with(&[0xef, 0xbb, 0xbf]),
        "localized Windows PowerShell scripts require a UTF-8 BOM"
    );
    let script = fs::read_to_string(script_path).expect("read Windows UAT runner");

    assert!(
        script.contains("仅在实际观察到要求的行为后输入 PASS"),
        "UAT confirmation prompt must be in Simplified Chinese"
    );
    assert!(
        script.contains("确认已安装的 GPTEasy 设置窗口可见且可正常操作。"),
        "UAT operator steps must be in Simplified Chinese"
    );
    assert!(
        !script.contains("Type PASS only after observing the required behavior"),
        "legacy English confirmation prompt must not remain"
    );
}

#[test]
fn windows_uat_covers_trusted_desktop_control_without_script_process_control() {
    let script = fs::read_to_string(repository_root().join("scripts/run-windows-uat.ps1"))
        .expect("read Windows UAT script");

    assert!(
        !script.contains("Get-AppxPackage"),
        "desktop package discovery must not be a release prerequisite"
    );
    assert!(!script.contains("desktopCodexVersion"));
    assert!(!script.contains("desktopPublisherId"));
    assert!(script.contains("desktop_status_and_start"));
    assert!(script.contains("desktop_confirmed_tree_restart"));
    assert!(script.contains("desktop_cli_isolation"));
    assert!(!script.contains("Stop-Process"));
    assert!(!script.contains("taskkill"));

    let readiness =
        fs::read_to_string(repository_root().join("scripts/test-release-readiness.ps1"))
            .expect("read release readiness script");
    assert!(!readiness.contains("desktopCodexVersion"));
    assert!(!readiness.contains("desktopPublisherId"));
}

#[test]
fn windows_candidate_runs_layout_and_release_contract_gates() {
    let script = fs::read_to_string(repository_root().join("scripts/build-windows-candidate.ps1"))
        .expect("read Windows candidate builder");

    assert!(script.contains("npm run test:layout"));
    assert!(script.contains("layoutTests = 'passed'"));
    assert!(script.contains("cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=1"));
    assert!(script.contains("scripts/test-release-contract.ps1"));
    assert!(script.contains("releaseContract = 'passed'"));
}

#[test]
fn release_contract_gate_confirms_current_domain_and_ui_documents_agree() {
    let root = repository_root();
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "scripts/test-release-contract.ps1",
            "-RepositoryRoot",
        ])
        .arg(&root)
        .current_dir(&root)
        .output()
        .expect("run release contract gate");

    assert!(
        output.status.success(),
        "release contract gate failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse contract report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["contradictions"], json!([]));
}

#[test]
fn windows_release_contract_is_the_unique_issue_28_uat_schema() {
    let contract = windows_release_contract();
    assert_eq!(contract["schemaVersion"], 1);
    assert_eq!(contract["issue"], 28);
    assert_eq!(
        contract["desktopConsumerControl"],
        "trusted_start_confirmed_tree_restart"
    );

    let checks = contract["requiredUatChecks"]
        .as_array()
        .expect("required UAT checks");
    let ids = required_uat_checks(&contract);
    let unique = ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), checks.len(), "UAT check ids must be unique");
    let criteria = checks
        .iter()
        .filter_map(|check| check["criterion"].as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        criteria,
        std::collections::HashSet::from([
            "executablePathIsolation",
            "singleInstanceWakeAndTray",
            "sharedSwitchConfirmation",
            "switchSuccessUpdatesCurrentProvider",
            "switchFailureRefreshesEnvironment",
            "passivePendingRestart",
            "pendingRestartAutoClear",
            "trustedDesktopStart",
            "confirmedDesktopTreeRestart",
            "cliLifecycleIsolation",
            "defaultLayout",
            "minimumLayout",
            "explicitExitSamePathCleanup",
        ])
    );
}

#[test]
fn windows_release_contract_covers_issue_39_session_management() {
    let root = repository_root();
    let contract = windows_release_contract();
    let ids = required_uat_checks(&contract)
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    for required in [
        "session_app_server_contract",
        "session_mutation_safety",
        "session_protocol_degradation",
        "session_process_lifecycle",
        "session_process_recovery",
    ] {
        assert!(ids.contains(required), "release contract misses {required}");
    }

    let session = fs::read_to_string(root.join("src-tauri/src/session.rs"))
        .expect("read session implementation");
    assert!(session.contains("CREATE_NO_WINDOW"));
    assert!(session.contains("JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE"));
    assert!(session.contains("kill_on_drop(true)"));
    assert!(session.contains("is_interactive_thread"));
}

#[test]
fn release_contract_gate_rejects_unconfirmed_desktop_termination() {
    let root = repository_root();
    let temp = TempDir::new().expect("contract fixture");
    let contract = windows_release_contract();

    fs::create_dir_all(temp.path().join("scripts")).expect("fixture scripts");
    fs::copy(
        root.join("scripts/windows-release-contract.json"),
        temp.path().join("scripts/windows-release-contract.json"),
    )
    .expect("copy release contract");
    for document in contract["documents"]
        .as_array()
        .expect("contract documents")
    {
        let relative = document["path"].as_str().expect("document path");
        let target = temp.path().join(relative);
        fs::create_dir_all(target.parent().expect("document parent"))
            .expect("create document parent");
        fs::copy(root.join(relative), &target).expect("copy contract document");
    }
    for decision in contract["supersededDecisions"]
        .as_array()
        .expect("superseded decisions")
    {
        let relative = decision["path"].as_str().expect("decision path");
        let target = temp.path().join(relative);
        fs::create_dir_all(target.parent().expect("decision parent"))
            .expect("create decision parent");
        fs::copy(root.join(relative), target).expect("copy superseded decision");
    }
    let tauri_target = temp.path().join("src-tauri/tauri.conf.json");
    fs::create_dir_all(tauri_target.parent().expect("Tauri parent")).expect("create Tauri parent");
    fs::copy(root.join("src-tauri/tauri.conf.json"), &tauri_target).expect("copy Tauri config");

    let ui_contract = temp.path().join("docs/ui/UI-SPEC.md");
    let mut contents = fs::read_to_string(&ui_contract).expect("read UI contract fixture");
    contents.push_str("\nGPTEasy 无需确认即可结束 ChatGPT/Codex 桌面版。\n");
    fs::write(ui_contract, contents).expect("add contradictory statement");

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts/test-release-contract.ps1"))
        .arg("-RepositoryRoot")
        .arg(temp.path())
        .output()
        .expect("run release contract fixture");

    assert!(
        !output.status.success(),
        "contradictory contract unexpectedly passed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse failure report");
    assert_eq!(report["passed"], false);
    assert!(
        report["contradictions"]
            .as_array()
            .expect("contradictions")
            .iter()
            .any(|value| value
                .as_str()
                .is_some_and(|message| message.contains("UI-SPEC.md")))
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
fn formal_release_readiness_allows_unsigned_installer() {
    let (_temp, _evidence, installer, manifest) = unsigned_uat_fixture();
    let output = run_maintainer_release_gate(&installer, &manifest, true);
    assert!(
        output.status.success(),
        "maintainer release gate failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse readiness report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["authenticodeStatus"], "NotSigned");
    assert_eq!(report["acceptance"], "maintainer-confirmed");
    assert_eq!(report["uatEvidenceChecked"], false);
}

#[test]
fn formal_release_readiness_requires_explicit_maintainer_acceptance() {
    let (_temp, _evidence, installer, manifest) = unsigned_uat_fixture();
    let output = run_maintainer_release_gate(&installer, &manifest, false);
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("parse readiness failure");
    assert!(
        report["errors"]
            .as_array()
            .expect("errors array")
            .iter()
            .any(|error| error
                .as_str()
                .is_some_and(|value| value.contains("maintainer acceptance")))
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
