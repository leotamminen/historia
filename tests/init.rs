//! CP1 integration tests: `historia init [dir]` creates the on-disk store exactly
//! per CLAUDE.md §9, handles all four targeting forms from §5, and refuses to
//! clobber an existing store. Drives the compiled binary as a subprocess in an
//! isolated temp dir (never touches the real filesystem).

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn assert_store_layout(store_dir: &Path) {
    assert!(store_dir.join("objects").is_dir(), "missing objects/ in {store_dir:?}");
    assert!(store_dir.join("snapshots").is_dir(), "missing snapshots/ in {store_dir:?}");
    assert!(!store_dir.join("lock").exists(), "lock must not be created by init");
    assert_eq!(
        fs::read_to_string(store_dir.join("format")).unwrap(),
        "historia format v1\n"
    );
    assert_eq!(fs::read_to_string(store_dir.join("HEAD")).unwrap(), "0\n");
}

#[test]
fn init_in_empty_dir_creates_full_layout() {
    let dir = tempdir().unwrap();

    Command::cargo_bin("historia")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(".historia"));

    assert_store_layout(&dir.path().join(".historia"));
}

#[test]
fn init_dot_targets_current_dir() {
    let dir = tempdir().unwrap();

    Command::cargo_bin("historia")
        .unwrap()
        .args(["init", "."])
        .current_dir(dir.path())
        .assert()
        .success();

    assert_store_layout(&dir.path().join(".historia"));
}

#[test]
fn init_dotdot_targets_parent_dir() {
    let dir = tempdir().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();

    Command::cargo_bin("historia")
        .unwrap()
        .args(["init", ".."])
        .current_dir(&sub)
        .assert()
        .success();

    assert_store_layout(&dir.path().join(".historia"));
    assert!(!sub.join(".historia").exists());
}

#[test]
fn init_path_creates_missing_directory_recursively() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("new").join("nested");
    assert!(!target.exists());

    Command::cargo_bin("historia")
        .unwrap()
        .arg("init")
        .arg(&target)
        .current_dir(dir.path())
        .assert()
        .success();

    assert!(target.is_dir());
    assert_store_layout(&target.join(".historia"));
}

#[test]
fn init_twice_fails_and_leaves_existing_store_untouched() {
    let dir = tempdir().unwrap();

    Command::cargo_bin("historia")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .success();

    let store_dir = dir.path().join(".historia");
    let head_before = fs::read_to_string(store_dir.join("HEAD")).unwrap();
    let format_before = fs::read_to_string(store_dir.join("format")).unwrap();

    Command::cargo_bin("historia")
        .unwrap()
        .arg("init")
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    assert_eq!(fs::read_to_string(store_dir.join("HEAD")).unwrap(), head_before);
    assert_eq!(fs::read_to_string(store_dir.join("format")).unwrap(), format_before);
}
