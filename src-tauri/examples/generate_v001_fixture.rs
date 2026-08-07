use std::env;
use std::fs;
use std::path::PathBuf;

use gpteasy_lib::state::{DatabaseStatus, StatePaths, StateStore};
use tempfile::TempDir;

fn main() {
    let destination = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run --example generate_v001_fixture -- <destination>");
    let temporary = TempDir::new().expect("create temporary state root");
    let store = StateStore::new(StatePaths::from_root(temporary.path()));
    let snapshot = store.bootstrap();
    assert_eq!(snapshot.status, DatabaseStatus::Initialized);

    let parent = destination.parent().expect("fixture destination parent");
    fs::create_dir_all(parent).expect("create fixture directory");
    fs::copy(store.paths().database(), &destination).expect("write database fixture");
    println!("generated {}", destination.display());
}
