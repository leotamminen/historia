//! CP14 integration tests: Ed25519 snapshot signing. A signing keypair
//! auto-generates on the first commit that needs one; `historia keygen`
//! explicitly (re)generates it; `verify` checks every signed snapshot's
//! signature and reports tampering distinctly from a broken chain link.
//! Drives the compiled binary as a subprocess in an isolated temp dir.

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
    historia()
        .args(["commit", "-m", message])
        .current_dir(dir)
        .assert()
}

fn verify(dir: &Path) -> assert_cmd::assert::Assert {
    historia().arg("verify").current_dir(dir).assert()
}

fn keygen(dir: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut full = vec!["keygen"];
    full.extend_from_slice(args);
    historia().args(full).current_dir(dir).assert()
}

fn private_key_path(dir: &Path) -> std::path::PathBuf {
    dir.join(".historia").join("signing_key")
}

fn public_key_path(dir: &Path) -> std::path::PathBuf {
    dir.join(".historia").join("signing_key.pub")
}

fn signature_path(dir: &Path, number: u64) -> std::path::PathBuf {
    dir.join(".historia").join("snapshots").join(format!("{number}.json.sig"))
}

fn manifest_path(dir: &Path, number: u64) -> std::path::PathBuf {
    dir.join(".historia").join("snapshots").join(format!("{number}.json"))
}

#[test]
fn fresh_store_first_commit_auto_generates_a_key_and_is_signed_and_verified() {
    let dir = tempdir().unwrap();
    init(dir.path());
    assert!(!private_key_path(dir.path()).exists());

    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first").success();

    assert!(private_key_path(dir.path()).is_file(), "key should auto-generate on first signed commit");
    assert!(public_key_path(dir.path()).is_file());
    assert!(signature_path(dir.path(), 1).is_file(), "snapshot 1 should be signed");

    verify(dir.path())
        .success()
        .stdout(predicate::str::contains("store OK"));
}

#[test]
fn key_missing_after_having_existed_fails_the_next_commit_cleanly_and_keygen_recovers() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first").success();
    assert!(private_key_path(dir.path()).is_file());

    // Simulate the private key having been deleted after it once existed.
    fs::remove_file(private_key_path(dir.path())).unwrap();

    fs::write(dir.path().join("a.txt"), b"2").unwrap();
    let assert = commit(dir.path(), "second").failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("signing key missing") && stderr.contains("keygen"),
        "expected a clean 'signing key missing... keygen' message, got: {stderr}"
    );
    // The second commit must not have silently succeeded/advanced HEAD.
    assert_eq!(
        fs::read_to_string(dir.path().join(".historia").join("HEAD")).unwrap().trim(),
        "1"
    );

    // `historia keygen` recovers (regenerates both files); requires --force
    // since the public key file is still present (a key "exists").
    keygen(dir.path(), &[]).failure();
    keygen(dir.path(), &["--force"]).success();
    assert!(private_key_path(dir.path()).is_file());

    // Commits now succeed and sign again.
    let assert = commit(dir.path(), "second, retried").success();
    assert
        .stdout(predicate::str::contains("snapshot 2"));
    assert!(signature_path(dir.path(), 2).is_file());
}

#[test]
fn keygen_without_force_refuses_if_a_key_exists_and_with_force_overwrites() {
    let dir = tempdir().unwrap();
    init(dir.path());
    keygen(dir.path(), &[]).success();
    let original_pub = fs::read_to_string(public_key_path(dir.path())).unwrap();

    keygen(dir.path(), &[])
        .failure()
        .stderr(predicate::str::contains("--force"));
    assert_eq!(fs::read_to_string(public_key_path(dir.path())).unwrap(), original_pub, "refused keygen must not touch the existing key");

    keygen(dir.path(), &["--force"]).success();
    let new_pub = fs::read_to_string(public_key_path(dir.path())).unwrap();
    assert_ne!(new_pub, original_pub, "--force must actually generate a new key");
}

#[test]
fn tampering_with_a_signed_manifest_is_reported_as_an_invalid_signature_not_a_chain_break() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first").success();

    let m1_path = manifest_path(dir.path(), 1);
    let mut m1: Value = serde_json::from_str(&fs::read_to_string(&m1_path).unwrap()).unwrap();
    m1["message"] = Value::String("tampered".to_string());
    fs::write(&m1_path, serde_json::to_string_pretty(&m1).unwrap()).unwrap();

    let assert = verify(dir.path()).failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains('1'), "expected the report to name snapshot 1:\n{stdout}");
    assert!(
        stdout.to_lowercase().contains("signature invalid"),
        "expected the report to clearly say 'signature invalid':\n{stdout}"
    );
}

#[test]
fn a_mix_of_unsigned_and_signed_snapshots_verifies_ok_and_checks_only_the_signed_ones() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first").success();

    // Simulate snapshot 1 predating any key: remove its signature sidecar.
    fs::remove_file(signature_path(dir.path(), 1)).unwrap();

    fs::write(dir.path().join("a.txt"), b"2").unwrap();
    commit(dir.path(), "second").success();
    assert!(signature_path(dir.path(), 2).is_file());

    verify(dir.path())
        .success()
        .stdout(predicate::str::contains("store OK"));
}
