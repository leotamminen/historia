//! CP12 integration tests: `historia motd` - offline status line, works
//! anywhere (no store required), never writes anything. Drives the compiled
//! binary as a subprocess in an isolated temp dir.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn historia() -> Command {
    Command::cargo_bin("historia").unwrap()
}

fn dir_listing(dir: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    names
}

fn known_facts() -> Vec<String> {
    let facts_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("facts.txt");
    fs::read_to_string(facts_path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn asset(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(name);
    fs::read_to_string(path).unwrap()
}

#[test]
fn motd_runs_and_exits_zero_in_a_plain_folder_with_no_store() {
    let dir = tempdir().unwrap();

    historia().arg("motd").current_dir(dir.path()).assert().success();
}

#[test]
fn motd_runs_and_exits_zero_inside_a_historia_folder() {
    let dir = tempdir().unwrap();
    historia().arg("init").current_dir(dir.path()).assert().success();

    historia().arg("motd").current_dir(dir.path()).assert().success();
}

#[test]
fn motd_prints_a_fact_from_the_embedded_list() {
    let dir = tempdir().unwrap();
    let facts = known_facts();
    assert!(!facts.is_empty(), "test setup: assets/facts.txt must not be empty");

    let assert = historia().arg("motd").current_dir(dir.path()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(
        facts.iter().any(|fact| stdout.contains(fact.as_str())),
        "motd output did not contain any known fact:\n{stdout}"
    );
}

#[test]
fn motd_shows_the_local_time_and_version() {
    let dir = tempdir().unwrap();

    historia()
        .arg("motd")
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("historia"))
        .stdout(predicate::str::is_match(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z").unwrap());
}

#[test]
fn motd_creates_no_store_and_writes_nothing_to_disk() {
    let dir = tempdir().unwrap();
    let before = dir_listing(dir.path());

    historia().arg("motd").current_dir(dir.path()).assert().success();

    assert!(!dir.path().join(".historia").exists(), "motd must never create a store");
    assert_eq!(dir_listing(dir.path()), before, "motd must write nothing to disk");
}

#[test]
fn motd_shows_both_the_icon_and_the_wordmark_before_the_status_info() {
    let dir = tempdir().unwrap();
    let wordmark = asset("wordmark.txt");
    let icon = asset("clock_icon.txt");
    let wordmark_line = wordmark.lines().next().unwrap();
    let icon_line = icon.lines().nth(1).unwrap(); // the ".-----." row, distinctive

    let assert = historia().arg("motd").current_dir(dir.path()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    let icon_pos = stdout.find(icon_line).expect("icon not found in motd output");
    let wordmark_pos = stdout.find(wordmark_line).expect("wordmark not found in motd output");
    let time_pos = stdout.find("Time (UTC)").expect("status info not found in motd output");

    assert!(icon_pos < wordmark_pos, "icon must appear before the wordmark");
    assert!(wordmark_pos < time_pos, "art must appear before the status info");
}

#[test]
fn motd_never_takes_the_lock_when_run_inside_an_existing_store() {
    let dir = tempdir().unwrap();
    historia().arg("init").current_dir(dir.path()).assert().success();
    let before = dir_listing(&dir.path().join(".historia"));

    historia().arg("motd").current_dir(dir.path()).assert().success();

    assert!(!dir.path().join(".historia").join("lock").exists(), "motd must never take the lock");
    assert_eq!(
        dir_listing(&dir.path().join(".historia")),
        before,
        "motd must not modify an existing store"
    );
}
