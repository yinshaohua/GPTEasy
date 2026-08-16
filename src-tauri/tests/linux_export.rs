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
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn bash_export_captures_every_verified_provider_in_catalog_order() {
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
    assert!(script.find("Alpha Provider") < script.find("Beta Provider"));
    assert!(!script.contains("OpenAI 登录模式"));
    assert!(
        script.contains("printf '  %s) %s (%s)%s\\n' \"$index\" \"$name\" \"$model\" \"$marker\"")
    );
    assert!(!script.contains("printf '  %s) %s (%s)%\""));
    assert!(!script.contains("provider_irovider_id"));
}

#[test]
fn zsh_export_captures_every_verified_provider_in_catalog_order() {
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
    assert!(script.find("Alpha Provider") < script.find("Beta Provider"));
    assert!(!script.contains("OpenAI 登录模式"));
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
        "https://alpha.example/v1",
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
fn shell_snapshots_check_codex_before_installing_only_the_selected_provider() {
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
mkdir -p -- "$codex_home" "$fake_bin"
printf '%s\n' 'custom_setting = true' >"$codex_home/config.toml"
printf '%s\n' '{"tokens":{"access_token":"keep-me"}}' >"$codex_home/auth.json"
config_before=$(sha256sum "$codex_home/config.toml")
auth_before=$(sha256sum "$codex_home/auth.json")
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"
missing=$(PATH="$fake_bin:/usr/bin:/bin" gpteasy <<<"1" 2>&1 || true)
[[ "$missing" == *'未找到 Codex CLI，请先安装 0.147.0 或更高版本'* ]]
[[ "$missing" != *'版本过低'* ]]
[[ "$config_before" == "$(sha256sum "$codex_home/config.toml")" ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
[[ ! -e "$codex_home/.gpteasy-shell" ]]
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
[[ "$config_before" == "$(sha256sum "$codex_home/config.toml")" ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
[[ ! -e "$codex_home/.gpteasy-shell" ]]

cat >"$fake_bin/codex" <<'SUPPORTED_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.147.0'
SUPPORTED_CODEX
chmod 700 "$fake_bin/codex"
menu=$(gpteasy <<<"1")
[[ "$menu" == *'Alpha Provider (alpha-model)'* ]]
grep -Fq '# GPTEasy schema-version: 1' "$codex_home/config.toml"
grep -Fq '# GPTEasy provider-id: 11111111-1111-4111-8111-111111111111' "$codex_home/config.toml"
grep -Fq '# GPTEasy source-id:' "$codex_home/config.toml"
grep -Fq 'model_providers.gpteasy.auth.command = "sh"' "$codex_home/config.toml"
grep -Fq 'custom_setting = true' "$codex_home/config.toml"
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
fn shell_snapshots_distinguish_current_updated_and_legacy_managed_blocks() {
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
before=$(sha256sum "$codex_home/config.toml")
if (gpteasy <<<"2" >/dev/null 2>&1); then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]

cp -- "$workspace/current-config" "$codex_home/config.toml"
sed -i 's|^model_providers.gpteasy.auth.args = .*|model_providers.gpteasy.auth.args = ["-c", "printf unsafe"]|' "$codex_home/config.toml"
before=$(sha256sum "$codex_home/config.toml")
if (gpteasy <<<"2" >/dev/null 2>&1); then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]

cp -- "$workspace/current-config" "$codex_home/config.toml"
credential=$(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token')
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
before=$(sha256sum "$codex_home/config.toml")
if (gpteasy <<<"2" >/dev/null 2>&1); then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]

printf '%s\n' '# >>> GPTEasy managed provider >>>' 'model = "broken"' >"$codex_home/config.toml"
before=$(sha256sum "$codex_home/config.toml")
if (gpteasy <<<"2" >/dev/null 2>&1); then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]

printf '%s\n' 'model = "external"' 'model_provider = "external"' >"$codex_home/config.toml"
before=$(sha256sum "$codex_home/config.toml")
if (gpteasy <<<"2" >/dev/null 2>&1); then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]
"##,
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
if (gpteasy <<<"1" >/dev/null 2>&1); then
    printf '%s\n' 'group-writable Codex home was accepted' >&2
    exit 1
fi
[[ "$unsafe_before" == "$(sha256sum "$unsafe_home/config.toml")" ]]
[[ ! -e "$unsafe_home/.gpteasy-shell" ]]

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
if (gpteasy current >/dev/null 2>&1); then
    printf '%s\n' 'unsafe credential permissions were accepted' >&2
    exit 1
fi
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
        "{label} black-box test failed\nstdout:\n{}\nstderr:\n{}",
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
        connection
            .execute(
                "INSERT INTO providers (
                    id, name, base_url, api_key, default_model, verified_at,
                    verification_fingerprint, sort_order
                 ) VALUES (?1, ?2, ?3, ?4, ?5, '1786800000', 'verified', ?6)",
                params![id, name, base_url, api_key, default_model, sort_order],
            )
            .expect("insert verified provider fixture");
    }
}
