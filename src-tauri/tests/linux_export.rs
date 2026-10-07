use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use gpteasy_lib::provider::{
    LinuxExportFailureCategory, LinuxShell, ProviderApplication, ProviderValidator,
    ValidationTimeouts,
};
use gpteasy_lib::state::{StatePaths, StateStore};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn bash_export_captures_every_verified_provider_and_reasoning_rules() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://api.deepseek.com/v1",
        "alpha-secret-key",
        "DeepSeek-R1",
        1,
    );
    fixture.insert_provider(
        "22222222-2222-4222-8222-222222222222",
        "Beta Provider",
        "https://beta.example/v1",
        "beta-secret-key",
        "Qwen3-Coder",
        2,
    );
    let destination = fixture.temp.path().join("gpteasy.sh");

    let exported = fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export verified provider snapshot");

    assert_eq!(exported.provider_count, 2);
    assert_eq!(exported.suggested_file_name, "gpteasy.sh");
    assert_eq!(exported.export_id.len(), 36);
    let script = fs::read_to_string(destination).expect("read exported Bash script");
    assert!(script.starts_with("#!/usr/bin/env bash\n"));
    assert!(script.contains("Alpha Provider"));
    assert!(script.contains("Beta Provider"));
    assert!(script.contains("alpha-secret-key"));
    assert!(script.contains("beta-secret-key"));
    assert!(script.contains("gpteasy__provider_reasoning_effort"));
    assert!(!script.contains("model_reasoning_effort = \"medium\""));
    assert!(script.find("Alpha Provider") < script.find("Beta Provider"));
    assert!(
        script.contains(
            "11111111-1111-4111-8111-111111111111\tAlpha Provider\thttps://api.deepseek.com/v1\tDeepSeek-R1\talpha-secret-key\thigh"
        ),
        "each provider must be editable as one top-of-script catalog record"
    );
    assert!(
        script.find("# 供应商目录。可脱离 GPTEasy 手工维护") < script.find("gpteasy__start_marker"),
        "the editable provider catalog must precede the runtime implementation"
    );
    assert!(!script.contains("OpenAI 登录"));
    assert!(script.contains(
        "function codex-full() {\n    command codex --dangerously-bypass-approvals-and-sandbox \"$@\"\n}"
    ));
    assert!(
        script.contains("printf '  %s) %s (%s)%s\\n' \"$index\" \"$name\" \"$model\" \"$marker\"")
    );
    assert!(!script.contains("printf '  %s) %s (%s)%\""));
    assert!(!script.contains("provider_irovider_id"));
}

#[test]
fn zsh_export_captures_every_verified_provider_and_reasoning_rules() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://api.deepseek.com/v1",
        "alpha-secret-key",
        "DeepSeek-R1",
        1,
    );
    fixture.insert_provider(
        "22222222-2222-4222-8222-222222222222",
        "Beta Provider",
        "https://beta.example/v1",
        "beta-secret-key",
        "Qwen3-Coder",
        2,
    );
    let destination = fixture.temp.path().join("gpteasy.zsh");

    let exported = fixture
        .application
        .export_linux_script(LinuxShell::Zsh, &destination, false)
        .expect("export verified provider snapshot");

    assert_eq!(exported.provider_count, 2);
    assert_eq!(exported.suggested_file_name, "gpteasy.zsh");
    assert_eq!(exported.export_id.len(), 36);
    let script = fs::read_to_string(destination).expect("read exported Zsh script");
    assert!(script.starts_with("#!/usr/bin/env zsh\n"));
    assert!(script.contains("Alpha Provider"));
    assert!(script.contains("Beta Provider"));
    assert!(script.contains("alpha-secret-key"));
    assert!(script.contains("beta-secret-key"));
    assert!(script.contains("gpteasy__provider_reasoning_effort"));
    assert!(!script.contains("model_reasoning_effort = \"medium\""));
    assert!(script.find("Alpha Provider") < script.find("Beta Provider"));
    assert!(!script.contains("OpenAI 登录"));
    assert!(script.contains(
        "function codex-full() {\n    command codex --dangerously-bypass-approvals-and-sandbox \"$@\"\n}"
    ));
}

#[test]
fn bash_and_zsh_exports_embed_the_same_static_reasoning_snapshot() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "gpt-6.1-sol",
        1,
    );
    fixture.insert_model_catalog(
        "11111111-1111-4111-8111-111111111111",
        &["gpt-6.1-sol", "provider-only"],
        Some(
            r#"{"schemaVersion":1,"targetEnvironment":"native","codexVersion":"0.160.1","executableFingerprint":"sha256:test","observedAtEpochSeconds":1,"status":"complete","models":[{"modelId":"gpt-6.1-sol","state":"known_non_empty","defaultReasoningEffort":"medium","supportedReasoningEfforts":["low","medium","xhigh"],"source":"codex_builtin","sourceId":"codex.model/list"},{"modelId":"provider-only","state":"not_found","defaultReasoningEffort":null,"supportedReasoningEfforts":[],"source":"codex_builtin","sourceId":"codex.model/list"}]}"#,
        ),
    );

    let bash_path = fixture.temp.path().join("gpteasy.sh");
    let zsh_path = fixture.temp.path().join("gpteasy.zsh");
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &bash_path, false)
        .expect("export Bash snapshot");
    fixture
        .application
        .export_linux_script(LinuxShell::Zsh, &zsh_path, false)
        .expect("export Zsh snapshot");

    let bash = fs::read_to_string(bash_path).expect("read Bash snapshot");
    let zsh = fs::read_to_string(zsh_path).expect("read Zsh snapshot");
    for script in [&bash, &zsh] {
        assert!(script.contains("GPTEASY_TEST_DAEMON_REFRESH_DEADLINE_SECONDS:-75"));
        assert!(script.contains("gpteasy__model_catalog_snapshot_schema='1'"));
        assert!(script.contains("\"slug\": \"gpt-6.1-sol\""));
        assert!(script.contains("\"default_reasoning_level\": \"medium\""));
        assert!(script.contains("\"effort\": \"xhigh\""));
        assert!(script.contains("\"slug\": \"provider-only\""));
        assert!(script.contains("供应商已发现模型，Codex 未识别精确模型能力"));
        assert!(!script.contains("app-server --stdio"));
        assert!(!script.contains("model-list"));
    }
    let bash_catalog = bash
        .split("gpteasy__provider_model_catalog() {")
        .nth(1)
        .expect("Bash catalog function")
        .split("gpteasy__provider_count=")
        .next()
        .expect("Bash catalog body");
    let zsh_catalog = zsh
        .split("gpteasy__provider_model_catalog() {")
        .nth(1)
        .expect("Zsh catalog function")
        .split("gpteasy__provider_count=")
        .next()
        .expect("Zsh catalog body");
    assert_eq!(bash_catalog, zsh_catalog);
}

#[test]
fn linux_export_downgrades_a_corrupt_snapshot_without_guessing_by_name() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "DeepSeek-R1",
        1,
    );
    fixture.insert_model_catalog(
        "11111111-1111-4111-8111-111111111111",
        &["DeepSeek-R1"],
        Some("{not-json"),
    );
    let destination = fixture.temp.path().join("gpteasy.sh");
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export unknown snapshot");
    let script = fs::read_to_string(destination).expect("read exported script");
    assert!(script.contains("\"description\": \"供应商已发现模型，能力未识别\""));
    assert!(!script.contains("\"default_reasoning_level\": \"high\""));
    assert!(!script.contains("\"effort\": \"low\""));
}

#[test]
fn bash_export_requires_a_verified_provider_without_creating_a_file() {
    let fixture = ExportFixture::new();
    let destination = fixture.temp.path().join("gpteasy.sh");

    let failure = fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect_err("empty catalog must fail closed");

    assert_eq!(
        failure.category,
        LinuxExportFailureCategory::NoVerifiedProviders
    );
    assert!(!destination.exists());
}

#[test]
fn bash_export_does_not_replace_an_existing_file_without_confirmation() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://api.deepseek.com/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    let destination = fixture.temp.path().join("gpteasy.sh");
    fs::write(&destination, b"user-owned original\n").expect("seed existing file");

    let failure = fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect_err("overwrite requires confirmation");

    assert_eq!(
        failure.category,
        LinuxExportFailureCategory::OverwriteConfirmationRequired
    );
    assert_eq!(
        fs::read(destination).expect("read original"),
        b"user-owned original\n"
    );
}

