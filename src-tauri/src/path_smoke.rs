use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use thiserror::Error;

pub const PATH_SMOKE_COMMAND: &str = "phase1-path-smoke";
pub const PATH_SMOKE_SCHEMA: &str = "gpteasy.phase1.path-smoke.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathSmokeReport {
    pub run_id: String,
    pub os: String,
    pub arch: String,
    pub schema: &'static str,
    pub reopened: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PathSmokeMarker {
    run_id: String,
    os: String,
    arch: String,
    schema: String,
}

#[derive(Debug, Error)]
pub enum PathSmokeError {
    #[error("opaque run ID must be 1-64 ASCII letters, digits, or hyphens")]
    InvalidRunId,
    #[error("phase1-path-smoke accepts exactly one opaque run ID")]
    InvalidArguments,
    #[error("failed to resolve the application local data root")]
    ResolveStateRoot(#[source] tauri::Error),
    #[error("failed to create the fixed path smoke directory")]
    CreateDirectory(#[source] io::Error),
    #[error("failed to read the path smoke marker")]
    ReadMarker(#[source] io::Error),
    #[error("path smoke marker does not match the requested contract")]
    MarkerMismatch,
    #[error("failed to serialize the path smoke marker")]
    SerializeMarker(#[source] serde_json::Error),
    #[error("failed to create the path smoke temporary marker")]
    CreateTemporaryMarker(#[source] io::Error),
    #[error("failed to write the path smoke temporary marker")]
    WriteTemporaryMarker(#[source] io::Error),
    #[error("failed to commit the path smoke marker")]
    CommitMarker(#[source] io::Error),
}

pub(crate) fn parse_cli_args<I>(args: I) -> Result<Option<String>, PathSmokeError>
where
    I: IntoIterator<Item = OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(None),
        [command, run_id] if command == PATH_SMOKE_COMMAND => {
            let run_id = run_id.to_str().ok_or(PathSmokeError::InvalidArguments)?;
            validate_run_id(run_id)?;
            Ok(Some(run_id.to_owned()))
        }
        _ => Err(PathSmokeError::InvalidArguments),
    }
}

fn validate_run_id(run_id: &str) -> Result<(), PathSmokeError> {
    if (1..=64).contains(&run_id.len())
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Ok(())
    } else {
        Err(PathSmokeError::InvalidRunId)
    }
}

fn expected_marker(run_id: &str) -> PathSmokeMarker {
    PathSmokeMarker {
        run_id: run_id.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        schema: PATH_SMOKE_SCHEMA.to_owned(),
    }
}

fn report_from(marker: &PathSmokeMarker, reopened: bool) -> PathSmokeReport {
    PathSmokeReport {
        run_id: marker.run_id.clone(),
        os: marker.os.clone(),
        arch: marker.arch.clone(),
        schema: PATH_SMOKE_SCHEMA,
        reopened,
    }
}

fn read_existing_marker(
    marker_path: &Path,
    expected: &PathSmokeMarker,
) -> Result<Option<PathSmokeReport>, PathSmokeError> {
    let bytes = match fs::read(marker_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(PathSmokeError::ReadMarker(error)),
    };
    let marker: PathSmokeMarker =
        serde_json::from_slice(&bytes).map_err(|_| PathSmokeError::MarkerMismatch)?;
    if marker != *expected {
        return Err(PathSmokeError::MarkerMismatch);
    }

    Ok(Some(report_from(&marker, true)))
}

fn write_new_marker(
    marker_path: &Path,
    temporary_path: &Path,
    marker: &PathSmokeMarker,
) -> Result<PathSmokeReport, PathSmokeError> {
    let mut bytes = serde_json::to_vec(marker).map_err(PathSmokeError::SerializeMarker)?;
    bytes.push(b'\n');

    let mut temporary = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary_path)
        .map_err(PathSmokeError::CreateTemporaryMarker)?;
    if let Err(error) = temporary
        .write_all(&bytes)
        .and_then(|_| temporary.sync_all())
    {
        drop(temporary);
        let _ = fs::remove_file(temporary_path);
        return Err(PathSmokeError::WriteTemporaryMarker(error));
    }
    drop(temporary);

    if let Err(error) = fs::rename(temporary_path, marker_path) {
        let _ = fs::remove_file(temporary_path);
        return Err(PathSmokeError::CommitMarker(error));
    }

    Ok(report_from(marker, false))
}

pub fn run_path_smoke<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
) -> Result<PathSmokeReport, PathSmokeError> {
    validate_run_id(run_id)?;

    let smoke_root = app
        .path()
        .app_local_data_dir()
        .map_err(PathSmokeError::ResolveStateRoot)?
        .join("contract-smoke")
        .join("path");
    fs::create_dir_all(&smoke_root).map_err(PathSmokeError::CreateDirectory)?;

    let marker_path = smoke_root.join(format!("{run_id}.json"));
    if let Some(report) = read_existing_marker(&marker_path, &expected_marker(run_id))? {
        return Ok(report);
    }

    let temporary_path = smoke_root.join(format!("{run_id}.json.tmp"));
    write_new_marker(&marker_path, &temporary_path, &expected_marker(run_id))
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::Path};

    use serde_json::Value;
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tempfile::tempdir;

    use super::{
        parse_cli_args, run_path_smoke, PathSmokeError, PATH_SMOKE_COMMAND, PATH_SMOKE_SCHEMA,
    };

    fn mock_app_at(root: &Path) -> tauri::App<tauri::test::MockRuntime> {
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = root.to_string_lossy().into_owned();
        mock_builder().build(context).expect("build mock app")
    }

    #[test]
    fn path_smoke_rejects_non_opaque_ids_before_resolving_paths() {
        let temp = tempdir().expect("create temp directory");
        let app = mock_app_at(&temp.path().join("app-root"));
        let invalid_ids = [
            "",
            ".",
            "..",
            "contains/slash",
            r"contains\backslash",
            "contains.dot",
            "contains_underscore",
            "包含Unicode",
            &"a".repeat(65),
        ];

        for run_id in invalid_ids {
            assert!(
                matches!(
                    run_path_smoke(app.handle(), run_id),
                    Err(PathSmokeError::InvalidRunId)
                ),
                "unexpected result for {run_id:?}"
            );
        }
        assert!(!temp.path().join("app-root").exists());
    }

    #[test]
    fn path_smoke_accepts_boundary_ids_and_reopens_fixed_marker() {
        let temp = tempdir().expect("create temp directory");
        let app_root = temp.path().join("app-root");
        let app = mock_app_at(&app_root);
        let run_id = format!("A-{}", "9".repeat(62));

        let first = run_path_smoke(app.handle(), &run_id).expect("first path smoke");
        let second = run_path_smoke(app.handle(), &run_id).expect("second path smoke");

        assert_eq!(first.run_id, run_id);
        assert_eq!(first.schema, PATH_SMOKE_SCHEMA);
        assert!(!first.reopened);
        assert!(second.reopened);
        assert_eq!(
            std::fs::read_dir(app_root.join("contract-smoke/path"))
                .expect("read fixed path smoke directory")
                .count(),
            1
        );
    }

    #[test]
    fn report_serialization_has_only_non_sensitive_contract_fields() {
        let temp = tempdir().expect("create temp directory");
        let app = mock_app_at(&temp.path().join("private-user-root"));
        let report = run_path_smoke(app.handle(), "safe-123").expect("run path smoke");
        let value = serde_json::to_value(report).expect("serialize report");
        let object = value.as_object().expect("report object");

        assert_eq!(
            object.keys().map(String::as_str).collect::<Vec<_>>(),
            ["arch", "os", "reopened", "run_id", "schema"]
        );
        assert_eq!(object["schema"], Value::String(PATH_SMOKE_SCHEMA.into()));
        assert!(!value.to_string().contains("private-user-root"));
    }

    #[test]
    fn cli_parser_only_accepts_exact_path_smoke_shape() {
        assert_eq!(parse_cli_args(Vec::<OsString>::new()).unwrap(), None);
        assert_eq!(
            parse_cli_args([
                OsString::from(PATH_SMOKE_COMMAND),
                OsString::from("run-123")
            ])
            .unwrap(),
            Some("run-123".into())
        );

        for args in [
            vec![OsString::from(PATH_SMOKE_COMMAND)],
            vec![
                OsString::from(PATH_SMOKE_COMMAND),
                OsString::from("run-123"),
                OsString::from("unexpected"),
            ],
            vec![OsString::from("unknown-command"), OsString::from("run-123")],
        ] {
            assert!(matches!(
                parse_cli_args(args),
                Err(PathSmokeError::InvalidArguments)
            ));
        }
    }
}
