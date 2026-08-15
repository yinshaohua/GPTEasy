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
fn bash_snapshot_sources_without_side_effects_and_rejects_unsafe_permissions() {
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
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export Bash snapshot");

    run_bash_black_box(
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
before=$(sha256sum "$codex_home/config.toml")
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
pushd "$workspace" >/dev/null
source ./gpteasy.sh
popd >/dev/null
after=$(sha256sum "$codex_home/config.toml")
[[ "$before" == "$after" ]]
[[ $(gpteasy help) == *'gpteasy current'* ]]
[[ $(gpteasy --help) == "$(gpteasy -h)" ]]
current=$(gpteasy current 2>&1 || true)
[[ "$current" == *'当前配置不包含可识别的 GPTEasy 管理区块'* ]]
chmod 644 "$script"
if gpteasy current >/dev/null 2>&1; then
    printf '%s\n' 'unsafe snapshot permissions were accepted' >&2
    exit 1
fi
gpteasy help >/dev/null
"#,
    );
}

#[test]
fn bash_snapshot_checks_codex_before_installing_only_the_selected_provider() {
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
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export Bash snapshot");

    run_bash_black_box(
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
cat >"$fake_bin/codex" <<'OLD_CODEX'
#!/usr/bin/env bash
printf '%s\n' 'codex-cli 0.146.0'
OLD_CODEX
chmod 700 "$fake_bin/codex"
export PATH="$fake_bin:$PATH"
export CODEX_HOME="$codex_home"
# shellcheck disable=SC1090
source "$script"
if gpteasy <<<"1" >/dev/null 2>&1; then
    printf '%s\n' 'unsupported Codex version was accepted' >&2
    exit 1
fi
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
mapfile -t credentials < <(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token')
[[ ${#credentials[@]} -eq 1 ]]
[[ $(cat "${credentials[0]}") == 'alpha-secret-key' ]]
[[ $(stat -c '%a' "${credentials[0]}") == '600' ]]
[[ $(find "$codex_home/.gpteasy-shell/shell-restore" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq 1 ]]
[[ $(gpteasy current) == *'Alpha Provider'* ]]
[[ $(gpteasy <<<"q") == *'Alpha Provider (alpha-model) [当前]'* ]]
"#,
    );
}

#[test]
fn bash_snapshot_restores_and_consumes_only_the_latest_of_five_restore_points() {
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
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export Bash snapshot");

    run_bash_black_box(
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
[[ "$auth_before" == "$(sha256sum "$codex_home/auth.json")" ]]

for choice in 2 1 2 1 2 1 2; do
    gpteasy <<<"$choice" >/dev/null
done
[[ $(find "$restore_root" -mindepth 1 -maxdepth 1 -type d | wc -l) -eq 5 ]]
! grep -R -Fq 'alpha-secret-key' "$restore_root"
! grep -R -Fq 'beta-secret-key' "$restore_root"
"#,
    );
}

#[test]
fn bash_snapshot_distinguishes_current_updated_and_legacy_managed_blocks() {
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
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export Bash snapshot");

    run_bash_black_box(
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
if gpteasy <<<"2" >/dev/null 2>&1; then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]

cp -- "$workspace/current-config" "$codex_home/config.toml"
sed -i 's|^model_providers.gpteasy.auth.args = .*|model_providers.gpteasy.auth.args = ["-c", "printf unsafe"]|' "$codex_home/config.toml"
before=$(sha256sum "$codex_home/config.toml")
if gpteasy <<<"2" >/dev/null 2>&1; then exit 1; fi
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

cp -- "$workspace/current-config" "$codex_home/config.toml"
sed -i 's/# GPTEasy schema-version: 1/# GPTEasy schema-version: 2/' "$codex_home/config.toml"
before=$(sha256sum "$codex_home/config.toml")
if gpteasy <<<"2" >/dev/null 2>&1; then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]

printf '%s\n' '# >>> GPTEasy managed provider >>>' 'model = "broken"' >"$codex_home/config.toml"
before=$(sha256sum "$codex_home/config.toml")
if gpteasy <<<"2" >/dev/null 2>&1; then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]

printf '%s\n' 'model = "external"' 'model_provider = "external"' >"$codex_home/config.toml"
before=$(sha256sum "$codex_home/config.toml")
if gpteasy <<<"2" >/dev/null 2>&1; then exit 1; fi
[[ "$before" == "$(sha256sum "$codex_home/config.toml")" ]]
"##,
    );
}

#[test]
fn bash_snapshot_preserves_safe_symlinks_and_rejects_hardlinks_and_concurrency() {
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
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export Bash snapshot");

    run_bash_black_box(
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
if gpteasy <<<"2" >/dev/null 2>&1; then
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
if gpteasy <<<"2" >/dev/null 2>&1; then
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
if gpteasy <<<"1" >/dev/null 2>&1; then
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
if gpteasy <<<"1" >/dev/null 2>&1; then
    printf '%s\n' 'hardlinked config was accepted' >&2
    exit 1
fi
[[ "$hardlink_before" == "$(sha256sum "$hardlink_home/config.toml")" ]]
"#,
    );
}

#[test]
fn bash_snapshot_reports_information_and_only_unlocks_confirmed_stale_shell_locks() {
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
    fixture
        .application
        .export_linux_script(LinuxShell::Bash, &destination, false)
        .expect("export Bash snapshot");

    run_bash_black_box(
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
[[ "$info" == *'Shell：Bash 4+'* ]]
[[ "$info" == *'供应商数量：1'* ]]
[[ "$info" == *'Codex CLI 最低版本：0.147.0'* ]]
[[ "$info" != *'alpha-secret-key'* ]]
[[ $(bash "$script" current) == *'Alpha Provider'* ]]

credential=$(find "$codex_home/.gpteasy-shell/credentials" -type f -name '*.token')
chmod 644 "$credential"
if gpteasy current >/dev/null 2>&1; then
    printf '%s\n' 'unsafe credential permissions were accepted' >&2
    exit 1
fi
gpteasy help >/dev/null
chmod 600 "$credential"

active="$codex_home/.gpteasy-shell/lock/active"
mkdir -m 700 -- "$active"
active_pid=$BASHPID
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
if gpteasy unlock <<<"y" >/dev/null 2>&1; then
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
if gpteasy unlock <<<"y" >/dev/null 2>&1; then
    printf '%s\n' 'desktop lock was removed by shell' >&2
    exit 1
fi
[[ -d "$active" ]]
"#,
    );
}

struct ExportFixture {
    temp: TempDir,
    store: StateStore,
    application: ProviderApplication,
}

fn run_bash_black_box(script: &Path, harness: &str) {
    #[cfg(windows)]
    let mut child = Command::new("wsl.exe")
        .args(["-d", "Ubuntu", "--"])
        .arg(std::env::var("GPTEASY_TEST_BASH").unwrap_or_else(|_| "bash".to_owned()))
        .args(["-s", "--", &windows_path_for_wsl(script)])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start WSL Bash black-box test");

    #[cfg(not(windows))]
    let mut child =
        Command::new(std::env::var("GPTEASY_TEST_BASH").unwrap_or_else(|_| "bash".to_owned()))
            .args(["-s", "--", script.to_str().expect("UTF-8 test path")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start Bash black-box test");

    child
        .stdin
        .take()
        .expect("Bash stdin")
        .write_all(harness.as_bytes())
        .expect("write Bash harness without credentials");
    let output = child
        .wait_with_output()
        .expect("wait for Bash black-box test");
    assert!(
        output.status.success(),
        "Bash black-box test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
fn windows_path_for_wsl(path: &Path) -> String {
    let windows_path = path.to_str().expect("UTF-8 test path").replace('\\', "/");
    let output = Command::new("wsl.exe")
        .args(["-d", "Ubuntu", "--", "wslpath", "-a", "-u", &windows_path])
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
