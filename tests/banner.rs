//! CP12 art enhancement: the "historia" wordmark banner appears at the top of
//! top-level help output (bare `historia`, `historia help`) so a first-time
//! user sees the identity naturally - but never on per-command help, to avoid
//! clutter. Drives the compiled binary as a subprocess.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn historia() -> Command {
    Command::cargo_bin("historia").unwrap()
}

fn wordmark_first_line() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("wordmark.txt");
    fs::read_to_string(path).unwrap().lines().next().unwrap().to_string()
}

#[test]
fn bare_historia_shows_the_wordmark() {
    let line = wordmark_first_line();

    historia().assert().success().stdout(predicate::str::contains(line));
}

#[test]
fn historia_help_shows_the_wordmark() {
    let line = wordmark_first_line();

    historia()
        .arg("help")
        .assert()
        .success()
        .stdout(predicate::str::contains(line));
}

#[test]
fn top_level_help_still_lists_all_commands() {
    let assert = historia().arg("help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    for name in ["init", "commit", "log", "status", "restore", "verify", "backup", "motd"] {
        assert!(stdout.contains(name), "help output missing command '{name}':\n{stdout}");
    }
}

#[test]
fn per_command_help_does_not_show_the_wordmark() {
    let line = wordmark_first_line();

    historia()
        .args(["help", "commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains(line).not());
}

#[test]
fn command_help_flag_also_has_no_wordmark() {
    let line = wordmark_first_line();

    historia()
        .args(["restore", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(line).not());
}