#[test]
fn dayway_ds_linux_export_applies_exact_deepseek_compatibility_profile() {
    let fixture = ExportFixture::new();
    let ds_id = "11111111-1111-4111-8111-111111111111";
    let dayway_id = "22222222-2222-4222-8222-222222222222";
    let models = [
        "deepseek-v4-pro",
        "deepseek-v4-flash",
        "deepseek-v4-pro-preview",
        "gpt-6.1-sol",
    ];
    let snapshot = serde_json::json!({
        "schemaVersion": 1, "targetEnvironment": "native", "codexVersion": "0.160.1",
        "executableFingerprint": "sha256:test", "observedAtEpochSeconds": 1, "status": "complete",
        "models": models.iter().map(|model| serde_json::json!({
            "modelId": model, "state": if *model == "gpt-6.1-sol" { "known_non_empty" } else { "not_found" },
            "defaultReasoningEffort": if *model == "gpt-6.1-sol" { Some("medium") } else { None },
            "supportedReasoningEfforts": if *model == "gpt-6.1-sol" { vec!["low", "medium", "high", "xhigh"] } else { vec![] },
            "source": "codex_builtin", "sourceId": "codex.model/list"
        })).collect::<Vec<_>>()
    }).to_string();
    for (id, name, order) in [(ds_id, "DayWay-DS", 1), (dayway_id, "DayWay", 2)] {
        fixture.insert_provider(
            id,
            name,
            "https://dayway.example/v1",
            "dayway-ds-secret-key",
            "deepseek-v4-pro",
            order,
        );
        fixture.insert_model_catalog(id, &models, Some(&snapshot));
    }
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "dayway-ds.sh",
            LinuxShell::Zsh => "dayway-ds.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export");
        let script = fs::read_to_string(&destination).expect("script");
        let ds = exported_model_catalog(&script, ds_id);
        let dayway = exported_model_catalog(&script, dayway_id);
        for model in &ds["models"].as_array().expect("models")[..] {
            let slug = model["slug"].as_str().expect("slug");
            let efforts = model["supported_reasoning_levels"]
                .as_array()
                .expect("levels")
                .iter()
                .map(|level| level["effort"].as_str().expect("effort"))
                .collect::<Vec<_>>();
            match slug {
                "deepseek-v4-pro" | "deepseek-v4-flash" => {
                    assert_eq!(model["default_reasoning_level"], "high");
                    assert_eq!(efforts, ["low", "medium", "high"]);
                }
                "gpt-6.1-sol" => assert_eq!(efforts, ["low", "medium", "high", "xhigh"]),
                _ => assert!(efforts.is_empty()),
            }
        }
        for model in dayway["models"].as_array().expect("models") {
            if model["slug"] != "gpt-6.1-sol" {
                assert!(
                    model["supported_reasoning_levels"]
                        .as_array()
                        .expect("levels")
                        .is_empty()
                );
            }
        }
        run_shell_black_box_with_canaries(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-ds-catalog.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
mkdir -p "$workspace/bin" "$workspace/codex"
cp -- "$1" "$workspace/export"
chmod 600 "$workspace/export"
export CODEX_HOME="$workspace/codex"
export PATH="$workspace/bin:$PATH"
cat >"$workspace/bin/codex" <<'CODEX'
#!/usr/bin/env bash
if [[ "$*" == --version ]]; then echo 'codex-cli 0.160.1'; exit 0; fi
[[ "$*" == 'app-server daemon restart' ]]
CODEX
chmod 700 "$workspace/bin/codex"
source "$workspace/export"
[[ ! -e "$CODEX_HOME/gpteasy-model-catalog.json" ]]
output=$(gpteasy <<<"1" 2>&1)
[[ "$output" == *'已切换到：DayWay-DS'* ]]
[[ "$output" == *'stage=restart result=command_completed'* ]]
grep -Fq 'model_reasoning_effort = "high"' "$CODEX_HOME/config.toml"
grep -Fq 'model_catalog_json = "gpteasy-model-catalog.json"' "$CODEX_HOME/config.toml"
[[ ! -e "$CODEX_HOME/.gpteasy-shell/lock/active" ]]
cp -- "$CODEX_HOME/gpteasy-model-catalog.json" "$1.ds.json"


output=$(gpteasy <<<"2" 2>&1)
[[ "$output" == *'已切换到：DayWay'* ]]
cp -- "$CODEX_HOME/gpteasy-model-catalog.json" "$1.dayway.json"
"#,
            &["dayway-ds-secret-key", "https://dayway.example/v1"],
        );
        // Local optional shell absence may skip the harness.
        if Path::new(&format!("{}.ds.json", destination.display())).exists() {
            for (suffix, expected) in [("ds", ds), ("dayway", dayway)] {
                let actual: serde_json::Value = serde_json::from_str(
                    &fs::read_to_string(format!("{}.{suffix}.json", destination.display()))
                        .expect("Linux catalog"),
                )
                .expect("Linux catalog JSON");
                assert_eq!(actual, expected);
            }
        }
    }
    let connection = Connection::open(fixture.store.paths().database()).expect("state");
    let stored: String = connection
        .query_row(
            "SELECT capability_snapshot_json FROM provider_model_catalog WHERE provider_id = ?1",
            [ds_id],
            |row| row.get(0),
        )
        .expect("stored snapshot");
    assert_eq!(
        stored, snapshot,
        "export must preserve the stored probe evidence"
    );
}

#[test]
fn dayway_ds_export_preserves_known_capabilities_and_unknown_evidence() {
    for case in [
        "known_empty",
        "known_non_empty",
        "conflict",
        "probe_failed",
        "missing",
        "corrupt",
        "partial",
        "fingerprint",
    ] {
        let fixture = ExportFixture::new();
        let id = "11111111-1111-4111-8111-111111111111";
        let model = "deepseek-v4-pro";
        fixture.insert_provider(
            id,
            "DayWay-DS",
            "https://dayway.example/v1",
            "test-key",
            model,
            1,
        );
        let snapshot = serde_json::json!({
            "schemaVersion": 1, "targetEnvironment": "native", "codexVersion": "0.160.1",
            "executableFingerprint": "sha256:test", "observedAtEpochSeconds": 1, "status": "complete",
            "models": [{"modelId": if case == "partial" { "other" } else { model },
                "state": if ["known_empty", "known_non_empty", "conflict", "probe_failed"].contains(&case) { case } else { "not_found" },
                "defaultReasoningEffort": if case == "known_non_empty" { Some("xhigh") } else { None },
                "supportedReasoningEfforts": if case == "known_non_empty" { vec!["xhigh"] } else { vec![] },
                "source": "codex_builtin", "sourceId": "codex.model/list"}]
        }).to_string();
        fixture.insert_model_catalog(
            id,
            &[model],
            match case {
                "missing" => None,
                "corrupt" => Some("not-json"),
                _ => Some(&snapshot),
            },
        );
        if case == "fingerprint" {
            Connection::open(fixture.store.paths().database())
                .expect("state")
                .execute(
                    "UPDATE provider_model_catalog SET verification_fingerprint = 'mismatch'",
                    [],
                )
                .expect("invalidate fingerprint");
        }
        for shell in shell_matrix_targets() {
            let destination = fixture.temp.path().join(match shell {
                LinuxShell::Bash => "boundary.sh",
                LinuxShell::Zsh => "boundary.zsh",
            });
            fixture
                .application
                .export_linux_script(shell, &destination, false)
                .expect("export");
            let script = fs::read_to_string(destination).expect("script");
            let catalog = exported_model_catalog(&script, id);
            let entry = &catalog["models"][0];
            let levels = entry["supported_reasoning_levels"]
                .as_array()
                .expect("levels");
            if case == "known_non_empty" {
                assert_eq!(levels.len(), 1, "{case}");
                assert_eq!(levels[0]["effort"], "xhigh", "{case}");
                assert_eq!(entry["default_reasoning_level"], "xhigh", "{case}");
            } else {
                assert!(levels.is_empty(), "{case}");
                assert!(entry["default_reasoning_level"].is_null(), "{case}");
            }
        }
    }
}

