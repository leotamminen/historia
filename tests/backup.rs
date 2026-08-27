//! CP11 integration tests: `historia backup <path> [--force]` copies the whole
//! store to another local path (CLAUDE.md CP11). Destination semantics: the
//! copy lands at `<path>/.historia`, so `<path>` becomes an ordinary
//! tracked-folder root and `cd <path> && historia verify` works normally.
//! Drives the compiled binary as a subprocess in an isolated temp dir.

use assert_cmd::Command;
use predicates::prelude::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn historia() -> Command {
    Command::cargo_bin("historia").unwrap()
}

fn init(dir: &Path) {
    historia().arg("init").current_dir(dir).assert().success();
}

fn commit(dir: &Path, message: &str) {
    historia()
        .args(["commit", "-m", message])
        .current_dir(dir)
        .assert()
        .success();
}

fn backup(source: &Path, dest: &Path) -> assert_cmd::assert::Assert {
    historia()
        .args(["backup", dest.to_str().unwrap()])
        .current_dir(source)
        .assert()
}

fn store_bytes(store_dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    collect(store_dir, store_dir, &mut out);
    out
}

fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.is_file() {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.insert(rel, fs::read(&path).unwrap());
        }
    }
}

#[test]
fn backup_to_a_fresh_destination_produces_a_store_that_verify_accepts() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"a").unwrap();
    fs::write(source.path().join("b.txt"), b"b").unwrap();
    commit(source.path(), "first");

    let dest = tempdir().unwrap();
    let dest_root = dest.path().join("backup-location");

    backup(source.path(), &dest_root)
        .success()
        .stdout(predicate::str::contains("2 object(s)"))
        .stdout(predicate::str::contains("1 snapshot(s)"));

    // The destination is a normal tracked-folder root: `cd` in and `verify`
    // works exactly like anywhere else.
    historia()
        .arg("verify")
        .current_dir(&dest_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("store OK"));

    // Same snapshots/objects as the source.
    let source_store = store_bytes(&source.path().join(".historia"));
    let dest_store = store_bytes(&dest_root.join(".historia"));
    assert_eq!(source_store, dest_store);
}

#[test]
fn source_is_unchanged_after_backup() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"a").unwrap();
    commit(source.path(), "first");

    let before = store_bytes(&source.path().join(".historia"));

    let dest = tempdir().unwrap();
    backup(source.path(), &dest.path().join("dest")).success();

    let after = store_bytes(&source.path().join(".historia"));
    assert_eq!(before, after);
    assert!(
        !source.path().join(".historia").join("lock").exists(),
        "backup must never take the source's lock"
    );
}

#[test]
fn destination_with_an_existing_store_is_refused_without_force() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"a").unwrap();
    commit(source.path(), "first");

    let dest = tempdir().unwrap();
    init(dest.path());
    let dest_head_before = fs::read_to_string(dest.path().join(".historia").join("HEAD")).unwrap();

    backup(source.path(), dest.path())
        .failure()
        .stderr(predicate::str::contains("already exists"));

    assert_eq!(
        fs::read_to_string(dest.path().join(".historia").join("HEAD")).unwrap(),
        dest_head_before,
        "a refused backup must not touch the existing destination"
    );
}

#[test]
fn destination_with_an_existing_store_is_overwritten_with_force() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"a").unwrap();
    commit(source.path(), "first");

    let dest = tempdir().unwrap();
    init(dest.path());

    historia()
        .args(["backup", dest.path().to_str().unwrap(), "--force"])
        .current_dir(source.path())
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(dest.path().join(".historia").join("HEAD")).unwrap(),
        "1\n"
    );
    historia().arg("verify").current_dir(dest.path()).assert().success();
}

#[test]
fn backup_creates_the_destination_path_if_missing() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"a").unwrap();
    commit(source.path(), "first");

    let dest = tempdir().unwrap();
    let nested = dest.path().join("a").join("b").join("c");
    assert!(!nested.exists());

    backup(source.path(), &nested).success();

    assert!(nested.join(".historia").is_dir());
}

#[test]
fn backup_outside_a_store_fails_cleanly() {
    let dir = tempdir().unwrap();
    let dest = tempdir().unwrap();

    backup(dir.path(), &dest.path().join("dest")).failure();
}

#[test]
fn backup_of_a_large_blob_round_trips_correctly() {
    let source = tempdir().unwrap();
    init(source.path());
    let content = vec![b'z'; 5 * 1024 * 1024]; // 5 MiB
    fs::write(source.path().join("big.bin"), &content).unwrap();
    commit(source.path(), "big file");

    let dest = tempdir().unwrap();
    backup(source.path(), &dest.path().join("dest")).success();

    historia()
        .arg("verify")
        .current_dir(dest.path().join("dest"))
        .assert()
        .success()
        .stdout(predicate::str::contains("store OK"));
}
