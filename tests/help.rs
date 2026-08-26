//! CP9 integration tests: `help <command>` detail, `-h`/`--help`/`man`/`info`
//! aliases, and `<command> --help`, all generated from the `cli::COMMANDS`
//! registry (CLAUDE.md §8). Drives the compiled binary as a subprocess.

use assert_cmd::Command;
use predicates::prelude::*;

fn historia() -> Command {
    Command::cargo_bin("historia").unwrap()
}

fn stdout_of(cmd: &mut Command) -> String {
    let assert = cmd.assert().success();
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

#[test]
fn help_init_shows_usage_and_dir_argument() {
    historia()
        .args(["help", "init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("historia init"))
        .stdout(predicate::str::contains("[dir]"));
}

#[test]
fn help_commit_shows_message_flag_and_allow_empty() {
    historia()
        .args(["help", "commit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("-m"))
        .stdout(predicate::str::contains("--message"))
        .stdout(predicate::str::contains("--allow-empty"));
}

#[test]
fn help_log_shows_usage() {
    historia()
        .args(["help", "log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("historia log"));
}

#[test]
fn help_status_shows_usage() {
    historia()
        .args(["help", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("historia status"));
}

#[test]
fn help_restore_shows_usage_and_n_and_path_arguments() {
    historia()
        .args(["help", "restore"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("<n>"))
        .stdout(predicate::str::contains("[path]"));
}

#[test]
fn help_verify_shows_usage() {
    historia()
        .args(["help", "verify"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("historia verify"));
}

#[test]
fn bare_help_flags_and_aliases_all_produce_the_same_general_help() {
    let baseline = stdout_of(historia().arg("help"));

    for invocation in [vec!["-h"], vec!["--help"], vec!["man"], vec!["info"]] {
        let out = stdout_of(historia().args(&invocation));
        assert_eq!(out, baseline, "{invocation:?} did not match `historia help`'s output");
    }
}

#[test]
fn man_and_info_with_a_target_behave_like_help_with_a_target() {
    let baseline = stdout_of(historia().args(["help", "restore"]));

    for invocation in [vec!["man", "restore"], vec!["info", "restore"]] {
        let out = stdout_of(historia().args(&invocation));
        assert_eq!(out, baseline, "{invocation:?} did not match `historia help restore`'s output");
    }
}

#[test]
fn command_help_flag_matches_help_command_detail() {
    let baseline = stdout_of(historia().args(["help", "commit"]));
    let via_long_flag = stdout_of(historia().args(["commit", "--help"]));
    let via_short_flag = stdout_of(historia().args(["commit", "-h"]));

    assert_eq!(via_long_flag, baseline);
    assert_eq!(via_short_flag, baseline);
}

#[test]
fn restore_short_help_flag_matches_help_restore() {
    let baseline = stdout_of(historia().args(["help", "restore"]));
    let via_flag = stdout_of(historia().args(["restore", "-h"]));

    assert_eq!(via_flag, baseline);
}

#[test]
fn help_flag_does_not_actually_run_the_command() {
    // `commit --help` must show help, not fail trying to commit with no store.
    historia()
        .args(["commit", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn unknown_help_target_exits_non_zero_and_lists_valid_commands() {
    let assert = historia().args(["help", "frobnicate"]).assert().failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();

    assert!(stderr.contains("frobnicate"), "stderr should name the unknown command:\n{stderr}");
    for name in ["init", "commit", "log", "status", "restore", "verify"] {
        assert!(stderr.contains(name), "stderr should list valid command '{name}':\n{stderr}");
    }
}