fn exported_model_catalog(script: &str, provider_id: &str) -> serde_json::Value {
    let marker = format!("GPTEASY_MODEL_CATALOG_{provider_id}");
    let open = format!("cat <<'{marker}'\n");
    let close = format!("\n{marker}\n");
    let json = script
        .split_once(&open)
        .expect("catalog start")
        .1
        .split_once(&close)
        .expect("catalog end")
        .0;
    serde_json::from_str(json).expect("exported model catalog JSON")
}

#[test]
fn bash_export_replaces_an_existing_file_after_native_confirmation() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    let destination = fixture.temp.path().join("gpteasy.sh");
    fs::write(&destination, b"user-owned original\n").expect("seed existing file");

    let result = fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, true)
        .expect("native confirmation permits overwrite");

    assert_eq!(result.provider_count, 1);
    let exported = fs::read_to_string(destination).expect("read exported script");
    assert!(exported.starts_with("#!/usr/bin/env bash\n"));
    assert!(!exported.contains("user-owned original"));
}

#[test]
fn shell_snapshots_source_without_side_effects_and_accept_common_permissions() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");

        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-bash-source.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy.sh"
codex_home="$workspace/codex home"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p -- "$codex_home"
printf '%s\n' 'custom_setting = true' >"$codex_home/config.toml"
printf '%s' '{"login":"unchanged"}' >"$codex_home/auth.json"
before=$(sha256sum "$codex_home/config.toml")
auth_before=$(sha256sum "$codex_home/auth.json")
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
pushd "$workspace" >/dev/null
source ./gpteasy.sh
popd >/dev/null
after=$(sha256sum "$codex_home/config.toml")
[[ "$before" == "$after" ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
[[ $(gpteasy help) == *'gpteasy current'* ]]
[[ $(gpteasy --help) == "$(gpteasy -h)" ]]
current=$(gpteasy current 2>&1 || true)
[[ "$current" == *'当前配置不包含可识别的 GPTEasy 管理区块'* ]]
chmod 664 "$script"
common_mode=$(gpteasy current 2>&1 || true)
[[ "$common_mode" == *'当前配置不包含可识别的 GPTEasy 管理区块'* ]]
[[ "$common_mode" != *'导出文件必须'* ]]
chmod 775 "$script"
executable_mode=$(gpteasy current 2>&1 || true)
[[ "$executable_mode" == *'当前配置不包含可识别的 GPTEasy 管理区块'* ]]
[[ "$executable_mode" != *'导出文件必须'* ]]

if [[ "$(id -u)" == 0 ]]; then
    chown 65534:65534 "$script"
    non_owner=$(gpteasy current 2>&1 || true)
    [[ "$non_owner" == *'当前配置不包含可识别的 GPTEasy 管理区块'* ]]
    [[ "$non_owner" != *'导出文件必须'* ]]
fi

hardlink="$workspace/gpteasy-hardlink.sh"
ln -- "$script" "$hardlink"
# shellcheck disable=SC1090
source "$hardlink"
hardlink_result=$(gpteasy current 2>&1 || true)
[[ "$hardlink_result" == *'单链接普通文件'* ]]
rm -f -- "$hardlink"
# shellcheck disable=SC1090
source "$script"

symlink="$workspace/gpteasy-symlink.sh"
ln -s -- "$script" "$symlink"
# shellcheck disable=SC1090
source "$symlink"
symlink_result=$(gpteasy current 2>&1 || true)
[[ "$symlink_result" == *'导出文件不能是符号链接'* ]]
rm -f -- "$symlink"
# shellcheck disable=SC1090
source "$script"
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
gpteasy help >/dev/null
"#,
        );
    }
}

