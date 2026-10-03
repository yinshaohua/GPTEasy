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
    fixture.insert_provider_with_models(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://api.deepseek.com/v1",
        "alpha-secret-key",
        "DeepSeek-R1",
        1,
        &["DeepSeek-R1", "deepseek-chat"],
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
            "11111111-1111-4111-8111-111111111111\tAlpha Provider\thttps://api.deepseek.com/v1\tDeepSeek-R1\talpha-secret-key\thigh\t"
        ),
        "each provider must be editable as one top-of-script catalog record"
    );
    let alpha_payload_start = script
        .find("gpteasy__provider_catalog_payload_11111111-1111-4111-8111-111111111111()")
        .expect("alpha model payload function");
    let alpha_payload_end = script[alpha_payload_start..]
        .find("\n}\n")
        .map(|offset| alpha_payload_start + offset)
        .expect("end of alpha model payload function");
    let alpha_payload = &script[alpha_payload_start..alpha_payload_end];
    assert!(alpha_payload.contains("DeepSeek-R1"));
    assert!(alpha_payload.contains("deepseek-chat"));
    assert!(alpha_payload.contains("low"));
    assert!(alpha_payload.contains("medium"));
    assert!(alpha_payload.contains("high"));
    assert!(alpha_payload.contains("xhigh"));
    assert!(!alpha_payload.contains("alpha-secret-key"));
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
    fixture.insert_provider_with_models(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://api.deepseek.com/v1",
        "alpha-secret-key",
        "DeepSeek-R1",
        1,
        &["DeepSeek-R1", "deepseek-chat"],
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
fn bash_export_replaces_an_existing_file_after_native_confirmation() {
    let fixture = ExportFixture::new();
    fixture.insert_provider_with_models(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
        &["alpha-model", "alpha-alt-model"],
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
saved_path=$PATH
export PATH=/gpteasy-source-must-not-execute-tools
source ./gpteasy.sh
export PATH=$saved_path
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
fn shell_snapshots_require_native_catalog_capability_and_reject_version_only_clis() {
    let fixture = ExportFixture::new();
    fixture.insert_provider_with_models(
        "11111111-1111-4111-8111-111111111111",
        "Alpha Provider",
        "https://alpha.example/v1",
        "alpha-secret-key",
        "alpha-model",
        1,
        &["alpha-model", "alpha-alt-model"],
    );
    fixture.insert_provider(
        "22222222-2222-4222-8222-222222222222",
        "Beta Provider",
        "https://beta.example/v1",
        "beta-secret-key",
        "beta-model",
        2,
    );
    fixture.insert_provider(
        "33333333-3333-4333-8333-333333333333",
        "Manual Provider",
        "https://manual.example/v1",
        "manual-secret-key",
        "manual-model",
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
workspace=$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-bash-switch.XXXXXX")
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/gpteasy.sh"
codex_home="$workspace/codex home"
fake_bin="$workspace/bin"
cp -- "$1" "$script"
chmod 600 "$script"
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
if PATH="$fake_bin:/usr/bin:/bin" gpteasy <<<"3" >/dev/null 2>&1; then exit 1; fi
[[ ! -e "$fresh_home/config.toml" ]]
export CODEX_HOME="$codex_home"
missing=$(PATH="$fake_bin:/usr/bin:/bin" gpteasy <<<"1" 2>&1 || true)
[[ "$missing" == *'未找到原生 Linux Codex CLI'* ]]
[[ "$config_before" == "$(sha256sum "$codex_home/config.toml")" ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
missing_config=$config_before
cat >"$fake_bin/codex" <<'OLD_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 99.0.0'
OLD_CODEX
chmod 700 "$fake_bin/codex"
if (gpteasy <<<"1" >/dev/null 2>&1); then
    printf '%s\n' 'unsupported Codex version was accepted' >&2
    exit 1
fi
too_old=$(gpteasy <<<"1" 2>&1 || true)
[[ "$too_old" == *'模型目录/schema 能力核验失败'* ]]
[[ "$too_old" != *'未找到 Codex CLI'* ]]
[[ "$missing_config" == "$(sha256sum "$codex_home/config.toml")" ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]

printf 'MZfixture' >"$fake_bin/codex"
chmod 700 "$fake_bin/codex"
interop=$(gpteasy <<<"1" 2>&1 || true)
[[ "$interop" == *'拒绝 Windows 互操作入口'* ]]
[[ "$missing_config" == "$(sha256sum "$codex_home/config.toml")" ]]
make_compatible_codex "$fake_bin/codex"
codex() { printf '%s\n' 'shell function must not select the CLI' >&2; return 97; }
menu=$(gpteasy <<<"1")
unset -f codex
[[ "$menu" == *'Alpha Provider (alpha-model)'* ]]
grep -Fq '# GPTEasy schema-version: 2' "$codex_home/config.toml"
grep -Fq 'model_catalog_json = ' "$codex_home/config.toml"
catalog_path=$(sed -n 's/^model_catalog_json = "\(.*\)"$/\1/p' "$codex_home/config.toml")
[[ -f "$catalog_path" ]]
[[ $(stat -c '%a' "$catalog_path") == '600' ]]
grep -Fq 'alpha-model' "$catalog_path"
grep -Fq 'alpha-alt-model' "$catalog_path"
grep -Fq '"effort": "low"' "$catalog_path"
grep -Fq '"effort": "medium"' "$catalog_path"
grep -Fq '"effort": "high"' "$catalog_path"
grep -Fq '"effort": "xhigh"' "$catalog_path"
! grep -Fq 'alpha-secret-key' "$catalog_path"
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
[[ $(find "$codex_home/.gpteasy-shell/shell-restore" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq 1 ]]
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
make_compatible_codex "$fake_bin/codex"
cat >"$fake_bin/date" <<'BACKWARDS_CLOCK'
#!/bin/sh
if [ "$*" = '-u +%Y%m%dT%H%M%S%N' ]; then
    if [ -f "$GPTEASY_CLOCK_MARKER" ]; then
        printf '%s\n' 20200101T000000000000000
    else
        printf '%s\n' 20990101T000000000000000
        touch "$GPTEASY_CLOCK_MARKER"
    fi
else
    exec /usr/bin/date "$@"
fi
BACKWARDS_CLOCK
chmod 700 "$fake_bin/date"
export GPTEASY_CLOCK_MARKER="$workspace/clock-once"
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

alpha_catalog=$(sed -n 's/^model_catalog_json = "\(.*\)"$/\1/p' "$codex_home/config.toml")
[[ -f "$alpha_catalog" ]]
gpteasy <<<"2" >/dev/null
beta_before_missing_restore=$(sha256sum "$codex_home/config.toml")
rm -f -- "$alpha_catalog"
missing_catalog_restore=$(gpteasy restore <<<"y" 2>&1 || true)
[[ "$missing_catalog_restore" == *'模型目录缺失、篡改或绑定失效'* ]]
[[ "$beta_before_missing_restore" == "$(sha256sum "$codex_home/config.toml")" ]]

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
make_compatible_codex "$fake_bin/codex"
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
    $0 == "# GPTEasy schema-version: 2" { next }
    index($0, "# GPTEasy source-id:") == 1 { next }
    index($0, "# GPTEasy credential-file:") == 1 { next }
    index($0, "# GPTEasy model-catalog-") == 1 { next }
    index($0, "model_catalog_json = ") == 1 { next }
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
sed -i 's/# GPTEasy schema-version: 2/# GPTEasy schema-version: 99/' "$codex_home/config.toml"
[[ "$(gpteasy current)" == *'管理冲突'* ]]
gpteasy <<<"2" >/dev/null
grep -Fq '# GPTEasy schema-version: 2' "$codex_home/config.toml"
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
make_compatible_codex "$fake_bin/codex"
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
[[ "$malformed" == *'配置已保存，但 CLI/共享后台服务可能仍使用旧配置'* ]]
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
make_compatible_codex "$fake_bin/codex"
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
make_compatible_codex "$fake_bin/codex"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"
gpteasy <<<"1" >/dev/null

info=$(gpteasy info)
[[ "$info" == *"目标环境：$codex_home"* ]]
[[ "$info" == *"Shell：$3"* ]]
[[ "$info" == *'供应商数量：1'* ]]
[[ "$info" == *'Codex CLI：明确切换时核验原生 Linux 目录/schema 能力'* ]]
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
make_compatible_codex "$fake_bin/codex"
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
make_compatible_codex "$fake_bin/codex"
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
fn shell_snapshots_reject_stale_combinations_and_payloads_before_config_commit() {
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
        let destination = fixture.temp.path().join(format!("binding-{shell:?}"));
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .unwrap();
        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
script="$workspace/export"
cp -- "$1" "$script"
export CODEX_HOME="$workspace/home"
mkdir -m 700 "$CODEX_HOME"
printf '%s\n' 'old = true' >"$CODEX_HOME/config.toml"
before=$(sha256sum "$CODEX_HOME/config.toml")
mkdir -m 700 "$workspace/bin"
make_compatible_codex "$workspace/bin/codex"
export PATH="$workspace/bin:$PATH"
cp -- "$1" "$script"
source "$script"
# First prove the original snapshot passes both the CLI and binding gates.
gpteasy <<<"1" >"$workspace/output" 2>&1 || { cat "$workspace/output" >&2; exit 1; }
gpteasy restore <<<"y" >"$workspace/output" 2>&1 || { cat "$workspace/output" >&2; exit 1; }
[[ "$before" == "$(sha256sum "$CODEX_HOME/config.toml")" ]]
for change in base_url credential model protocol policy source; do
    case "$change" in
        base_url) sed 's@https://alpha.example/v1@https://changed.example/v1@g' "$1" >"$script" ;;
        credential) sed 's/alpha-secret-key/changed-secret-key/g' "$1" >"$script" ;;
        model) sed 's/alpha-model\t/changed-model\t/' "$1" >"$script" ;;
        protocol) sed "s/gpteasy__catalog_protocol='codex-model-catalog-v1'/gpteasy__catalog_protocol='codex-model-catalog-v99'/" "$1" >"$script" ;;
        policy) sed "s/gpteasy__catalog_policy='common-reasoning-selector-v1'/gpteasy__catalog_policy='changed-policy'/" "$1" >"$script" ;;
        source) sed "s/^gpteasy__export_id=.*/gpteasy__export_id='99999999-9999-4999-8999-999999999999'/" "$1" >"$script" ;;
    esac
    source "$script"
    if gpteasy <<<"1" >"$workspace/output" 2>&1; then
        printf 'stale %s was accepted\n' "$change" >&2
        exit 1
    fi
    grep -Fq '载荷与供应商快照绑定不一致' "$workspace/output"
    [[ "$before" == "$(sha256sum "$CODEX_HOME/config.toml")" ]]
done
sed 's/Alpha Provider/Display Renamed/g' "$1" >"$script"
source "$script"
gpteasy <<<"1" >"$workspace/output" 2>&1
grep -Fq 'Display Renamed' "$CODEX_HOME/config.toml"
gpteasy restore <<<"y" >"$workspace/output" 2>&1
cp -- "$1" "$script"
sed -i 's/"context_window": 128000/"context_window": 127999/g' "$script"
source "$script"
if gpteasy <<<"1" >"$workspace/output" 2>&1; then exit 1; fi
[[ "$before" == "$(sha256sum "$CODEX_HOME/config.toml")" ]]
"#,
        );
    }
}

#[test]
fn export_rejects_missing_corrupt_stale_and_control_character_snapshots() {
    for mutation in [
        "DELETE FROM provider_model_catalog",
        "UPDATE provider_model_catalog SET models_json = 'broken'",
        "UPDATE provider_model_catalog SET models_json = '[]'",
        "UPDATE provider_model_catalog SET verification_fingerprint = 'stale'",
        "UPDATE providers SET api_key = 'changed'",
        "UPDATE providers SET name = 'unsafe' || char(1)",
    ] {
        let fixture = ExportFixture::new();
        fixture.insert_provider(
            "11111111-1111-4111-8111-111111111111",
            "Alpha",
            "https://alpha.example/v1",
            "private-key",
            "alpha-model",
            1,
        );
        Connection::open(fixture.store.paths().database())
            .unwrap()
            .execute(mutation, [])
            .unwrap();
        let destination = fixture.temp.path().join("rejected.sh");
        assert!(
            fixture
                .application
                .export_linux_script(LinuxShell::Bash, &destination, false)
                .is_err(),
            "{mutation}"
        );
        assert!(!destination.exists());
    }
}

#[test]
fn shell_snapshots_restore_catalogs_from_another_export_without_the_old_provider() {
    for shell in shell_matrix_targets() {
        let fixture = ExportFixture::new();
        fixture.insert_provider(
            "11111111-1111-4111-8111-111111111111",
            "Alpha",
            "https://alpha.example/v1",
            "private-alpha",
            "alpha-model",
            1,
        );
        let destination = fixture.temp.path().join(format!("cross-{shell:?}"));
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .unwrap();
        Connection::open(fixture.store.paths().database())
            .unwrap()
            .execute("DELETE FROM providers", [])
            .unwrap();
        fixture.insert_provider(
            "22222222-2222-4222-8222-222222222222",
            "Beta",
            "https://beta.example/v1",
            "private-beta",
            "beta-model",
            1,
        );
        let second = destination.with_file_name(format!("cross-{shell:?}.second"));
        fixture
            .application
            .export_linux_script(shell, &second, false)
            .unwrap();
        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
export CODEX_HOME="$workspace/中文 home \ quote\""
mkdir -m 700 "$CODEX_HOME" "$workspace/bin"
make_compatible_codex "$workspace/bin/codex"
export PATH="$workspace/bin:$PATH"
# The external catalog is preserved byte for byte, without being adopted.
printf '%s\n' 'external catalog bytes' >"$workspace/external.json"
printf 'model_catalog_json = "%s/external.json"\ncustom = true\n' "$workspace" >"$CODEX_HOME/config.toml"
cp "$CODEX_HOME/config.toml" "$workspace/original"
external_hash=$(sha256sum "$workspace/external.json")
source "$1"
gpteasy <<<"1" >"$workspace/output" 2>&1
cp "$CODEX_HOME/config.toml" "$workspace/alpha"
old_catalog=$(find "$CODEX_HOME/.gpteasy-shell/model-catalogs" -type f -name '*.json' -print -quit)
cp "$old_catalog" "$workspace/catalog"
source "$1.second"
gpteasy <<<"1" >"$workspace/output" 2>&1
before=$(sha256sum <"$CODEX_HOME/config.toml")
for corruption in missing bytes symlink hardlink; do
    rm -f "$old_catalog"
    case "$corruption" in
        missing) ;;
        bytes) printf '%s' 'broken' >"$old_catalog" ;;
        symlink) ln -s "$workspace/catalog" "$old_catalog" ;;
        hardlink) ln "$workspace/catalog" "$old_catalog" ;;
    esac
    if gpteasy restore <<<"y" >"$workspace/output" 2>&1; then exit 1; fi
    [[ "$before" == "$(sha256sum <"$CODEX_HOME/config.toml")" ]]
    [[ ! -d "$CODEX_HOME/.gpteasy-shell/lock/active" ]]
