//! CP3 integration tests: `historia commit -m "msg"` (alias `snapshot`) captures
//! the whole tracked folder as a numbered snapshot per CLAUDE.md §5, §9. Drives
//! the compiled binary as a subprocess in an isolated temp dir (never touches the
//! real filesystem outside it).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn historia() -> Command {
    Command::cargo_bin("historia").unwrap()
}

fn init(dir: &Path) {
    historia().arg("init").current_dir(dir).assert().success();
}

fn commit(dir: &Path, message: &str) -> assert_cmd::assert::Assert {
    historia().args(["commit", "-m", message]).current_dir(dir).assert()
}

fn read_head(dir: &Path) -> u64 {
    fs::read_to_string(dir.join(".historia").join("HEAD"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn read_manifest(dir: &Path, number: u64) -> Value {
    let path = dir.join(".historia").join("snapshots").join(format!("{number}.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn blob_path(dir: &Path, hash: &str) -> PathBuf {
    dir.join(".historia").join("objects").join(&hash[..2]).join(&hash[2..])
}

fn entry_paths(manifest: &Value) -> Vec<&str> {
    let mut paths: Vec<&str> = manifest["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    paths.sort();
    paths
}

fn list_objects(dir: &Path) -> Vec<PathBuf> {
    let objects = dir.join(".historia").join("objects");
    let mut result = Vec::new();
    for shard in fs::read_dir(&objects).unwrap() {
        let shard = shard.unwrap().path();
        if shard.is_dir() {
            for f in fs::read_dir(&shard).unwrap() {
                result.push(f.unwrap().path());
            }
        }
    }
    result.sort();
    result
}

#[test]
fn commit_in_fresh_store_creates_snapshot_1_with_manifest_and_blobs() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    fs::create_dir_all(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub").join("b.txt"), b"world").unwrap();

    commit(dir.path(), "first snapshot")
        .success()
        .stdout(predicate::str::contains("snapshot 1"));

    assert_eq!(read_head(dir.path()), 1);
    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(manifest["number"], 1);
    assert_eq!(manifest["parent"], 0);
    assert_eq!(manifest["message"], "first snapshot");
    assert_eq!(entry_paths(&manifest), vec!["a.txt", "sub/b.txt"]);

    for entry in manifest["entries"].as_array().unwrap() {
        let hash = entry["hash"].as_str().unwrap();
        assert!(blob_path(dir.path(), hash).is_file(), "missing blob for {hash}");
    }
}

#[test]
fn unchanged_working_folder_skips_and_allow_empty_forces() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    commit(dir.path(), "first").success();
    assert_eq!(read_head(dir.path()), 1);

    commit(dir.path(), "again").success().stdout(predicate::str::contains(
        "nothing to snapshot, working folder matches snapshot 1",
    ));
    assert_eq!(read_head(dir.path()), 1, "skip-if-unchanged must not create a snapshot");

    historia()
        .args(["commit", "-m", "forced", "--allow-empty"])
        .current_dir(dir.path())
        .assert()
        .success();
    assert_eq!(read_head(dir.path()), 2, "--allow-empty must force a snapshot");
}

#[test]
fn changing_one_file_creates_new_snapshot_and_dedups_unchanged_blobs() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"unchanged").unwrap();
    fs::write(dir.path().join("b.txt"), b"will change").unwrap();
    commit(dir.path(), "first").success();
    let objects_before = list_objects(dir.path());

    fs::write(dir.path().join("b.txt"), b"changed!").unwrap();
    commit(dir.path(), "second").success();

    assert_eq!(read_head(dir.path()), 2);
    let objects_after = list_objects(dir.path());
    assert_eq!(
        objects_after.len(),
        objects_before.len() + 1,
        "only b.txt's new content should add a blob; a.txt's should dedup"
    );
    for obj in &objects_before {
        assert!(objects_after.contains(obj), "old blob {obj:?} should still be present");
    }
}

#[test]
fn default_ignores_are_excluded_from_the_snapshot() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
    fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("node_modules").join("dep.js"), b"ignored").unwrap();
    fs::create_dir_all(dir.path().join("target")).unwrap();
    fs::write(dir.path().join("target").join("out.bin"), b"ignored").unwrap();

    commit(dir.path(), "first").success();

    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(entry_paths(&manifest), vec!["keep.txt"]);
}

#[test]
fn manifest_never_references_a_missing_blob_write_order_invariant() {
    // Rule 5: blobs are written before the manifest, so a manifest visible on
    // disk must never reference a hash that isn't already stored.
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"content a").unwrap();
    fs::write(dir.path().join("b.txt"), b"content b").unwrap();
    commit(dir.path(), "first").success();

    let manifest = read_manifest(dir.path(), 1);
    for entry in manifest["entries"].as_array().unwrap() {
        let hash = entry["hash"].as_str().unwrap();
        assert!(blob_path(dir.path(), hash).is_file(), "manifest references missing blob {hash}");
    }
}

#[test]
fn commit_releases_the_lock_when_done() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"hi").unwrap();

    commit(dir.path(), "first").success();

    assert!(!dir.path().join(".historia").join("lock").exists());
}

#[test]
fn commit_without_message_fails_cleanly() {
    let dir = tempdir().unwrap();
    init(dir.path());

    historia().arg("commit").current_dir(dir.path()).assert().failure();
}

#[test]
fn commit_outside_a_store_fails_cleanly() {
    let dir = tempdir().unwrap();

    historia()
        .args(["commit", "-m", "x"])
        .current_dir(dir.path())
        .assert()
        .failure();
}

#[test]
fn commit_from_a_subdirectory_still_captures_the_whole_folder() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("root.txt"), b"root").unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("nested.txt"), b"nested").unwrap();

    historia().args(["commit", "-m", "first"]).current_dir(&sub).assert().success();

    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(entry_paths(&manifest), vec!["root.txt", "sub/nested.txt"]);
}

#[cfg(unix)]
#[test]
fn symlink_is_skipped_with_warning_and_not_stored() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("real.txt"), b"real").unwrap();
    let link = dir.path().join("link.txt");
    symlink(dir.path().join("real.txt"), &link).unwrap();

    commit(dir.path(), "first")
        .success()
        .stdout(predicate::str::contains("skipped symlink"));

    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(entry_paths(&manifest), vec!["real.txt"]);
}

#[cfg(windows)]
#[test]
fn symlink_is_skipped_with_warning_and_not_stored() {
    use std::os::windows::fs::symlink_file;

    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("real.txt"), b"real").unwrap();
    let link = dir.path().join("link.txt");
    if symlink_file(dir.path().join("real.txt"), &link).is_err() {
        eprintln!(
            "skipping symlink_is_skipped_with_warning_and_not_stored: this environment cannot \
             create symlinks (needs admin or Developer Mode on Windows)"
        );
        return;
    }

    commit(dir.path(), "first")
        .success()
        .stdout(predicate::str::contains("skipped symlink"));

    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(entry_paths(&manifest), vec!["real.txt"]);
}
