use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::StateStore;

use super::catalog;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxShell {
    Bash,
    Zsh,
}

impl LinuxShell {
    fn definition(self) -> &'static ShellDefinition {
        match self {
            Self::Bash => &BASH_DEFINITION,
            Self::Zsh => &ZSH_DEFINITION,
        }
    }

    pub(crate) fn suggested_file_name(self) -> &'static str {
        self.definition().suggested_file_name
    }

    pub(crate) fn executable(self) -> &'static str {
        self.definition().executable
    }

    pub(crate) fn display_name(self) -> &'static str {
        self.definition().display_name
    }

    pub(crate) fn extension(self) -> &'static str {
        self.definition().extension
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxExportFailureCategory {
    NoVerifiedProviders,
    OverwriteConfirmationRequired,
    UnsafeDestination,
    StateUnavailable,
    WriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxExportFailure {
    pub category: LinuxExportFailureCategory,
    pub message_id: &'static str,
}

impl LinuxExportFailure {
    fn new(category: LinuxExportFailureCategory, message_id: &'static str) -> Self {
        Self {
            category,
            message_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinuxExportResult {
    pub export_id: String,
    pub provider_count: usize,
    pub suggested_file_name: &'static str,
}

struct ShellSyntax {
    setup: &'static str,
    function_options: &'static str,
    process_id: &'static str,
    select_read: &'static str,
    restore_read: &'static str,
    unlock_read: &'static str,
    direct_execution: &'static str,
}

struct ShellDefinition {
    suggested_file_name: &'static str,
    executable: &'static str,
    display_name: &'static str,
    extension: &'static str,
    syntax: ShellSyntax,
}

const BASH_DEFINITION: ShellDefinition = ShellDefinition {
    suggested_file_name: "gpteasy.sh",
    executable: "bash",
    display_name: "Bash 4+",
    extension: "sh",
    syntax: ShellSyntax {
        setup: "case ${BASH_SOURCE[0]} in\n    /*) gpteasy__script_path=${BASH_SOURCE[0]} ;;\n    *) gpteasy__script_path=$PWD/${BASH_SOURCE[0]} ;;\nesac",
        function_options: "",
        process_id: "    process_id=${BASHPID:-$$}",
        select_read: "    read -r -p '请选择供应商编号，或输入 q 取消：' choice",
        restore_read: "    read -r -p '确认恢复？[y/N] ' choice",
        unlock_read: "    read -r -p '确认删除该失效锁？[y/N] ' choice",
        direct_execution: "if [[ \"${BASH_SOURCE[0]}\" == \"$0\" ]]; then\n    gpteasy \"$@\"\nfi",
    },
};

const ZSH_DEFINITION: ShellDefinition = ShellDefinition {
    suggested_file_name: "gpteasy.zsh",
    executable: "zsh",
    display_name: "Zsh 5+",
    extension: "zsh",
    syntax: ShellSyntax {
        setup: "gpteasy__script_path=${(%):-%x}\ncase $gpteasy__script_path in\n    /*) ;;\n    *) gpteasy__script_path=$PWD/$gpteasy__script_path ;;\nesac",
        function_options: "    emulate -L zsh\n    setopt local_options nonomatch pipefail",
        process_id: "    if ! zmodload zsh/system 2>/dev/null; then\n        rmdir -- \"$active\" 2>/dev/null || true\n        return 1\n    fi\n    process_id=${sysparams[pid]}",
        select_read: "    read -r 'choice?请选择供应商编号，或输入 q 取消：'",
        restore_read: "    read -r 'choice?确认恢复？[y/N] '",
        unlock_read: "    read -r 'choice?确认删除该失效锁？[y/N] '",
        direct_execution: "if [[ \"$ZSH_EVAL_CONTEXT\" == toplevel ]]; then\n    gpteasy \"$@\"\nfi",
    },
};

pub(super) fn export(
    state_store: &StateStore,
    shell: LinuxShell,
    destination: &Path,
    confirm_overwrite: bool,
) -> Result<LinuxExportResult, LinuxExportFailure> {
    let providers = catalog::list_provider_records(state_store).map_err(|_| {
        LinuxExportFailure::new(
            LinuxExportFailureCategory::StateUnavailable,
            "linux_export.state_unavailable",
        )
    })?;
    if providers.is_empty() {
        return Err(LinuxExportFailure::new(
            LinuxExportFailureCategory::NoVerifiedProviders,
            "linux_export.no_verified_providers",
        ));
    }
    validate_snapshot(&providers)?;
    let original = read_destination(destination)?;
    if original.is_some() && !confirm_overwrite {
        return Err(LinuxExportFailure::new(
            LinuxExportFailureCategory::OverwriteConfirmationRequired,
            "linux_export.overwrite_confirmation_required",
        ));
    }

    let export_id = Uuid::new_v4().to_string();
    let script = render(shell, &export_id, &providers);
    atomic_write(destination, script.as_bytes(), original.as_deref())?;
    Ok(LinuxExportResult {
        export_id,
        provider_count: providers.len(),
        suggested_file_name: shell.suggested_file_name(),
    })
}

fn read_destination(destination: &Path) -> Result<Option<Vec<u8>>, LinuxExportFailure> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) if !metadata.file_type().is_file() => Err(unsafe_destination()),
        Ok(_) => fs::read(destination).map(Some).map_err(|_| write_failed()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(write_failed()),
    }
}

fn render(shell: LinuxShell, export_id: &str, providers: &[catalog::ProviderRecord]) -> String {
    let mut script = format!(
        "#!/usr/bin/env {}\n# GPTEasy {} Linux provider snapshot. This file contains sensitive credentials.\ngpteasy__schema_version='1'\n",
        shell.executable(),
        shell.display_name(),
    );
    script.push_str(&format!(
        "gpteasy__export_id={}\n\n",
        shell_quote(export_id)
    ));
    script.push_str("# 供应商目录。可脱离 GPTEasy 手工维护：每行依次为供应商 ID、名称、服务地址、默认模型、API Key，并以 Tab 分隔；可使用空行和 # 注释。字段不可包含 Tab 或换行。\n");
    script.push_str("gpteasy__provider_catalog() {\n    cat <<'GPTEASY_PROVIDER_CATALOG'\n");
    for provider in providers {
        script.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            provider.summary.id,
            provider.summary.name,
            provider.summary.base_url,
            provider.summary.default_model,
            provider.api_key,
        ));
    }
    script.push_str("GPTEASY_PROVIDER_CATALOG\n}\n");
    script.push_str(
        "gpteasy__provider_count=$(gpteasy__provider_catalog | awk 'NF && $1 !~ /^#/ { count += 1 } END { print count + 0 }')\n\n",
    );
    script.push_str(&format!(
        r#"
gpteasy__export_credential_directory='.gpteasy-shell/credentials/{export_id}'

gpteasy__provider_id_is_safe() {{
    [[ "$1" =~ ^[[:xdigit:]]{{8}}-[[:xdigit:]]{{4}}-[[:xdigit:]]{{4}}-[[:xdigit:]]{{4}}-[[:xdigit:]]{{12}}$ ]]
}}

gpteasy__toml_string() {{
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/^/"/; s/$/"/'
}}

gpteasy__print_block() {{
    local provider_id=$1 name model base_url credential_relative
    gpteasy__provider_id_is_safe "$provider_id" || return 1
    name=$(gpteasy__provider_name "$provider_id") || return 1
    model=$(gpteasy__provider_model "$provider_id") || return 1
    base_url=$(gpteasy__provider_base_url "$provider_id") || return 1
    [[ -n "$name" && -n "$model" && -n "$base_url" ]] || return 1
    credential_relative="$gpteasy__export_credential_directory/$provider_id.token"
    printf '%s\n' '# >>> GPTEasy managed provider >>>'
    printf '%s\n' '# GPTEasy schema-version: 1'
    printf '# GPTEasy provider-id: %s\n' "$provider_id"
    printf '# GPTEasy source-id: %s\n' "$gpteasy__export_id"
    printf '# GPTEasy credential-file: %s\n' "$credential_relative"
    printf 'model = %s\n' "$(gpteasy__toml_string "$model")"
    printf '%s\n' 'model_provider = "gpteasy"'
    printf 'model_providers.gpteasy.name = %s\n' "$(gpteasy__toml_string "$name")"
    printf 'model_providers.gpteasy.base_url = %s\n' "$(gpteasy__toml_string "$base_url")"
    printf '%s\n' 'model_providers.gpteasy.wire_api = "responses"'
    printf '%s\n' 'model_providers.gpteasy.supports_websockets = false'
    printf '%s\n' 'model_providers.gpteasy.auth.command = "sh"'
    printf 'model_providers.gpteasy.auth.args = ["-c", '\''cat -- "${{CODEX_HOME:-$HOME/.codex}}/%s"'\'']\n' "$credential_relative"
    printf '%s\n' '# <<< GPTEasy managed provider <<<'
}}
"#
    ));
    script.push_str(&render_runtime(shell));
    script
}