done
rm -f "$old_catalog"
cp "$workspace/catalog" "$old_catalog"
gpteasy restore <<<"y" >"$workspace/output" 2>&1
cmp -s "$workspace/alpha" "$CODEX_HOME/config.toml"
grep -Fq '配置已保存，但 CLI/共享后台服务可能仍使用旧配置' "$workspace/output"
gpteasy restore <<<"y" >"$workspace/output" 2>&1
cmp -s "$workspace/original" "$CODEX_HOME/config.toml"
[[ "$external_hash" == "$(sha256sum "$workspace/external.json")" ]]
[[ -f "$old_catalog" ]]
# A previously missing config is restored to absence, without adding a catalog.
export CODEX_HOME="$workspace/empty"
gpteasy <<<"1" >"$workspace/output" 2>&1
gpteasy restore <<<"y" >"$workspace/output" 2>&1
[[ ! -e "$CODEX_HOME/config.toml" ]]
"#,
        );
    }
}

#[test]
fn shell_snapshots_keep_references_safe_at_both_atomic_commit_boundaries() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha",
        "https://alpha.example/v1",
        "private-key",
        "alpha-model",
        1,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(format!("fault-{shell:?}"));
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .unwrap();
        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
mkdir -m 700 "$workspace/bin"
make_compatible_codex "$workspace/bin/codex"
cat >"$workspace/bin/mv" <<'MV'
#!/bin/sh
case "$GPTEASY_FAULT:$*" in
    catalog_move:*'.model-catalog.'*) exit 1 ;;
    config_move:*'.config.toml.gpteasy.'*) exit 1 ;;
