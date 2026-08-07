use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repository_root() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("src-tauri is below repository root")
        .to_path_buf()
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).expect("read local-only contract input")
}

fn toml_section_keys(source: &str, expected_section: &str) -> BTreeSet<String> {
    let mut active = false;
    let mut keys = BTreeSet::new();
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active = line == format!("[{expected_section}]");
            continue;
        }
        if active && !line.is_empty() && !line.starts_with('#') {
            let (key, _) = line.split_once('=').expect("dependency assignment");
            keys.insert(key.trim().to_owned());
        }
    }
    keys
}

fn registered_commands(source: &str) -> BTreeSet<String> {
    let start = source
        .find("tauri::generate_handler![")
        .expect("production invoke handler exists");
    let body = &source[start + "tauri::generate_handler![".len()..];
    let end = body.find(']').expect("production invoke handler closes");
    body[..end]
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .rsplit("::")
                .next()
                .expect("registered command name")
                .to_owned()
        })
        .collect()
}

fn struct_fields(source: &str, name: &str) -> BTreeSet<String> {
    let declaration = format!("struct {name}");
    let start = source.find(&declaration).expect("serialized DTO exists");
    let body = &source[start..];
    let open = body.find('{').expect("DTO body opens");
    let close = body[open + 1..].find('}').expect("DTO body closes");
    body[open + 1..open + 1 + close]
        .lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(',').trim();
            let (field, _) = line.split_once(':')?;
            Some(field.trim().trim_start_matches("pub ").to_owned())
        })
        .collect()
}

