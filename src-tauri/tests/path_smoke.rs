use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use gpteasy_lib::path_smoke::{run_path_smoke, PathSmokeError, PATH_SMOKE_SCHEMA};
use serde_json::Value;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tempfile::tempdir;

const CHILD_MODE_ENV: &str = "GPTEASY_PATH_SMOKE_TEST_CHILD";
const CHILD_ROOT_ENV: &str = "GPTEASY_PATH_SMOKE_TEST_ROOT";
const CHILD_RUN_ID_ENV: &str = "GPTEASY_PATH_SMOKE_TEST_RUN_ID";
const CHILD_EXPECT_INVALID_ENV: &str = "GPTEASY_PATH_SMOKE_TEST_EXPECT_INVALID";
const REPORT_PREFIX: &str = "GPTEASY_PATH_SMOKE_REPORT=";
const INVALID_PREFIX: &str = "GPTEASY_PATH_SMOKE_INVALID=";

#[derive(Debug, Clone, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    File(Vec<u8>),
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
    fn visit(root: &Path, current: &Path, entries: &mut BTreeMap<PathBuf, SnapshotEntry>) {
        let mut children = fs::read_dir(current)
            .expect("read snapshot directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect snapshot directory");
        children.sort_by_key(|entry| entry.file_name());

        for entry in children {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("snapshot path is inside root")
                .to_path_buf();
            let file_type = entry.file_type().expect("read snapshot file type");
            if file_type.is_dir() {
                entries.insert(relative, SnapshotEntry::Directory);
                visit(root, &path, entries);
            } else if file_type.is_file() {
                entries.insert(
                    relative,
                    SnapshotEntry::File(fs::read(&path).expect("read snapshot file")),
                );
            } else {
                panic!("unexpected non-file entry in snapshot");
            }
        }
    }

    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn spawn_child(root: &Path, run_id: &str, expect_invalid: bool) -> Output {
    Command::new(env::current_exe().expect("resolve integration test executable"))
        .args([
            OsString::from("--exact"),
            OsString::from("path_smoke_child_process"),
            OsString::from("--nocapture"),
            OsString::from("--test-threads=1"),
        ])
        .env(CHILD_MODE_ENV, "1")
        .env(CHILD_ROOT_ENV, root.as_os_str())
        .env(CHILD_RUN_ID_ENV, run_id)
        .env(
            CHILD_EXPECT_INVALID_ENV,
            if expect_invalid { "1" } else { "0" },
        )
        .output()
        .expect("spawn path smoke child process")
}

fn output_text(output: &Output) -> (String, String) {
    (
        String::from_utf8(output.stdout.clone()).expect("child stdout is UTF-8"),
        String::from_utf8(output.stderr.clone()).expect("child stderr is UTF-8"),
    )
}

fn assert_sanitized_output(output: &Output, private_root: &Path) -> (String, String) {
    let (stdout, stderr) = output_text(output);
    assert!(
        output.status.success(),
        "child failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let private_root = private_root.to_string_lossy();
    assert!(!stdout.contains(private_root.as_ref()));
    assert!(!stderr.contains(private_root.as_ref()));
    if let Some(user_profile) = env::var_os("USERPROFILE") {
        let user_profile = user_profile.to_string_lossy();
        assert!(!stdout.contains(user_profile.as_ref()));
        assert!(!stderr.contains(user_profile.as_ref()));
    }

    (stdout, stderr)
}

fn report_from(output: &Output, private_root: &Path) -> Value {
    let (stdout, _) = assert_sanitized_output(output, private_root);
    let json = stdout
        .lines()
        .find_map(|line| {
            line.find(REPORT_PREFIX)
                .map(|index| &line[index + REPORT_PREFIX.len()..])
        })
        .expect("child emitted path smoke report");
    let report: Value = serde_json::from_str(json).expect("parse child report");
    let object = report.as_object().expect("path smoke report object");

    assert_eq!(
        object.keys().map(String::as_str).collect::<Vec<_>>(),
        ["arch", "os", "reopened", "run_id", "schema"]
    );
    assert_eq!(object["schema"], PATH_SMOKE_SCHEMA);
    report
}

#[test]
fn path_smoke_child_process() {
    if env::var_os(CHILD_MODE_ENV).is_none() {
        return;
    }

    let root = PathBuf::from(env::var_os(CHILD_ROOT_ENV).expect("child test root"));
    let run_id = env::var(CHILD_RUN_ID_ENV).expect("child run ID");
    let expect_invalid = env::var(CHILD_EXPECT_INVALID_ENV).as_deref() == Ok("1");
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = root.to_string_lossy().into_owned();
    let app = mock_builder().build(context).expect("build child mock app");

    if expect_invalid {
        assert!(matches!(
            run_path_smoke(app.handle(), &run_id),
            Err(PathSmokeError::InvalidRunId)
        ));
        println!("{INVALID_PREFIX}true");
    } else {
        let report = run_path_smoke(app.handle(), &run_id).expect("run child path smoke");
        println!(
            "{REPORT_PREFIX}{}",
            serde_json::to_string(&report).expect("serialize child report")
        );
    }
}

#[test]
fn independent_processes_reopen_only_the_fixed_temp_root() {
    let temp = tempdir().expect("create integration temp directory");
    let app_root = temp.path().join("app-local");
    let outside_root = temp.path().join("outside");
    fs::create_dir(&outside_root).expect("create outside canary directory");
    fs::write(outside_root.join("canary.txt"), b"unchanged").expect("write outside canary");
    let baseline = snapshot(temp.path());

    for invalid_id in [
        "",
        "..",
        "../escape",
        r"outside\escape",
        "contains.dot",
        "包含Unicode",
        &"a".repeat(65),
    ] {
        let output = spawn_child(&app_root, invalid_id, true);
        let (stdout, _) = assert_sanitized_output(&output, temp.path());
        assert!(stdout.contains(&format!("{INVALID_PREFIX}true")));
        assert_eq!(snapshot(temp.path()), baseline);
    }

    let run_id = "install-path-smoke-20260806";
    let first = report_from(&spawn_child(&app_root, run_id, false), temp.path());
    let second = report_from(&spawn_child(&app_root, run_id, false), temp.path());

    assert_eq!(first["run_id"], run_id);
    assert_eq!(first["reopened"], false);
    assert_eq!(second["run_id"], run_id);
    assert_eq!(second["reopened"], true);

    let marker_relative = PathBuf::from("app-local")
        .join("contract-smoke")
        .join("path")
        .join(format!("{run_id}.json"));
    let after = snapshot(temp.path());
    let added = after
        .iter()
        .filter(|(path, _)| !baseline.contains_key(*path))
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        added.keys().cloned().collect::<Vec<_>>(),
        [
            PathBuf::from("app-local"),
            PathBuf::from("app-local").join("contract-smoke"),
            PathBuf::from("app-local")
                .join("contract-smoke")
                .join("path"),
            marker_relative.clone(),
        ]
    );

    let marker = match added.get(&marker_relative).expect("fixed marker exists") {
        SnapshotEntry::File(bytes) => {
            serde_json::from_slice::<Value>(bytes).expect("parse fixed marker")
        }
        SnapshotEntry::Directory => panic!("fixed marker must be a file"),
    };
    assert_eq!(
        marker
            .as_object()
            .expect("marker object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["arch", "os", "run_id", "schema"]
    );
    assert_eq!(marker["run_id"], run_id);
    assert_eq!(marker["schema"], PATH_SMOKE_SCHEMA);
    assert_eq!(
        fs::read(outside_root.join("canary.txt")).expect("read outside canary"),
        b"unchanged"
    );
}