esac
exec /usr/bin/mv "$@"
MV
cat >"$workspace/bin/sync" <<'SYNC'
#!/bin/sh
if [ "$GPTEASY_FAULT" = after_commit ] && [ "$2" = "$CODEX_HOME" ] && grep -Fq '# GPTEasy schema-version: 2' "$CODEX_HOME/config.toml"; then exit 1; fi
if [ "$GPTEASY_FAULT" = corrupt_catalog ] && [ "$2" = "$CODEX_HOME"/.gpteasy-shell/model-catalogs/* ]; then
    find "$2" -type f -name '*.json' -exec sh -c 'printf broken >"$1"' sh {} \;
fi
exec /usr/bin/sync "$@"
SYNC
chmod 700 "$workspace/bin/mv" "$workspace/bin/sync"
export PATH="$workspace/bin:$PATH"
source "$1"
for GPTEASY_FAULT in catalog_move config_move after_commit corrupt_catalog broad_private; do
    export GPTEASY_FAULT CODEX_HOME="$workspace/$GPTEASY_FAULT"
    mkdir -m 700 "$CODEX_HOME"
    printf '%s\n' 'old_config = true' >"$CODEX_HOME/config.toml"
    cp "$CODEX_HOME/config.toml" "$workspace/original"
    if [[ "$GPTEASY_FAULT" == broad_private ]]; then
        mkdir -m 755 "$CODEX_HOME/.gpteasy-shell"
    fi
    if gpteasy <<<"1" >"$workspace/output" 2>&1; then exit 1; fi
    [[ ! -d "$CODEX_HOME/.gpteasy-shell/lock/active" ]]
    if [[ "$GPTEASY_FAULT" == after_commit ]]; then
        grep -Fq '# GPTEasy schema-version: 2' "$CODEX_HOME/config.toml"
        grep -Fq 'stage=config_readback catalog_state=indeterminate' "$workspace/output"
        grep -Fq '配置已保存，但 CLI/共享后台服务可能仍使用旧配置' "$workspace/output"
        catalog=$(sed -n 's/^model_catalog_json = "\(.*\)"$/\1/p' "$CODEX_HOME/config.toml")
        [[ -s "$catalog" ]]
        [[ $(find "$CODEX_HOME/.gpteasy-shell/shell-restore" -name config.toml | wc -l) -eq 1 ]]
        export GPTEASY_FAULT=none
        gpteasy restore <<<"y" >"$workspace/output" 2>&1
        cmp -s "$workspace/original" "$CODEX_HOME/config.toml"
    else
        cmp -s "$workspace/original" "$CODEX_HOME/config.toml"
        if [[ "$GPTEASY_FAULT" == config_move ]]; then
            [[ $(find "$CODEX_HOME/.gpteasy-shell/model-catalogs" -name '*.json' | wc -l) -eq 1 ]]
        fi
    fi
done
"#,
        );
    }
}

#[test]
fn shell_snapshots_reject_restore_point_changes_during_confirmation() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha",
        "https://alpha.example/v1",
        "private-key",
        "alpha-model",
        1,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(format!("confirmation-{shell:?}"));
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .unwrap();
        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
export CODEX_HOME="$workspace/home"
mkdir -m 700 "$CODEX_HOME" "$workspace/bin"
make_compatible_codex "$workspace/bin/codex"
export PATH="$workspace/bin:$PATH"
printf '%s\n' 'original = true' >"$CODEX_HOME/config.toml"
source "$1"
gpteasy <<<"1" >/dev/null
before=$(sha256sum "$CODEX_HOME/config.toml")
backup=$(find "$CODEX_HOME/.gpteasy-shell/shell-restore" -name config.toml -print -quit)
mkfifo "$workspace/input"
exec 7<>"$workspace/input"
(gpteasy restore <"$workspace/input" >"$workspace/output" 2>&1) &
restore_pid=$!
ready=0
for attempt in $(seq 1 100); do
    if grep -Fq '警告：恢复可能覆盖' "$workspace/output"; then ready=1; break; fi
    sleep 0.05
done
[[ "$ready" == 1 ]]
printf '%s\n' 'changed = true' >"$backup"
printf '%s\n' y >&7
if wait "$restore_pid"; then exit 1; fi
exec 7>&-
grep -Fq '恢复点在确认期间发生变化' "$workspace/output"
grep -Fq 'stage=restore_precommit catalog_state=backup_rejected' "$workspace/output"
[[ "$before" == "$(sha256sum "$CODEX_HOME/config.toml")" ]]
[[ ! -d "$CODEX_HOME/.gpteasy-shell/lock/active" ]]
"#,
        );
    }
}

#[test]
fn shell_snapshots_survive_process_death_between_and_after_catalog_commits() {
    let fixture = ExportFixture::new();
    fixture.insert_provider(
        "11111111-1111-4111-8111-111111111111",
        "Alpha",
        "https://alpha.example/v1",
        "private-key",
        "alpha-model",
        1,
    );
    for shell in shell_matrix_targets() {
        let destination = fixture.temp.path().join(format!("crash-{shell:?}"));
        fixture
            .application
            .export_linux_script(shell, &destination, false)
            .unwrap();
        run_shell_black_box(
            shell,
            &destination,
            r#"
set -euo pipefail
workspace=$(mktemp -d)
trap 'rm -rf -- "$workspace"' EXIT
mkdir -m 700 "$workspace/bin"
make_compatible_codex "$workspace/bin/codex"
cat >"$workspace/bin/mv" <<'CRASH_MV'
#!/bin/sh
case "$GPTEASY_CRASH:$*" in
    after_catalog:*'.model-catalog.'*|after_config:*'.config.toml.gpteasy.'*)
        /usr/bin/mv "$@" || exit 1
        kill -KILL "$PPID"
        exit 0
        ;;
esac
exec /usr/bin/mv "$@"
CRASH_MV
chmod 700 "$workspace/bin/mv"
export PATH="$workspace/bin:$PATH"
source "$1"
for GPTEASY_CRASH in after_catalog after_config; do
    export GPTEASY_CRASH CODEX_HOME="$workspace/$GPTEASY_CRASH"
    mkdir -m 700 "$CODEX_HOME"
    printf '%s\n' 'original = true' >"$CODEX_HOME/config.toml"
    cp "$CODEX_HOME/config.toml" "$workspace/original"
    if "$2" "$1" <<<"1" >"$workspace/output" 2>&1; then exit 1; fi
    [[ -d "$CODEX_HOME/.gpteasy-shell/lock/active" ]]
    catalog=$(find "$CODEX_HOME/.gpteasy-shell/model-catalogs" -type f -name '*.json' -print -quit)
    [[ -s "$catalog" ]]
    grep -Fq '"context_window": 128000' "$catalog"
    [[ $(find "$CODEX_HOME/.gpteasy-shell/shell-restore" -name config.toml | wc -l) -eq 1 ]]
    if [[ "$GPTEASY_CRASH" == after_catalog ]]; then
        cmp -s "$workspace/original" "$CODEX_HOME/config.toml"
    else
        grep -Fq '# GPTEasy schema-version: 2' "$CODEX_HOME/config.toml"
        grep -Fq "$catalog" "$CODEX_HOME/config.toml"
    fi
    gpteasy unlock <<<"y" >"$workspace/output" 2>&1
    [[ ! -d "$CODEX_HOME/.gpteasy-shell/lock/active" ]]
    export GPTEASY_CRASH=none
    gpteasy restore <<<"y" >"$workspace/output" 2>&1
    cmp -s "$workspace/original" "$CODEX_HOME/config.toml"
    [[ -s "$catalog" ]]
done
"#,
        );
    }
}

struct ExportFixture {
    temp: TempDir,
    store: StateStore,
    application: ProviderApplication,
}

const SHELL_FIXTURE_SETUP: &str = r###"umask 077
trap 'printf "black-box failed at line %s\n" "$LINENO" >&2' ERR

make_compatible_codex() {
    cat >"$1" <<'GPTEASY_TEST_NATIVE_CODEX'
#!/bin/sh
case "$1" in
  --version) printf '%s\n' 'codex-cli 0.147.0'; exit 0 ;;
  app-server) [ "$2" = '--listen' ] || exit 2 ;;
  *) exit 2 ;;
esac
[ -f "$CODEX_HOME/catalog.json" ] || exit 3
grep -Fq '"slug": "gpteasy-catalog-schema-probe-v1"' "$CODEX_HOME/catalog.json" || exit 3
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{"userAgent":"fixture-native-linux"}}' ;;
    *'"method":"model/list"'*)
      printf '%s\n' '{"id":2,"result":{"data":[{"model":"gpteasy-catalog-schema-probe-v1","defaultReasoningEffort":"high","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"medium"},{"reasoningEffort":"high"},{"reasoningEffort":"xhigh"}]}],"nextCursor":null}}'
      ;;
  esac
done
GPTEASY_TEST_NATIVE_CODEX
    chmod 700 "$1"
}
"###;

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
        .write_all(SHELL_FIXTURE_SETUP.as_bytes())
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
        self.insert_provider_with_models(
            id,
            name,
            base_url,
            api_key,
            default_model,
            sort_order,
            &[default_model],
        );
    }

    fn insert_provider_with_models(
        &self,
        id: &str,
        name: &str,
        base_url: &str,
        api_key: &str,
        default_model: &str,
        sort_order: i64,
        models: &[&str],
    ) {
        let connection = Connection::open(self.store.paths().database()).expect("open state");
        let fingerprint = combination_fingerprint(base_url, default_model, api_key);
        connection
            .execute(
                "INSERT INTO providers (
                    id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint, sort_order
            ) VALUES (?1, ?2, ?3, ?4, ?5, '1786800000', ?7, ?6)",
                params![
                    id,
                    name,
                    base_url,
                    api_key,
                    default_model,
                    sort_order,
                    fingerprint
                ],
            )
            .expect("insert verified provider fixture");
        connection
            .execute(
                "INSERT INTO provider_model_catalog(
                     provider_id, verification_fingerprint, models_json
                 ) VALUES (?1, ?3, ?2)",
                params![
                    id,
                    serde_json::to_string(models).expect("encode models"),
                    fingerprint
                ],
            )
            .expect("insert provider model catalog fixture");
    }
}

fn combination_fingerprint(base_url: &str, model: &str, api_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-provider-combination-v1\0");
    hasher.update(base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(model.as_bytes());
    hasher.update(b"\0");
    hasher.update(api_key.as_bytes());
    format!("{:x}", hasher.finalize())
}