#[test]
fn shell_snapshots_preconfigure_without_codex_and_reject_incompatible_versions() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    fixture.insert_provider(
        "22222222-2222-4222-8222-222222222222",
        "Beta Provider",
        "https://beta.example/v1",
        "beta-secret-key",
        "beta-model",
        2,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");

        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-bash-switch.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy.sh"
codex_home="$workspace/codex home"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
# The catalog is the sole maintenance point: this provider did not exist when GPTEasy exported the script.
sed -i '/^GPTEASY_PROVIDER_CATALOG$/i 33333333-3333-4333-8333-333333333333\tManual Provider\thttps://manual.example/v1\tmanual-model\tmanual-secret-key' "$script"
mkdir -p -- "$codex_home" "$fake_bin"
printf '%s\n' 'custom_setting = true' >"$codex_home/config.toml"
printf '%s\n' '{"tokens":{"access_token":"keep-me"}}' >"$codex_home/auth.json"
config_before=$(sha256sum "$codex_home/config.toml")
auth_before=$(sha256sum "$codex_home/auth.json")
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"
fresh_home="$workspace/fresh codex home"
export CODEX_HOME="$fresh_home"
fresh=$(PATH="$fake_bin:/usr/bin:/bin" gpteasy <<<"3" 2>&1)
[[ "$fresh" == *'已预先配置：Manual Provider'* ]]
[[ -f "$fresh_home/config.toml" ]]
[[ $(stat -c '%a' "$fresh_home") == '700' ]]
[[ $(find "$fresh_home/.gpteasy-shell/credentials" -type f -name '*.token' | wc -l) -eq 1 ]]
grep -Fq '# GPTEasy provider-id: 33333333-3333-4333-8333-333333333333' "$fresh_home/config.toml"
manual_credential=$(find "$fresh_home/.gpteasy-shell/credentials" -type f -name '*.token' -print -quit)
[[ $(cat "$manual_credential") == 'manual-secret-key' ]]
export CODEX_HOME="$codex_home"
missing=$(PATH="$fake_bin:/usr/bin:/bin" gpteasy <<<"1" 2>&1 || true)
[[ "$missing" == *'已预先配置：Alpha Provider'* ]]
[[ "$missing" == *'当前未安装 Codex CLI'* ]]
[[ "$missing" != *'版本过低'* ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
grep -Fq '# GPTEasy schema-version: 1' "$codex_home/config.toml"
missing_config=$(sha256sum "$codex_home/config.toml")
[[ -e "$codex_home/.gpteasy-shell" ]]
cat >"$fake_bin/codex" <<'OLD_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.146.0'
OLD_CODEX
chmod 700 "$fake_bin/codex"
if (gpteasy <<<"1" >/dev/null 2>&1); then
    printf '%s\n' 'unsupported Codex version was accepted' >&2
    exit 1
fi
too_old=$(gpteasy <<<"1" 2>&1 || true)
[[ "$too_old" == *'Codex CLI 版本过低，请升级到 0.147.0 或更高版本'* ]]
[[ "$too_old" != *'未找到 Codex CLI'* ]]
[[ "$missing_config" == "$(sha256sum "$codex_home/config.toml")" ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]

cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env bash
if [[ "$*" == 'app-server daemon restart' ]]; then
    printf '%s\n' "$*" >>"$GPTEASY_DAEMON_CALLS"
    exit "${GPTEASY_DAEMON_RESTART_STATUS:-0}"
fi
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
chmod 700 "$fake_bin/codex"
export GPTEASY_DAEMON_CALLS="$workspace/daemon-calls"
export GPTEASY_DAEMON_RESTART_STATUS=17
menu=$(gpteasy <<<"1" 2>&1)
[[ "$menu" == *'Alpha Provider (alpha-model)'* ]]
[[ "$menu" == *'已切换到：Alpha Provider'* ]]
[[ "$menu" == *'stage=restart result=command_failed exit_code=17'* ]]
[[ "$(cat "$GPTEASY_DAEMON_CALLS")" == 'app-server daemon restart' ]]
grep -Fq '# GPTEasy schema-version: 1' "$codex_home/config.toml"
grep -Fq '# GPTEasy provider-id: 11111111-1111-4111-8111-111111111111' "$codex_home/config.toml"
grep -Fq '# GPTEasy source-id:' "$codex_home/config.toml"
grep -Fq 'model_providers.gpteasy.auth.command = "sh"' "$codex_home/config.toml"
! grep -Fq 'requires_openai_auth' "$codex_home/config.toml"
! grep -Fq 'custom_setting = true' "$codex_home/config.toml"
! grep -Fq 'alpha-secret-key' "$codex_home/config.toml"
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
credential_count=$(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token' | wc -l)
[[ "$credential_count" -eq 1 ]]
credential=$(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token' -print -quit)
[[ $(cat "$credential") == 'alpha-secret-key' ]]
[[ $(stat -c '%a' "$credential") == '600' ]]
[[ $(find "$codex_home/.gpteasy-shell/shell-restore" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq 2 ]]
[[ $(gpteasy current) == *'Alpha Provider'* ]]
[[ $(gpteasy <<<"q") == *'Alpha Provider (alpha-model) [当前]'* ]]
restore_without_codex=$(PATH=/usr/bin:/bin gpteasy restore <<<"n" 2>&1 || true)
[[ "$restore_without_codex" == *'当前状态：Alpha Provider'* ]]
[[ "$restore_without_codex" != *'未找到 Codex CLI'* ]]
unlock_without_codex=$(PATH=/usr/bin:/bin gpteasy unlock 2>&1 || true)
[[ "$unlock_without_codex" == *'当前没有 shell owner 锁'* ]]
[[ "$unlock_without_codex" != *'未找到 Codex CLI'* ]]
chmod 755 "$codex_home"
chmod 644 "$codex_home/config.toml"
broad_permissions=$(gpteasy <<<"2" 2>&1)
[[ "$broad_permissions" == *'已切换到：Beta Provider'* ]]
[[ "$broad_permissions" == *'权限允许其他用户访问'* ]]
if [[ "$(id -u)" != 0 ]]; then
    export CODEX_HOME=/usr
    foreign_owner=$(gpteasy <<<"1" 2>&1 || true)
    [[ "$foreign_owner" == *'目标环境身份不匹配'* ]]
    [[ "$foreign_owner" == *'/usr'* ]]
    [[ "$foreign_owner" == *'sudo -u <用户> -H'* ]]
    export CODEX_HOME="$codex_home"
fi
"#,
        );
    }
}

#[test]
fn shell_snapshots_restore_and_consume_only_the_latest_of_five_restore_points() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    fixture.insert_provider(
        "22222222-2222-4222-8222-222222222222",
        "Beta Provider",
        "https://beta.example/v1",
        "beta-secret-key",
        "beta-model",
        2,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");

        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-bash-restore.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy.sh"
codex_home="$workspace/codex"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p -- "$codex_home" "$fake_bin"
printf '%s\n' 'custom_setting = true' >"$codex_home/config.toml"
printf '%s\n' '{"tokens":{"access_token":"keep-me"}}' >"$codex_home/auth.json"
auth_before=$(sha256sum "$codex_home/auth.json")
cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
chmod 700 "$fake_bin/codex"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"
gpteasy <<<"1" >/dev/null
cp -- "$codex_home/config.toml" "$workspace/alpha-config"
gpteasy <<<"2" >/dev/null
restore_root="$codex_home/.gpteasy-shell/shell-restore"
[[ $(find "$restore_root" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq 2 ]]
credentials_root="$codex_home/.gpteasy-shell/credentials"
[[ $(find "$credentials_root" -type f -name '*.token' | wc -l) -eq 2 ]]
desktop_backups="$codex_home/.gpteasy-shell/desktop-backups"
orphan_relative='.gpteasy-shell/credentials/desktop-old/33333333-3333-4333-8333-333333333333.token'
mkdir -m 700 -- "$desktop_backups" "$credentials_root/desktop-old"
printf '%s' 'desktop-backup-secret' >"$codex_home/$orphan_relative"
chmod 600 "$codex_home/$orphan_relative"
printf '# GPTEasy credential-file: %s\n' "$orphan_relative" >"$desktop_backups/config-desktop.toml"
chmod 600 "$desktop_backups/config-desktop.toml"
beta_before=$(sha256sum "$codex_home/config.toml")
cancelled=$(gpteasy restore <<<"n")
[[ "$cancelled" == *'当前状态：Beta Provider'* ]]
[[ "$cancelled" == *'恢复目标：Alpha Provider'* ]]
[[ "$cancelled" == *'可能覆盖桌面 GPTEasy 或其它脚本之后完成的修改'* ]]
[[ "$beta_before" == "$(sha256sum "$codex_home/config.toml")" ]]
[[ $(find "$restore_root" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq 2 ]]
restored=$(gpteasy restore <<<"y")
[[ "$restored" == *'已恢复最近一次 shell 切换前的配置'* ]]
cmp -s -- "$workspace/alpha-config" "$codex_home/config.toml"
[[ $(find "$restore_root" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq 1 ]]
[[ -f "$desktop_backups/config-desktop.toml" ]]
[[ -f "$codex_home/$orphan_relative" ]]
[[ $(find "$credentials_root" -type f -name '*.token' | wc -l) -eq 2 ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]

rm -f -- "$desktop_backups/config-desktop.toml"
gpteasy <<<"2" >/dev/null
[[ ! -e "$codex_home/$orphan_relative" ]]

for choice in 1 2 1 2 1 2; do
    gpteasy <<<"$choice" >/dev/null
done
[[ $(find "$restore_root" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq 5 ]]
! grep -R -Fq 'alpha-secret-key' "$restore_root"
! grep -R -Fq 'beta-secret-key' "$restore_root"
"#,
        );
    }
}

#[test]
fn shell_snapshots_distinguish_states_and_force_rebuild_config_conflicts() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    fixture.insert_provider(
        "22222222-2222-4222-8222-222222222222",
        "Beta Provider",
        "https://beta.example/v1",
        "beta-secret-key",
        "beta-model",
        2,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");

        run_shell_black_box(
            shell,
            &destination,
            r##"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-bash-state.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy.sh"
codex_home="$workspace/codex"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p -- "$codex_home" "$fake_bin"
printf '%s\n' 'custom_setting = true' >"$codex_home/config.toml"
printf '%s\n' '{"tokens":{"access_token":"keep-me"}}' >"$codex_home/auth.json"
auth_before=$(sha256sum "$codex_home/auth.json")
cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
chmod 700 "$fake_bin/codex"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"
gpteasy <<<"1" >/dev/null
cp -- "$codex_home/config.toml" "$workspace/current-config"
menu=$(gpteasy <<<"q")
[[ "$menu" == *'Alpha Provider (alpha-model) [当前]'* ]]

sed -i '/# GPTEasy source-id:/d' "$codex_home/config.toml"
gpteasy <<<"2" >/dev/null
grep -Fq '# GPTEasy provider-id: 22222222-2222-4222-8222-222222222222' "$codex_home/config.toml"

cp -- "$workspace/current-config" "$codex_home/config.toml"
sed -i 's|^model_providers.gpteasy.auth.args = .*|model_providers.gpteasy.auth.args = ["-c", "printf unsafe"]|' "$codex_home/config.toml"
gpteasy <<<"2" >/dev/null
grep -Fq '# GPTEasy provider-id: 22222222-2222-4222-8222-222222222222' "$codex_home/config.toml"

cp -- "$workspace/current-config" "$codex_home/config.toml"
credential=$(find "$codex_home/.gpteasy-shell/credentials" -type f -name '11111111-1111-4111-8111-111111111111.token' -print -quit)
printf '%s' 'changed-key' >"$credential"
chmod 600 "$credential"
menu=$(gpteasy <<<"q")
[[ "$menu" == *'Alpha Provider (alpha-model) [当前，有更新]'* ]]
printf '%s' 'alpha-secret-key' >"$credential"

awk '
    $0 == "# GPTEasy schema-version: 1" { next }
    index($0, "# GPTEasy source-id:") == 1 { next }
    index($0, "# GPTEasy credential-file:") == 1 { next }
    index($0, "model_providers.gpteasy.auth.") == 1 { next }
    { print }
' "$workspace/current-config" >"$codex_home/config.toml"
menu=$(gpteasy <<<"q")
[[ "$menu" == *'Alpha Provider (alpha-model) [当前，旧格式]'* ]]
gpteasy <<<"2" >/dev/null
grep -Fq '# GPTEasy provider-id: 22222222-2222-4222-8222-222222222222' "$codex_home/config.toml"
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]

cp -- "$workspace/current-config" "$codex_home/config.toml"
sed -i 's/11111111-1111-4111-8111-111111111111/33333333-3333-4333-8333-333333333333/g' "$codex_home/config.toml"
outside=$(gpteasy current)
[[ "$outside" == *'当前供应商不在此 Linux 供应商快照中：33333333-3333-4333-8333-333333333333'* ]]
menu=$(gpteasy <<<"q")
[[ "$menu" != *'[当前]'* ]]

cp -- "$workspace/current-config" "$codex_home/config.toml"
sed -i 's/# GPTEasy schema-version: 1/# GPTEasy schema-version: 2/' "$codex_home/config.toml"
gpteasy <<<"2" >/dev/null
grep -Fq '# GPTEasy schema-version: 1' "$codex_home/config.toml"
grep -Fq '# GPTEasy provider-id: 22222222-2222-4222-8222-222222222222' "$codex_home/config.toml"

printf '%s\n' '# >>> GPTEasy managed provider >>>' 'model = "broken"' >"$codex_home/config.toml"
gpteasy <<<"2" >/dev/null
grep -Fq '# GPTEasy provider-id: 22222222-2222-4222-8222-222222222222' "$codex_home/config.toml"
grep -Fq '# <<< GPTEasy managed provider <<<' "$codex_home/config.toml"

printf '%s\n' 'model = "external"' 'model_provider = "external"' >"$codex_home/config.toml"
gpteasy <<<"2" >/dev/null
grep -Fq 'model = "beta-model"' "$codex_home/config.toml"
grep -Fq 'model_provider = "gpteasy"' "$codex_home/config.toml"
! grep -Fq 'model = "external"' "$codex_home/config.toml"
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
"##,
        );
    }
}

#[test]
fn shell_snapshots_force_new_provider_config_over_existing_external_config() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://api.deepseek.com/v1",
        "alpha-secret-key",
        "DeepSeek-R1",
        1,
    );
    fixture.insert_provider(
        "22222222-2222-4222-8222-222222222222",
        "Qwen Provider",
        "https://qwen.example/v1",
        "qwen-secret-key",
        "Qwen3-Coder",
        2,
    );
    fixture.insert_provider(
        "33333333-3333-4333-8333-333333333333",
        "Custom Provider",
        "https://custom.example/v1",
        "custom-secret-key",
        "alpha-model",
        3,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");

        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-external-config.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy.sh"
codex_home="$workspace/codex"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p -- "$codex_home" "$fake_bin"
cat >"$codex_home/config.toml" <<'EXTERNAL_CONFIG'
model_provider = "custom"

model = "gpt-5.6-sol"
model_reasoning_effort = "high"

[features]
goals = true
js_repl = false

[tui]
status_line = ["current-dir", "model-with-reasoning"]

[model_providers.custom]
name = "Existing Provider"
wire_api = "responses"
base_url = "https://existing.example/v1"
env_key = "EXISTING_API_KEY"

[projects."/data/new-api"]
trust_level = "trusted"

[mcp_servers.openaiDeveloperDocs]
command = "/data/opt/mcp-remote/mcp-remote"
args = ["https://developers.openai.com/mcp", "--silent"]
EXTERNAL_CONFIG
cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
chmod 700 "$fake_bin/codex"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"

original=$(cat "$codex_home/config.toml")
switched=$(gpteasy <<<"1")
[[ "$switched" == *'已切换到：Alpha Provider'* ]]
grep -Fq '# GPTEasy provider-id: 11111111-1111-4111-8111-111111111111' "$codex_home/config.toml"
grep -Fq 'model = "DeepSeek-R1"' "$codex_home/config.toml"
grep -Fq 'model_provider = "gpteasy"' "$codex_home/config.toml"
! grep -Fq 'model = "gpt-5.6-sol"' "$codex_home/config.toml"
! grep -Fq 'model_provider = "custom"' "$codex_home/config.toml"
! grep -Fq '[model_providers.custom]' "$codex_home/config.toml"
grep -Fq 'model_reasoning_effort = "high"' "$codex_home/config.toml"
! grep -Fq '[projects."/data/new-api"]' "$codex_home/config.toml"
restore=$(find "$codex_home/.gpteasy-shell/shell-restore" -type f -name config.toml -print -quit)
[[ -n "$restore" ]]
[[ "$original" == "$(cat "$restore")" ]]

qwen=$(gpteasy <<<"2")
[[ "$qwen" == *'已切换到：Qwen Provider'* ]]
grep -Fq 'model = "Qwen3-Coder"' "$codex_home/config.toml"
! grep -Fq 'model_reasoning_effort' "$codex_home/config.toml"

custom=$(gpteasy <<<"3")
[[ "$custom" == *'已切换到：Custom Provider'* ]]
grep -Fq 'model = "alpha-model"' "$codex_home/config.toml"
! grep -Fq 'model_reasoning_effort' "$codex_home/config.toml"

deepseek=$(gpteasy <<<"1")
[[ "$deepseek" == *'已切换到：Alpha Provider'* ]]
grep -Fq 'model = "DeepSeek-R1"' "$codex_home/config.toml"
grep -Fq 'model_reasoning_effort = "high"' "$codex_home/config.toml"
cat >"$codex_home/config.toml" <<'OLD_GPTEASY_CONFIG'
model_provider = "gpteasy"
model = "old-model"
[model_providers.gpteasy]
base_url = "https://old.example/v1"
OLD_GPTEASY_CONFIG
old_gpteasy=$(gpteasy <<<"1")
[[ "$old_gpteasy" == *'已切换到：Alpha Provider'* ]]
grep -Fq 'model = "DeepSeek-R1"' "$codex_home/config.toml"
! grep -Fq 'model = "old-model"' "$codex_home/config.toml"
! grep -Fq 'https://old.example/v1' "$codex_home/config.toml"

cat >"$codex_home/config.toml" <<'MALFORMED_CONFIG'
# >>> GPTEasy managed provider >>>
# GPTEasy credential-file: ../../unsafe.token
model = "unterminated
MALFORMED_CONFIG
malformed=$(gpteasy <<<"1" 2>&1)
[[ "$malformed" == *'已切换到：Alpha Provider'* ]]
[[ "$malformed" == *'新配置已经生效'* ]]
grep -Fq 'model = "DeepSeek-R1"' "$codex_home/config.toml"
grep -Fq '# <<< GPTEasy managed provider <<<' "$codex_home/config.toml"
cp -- "$codex_home/config.toml" "$1"
"#,
        );

        let (environment, default_executable) = match shell {
            LinuxShell::Bash => ("GPTEASY_TEST_BASH", "bash"),
            LinuxShell::Zsh => ("GPTEASY_TEST_ZSH", "zsh"),
        };
        let executable =
            std::env::var(environment).unwrap_or_else(|_| default_executable.to_owned());
        if !shell_is_available(&executable) {
            continue;
        }
        let rendered = fs::read_to_string(&destination).expect("read switched config");
        let parsed = rendered
            .parse::<toml_edit::DocumentMut>()
            .expect("switched config must remain valid TOML");
        assert_eq!(parsed["model"].as_str(), Some("DeepSeek-R1"));
        assert_eq!(parsed["model_reasoning_effort"].as_str(), Some("high"));
        assert_eq!(parsed["model_provider"].as_str(), Some("gpteasy"));
        assert_eq!(
            parsed["tui"]["status_line"]
                .as_array()
                .expect("status line array")
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>(),
            vec![
                "current-dir",
                "model-with-reasoning",
                "context-used",
                "used-tokens",
                "total-input-tokens",
                "total-output-tokens",
            ]
        );
    }
}

#[test]
fn shell_snapshots_preserve_safe_symlinks_and_reject_hardlinks_and_concurrency() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    fixture.insert_provider(
        "22222222-2222-4222-8222-222222222222",
        "Beta Provider",
        "https://beta.example/v1",
        "beta-secret-key",
        "beta-model",
        2,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");

        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-bash-files.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy.sh"
codex_home="$workspace/codex"
real_home="$workspace/real target"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p -- "$codex_home" "$real_home" "$fake_bin"
printf '%s\n' 'custom_setting = true' >"$real_home/config.toml"
ln -s '../real target/config.toml' "$codex_home/config.toml"
cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
cat >"$fake_bin/sync" <<'SYNC_WRAPPER'
#!/usr/bin/env bash
if [[ -n "${GPTEASY_CONCURRENT_TARGET:-}" && -f "${GPTEASY_CONCURRENT_ONCE:-}" && "$*" == *'.config.toml.gpteasy.'* ]]; then
    printf '%s\n' 'external_change = true' >>"$GPTEASY_CONCURRENT_TARGET"
    rm -f -- "$GPTEASY_CONCURRENT_ONCE"
fi
if [[ -n "${GPTEASY_REPLACE_TARGET:-}" && -f "${GPTEASY_REPLACE_ONCE:-}" && "$*" == *'.config.toml.gpteasy.'* ]]; then
    cp -p -- "$GPTEASY_REPLACE_TARGET" "$GPTEASY_REPLACE_TARGET.replacement"
    mv -f -- "$GPTEASY_REPLACE_TARGET.replacement" "$GPTEASY_REPLACE_TARGET"
    rm -f -- "$GPTEASY_REPLACE_ONCE"
fi
exec /usr/bin/sync "$@"
SYNC_WRAPPER
chmod 700 "$fake_bin/codex" "$fake_bin/sync"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"
link_before=$(readlink "$codex_home/config.toml")
gpteasy <<<"1" >/dev/null
[[ -L "$codex_home/config.toml" ]]
[[ $(readlink "$codex_home/config.toml") == "$link_before" ]]
grep -Fq '# GPTEasy provider-id: 11111111-1111-4111-8111-111111111111' "$real_home/config.toml"

touch "$workspace/change-once"
export GPTEASY_CONCURRENT_TARGET="$real_home/config.toml"
export GPTEASY_CONCURRENT_ONCE="$workspace/change-once"
before_restore_count=$(find "$codex_home/.gpteasy-shell/shell-restore" -mindepth 1 -maxdepth 1 -type d | wc -l)
if (gpteasy <<<"2" >/dev/null 2>&1); then
    printf '%s\n' 'concurrent config change was overwritten' >&2
    exit 1
fi
grep -Fq 'external_change = true' "$real_home/config.toml"
grep -Fq '# GPTEasy provider-id: 11111111-1111-4111-8111-111111111111' "$real_home/config.toml"
[[ $(find "$codex_home/.gpteasy-shell/shell-restore" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq "$before_restore_count" ]]
[[ $(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token' | wc -l) -eq 1 ]]
unset GPTEASY_CONCURRENT_TARGET GPTEASY_CONCURRENT_ONCE

touch "$workspace/replace-once"
export GPTEASY_REPLACE_TARGET="$real_home/config.toml"
export GPTEASY_REPLACE_ONCE="$workspace/replace-once"
before=$(sha256sum "$real_home/config.toml")
if (gpteasy <<<"2" >/dev/null 2>&1); then
    printf '%s\n' 'replaced symlink target inode was accepted' >&2
    exit 1
fi
[[ "$before" == "$(sha256sum "$real_home/config.toml")" ]]
grep -Fq '# GPTEasy provider-id: 11111111-1111-4111-8111-111111111111' "$real_home/config.toml"
unset GPTEASY_REPLACE_TARGET GPTEASY_REPLACE_ONCE

unsafe_home="$workspace/unsafe-codex"
mkdir -m 700 -- "$unsafe_home"
printf '%s\n' 'unsafe_parent = true' >"$unsafe_home/config.toml"
chmod 777 "$unsafe_home"
export CODEX_HOME="$unsafe_home"
unsafe_before=$(sha256sum "$unsafe_home/config.toml")
unsafe_result=$(gpteasy <<<"1" 2>&1)
[[ "$unsafe_result" == *'已切换到：Alpha Provider'* ]]
[[ "$unsafe_result" == *'权限允许其他用户访问'* ]]
[[ "$unsafe_before" != "$(sha256sum "$unsafe_home/config.toml")" ]]
[[ -e "$unsafe_home/.gpteasy-shell" ]]

hardlink_home="$workspace/hardlink-codex"
mkdir -p -- "$hardlink_home"
printf '%s\n' 'hardlinked = true' >"$hardlink_home/config.toml"
ln "$hardlink_home/config.toml" "$workspace/config-alias.toml"
export CODEX_HOME="$hardlink_home"
hardlink_before=$(sha256sum "$hardlink_home/config.toml")
if (gpteasy <<<"1" >/dev/null 2>&1); then
    printf '%s\n' 'hardlinked config was accepted' >&2
    exit 1
fi
[[ "$hardlink_before" == "$(sha256sum "$hardlink_home/config.toml")" ]]

linked_home_target="$workspace/linked-home-target"
linked_home="$workspace/linked-home"
mkdir -m 700 -- "$linked_home_target"
ln -s -- "$linked_home_target" "$linked_home"
export CODEX_HOME="$linked_home"
linked_home_result=$(gpteasy <<<"1" 2>&1 || true)
[[ "$linked_home_result" == *'CODEX_HOME 不能是符号链接'* ]]
[[ ! -e "$linked_home_target/.gpteasy-shell" ]]
"#,
        );
    }
}

#[test]
fn shell_snapshots_report_information_and_only_unlock_confirmed_stale_shell_locks() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");

        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-bash-lock.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy.sh"
codex_home="$workspace/codex"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p -- "$codex_home" "$fake_bin"
printf '%s\n' 'custom_setting = true' >"$codex_home/config.toml"
cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
chmod 700 "$fake_bin/codex"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"
gpteasy <<<"1" >/dev/null

info=$(gpteasy info)
[[ "$info" == *"目标环境：$codex_home"* ]]
[[ "$info" == *"Shell：$3"* ]]
[[ "$info" == *'供应商数量：1'* ]]
[[ "$info" == *'Codex CLI 最低版本：0.147.0'* ]]
[[ "$info" != *'alpha-secret-key'* ]]
[[ $("$2" "$script" current) == *'Alpha Provider'* ]]

subshell_owner_pid=$(
    gpteasy__prepare_private_state
    gpteasy__acquire_lock switch
    gpteasy__lock_value "$gpteasy__active_lock/owner" pid
    gpteasy__release_lock
)
[[ "$subshell_owner_pid" != "$$" ]]

credential=$(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token')
chmod 644 "$credential"
current_with_broad_credential=$(gpteasy current)
[[ "$current_with_broad_credential" == *'Alpha Provider'* ]]
gpteasy help >/dev/null
chmod 600 "$credential"

active="$codex_home/.gpteasy-shell/lock/active"
mkdir -m 700 -- "$active"
active_pid=$$
start=$(awk '{print $22}' "/proc/$active_pid/stat")
cat >"$active/owner" <<ACTIVE_LOCK
owner=shell
token=active-token
pid=$active_pid
process_start=$start
operation=switch
ACTIVE_LOCK
chmod 600 "$active/owner"
blocked=$(gpteasy <<<"1" 2>&1 || true)
[[ "$blocked" == *'shell'* && "$blocked" == *'switch'* ]]
if (gpteasy unlock <<<"y" >/dev/null 2>&1); then
    printf '%s\n' 'active shell lock was removed' >&2
    exit 1
fi
[[ -d "$active" ]]

cat >"$active/owner" <<STALE_LOCK
owner=shell
token=stale-token
pid=99999999
process_start=1
operation=restore
STALE_LOCK
chmod 600 "$active/owner"
cancelled=$(gpteasy unlock <<<"n")
[[ "$cancelled" == *'已取消'* ]]
[[ -d "$active" ]]
removed=$(gpteasy unlock <<<"y")
[[ "$removed" == *'已删除失效的 shell 锁'* ]]
[[ ! -e "$active" ]]

mkdir -m 700 -- "$active"
cat >"$active/owner" <<DESKTOP_LOCK
owner=desktop
token=desktop-token
pid=99999999
process_start=1
operation=switch
DESKTOP_LOCK
chmod 600 "$active/owner"
if (gpteasy unlock <<<"y" >/dev/null 2>&1); then
    printf '%s\n' 'desktop lock was removed by shell' >&2
    exit 1
fi
[[ -d "$active" ]]
"#,
        );
    }
}

#[test]
fn shell_snapshots_preserve_special_provider_data_without_glob_expansion() {
    const NAME: &str = "Long Provider $HOME * ? [abc] with \"quotes\" and Unicode 测试供应商名称";
    const MODEL: &str = "model-$HOME-*-?-[abc]-\"quoted\"-模型";
    const API_KEY: &str = "token-$HOME-*-?-[abc]-backslash\\-Unicode-密钥";
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        NAME,
        "https://special.example/v1/$HOME/*?value=[abc]",
        API_KEY,
        MODEL,
        1,
    );

    let harness = format!(
        r#"
set -euo pipefail
workspace=$(mktemp -d "${{TMPDIR:-/tmp}}/gpteasy-special.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy-export"
codex_home="$workspace/codex home"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p -- "$codex_home" "$fake_bin" "$workspace/glob"
touch "$workspace/glob/model-expanded" "$workspace/glob/provider-expanded"
printf '%s\n' '{{"tokens":{{"access_token":"keep-me"}}}}' >"$codex_home/auth.json"
auth_before=$(sha256sum "$codex_home/auth.json")
cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
chmod 700 "$fake_bin/codex"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
source "$script"
expected_name=$(cat <<'EXPECTED_NAME'
{NAME}
EXPECTED_NAME
)
expected_model=$(cat <<'EXPECTED_MODEL'
{MODEL}
EXPECTED_MODEL
)
expected_key=$(cat <<'EXPECTED_KEY'
{API_KEY}
EXPECTED_KEY
)
cd "$workspace/glob"
menu=$(gpteasy <<<"q")
[[ "$menu" == *"$expected_name ($expected_model)"* ]]
gpteasy <<<"1" >/dev/null
[[ $(gpteasy__provider_name '11111111-1111-4111-8111-111111111111') == "$expected_name" ]]
[[ $(gpteasy__provider_model '11111111-1111-4111-8111-111111111111') == "$expected_model" ]]
credential=$(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token' -print -quit)
cmp -s -- "$credential" <(printf '%s' "$expected_key")
grep -Fq '$HOME-*' "$codex_home/config.toml"
grep -Fq '[abc]' "$codex_home/config.toml"
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
"#,
    );

    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");
        run_shell_black_box(shell, &destination, &harness);
    }
}

#[test]
fn shell_snapshots_keep_acceptance_canary_out_of_public_surfaces_and_auth_json() {
    let canary = std::env::var("GPTEASY_ACCEPTANCE_KEY_A")
        .unwrap_or_else(|_| format!("gpteasy-shell-canary-{}", Uuid::new_v4()));
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Canary Provider",
        "https://canary.example/v1",
        &canary,
        "canary-model",
        1,
    );

    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "gpteasy.sh",
            LinuxShell::Zsh => "gpteasy.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export canary shell snapshot");

        run_shell_black_box_with_canaries(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-canary.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy-export"
codex_home="$workspace/codex"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p -- "$codex_home" "$fake_bin"
printf '%s' '{"login":"unchanged"}' >"$codex_home/auth.json"
auth_before=$(sha256sum "$codex_home/auth.json")
cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env sh
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
chmod 700 "$fake_bin/codex"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
source "$script"
gpteasy <<<"1"
gpteasy current
gpteasy info
"$2" "$script" current
credential=$(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token' -print -quit)
[[ -s "$credential" ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
"#,
            &[&canary],
        );
    }
}

#[test]
fn shell_switch_returns_when_daemon_restart_hangs_without_rolling_back() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(match shell {
            LinuxShell::Bash => "bounded.sh",
            LinuxShell::Zsh => "bounded.zsh",
        });
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .expect("export shell snapshot");
        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-daemon-timeout.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy-export"
cp -- "$1" "$script"
chmod 600 "$script"
mkdir -p "$workspace/bin" "$workspace/codex"
export CODEX_HOME="$workspace/codex"
export PATH="$workspace/bin:$PATH"
export GPTEASY_DAEMON_CALLS="$workspace/calls"
export GPTEASY_TEST_DAEMON_REFRESH_DEADLINE_SECONDS=1
cat >"$workspace/bin/codex" <<'CODEX'
#!/usr/bin/env bash
if [[ "$*" == --version ]]; then
    echo 'codex-cli 0.147.0'
    exit 0
fi
printf '%s\n' "$*" >>"$GPTEASY_DAEMON_CALLS"
# Exercise a stuck draining operation that even ignores graceful termination.
trap '' TERM
while :; do sleep 1; done
CODEX
chmod 700 "$workspace/bin/codex"
# An outer deadline catches the original hang, including inherited output pipes.
if ! timeout --kill-after=1s 16s "$2" -c 'source "$1"; gpteasy <<<"1"; echo SWITCH_RETURNED' shell "$script" >"$workspace/output" 2>&1; then
    echo 'FAIL: provider switch did not return after daemon refresh hung' >&2
    exit 1
fi
output=$(cat "$workspace/output")
[[ "$output" == *'SWITCH_RETURNED'* ]]
[[ "$output" == *'已切换到：Alpha Provider'* ]]
[[ "$output" == *'stage=restart result=timeout'* ]]
[[ "$output" == *'配置已生效'* ]]
[[ "$output" != *'alpha-secret-key'* ]]
[[ "$output" != *'https://alpha.example/v1'* ]]
[[ "$(cat "$GPTEASY_DAEMON_CALLS")" == 'app-server daemon restart' ]]
[[ ! -e "$CODEX_HOME/.gpteasy-shell/lock/active" ]]
source "$script"
[[ "$(gpteasy current)" == *'Alpha Provider'* ]]
[[ -s "$CODEX_HOME/config.toml" ]]
[[ -n "$(find "$CODEX_HOME/.gpteasy-shell/credentials" -name '*.token' -print -quit)" ]]

# A graceful restart can take longer than the old 10-second deadline.
cat >"$workspace/bin/codex" <<'CODEX'
#!/usr/bin/env bash
if [[ "$*" == --version ]]; then echo 'codex-cli 0.147.0'; exit 0; fi
if [[ "$*" == 'app-server daemon restart' ]]; then
    sleep 2
fi
if read -r unexpected; then exit 23; fi
printf '%s\n' "$*" >>"$GPTEASY_DAEMON_CALLS"
exit 0
CODEX
chmod 700 "$workspace/bin/codex"
export GPTEASY_TEST_DAEMON_REFRESH_DEADLINE_SECONDS=3
slow=$(gpteasy <<<"1" 2>&1)
[[ "$slow" == *'stage=restart result=started deadline_seconds=3'* ]]
[[ "$slow" == *'stage=restart result=command_completed'* ]]
unset GPTEASY_TEST_DAEMON_REFRESH_DEADLINE_SECONDS

# Success only proves the control command completed, not daemon readiness.
cat >"$workspace/bin/codex" <<'CODEX'
#!/usr/bin/env bash
if [[ "$*" == --version ]]; then echo 'codex-cli 0.147.0'; exit 0; fi
if read -r unexpected; then exit 23; fi
printf '%s\n' "$*" >>"$GPTEASY_DAEMON_CALLS"
echo 'raw diagnostic alpha-secret-key https://alpha.example/v1' >&2
exit "${GPTEASY_DAEMON_RESTART_STATUS:-0}"
CODEX
chmod 700 "$workspace/bin/codex"
export GPTEASY_DAEMON_RESTART_STATUS=0
good=$(gpteasy <<<"1" 2>&1)
[[ "$good" == *'stage=restart result=command_completed'* ]]
[[ "$good" != *'raw diagnostic'* ]]
export GPTEASY_DAEMON_RESTART_STATUS=17
failed=$(gpteasy <<<"1" 2>&1)
[[ "$failed" == *'stage=restart result=command_failed exit_code=17'* ]]
[[ "$failed" == *'配置已生效'* ]]
[[ "$failed" != *'raw diagnostic'* ]]
[[ "$(gpteasy current)" == *'Alpha Provider'* ]]
# No CLI: do not try to refresh a preconfigured environment.
before=$(cat "$GPTEASY_DAEMON_CALLS")
gpteasy__codex_cli_state=missing
missing=$(gpteasy__restart_daemon_best_effort 2>&1)
[[ -z "$missing" ]]
[[ "$before" == "$(cat "$GPTEASY_DAEMON_CALLS")" ]]
# No deadline utility: fail closed on refresh, not on the committed switch.
gpteasy__codex_cli_state=ready
skipped=$(PATH="$workspace/bin" gpteasy__restart_daemon_best_effort 2>&1)
[[ "$skipped" == *'stage=restart result=skipped_no_timeout'* ]]
[[ "$before" == "$(cat "$GPTEASY_DAEMON_CALLS")" ]]
"#,
        );
    }
}

