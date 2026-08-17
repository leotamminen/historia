//! CP8 integration tests: `historia verify` checks blob and manifest integrity
//! (CLAUDE.md §8, §9). Read-only - never writes, never takes the lock. Drives
//! the compiled binary as a subprocess in an isolated temp dir.

use assert_cmd::Command;
use predicates::prelude::*;
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

fn verify(dir: &Path) -> assert_cmd::assert::Assert {
    historia().arg("verify").current_dir(dir).assert()
}

/// Read the store's whole byte content (every file under `.historia/`), keyed
/// by relative path, for a before/after "verify changed nothing" comparison.
fn store_bytes(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    collect(&dir.join(".historia"), &dir.join(".historia"), &mut out);
    out
}

fn collect(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
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

fn first_blob_path(dir: &Path) -> std::path::PathBuf {
    let objects = dir.join(".historia").join("objects");
    for shard in fs::read_dir(&objects).unwrap() {
        let shard = shard.unwrap().path();
        if shard.is_dir() {
            for entry in fs::read_dir(&shard).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    return path;
                }
            }
        }
    }
    panic!("no blob found under {objects:?}");
}

#[test]
fn verify_on_a_freshly_committed_store_is_ok_with_correct_counts() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    fs::write(dir.path().join("b.txt"), b"b").unwrap();
    commit(dir.path(), "first");

    verify(dir.path())
        .success()
        .stdout(predicate::str::contains("2 object(s) checked"))
        .stdout(predicate::str::contains("1 snapshot(s) checked"))
        .stdout(predicate::str::contains("store OK"));
}

#[test]
fn verify_on_an_empty_store_is_ok() {
    let dir = tempdir().unwrap();
    init(dir.path());

    verify(dir.path())
        .success()
        .stdout(predicate::str::contains("0 object(s) checked"))
        .stdout(predicate::str::contains("0 snapshot(s) checked"))
        .stdout(predicate::str::contains("store OK"));
}

#[test]
fn a_corrupted_blob_is_detected_names_the_object_and_exits_non_zero() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"original content").unwrap();
    commit(dir.path(), "first");

    let blob_path = first_blob_path(dir.path());
    let hash = format!(
        "{}{}",
        blob_path.parent().unwrap().file_name().unwrap().to_string_lossy(),
        blob_path.file_name().unwrap().to_string_lossy()
    );
    fs::write(&blob_path, b"corrupted!").unwrap();

    verify(dir.path())
        .failure()
        .stdout(predicate::str::contains("corrupted"))
        .stdout(predicate::str::contains(&hash));
}

#[test]
fn a_deleted_blob_referenced_by_a_manifest_is_reported_as_dangling() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("gone.txt"), b"will be deleted from the store").unwrap();
    commit(dir.path(), "first");

    fs::remove_file(first_blob_path(dir.path())).unwrap();

    verify(dir.path())
        .failure()
        .stdout(predicate::str::contains('1'))
        .stdout(predicate::str::contains("gone.txt"));
}

#[test]
fn a_bad_format_marker_is_reported_and_exits_non_zero() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join(".historia").join("format"), "garbage\n").unwrap();

    verify(dir.path())
        .failure()
        .stdout(predicate::str::contains("format").or(predicate::str::contains("Format")));
}

#[test]
fn an_out_of_range_head_is_reported_and_exits_non_zero() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    commit(dir.path(), "first");

    fs::write(dir.path().join(".historia").join("HEAD"), "99\n").unwrap();

    verify(dir.path())
        .failure()
        .stdout(predicate::str::contains("HEAD"))
        .stdout(predicate::str::contains("99"));
}

#[test]
fn verify_never_creates_a_lock_file_or_modifies_the_store() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    commit(dir.path(), "first");

    let before = store_bytes(dir.path());

    verify(dir.path()).success();

    assert!(!dir.path().join(".historia").join("lock").exists(), "verify must never take the lock");
    assert_eq!(store_bytes(dir.path()), before, "verify must never modify the store");
}

#[test]
fn verify_outside_a_store_fails_cleanly() {
    let dir = tempdir().unwrap();

    verify(dir.path()).failure();
}
