//! CP6 integration tests: `historia restore <n> [path]` (CLAUDE.md Rules 3, 4,
//! 5, 6, 7). The round-trip test is the definition of done for this checkpoint.
//! Drives the compiled binary as a subprocess in an isolated temp dir.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
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

fn commit(dir: &Path, message: &str) -> assert_cmd::assert::Assert {
    historia().args(["commit", "-m", message]).current_dir(dir).assert()
}

fn restore(dir: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut full = vec!["restore"];
    full.extend_from_slice(args);
    historia().args(full).current_dir(dir).assert()
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

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The same default-ignore names CLAUDE.md §5 specifies, mirrored here (tests
/// have no access to the crate's internals - no `lib` target, CLAUDE.md §8) so
/// this helper can tell "tracked" from "ignored" the same way the binary does.
const IGNORED_NAMES: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".historia"];

/// Snapshot every currently tracked file's content under `root`, keyed by its
/// forward-slash relative path - the same shape a manifest's entries describe,
/// for direct content-identical comparison across a restore.
fn tracked_working_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    collect(root, root, &mut out);
    out
}

fn collect(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if IGNORED_NAMES.contains(&name.to_string_lossy().as_ref()) {
            continue;
        }
        let file_type = entry.file_type().unwrap();
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        } else if file_type.is_dir() {
            collect(root, &path, out);
        } else if file_type.is_file() {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.insert(rel, fs::read(&path).unwrap());
        }
    }
}

#[test]
fn round_trip_edit_add_delete_then_restore_is_content_identical() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("keep.txt"), b"keep me").unwrap();
    fs::write(dir.path().join("edit.txt"), b"before edit").unwrap();
    fs::write(dir.path().join("remove.txt"), b"will be deleted").unwrap();
    fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("node_modules").join("dep.js"), b"ignored content").unwrap();
    commit(dir.path(), "snapshot A").success();
    assert_eq!(read_head(dir.path()), 1);

    let snapshot_a_working = tracked_working_files(dir.path());
    let manifest_a = read_manifest(dir.path(), 1);
    let ignored_before = fs::read(dir.path().join("node_modules").join("dep.js")).unwrap();

    // Mutate: edit, add, delete.
    fs::write(dir.path().join("edit.txt"), b"after edit").unwrap();
    fs::write(dir.path().join("new.txt"), b"brand new").unwrap();
    fs::remove_file(dir.path().join("remove.txt")).unwrap();

    restore(dir.path(), &["1"])
        .success()
        .stdout(predicate::str::contains("restored to snapshot 1"));

    // The pre-restore (mutated) state was captured as a new safety snapshot.
    assert_eq!(read_head(dir.path()), 2, "safety snapshot must be created before restoring");
    let safety_manifest = read_manifest(dir.path(), 2);
    assert_eq!(safety_manifest["parent"], 1);
    assert!(safety_manifest["message"].as_str().unwrap().contains("safety snapshot"));

    // The tracked set is content-identical to snapshot A: same paths, same bytes.
    let restored_working = tracked_working_files(dir.path());
    assert_eq!(restored_working, snapshot_a_working);

    // Verify-style integrity: every restored file's hash matches the manifest.
    for entry in manifest_a["entries"].as_array().unwrap() {
        let path = entry["path"].as_str().unwrap();
        let expected_hash = entry["hash"].as_str().unwrap();
        let bytes = fs::read(dir.path().join(path)).unwrap();
        assert_eq!(sha256_hex(&bytes), expected_hash, "hash mismatch for restored '{path}'");
    }

    // Ignored paths present before restore are untouched by it.
    assert_eq!(
        fs::read(dir.path().join("node_modules").join("dep.js")).unwrap(),
        ignored_before,
        "restore must never touch ignored paths"
    );

    // The pre-restore state (safety snapshot 2) can itself be restored.
    restore(dir.path(), &["2"]).success();
    assert_eq!(fs::read_to_string(dir.path().join("edit.txt")).unwrap(), "after edit");
    assert_eq!(fs::read_to_string(dir.path().join("new.txt")).unwrap(), "brand new");
    assert!(!dir.path().join("remove.txt").exists());
    assert_eq!(read_head(dir.path()), 3, "restoring 2 takes its own safety snapshot (3)");
}

#[test]
fn single_file_restore_only_touches_that_file() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"original a").unwrap();
    fs::write(dir.path().join("b.txt"), b"original b").unwrap();
    commit(dir.path(), "snapshot A").success();

    fs::write(dir.path().join("a.txt"), b"mutated a").unwrap();
    fs::write(dir.path().join("b.txt"), b"mutated b").unwrap();

    restore(dir.path(), &["1", "a.txt"])
        .success()
        .stdout(predicate::str::contains("restored 'a.txt' from snapshot 1"));

    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "original a");
    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "mutated b",
        "single-file restore must never touch other files"
    );
    assert_eq!(read_head(dir.path()), 2, "safety snapshot must still be created");
}

#[test]
fn restore_missing_path_fails_and_deletes_nothing() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    commit(dir.path(), "snapshot A").success();
    fs::write(dir.path().join("b.txt"), b"present at restore time").unwrap();

    restore(dir.path(), &["1", "missing.txt"]).failure();

    assert!(dir.path().join("a.txt").exists());
    assert!(dir.path().join("b.txt").exists(), "single-file restore must delete nothing, even on failure");
    // Per spec: the safety snapshot is still taken even though the path lookup fails.
    assert_eq!(read_head(dir.path()), 2);
}

#[test]
fn restore_to_a_nonexistent_snapshot_fails_and_creates_no_safety_snapshot() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    commit(dir.path(), "snapshot A").success();

    restore(dir.path(), &["99"]).failure();

    assert_eq!(read_head(dir.path()), 1, "an invalid restore target must not create any snapshot");
}

#[test]
fn restore_refuses_cleanly_if_the_lock_is_held() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    commit(dir.path(), "snapshot A").success();

    let lock_path = dir.path().join(".historia").join("lock");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(&lock_path, format!("{}\n{now}\n", std::process::id())).unwrap();

    restore(dir.path(), &["1"])
        .failure()
        .stderr(predicate::str::contains("locked"));

    assert_eq!(read_head(dir.path()), 1, "a refused restore must not mutate the store");
    assert!(!dir.path().join(".historia").join("snapshots").join("2.json").exists());
}

#[test]
fn restore_releases_the_lock_when_done() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    commit(dir.path(), "snapshot A").success();

    restore(dir.path(), &["1"]).success();

    assert!(!dir.path().join(".historia").join("lock").exists());
}

#[test]
fn restore_outside_a_store_fails_cleanly() {
    let dir = tempdir().unwrap();

    restore(dir.path(), &["1"]).failure();
}
