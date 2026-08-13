//! CP5 integration tests: `historia status` reports added/modified/deleted since
//! HEAD (CLAUDE.md §5). Read-only - never writes, never takes the lock. Drives
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

fn status(dir: &Path) -> assert_cmd::assert::Assert {
    historia().arg("status").current_dir(dir).assert()
}

#[test]
fn status_on_empty_store_prints_friendly_message() {
    let dir = tempdir().unwrap();
    init(dir.path());

    status(dir.path()).success().stdout("no snapshots yet\n");
}

#[test]
fn no_changes_reports_matches_snapshot() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    commit(dir.path(), "first");

    status(dir.path())
        .success()
        .stdout("working folder matches snapshot 1\n");
}

#[test]
fn fresh_file_is_added() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    commit(dir.path(), "first");

    fs::write(dir.path().join("new.txt"), b"new content").unwrap();

    status(dir.path())
        .success()
        .stdout(predicate::str::contains("Added:\n  new.txt\n"))
        .stdout(predicate::str::contains("Modified:").not())
        .stdout(predicate::str::contains("Deleted:").not());
}

#[test]
fn edited_file_is_modified() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    commit(dir.path(), "first");

    fs::write(dir.path().join("a.txt"), b"edited").unwrap();

    status(dir.path())
        .success()
        .stdout(predicate::str::contains("Modified:\n  a.txt\n"))
        .stdout(predicate::str::contains("Added:").not())
        .stdout(predicate::str::contains("Deleted:").not());
}

#[test]
fn removed_file_is_deleted() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    commit(dir.path(), "first");

    fs::remove_file(dir.path().join("a.txt")).unwrap();

    status(dir.path())
        .success()
        .stdout(predicate::str::contains("Deleted:\n  a.txt\n"))
        .stdout(predicate::str::contains("Added:").not())
        .stdout(predicate::str::contains("Modified:").not());
}

#[test]
fn combination_of_added_modified_and_deleted() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
    fs::write(dir.path().join("edit.txt"), b"before").unwrap();
    fs::write(dir.path().join("gone.txt"), b"bye").unwrap();
    commit(dir.path(), "first");

    fs::write(dir.path().join("edit.txt"), b"after").unwrap();
    fs::remove_file(dir.path().join("gone.txt")).unwrap();
    fs::write(dir.path().join("new.txt"), b"new").unwrap();

    let assert = status(dir.path()).success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("Added:\n  new.txt\n"), "{stdout}");
    assert!(stdout.contains("Modified:\n  edit.txt\n"), "{stdout}");
    assert!(stdout.contains("Deleted:\n  gone.txt\n"), "{stdout}");
    assert!(!stdout.contains("keep.txt"), "unchanged file must not appear:\n{stdout}");
}

#[test]
fn ignored_paths_are_excluded_from_status() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    commit(dir.path(), "first");

    fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("node_modules").join("dep.js"), b"ignored").unwrap();

    status(dir.path())
        .success()
        .stdout("working folder matches snapshot 1\n");
}

#[cfg(unix)]
#[test]
fn symlinks_are_excluded_from_status() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("real.txt"), b"real").unwrap();
    commit(dir.path(), "first");

    symlink(dir.path().join("real.txt"), dir.path().join("link.txt")).unwrap();

    status(dir.path())
        .success()
        .stdout("working folder matches snapshot 1\n");
}

#[cfg(windows)]
#[test]
fn symlinks_are_excluded_from_status() {
    use std::os::windows::fs::symlink_file;

    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("real.txt"), b"real").unwrap();
    commit(dir.path(), "first");

    if symlink_file(dir.path().join("real.txt"), dir.path().join("link.txt")).is_err() {
        eprintln!(
            "skipping symlinks_are_excluded_from_status: this environment cannot create \
             symlinks (needs admin or Developer Mode on Windows)"
        );
        return;
    }

    status(dir.path())
        .success()
        .stdout("working folder matches snapshot 1\n");
}

#[test]
fn status_never_writes_to_the_store() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"hello").unwrap();
    commit(dir.path(), "first");
    fs::write(dir.path().join("a.txt"), b"edited").unwrap();
    fs::write(dir.path().join("new.txt"), b"new").unwrap();

    let head_before = fs::read_to_string(dir.path().join(".historia").join("HEAD")).unwrap();
    let manifest_before =
        fs::read_to_string(dir.path().join(".historia").join("snapshots").join("1.json")).unwrap();

    status(dir.path()).success();

    assert!(!dir.path().join(".historia").join("lock").exists(), "status must never take the lock");
    assert_eq!(
        fs::read_to_string(dir.path().join(".historia").join("HEAD")).unwrap(),
        head_before,
        "status must never modify HEAD"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".historia").join("snapshots").join("1.json")).unwrap(),
        manifest_before,
        "status must never modify a manifest"
    );
    assert!(
        !dir.path().join(".historia").join("snapshots").join("2.json").exists(),
        "status must never create a new snapshot"
    );
}

#[test]
fn status_outside_a_store_fails_cleanly() {
    let dir = tempdir().unwrap();

    status(dir.path()).failure();
}
