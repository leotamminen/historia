//! CP0 smoke test: `--version` and `help` work, unknown commands fail cleanly.
//! Drives the compiled binary as a subprocess (CLAUDE.md §5, §6 - every command is
//! non-interactive with clean exit codes).

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn version_flag_prints_version_and_exits_zero() {
    Command::cargo_bin("historia")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_lists_all_mvp_commands_and_exits_zero() {
    let assert = Command::cargo_bin("historia")
        .unwrap()
        .arg("help")
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    for name in ["init", "commit", "log", "status", "restore", "verify"] {
        assert!(
            stdout.contains(name),
            "help output missing command '{name}':\n{stdout}"
        );
    }
}

#[test]
fn unknown_command_exits_non_zero() {
    Command::cargo_bin("historia")
        .unwrap()
        .arg("frobnicate")
        .assert()
        .failure();
}