struct ExportFixture {
    temp: TempDir,
    store: StateStore,
    application: ProviderApplication,
}

fn run_shell_black_box(shell: LinuxShell, script: &Path, harness: &str) {
    run_shell_black_box_with_canaries(shell, script, harness, &[]);
}

fn run_shell_black_box_with_canaries(
    shell: LinuxShell,
    script: &Path,
    harness: &str,
    extra_canaries: &[&str],
) {
    let (environment, default_executable, label, display_name) = match shell {
        LinuxShell::Bash => ("GPTEASY_TEST_BASH", "bash", "Bash", "Bash 4+"),
        LinuxShell::Zsh => ("GPTEASY_TEST_ZSH", "zsh", "Zsh", "Zsh 5+"),
    };
    let executable = std::env::var(environment).unwrap_or_else(|_| default_executable.to_owned());
    if !shell_is_available(&executable) {
        if std::env::var("GPTEASY_REQUIRE_SHELL_MATRIX").as_deref() == Ok("1") {
            panic!("required {label} executable is unavailable: {executable}");
        }
        eprintln!("skipping unavailable {label} executable: {executable}");
        return;
    }
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("wsl.exe");
        command
            .args(["-d", &wsl_test_distribution(), "--"])
            .arg(&executable)
            .args([
                "-s",
                "--",
                &windows_path_for_wsl(script),
                &executable,
                display_name,
            ]);
        command
    };

    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new(&executable);
        command.args([
            "-s",
            "--",
            script.to_str().expect("UTF-8 test path"),
            &executable,
            display_name,
        ]);
        command
    };

    for canary in extra_canaries {
        assert_process_arguments_are_clean(label, &command, canary);
    }
    for name in ["GPTEASY_ACCEPTANCE_KEY_A", "GPTEASY_ACCEPTANCE_KEY_B"] {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            assert_process_arguments_are_clean(label, &command, &value);
        }
    }

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("start {label} black-box test: {error}"));

    let mut stdin = child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("{label} stdin"));
    stdin
        .write_all(b"umask 077\n")
        .unwrap_or_else(|error| panic!("set a private {label} fixture umask: {error}"));
    stdin
        .write_all(harness.as_bytes())
        .unwrap_or_else(|error| panic!("write {label} harness without credentials: {error}"));
    drop(stdin);
    let output = child
        .wait_with_output()
        .expect("wait for Bash black-box test");
    for canary in extra_canaries {
        assert_public_output_is_clean(label, &output, canary);
    }
    for name in ["GPTEASY_ACCEPTANCE_KEY_A", "GPTEASY_ACCEPTANCE_KEY_B"] {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            assert_public_output_is_clean(label, &output, &value);
        }
    }
    assert!(
        output.status.success(),
        "{label} black-box test failed ({:?})\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_public_output_is_clean(label: &str, output: &std::process::Output, canary: &str) {
    assert!(
        !contains_bytes(&output.stdout, canary.as_bytes()),
        "API key canary leaked into {label} standard output"
    );
    assert!(
        !contains_bytes(&output.stderr, canary.as_bytes()),
        "API key canary leaked into {label} standard error"
    );
}

