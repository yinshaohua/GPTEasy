use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::codex_config::STATUS_LINE_TOML;
use crate::state::StateStore;

use super::catalog;
use super::model_catalog;
use super::reasoning;

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
        "#!/usr/bin/env {}\n# GPTEasy {} Linux provider snapshot. This file contains sensitive credentials.\ngpteasy__schema_version='2'\ngpteasy__catalog_protocol='codex-model-catalog-v1'\ngpteasy__catalog_policy='common-reasoning-selector-v1'\n",
        shell.executable(),
        shell.display_name(),
    );
    let status_line = STATUS_LINE_TOML
        .lines()
        .map(|line| format!("    printf '%s\\n' {}", shell_quote(line)))
        .collect::<Vec<_>>()
        .join("\n");
    script.push_str(&format!(
        "gpteasy__export_id={}\n\n",
        shell_quote(export_id)
    ));
    script.push_str("# 供应商目录。可脱离 GPTEasy 手工维护显示名称；关键组合变更必须重新验证和导出。API Key 仅用于明确切换时写入私有凭据工件；模型目录载荷不包含凭据。字段不可包含 Tab 或换行。\n");
    script.push_str("gpteasy__provider_catalog() {\n    cat <<'GPTEASY_PROVIDER_CATALOG'\n");
    let mut payload_functions = String::new();
    for provider in providers {
        let payload =
            model_catalog::render(&provider.discovered_models, &provider.summary.default_model)
                .expect("validated provider model catalog");
        let artifact_id = Uuid::new_v4().to_string();
        let payload_sha256 = sha256_hex(&payload);
        script.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            provider.summary.id,
            provider.summary.name,
            provider.summary.base_url,
            provider.summary.default_model,
            provider.api_key,
            provider
                .reasoning_selection
                .effort
                .as_deref()
                .unwrap_or("-"),
            artifact_id,
            payload_sha256,
            provider.verification_fingerprint,
        ));
        payload_functions.push_str(&format!(
            "gpteasy__provider_catalog_payload_{}() {{ printf '%s' {}; }}\n",
            provider.summary.id,
            shell_quote(std::str::from_utf8(&payload).expect("catalog JSON is UTF-8")),
        ));
        payload_functions.push_str(&format!(
            "gpteasy__provider_catalog_binding_{}() {{ printf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' {} {} {} {} {} {} {} {}; }}\n",
            provider.summary.id,
            shell_quote(&provider.summary.id),
            shell_quote(&provider.summary.default_model),
            shell_quote(&artifact_id),
            shell_quote(&payload_sha256),
            shell_quote(&provider.verification_fingerprint),
            shell_quote(export_id),
            shell_quote("codex-model-catalog-v1"),
            shell_quote(model_catalog::REASONING_SELECTOR_POLICY),
        ));
    }
    script.push_str("GPTEASY_PROVIDER_CATALOG\n}\n");
    script.push_str(&payload_functions);
    script.push_str("\ngpteasy__provider_catalog_payload() {\n    case \"$1\" in\n");
    for provider in providers {
        script.push_str(&format!(
            "        {}) gpteasy__provider_catalog_payload_{} ;;\n",
            provider.summary.id, provider.summary.id,
        ));
    }
    script.push_str("        *) return 1 ;;\n    esac\n}\n");
    script.push_str("\ngpteasy__provider_catalog_binding() {\n    case \"$1\" in\n");
    for provider in providers {
        script.push_str(&format!(
            "        {}) gpteasy__provider_catalog_binding_{} ;;\n",
            provider.summary.id, provider.summary.id,
        ));
    }
    script.push_str("        *) return 1 ;;\n    esac\n}\n");
    script.push_str(&format!("gpteasy__provider_count={}\n\n", providers.len()));
    let probe_payload = model_catalog::render(
        &["gpteasy-catalog-schema-probe-v1".to_owned()],
        "gpteasy-catalog-schema-probe-v1",
    )
    .expect("fixed probe model catalog");
    script.push_str(&format!(
        "gpteasy__catalog_probe_payload() {{ printf '%s' {}; }}\n",
        shell_quote(std::str::from_utf8(&probe_payload).expect("probe JSON is UTF-8")),
    ));
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
    local provider_id=$1 name model reasoning_effort base_url credential_relative catalog_path artifact_id catalog_sha256 verification_fingerprint
    gpteasy__provider_id_is_safe "$provider_id" || return 1
    name=$(gpteasy__provider_name "$provider_id") || return 1
    model=$(gpteasy__provider_model "$provider_id") || return 1
    reasoning_effort=$(gpteasy__provider_reasoning_effort "$provider_id") || return 1
    base_url=$(gpteasy__provider_base_url "$provider_id") || return 1
    artifact_id=$(gpteasy__provider_catalog_artifact "$provider_id") || return 1
    catalog_sha256=$(gpteasy__provider_catalog_sha256 "$provider_id") || return 1
    verification_fingerprint=$(gpteasy__provider_verification_fingerprint "$provider_id") || return 1
    catalog_path=${{gpteasy__model_catalog_path:-}}
    [[ -n "$name" && -n "$model" && -n "$base_url" && -n "$catalog_path" ]] || return 1
    credential_relative="$gpteasy__export_credential_directory/$provider_id.token"
    printf '%s\n' '# >>> GPTEasy managed provider >>>'
    printf '%s\n' '# GPTEasy schema-version: 2'
    printf '# GPTEasy provider-id: %s\n' "$provider_id"
    printf '# GPTEasy source-id: %s\n' "$gpteasy__export_id"
    printf '# GPTEasy credential-file: %s\n' "$credential_relative"
    printf '# GPTEasy model-catalog-protocol: %s\n' "$gpteasy__catalog_protocol"
    printf '# GPTEasy model-catalog-policy: %s\n' "$gpteasy__catalog_policy"
    printf '# GPTEasy model-catalog-file: .gpteasy-shell/model-catalogs/%s/%s.json\n' "$gpteasy__export_id" "$artifact_id"
    printf '# GPTEasy model-catalog-artifact: %s\n' "$artifact_id"
    printf '# GPTEasy model-catalog-sha256: %s\n' "$catalog_sha256"
    printf '# GPTEasy model-catalog-provider-fingerprint: %s\n' "$verification_fingerprint"
    printf 'model = %s\n' "$(gpteasy__toml_string "$model")"
    if [[ -n "$reasoning_effort" ]]; then
        printf 'model_reasoning_effort = %s\n' "$(gpteasy__toml_string "$reasoning_effort")"
    fi
    printf '%s\n' 'model_provider = "gpteasy"'
    printf 'model_catalog_json = %s\n' "$(gpteasy__toml_string "$catalog_path")"
    printf 'model_providers.gpteasy.name = %s\n' "$(gpteasy__toml_string "$name")"
    printf 'model_providers.gpteasy.base_url = %s\n' "$(gpteasy__toml_string "$base_url")"
    printf '%s\n' 'model_providers.gpteasy.wire_api = "responses"'
    printf '%s\n' 'model_providers.gpteasy.supports_websockets = false'
    printf '%s\n' 'model_providers.gpteasy.auth.command = "sh"'
    printf 'model_providers.gpteasy.auth.args = ["-c", '\''cat -- "${{CODEX_HOME:-$HOME/.codex}}/%s"'\'']\n' "$credential_relative"
    printf '%s\n' '# <<< GPTEasy managed provider <<<'
    printf '%s\n' ''
{status_line}
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
                &provider.api_key,
                &provider.summary.default_model,
            ]
            .into_iter()
            .all(|value| !value.chars().any(char::is_control))
            && provider.verification_fingerprint
                == super::combination_fingerprint(
                    &provider.summary.base_url,
                    &provider.api_key,
                    &provider.summary.default_model,
                )
            && provider
                .reasoning_selection
                .effort
                .as_deref()
                .is_none_or(reasoning::is_valid_effort)
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

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
