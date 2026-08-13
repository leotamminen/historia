//! CP4 integration tests: `historia log` lists snapshots newest first (CLAUDE.md
//! §5). Read-only - never writes, never takes the lock. Drives the compiled
//! binary as a subprocess in an isolated temp dir.

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

#[test]
fn log_on_an_empty_store_prints_the_empty_message() {
    let dir = tempdir().unwrap();
    init(dir.path());

    historia()
        .arg("log")
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout("no snapshots yet\n");
}

#[test]
fn log_after_n_commits_prints_n_lines_newest_first() {
    let dir = tempdir().unwrap();
    init(dir.path());

    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first");
    fs::write(dir.path().join("a.txt"), b"2").unwrap();
    commit(dir.path(), "second");
    fs::write(dir.path().join("a.txt"), b"3").unwrap();
    commit(dir.path(), "third");

    let assert = historia().arg("log").current_dir(dir.path()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();

    assert_eq!(lines.len(), 3, "expected 3 log lines, got:\n{stdout}");
    // Newest first.
    assert!(lines[0].trim_start().starts_with('3'));
    assert!(lines[0].contains("third"));
    assert!(lines[1].trim_start().starts_with('2'));
    assert!(lines[1].contains("second"));
    assert!(lines[2].trim_start().starts_with('1'));
    assert!(lines[2].contains("first"));
}

#[test]
fn log_includes_the_iso8601_timestamp() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first");

    historia()
        .arg("log")
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z").unwrap());
}

#[test]
fn log_never_writes_to_the_store() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first");

    let head_before = fs::read_to_string(dir.path().join(".historia").join("HEAD")).unwrap();
    let manifest_before =
        fs::read_to_string(dir.path().join(".historia").join("snapshots").join("1.json")).unwrap();

    historia().arg("log").current_dir(dir.path()).assert().success();

    assert!(!dir.path().join(".historia").join("lock").exists(), "log must never take the lock");
    assert_eq!(
        fs::read_to_string(dir.path().join(".historia").join("HEAD")).unwrap(),
        head_before,
        "log must never modify HEAD"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".historia").join("snapshots").join("1.json")).unwrap(),
        manifest_before,
        "log must never modify a manifest"
    );
}

#[test]
fn log_outside_a_store_fails_cleanly() {
    let dir = tempdir().unwrap();

    historia().arg("log").current_dir(dir.path()).assert().failure();
}