fn assert_process_arguments_are_clean(label: &str, command: &Command, canary: &str) {
    assert!(
        !command.get_program().to_string_lossy().contains(canary)
            && !command
                .get_args()
                .any(|argument| argument.to_string_lossy().contains(canary)),
        "API key canary leaked into {label} child process arguments"
    );
}

fn shell_matrix_targets() -> Vec<LinuxShell> {
    match std::env::var("GPTEASY_TEST_MATRIX_SHELL").as_deref() {
        Ok("bash") => vec![LinuxShell::Bash],
        Ok("zsh") => vec![LinuxShell::Zsh],
        Ok(value) => panic!("unsupported GPTEASY_TEST_MATRIX_SHELL value: {value}"),
        Err(_) => vec![LinuxShell::Bash, LinuxShell::Zsh],
    }
}

fn shell_is_available(executable: &str) -> bool {
    #[cfg(windows)]
    let output = Command::new("wsl.exe")
        .args([
            "-d",
            &wsl_test_distribution(),
            "--",
            executable,
            "--version",
        ])
        .output();

    #[cfg(not(windows))]
    let output = Command::new(executable).arg("--version").output();

    output.is_ok_and(|output| output.status.success())
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[cfg(windows)]
fn wsl_test_distribution() -> String {
    std::env::var("GPTEASY_TEST_WSL_DISTRIBUTION").unwrap_or_else(|_| "Ubuntu".to_owned())
}

#[cfg(windows)]
fn windows_path_for_wsl(path: &Path) -> String {
    let windows_path = path.to_str().expect("UTF-8 test path").replace('\\', "/");
    let output = Command::new("wsl.exe")
        .args([
            "-d",
            &wsl_test_distribution(),
            "--",
            "wslpath",
            "-a",
            "-u",
            &windows_path,
        ])
        .output()
        .expect("translate Windows test path for WSL");
    assert!(
        output.status.success(),
        "translate Windows test path for WSL"
    );
    String::from_utf8(output.stdout)
        .expect("WSL path is UTF-8")
        .trim()
        .to_owned()
}

impl ExportFixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temporary export fixture");
        let store = StateStore::new(StatePaths::from_root(temp.path().join("state")));
        assert!(store.bootstrap().is_ready());
        let application = ProviderApplication::new(
            store.clone(),
            ProviderValidator::new(ValidationTimeouts::default()),
        );
        Self {
            temp,
            store,
            application,
        }
    }

    fn insert_provider(
        &self,
        id: &str,
        name: &str,
        base_url: &str,
        api_key: &str,
        default_model: &str,
        sort_order: i64,
    ) {
        let connection = Connection::open(self.store.paths().database()).expect("open state");
        let fingerprint = provider_combination_fingerprint(base_url, api_key, default_model);
        connection
            .execute(
                "INSERT INTO providers (
                    id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint, sort_order
                 ) VALUES (?1, ?2, ?3, ?4, ?5, '1786800000', ?6, ?7)",
                params![
                    id,
                    name,
                    base_url,
                    api_key,
                    default_model,
                    fingerprint,
                    sort_order
                ],
            )
            .expect("insert verified provider fixture");
    }

    fn insert_model_catalog(
        &self,
        provider_id: &str,
        models: &[&str],
        capability_snapshot_json: Option<&str>,
    ) {
        let connection = Connection::open(self.store.paths().database()).expect("open state");
        let (base_url, api_key, default_model): (String, String, String) = connection
            .query_row(
                "SELECT base_url, api_key, default_model FROM providers WHERE id = ?1",
                [provider_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("provider fixture exists");
        let fingerprint = provider_combination_fingerprint(&base_url, &api_key, &default_model);
        let models_json = serde_json::to_string(models).expect("serialize model fixture");
        connection
            .execute(
                "INSERT INTO provider_model_catalog(
                    provider_id, verification_fingerprint, models_json, capability_snapshot_json
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    provider_id,
                    fingerprint,
                    models_json,
                    capability_snapshot_json
                ],
            )
            .expect("insert model catalog fixture");
    }
}

fn provider_combination_fingerprint(base_url: &str, api_key: &str, model: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-provider-combination-v1\0");
    hasher.update(base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(model.as_bytes());
    hasher.update(b"\0");
    hasher.update(api_key.as_bytes());
    format!("{:x}", hasher.finalize())
}