fn render_runtime(shell: LinuxShell) -> String {
    let definition = shell.definition();
    let syntax = &definition.syntax;
    include_str!("shell_runtime.sh")
        .replace("{{GPTEASY_SHELL_SETUP}}", syntax.setup)
        .replace("{{GPTEASY_FUNCTION_OPTIONS}}", syntax.function_options)
        .replace("{{GPTEASY_PROCESS_ID}}", syntax.process_id)
        .replace("{{GPTEASY_SELECT_READ}}", syntax.select_read)
        .replace("{{GPTEASY_RESTORE_READ}}", syntax.restore_read)
        .replace("{{GPTEASY_UNLOCK_READ}}", syntax.unlock_read)
        .replace("{{GPTEASY_SHELL_LABEL}}", shell.display_name())
        .replace("{{GPTEASY_DIRECT_EXECUTION}}", syntax.direct_execution)
}

fn validate_snapshot(providers: &[catalog::ProviderRecord]) -> Result<(), LinuxExportFailure> {
    let valid = providers.iter().all(|provider| {
        Uuid::parse_str(&provider.summary.id).is_ok()
            && [
                &provider.summary.name,
                &provider.summary.base_url,
                &provider.summary.default_model,
                &provider.api_key,
            ]
            .into_iter()
            .all(|value| !value.contains(['\0', '\r', '\n', '\t']))
    });
    if valid {
        Ok(())
    } else {
        Err(LinuxExportFailure::new(
            LinuxExportFailureCategory::StateUnavailable,
            "linux_export.snapshot_invalid",
        ))
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn atomic_write(
    destination: &Path,
    bytes: &[u8],
    original: Option<&[u8]>,
) -> Result<(), LinuxExportFailure> {
    let parent = destination.parent().ok_or_else(unsafe_destination)?;
    if !parent.is_dir() {
        return Err(unsafe_destination());
    }
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(unsafe_destination)?;
    let temporary = parent.join(format!(".{file_name}.gpteasy-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|_| write_failed())?;
        file.write_all(bytes).map_err(|_| write_failed())?;
        file.sync_all().map_err(|_| write_failed())?;
        drop(file);
        if read_destination(destination)?.as_deref() != original {
            return Err(LinuxExportFailure::new(
                LinuxExportFailureCategory::WriteFailed,
                "linux_export.concurrent_modification",
            ));
        }
        atomic_replace(destination, &temporary, original.is_some())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn atomic_replace(
    destination: &Path,
    temporary: &Path,
    destination_exists: bool,
) -> Result<(), LinuxExportFailure> {
    if !destination_exists {
        return fs::rename(temporary, destination).map_err(|_| write_failed());
    }
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            temporary.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        Err(write_failed())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(
    destination: &Path,
    temporary: &Path,
    _destination_exists: bool,
) -> Result<(), LinuxExportFailure> {
    fs::rename(temporary, destination).map_err(|_| write_failed())?;
    // The rename is the commit point; a later durability warning cannot preserve the original.
    if let Some(parent) = destination.parent()
        && let Ok(directory) = fs::File::open(parent)
    {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn unsafe_destination() -> LinuxExportFailure {
    LinuxExportFailure::new(
        LinuxExportFailureCategory::UnsafeDestination,
        "linux_export.unsafe_destination",
    )
}

fn write_failed() -> LinuxExportFailure {
    LinuxExportFailure::new(
        LinuxExportFailureCategory::WriteFailed,
        "linux_export.write_failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_runtime_limits_shell_divergence_to_declared_syntax_slots() {
        let template = include_str!("shell_runtime.sh");
        let slots = [
            "{{GPTEASY_SHELL_SETUP}}",
            "{{GPTEASY_FUNCTION_OPTIONS}}",
            "{{GPTEASY_PROCESS_ID}}",
            "{{GPTEASY_SELECT_READ}}",
            "{{GPTEASY_RESTORE_READ}}",
            "{{GPTEASY_UNLOCK_READ}}",
            "{{GPTEASY_SHELL_LABEL}}",
            "{{GPTEASY_DIRECT_EXECUTION}}",
        ];

        assert_eq!(template.matches("{{GPTEASY_").count(), slots.len());
        for slot in slots {
            assert_eq!(template.matches(slot).count(), 1, "unexpected slot {slot}");
        }
        assert!(!template.to_ascii_lowercase().contains("bash"));
        assert!(!template.to_ascii_lowercase().contains("zsh"));

        let bash = render_runtime(LinuxShell::Bash);
        let zsh = render_runtime(LinuxShell::Zsh);
        assert!(!bash.contains("{{GPTEASY_"));
        assert!(!zsh.contains("{{GPTEASY_"));
        assert!(bash.contains("read -r -p '请选择供应商编号"));
        assert!(zsh.contains("read -r 'choice?请选择供应商编号"));
        assert!(!bash.contains("emulate -L zsh"));
        assert!(zsh.contains("emulate -L zsh"));
    }
}