#[test]
fn local_state_root_dependencies_capability_and_commands_match_exact_allowlists() {
    let manifest = read(manifest_dir().join("Cargo.toml"));
    let package: Value = serde_json::from_str(&read(repository_root().join("package.json")))
        .expect("parse package.json");
    let capability: Value =
        serde_json::from_str(&read(manifest_dir().join("capabilities/default.json")))
            .expect("parse default capability");
    let lib_source = read(manifest_dir().join("src/lib.rs"));

    let expected_cargo = BTreeMap::from([
        (
            "build-dependencies",
            BTreeSet::from(["tauri-build".to_owned()]),
        ),
        (
            "dependencies",
            BTreeSet::from([
                "chrono".to_owned(),
                "rusqlite".to_owned(),
                "serde".to_owned(),
                "serde_json".to_owned(),
                "sha2".to_owned(),
                "tauri".to_owned(),
                "thiserror".to_owned(),
                "uuid".to_owned(),
            ]),
        ),
        (
            "target.'cfg(windows)'.dependencies",
            BTreeSet::from(["windows-sys".to_owned()]),
        ),
        (
            "dev-dependencies",
            BTreeSet::from(["tauri".to_owned(), "tempfile".to_owned()]),
        ),
    ]);
    for (section, expected) in expected_cargo {
        assert_eq!(toml_section_keys(&manifest, section), expected, "{section}");
    }

    let dependency_keys = |section: &str| {
        package[section]
            .as_object()
            .expect("package dependency object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        dependency_keys("dependencies"),
        BTreeSet::from([
            "@tauri-apps/api".to_owned(),
            "react".to_owned(),
            "react-dom".to_owned(),
        ])
    );
    assert_eq!(
        dependency_keys("devDependencies"),
        BTreeSet::from([
            "@tauri-apps/cli".to_owned(),
            "@types/react".to_owned(),
            "@types/react-dom".to_owned(),
            "@vitejs/plugin-react".to_owned(),
            "typescript".to_owned(),
            "vite".to_owned(),
        ])
    );

    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert_eq!(
        capability["permissions"],
        serde_json::json!(["core:default"])
    );
    assert!(lib_source.contains(".app_local_data_dir()"));
    assert!(!lib_source.contains(".app_data_dir()"));
    assert_eq!(
        registered_commands(&lib_source),
        BTreeSet::from([
            "bootstrap_state".to_owned(),
            "bootstrap_state_snapshot".to_owned(),
            "replace_state_snapshot".to_owned(),
            "update_app_settings".to_owned(),
        ])
    );
    assert!(
        lib_source.contains("phase1-state-smoke"),
        "production CLI name is not registered in run_with_args"
    );
}

#[test]
fn public_state_dtos_and_frontend_imports_stay_inside_local_only_allowlists() {
    let commands = read(manifest_dir().join("src/commands.rs"));
    let expected_fields = BTreeMap::from([
        (
            "AppSettings",
            BTreeSet::from([
                "close_to_tray_notice_seen".to_owned(),
                "last_update_check_at".to_owned(),
                "launch_at_login_desired".to_owned(),
                "locale".to_owned(),
                "onboarding_completed".to_owned(),
                "theme".to_owned(),
            ]),
        ),
        (
            "BootstrapState",
            BTreeSet::from(["schema_version".to_owned(), "settings".to_owned()]),
        ),
        ("PublicStoreError", BTreeSet::from(["code".to_owned()])),
        (
            "PublicStateSnapshot",
            BTreeSet::from([
                "counts".to_owned(),
                "environments".to_owned(),
                "providers".to_owned(),
                "schema_version".to_owned(),
                "settings".to_owned(),
                "state_digest".to_owned(),
            ]),
        ),
        (
            "PublicStateCounts",
            BTreeSet::from([
                "managed_environments".to_owned(),
                "providers".to_owned(),
                "verified_providers".to_owned(),
            ]),
        ),
        (
            "PublicProvider",
            BTreeSet::from([
                "combination_fingerprint".to_owned(),
                "id".to_owned(),
                "provider_kind".to_owned(),
                "verification_status".to_owned(),
            ]),
        ),
        (
            "PublicEnvironment",
            BTreeSet::from([
                "current_provider_id".to_owned(),
                "environment_kind".to_owned(),
                "id".to_owned(),
            ]),
        ),
        (
            "PublicCompleteAppSettings",
            BTreeSet::from([
                "close_to_tray_notice_seen".to_owned(),
                "last_update_check_at".to_owned(),
                "launch_at_login_desired".to_owned(),
                "locale".to_owned(),
                "onboarding_completed".to_owned(),
                "theme".to_owned(),
                "updated_at".to_owned(),
            ]),
        ),
    ]);
    for (dto, expected) in expected_fields {
        assert_eq!(struct_fields(&commands, dto), expected, "{dto}");
    }

    let frontend_root = repository_root().join("src");
    let mut imports = BTreeSet::new();
    let mut source = String::new();
    for entry in fs::read_dir(frontend_root).expect("read frontend source") {
        let path = entry.expect("frontend entry").path();
        if path.extension().is_some_and(|extension| extension == "tsx") {
            let content = read(&path);
            for line in content.lines().map(str::trim) {
                if line.starts_with("import ") {
                    let quoted = line
                        .rsplit_once('"')
                        .and_then(|(prefix, _)| prefix.rsplit_once('"'))
                        .map(|(_, import)| import)
                        .expect("quoted frontend import");
                    imports.insert(quoted.to_owned());
                }
            }
            source.push_str(&content);
        }
    }
    assert_eq!(
        imports,
        BTreeSet::from([
            "./App".to_owned(),
            "./global.css".to_owned(),
            "react".to_owned(),
            "react-dom/client".to_owned(),
        ])
    );
    for forbidden_api in [
        "fetch(",
        "XMLHttpRequest",
        "WebSocket",
        "localStorage",
        "sessionStorage",
        "indexedDB",
    ] {
        assert!(
            !source.contains(forbidden_api),
            "forbidden API {forbidden_api}"
        );
    }

    let coverage = read(
        repository_root().join(".planning/phases/01-trusted-local-state-contract/COVERAGE.md"),
    );
    assert_eq!(
        coverage.trim(),
        "No external API integration: Phase 1 仅实现当前用户本地 SQLite 状态与版本化 Codex/宿主/WSL2/签名打包契约探针；SQLite Online Backup API 是进程内数据库库接口，不是外部服务能力面。"
    );
}
