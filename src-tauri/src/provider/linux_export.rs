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
}

impl LinuxShell {
    pub(crate) fn suggested_file_name(self) -> &'static str {
        match self {
            Self::Bash => "gpteasy.sh",
        }
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
    let script = render_bash(&export_id, &providers);
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

fn render_bash(export_id: &str, providers: &[catalog::ProviderRecord]) -> String {
    let mut script = String::from(
        "#!/usr/bin/env bash\n\
# GPTEasy Bash 4+ Linux provider snapshot. This file contains sensitive credentials.\n\
gpteasy__schema_version='1'\n",
    );
    script.push_str(&format!(
        "gpteasy__export_id={}\ngpteasy__provider_count={}\n\n",
        shell_quote(export_id),
        providers.len()
    ));
    script.push_str("gpteasy__provider_id() {\n    case \"$1\" in\n");
    for (index, provider) in providers.iter().enumerate() {
        script.push_str(&format!(
            "        {}) printf '%s\\n' {} ;;\n",
            index + 1,
            shell_quote(&provider.summary.id)
        ));
    }
    script.push_str("        *) return 1 ;;\n    esac\n}\n\n");
    script.push_str("gpteasy__provider_name() {\n    case \"$1\" in\n");
    for provider in providers {
        script.push_str(&format!(
            "        {}) printf '%s\\n' {} ;;\n",
            shell_pattern(&provider.summary.id),
            shell_quote(&provider.summary.name)
        ));
    }
    script.push_str("        *) return 1 ;;\n    esac\n}\n\n");
    script.push_str("gpteasy__provider_model() {\n    case \"$1\" in\n");
    for provider in providers {
        script.push_str(&format!(
            "        {}) printf '%s\\n' {} ;;\n",
            shell_pattern(&provider.summary.id),
            shell_quote(&provider.summary.default_model)
        ));
    }
    script.push_str("        *) return 1 ;;\n    esac\n}\n\n");
    script.push_str("gpteasy__provider_base_url() {\n    case \"$1\" in\n");
    for provider in providers {
        script.push_str(&format!(
            "        {}) printf '%s\\n' {} ;;\n",
            shell_pattern(&provider.summary.id),
            shell_quote(&provider.summary.base_url)
        ));
    }
    script.push_str("        *) return 1 ;;\n    esac\n}\n\n");
    script.push_str("gpteasy__print_credential() {\n    case \"$1\" in\n");
    for provider in providers {
        script.push_str(&format!(
            "        {}) printf '%s' {} ;;\n",
            shell_pattern(&provider.summary.id),
            shell_quote(&provider.api_key)
        ));
    }
    script.push_str("        *) return 1 ;;\n    esac\n}\n");
    script.push_str("\ngpteasy__print_block() {\n    case \"$1\" in\n");
    for (index, provider) in providers.iter().enumerate() {
        let credential_relative = format!(
            ".gpteasy-shell/credentials/{export_id}/{}.token",
            provider.summary.id
        );
        let auth_script = format!("cat -- \"${{CODEX_HOME:-$HOME/.codex}}/{credential_relative}\"");
        script.push_str(&format!(
            "        {})\n            cat <<'GPTEASY_BLOCK_{}'\n",
            shell_pattern(&provider.summary.id),
            index + 1
        ));
        script.push_str("# >>> GPTEasy managed provider >>>\n");
        script.push_str("# GPTEasy schema-version: 1\n");
        script.push_str(&format!(
            "# GPTEasy provider-id: {}\n# GPTEasy source-id: {export_id}\n# GPTEasy credential-file: {credential_relative}\n",
            provider.summary.id
        ));
        script.push_str(&format!(
            "model = {}\nmodel_provider = \"gpteasy\"\nmodel_providers.gpteasy.name = {}\nmodel_providers.gpteasy.base_url = {}\nmodel_providers.gpteasy.wire_api = \"responses\"\nmodel_providers.gpteasy.supports_websockets = false\nmodel_providers.gpteasy.requires_openai_auth = false\nmodel_providers.gpteasy.auth.command = \"sh\"\nmodel_providers.gpteasy.auth.args = [\"-c\", {}]\n",
            toml_string(&provider.summary.default_model),
            toml_string(&provider.summary.name),
            toml_string(&provider.summary.base_url),
            toml_string(&auth_script),
        ));
        script.push_str(&format!(
            "# <<< GPTEasy managed provider <<<\nGPTEASY_BLOCK_{}\n            ;;\n",
            index + 1
        ));
    }
    script.push_str("        *) return 1 ;;\n    esac\n}\n");
    script.push_str(include_str!("bash_runtime.sh"));
    script
}

fn validate_snapshot(providers: &[catalog::ProviderRecord]) -> Result<(), LinuxExportFailure> {
    let valid = providers.iter().all(|provider| {
        Uuid::parse_str(&provider.summary.id).is_ok()
            && !provider.api_key.contains(['\0', '\r', '\n'])
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

fn toml_string(value: &str) -> String {
    toml_edit::Value::from(value).to_string()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_pattern(value: &str) -> String {
    shell_quote(value)
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
