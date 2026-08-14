//! CP7 integration tests: `.historiaignore` (gitignore syntax) layered on the
//! CLAUDE.md §5 default ignores. Since `commit`, `status`, and `restore` all
//! share the one walker (`core::walk` + `core::ignore`), a pattern honored by
//! one must be honored by all three - that shared-behavior guarantee is exactly
//! what this file asserts, not just "the walker works" (already covered by
//! `core::ignore`'s and `core::walk`'s own unit tests).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
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

fn status(dir: &Path) -> assert_cmd::assert::Assert {
    historia().arg("status").current_dir(dir).assert()
}

fn read_manifest(dir: &Path, number: u64) -> Value {
    let path = dir.join(".historia").join("snapshots").join(format!("{number}.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
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

#[test]
fn a_historiaignore_pattern_excludes_a_file_from_commit() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join(".historiaignore"), "*.log\n").unwrap();
    fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
    fs::write(dir.path().join("debug.log"), b"noisy").unwrap();

    commit(dir.path(), "first").success();

    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(entry_paths(&manifest), vec![".historiaignore", "keep.txt"]);
}

#[test]
fn a_historiaignore_pattern_excludes_a_file_from_status() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
    commit(dir.path(), "first").success();

    // Add the ignore file and an ignored file after the baseline commit: status
    // must not report the new .log file as "added".
    fs::write(dir.path().join(".historiaignore"), "*.log\n").unwrap();
    fs::write(dir.path().join("debug.log"), b"noisy").unwrap();

    status(dir.path())
        .success()
        .stdout(predicate::str::contains("Added:\n  .historiaignore\n"))
        .stdout(predicate::str::contains("debug.log").not());
}

#[test]
fn a_historiaignore_pattern_excludes_a_file_from_restore() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join(".historiaignore"), "*.log\n").unwrap();
    fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
    commit(dir.path(), "first").success();

    // A .log file present at restore time (never tracked) must survive a
    // whole-folder restore untouched, exactly like any other ignored path.
    fs::write(dir.path().join("debug.log"), b"should survive").unwrap();

    historia()
        .args(["restore", "1"])
        .current_dir(dir.path())
        .assert()
        .success();

    assert_eq!(fs::read(dir.path().join("debug.log")).unwrap(), b"should survive");
}

#[test]
fn default_ignores_still_apply_with_a_historiaignore_present() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join(".historiaignore"), "*.log\n").unwrap();
    fs::write(dir.path().join("keep.txt"), b"keep").unwrap();
    fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("node_modules").join("dep.js"), b"ignored").unwrap();

    commit(dir.path(), "first").success();

    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(entry_paths(&manifest), vec![".historiaignore", "keep.txt"]);
}

#[test]
fn a_negation_re_includes_a_file_a_broader_pattern_excluded() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join(".historiaignore"), "*.log\n!keep.log\n").unwrap();
    fs::write(dir.path().join("debug.log"), b"noisy").unwrap();
    fs::write(dir.path().join("keep.log"), b"important").unwrap();

    commit(dir.path(), "first").success();

    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(entry_paths(&manifest), vec![".historiaignore", "keep.log"]);
}

#[test]
fn nothing_can_re_include_the_store_directory() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join(".historiaignore"), "!.historia\n!.historia/**\n").unwrap();
    fs::write(dir.path().join("keep.txt"), b"keep").unwrap();

    commit(dir.path(), "first").success();

    let manifest = read_manifest(dir.path(), 1);
    for path in entry_paths(&manifest) {
        assert!(
            path != ".historia" && !path.starts_with(".historia/"),
            "the store directory must never be tracked, got {path}"
        );
    }
}

#[test]
fn no_historiaignore_file_behaves_exactly_like_defaults_only() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"a").unwrap();
    fs::create_dir_all(dir.path().join("target")).unwrap();
    fs::write(dir.path().join("target").join("out.bin"), b"ignored").unwrap();

    commit(dir.path(), "first").success();

    let manifest = read_manifest(dir.path(), 1);
    assert_eq!(entry_paths(&manifest), vec!["a.txt"]);
}
