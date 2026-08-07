use std::{fs, path::Path};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    format_version: u32,
    fixtures: Vec<FixtureEntry>,
}

#[derive(Debug, Deserialize)]
struct FixtureEntry {
    fixture_id: String,
    database_path: String,
    file_sha256: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn manifest_drives_every_historical_fixture_through_the_upgrade_matrix() {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("tests/fixtures/databases");
    let manifest: FixtureManifest = serde_json::from_slice(
        &fs::read(fixture_root.join("manifest.json")).expect("read fixture manifest"),
    )
    .expect("parse fixture manifest");

    assert_eq!(manifest.format_version, 1);
    assert!(!manifest.fixtures.is_empty(), "fixture manifest must not be empty");

    for fixture in &manifest.fixtures {
        let source = fixture_root.join(&fixture.database_path);
        let before_bytes = fs::read(&source).expect("read committed fixture");
        let before_modified = fs::metadata(&source)
            .expect("read committed fixture metadata")
            .modified()
            .expect("read committed fixture mtime");
        assert_eq!(sha256_hex(&before_bytes), fixture.file_sha256);

        upgrade_fixture_copy(&source, fixture);

        assert_eq!(fs::read(&source).expect("reread committed fixture"), before_bytes);
        assert_eq!(
            fs::metadata(&source)
                .expect("reread committed fixture metadata")
                .modified()
                .expect("reread committed fixture mtime"),
            before_modified,
            "{} modified the committed fixture",
            fixture.fixture_id
        );
    }
}
