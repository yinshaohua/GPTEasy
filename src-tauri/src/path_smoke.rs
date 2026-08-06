use std::ffi::OsString;

use serde::Serialize;
use tauri::{AppHandle, Runtime};
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

#[derive(Debug, Error)]
pub enum PathSmokeError {
    #[error("opaque run ID must be 1-64 ASCII letters, digits, or hyphens")]
    InvalidRunId,
    #[error("phase1-path-smoke accepts exactly one opaque run ID")]
    InvalidArguments,
    #[error("path smoke is not implemented")]
    NotImplemented,
}

pub(crate) fn parse_cli_args<I>(args: I) -> Result<Option<String>, PathSmokeError>
where
    I: IntoIterator<Item = OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    if args.is_empty() {
        Ok(None)
    } else {
        Err(PathSmokeError::InvalidArguments)
    }
}

pub fn run_path_smoke<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
) -> Result<PathSmokeReport, PathSmokeError> {
    let _ = (app, run_id);
    Err(PathSmokeError::NotImplemented)
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
            vec![
                OsString::from("unknown-command"),
                OsString::from("run-123"),
            ],
        ] {
            assert!(matches!(
                parse_cli_args(args),
                Err(PathSmokeError::InvalidArguments)
            ));
        }
    }
}
