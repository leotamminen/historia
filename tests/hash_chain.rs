//! CP13 integration tests: hash-chain / tamper-evident history. Each manifest
//! (after the first) records the SHA-256 hash of its parent manifest's exact
//! on-disk bytes; `verify` recomputes and checks every such link. Pre-CP13
//! manifests (no `parent_hash` field at all) are "pre-chain" and never fail
//! this check. Drives the compiled binary as a subprocess in an isolated temp
//! dir.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
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

fn verify(dir: &Path) -> assert_cmd::assert::Assert {
    historia().arg("verify").current_dir(dir).assert()
}

fn manifest_path(dir: &Path, number: u64) -> std::path::PathBuf {
    dir.join(".historia").join("snapshots").join(format!("{number}.json"))
}

fn read_manifest_json(dir: &Path, number: u64) -> Value {
    serde_json::from_str(&fs::read_to_string(manifest_path(dir, number)).unwrap()).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn each_snapshot_after_the_first_has_a_correct_parent_hash_and_verify_confirms_the_chain() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first");
    fs::write(dir.path().join("a.txt"), b"2").unwrap();
    commit(dir.path(), "second");
    fs::write(dir.path().join("a.txt"), b"3").unwrap();
    commit(dir.path(), "third");

    // Snapshot 1 is a fresh store's genesis: no parent to hash, so no field.
    let m1 = read_manifest_json(dir.path(), 1);
    assert!(m1.get("parent_hash").is_none(), "snapshot 1 (genesis) must have no parent_hash");

    // Snapshot 2's parent_hash must equal an independently computed SHA-256 of
    // snapshot 1's exact on-disk bytes.
    let m1_bytes = fs::read(manifest_path(dir.path(), 1)).unwrap();
    let m2 = read_manifest_json(dir.path(), 2);
    assert_eq!(m2["parent_hash"], sha256_hex(&m1_bytes));

    // Same for snapshot 3 -> snapshot 2.
    let m2_bytes = fs::read(manifest_path(dir.path(), 2)).unwrap();
    let m3 = read_manifest_json(dir.path(), 3);
    assert_eq!(m3["parent_hash"], sha256_hex(&m2_bytes));

    verify(dir.path())
        .success()
        .stdout(predicate::str::contains("store OK"));
}

#[test]
fn tampering_with_a_past_manifest_is_detected_by_verify() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first");
    fs::write(dir.path().join("a.txt"), b"2").unwrap();
    commit(dir.path(), "second");

    // Edit snapshot 1's manifest after snapshot 2's parent_hash was computed
    // against its original bytes.
    let m1_path = manifest_path(dir.path(), 1);
    let mut m1: Value = serde_json::from_str(&fs::read_to_string(&m1_path).unwrap()).unwrap();
    m1["message"] = Value::String("tampered".to_string());
    fs::write(&m1_path, serde_json::to_string_pretty(&m1).unwrap()).unwrap();

    verify(dir.path())
        .failure()
        .stdout(predicate::str::contains('2'))
        .stdout(predicate::str::contains("chain"));
}

#[test]
fn pre_chain_manifests_mixed_with_chained_ones_still_verify_ok() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first");
    fs::write(dir.path().join("a.txt"), b"2").unwrap();
    commit(dir.path(), "second");

    // Simulate snapshot 2 predating CP13: strip its parent_hash field
    // entirely (not just null it - genuinely absent, like an old manifest).
    let m2_path = manifest_path(dir.path(), 2);
    let mut m2: Value = serde_json::from_str(&fs::read_to_string(&m2_path).unwrap()).unwrap();
    m2.as_object_mut().unwrap().remove("parent_hash");
    fs::write(&m2_path, serde_json::to_string_pretty(&m2).unwrap()).unwrap();

    // A later, newly chained snapshot anchors to snapshot 2's now-current bytes.
    fs::write(dir.path().join("a.txt"), b"3").unwrap();
    commit(dir.path(), "third");

    verify(dir.path())
        .success()
        .stdout(predicate::str::contains("store OK"));
}

#[test]
fn parent_hash_is_computed_identically_whether_checked_by_verify_or_recorded_at_commit_time() {
    let dir = tempdir().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), b"1").unwrap();
    commit(dir.path(), "first");
    fs::write(dir.path().join("a.txt"), b"2").unwrap();
    commit(dir.path(), "second");

    // Recorded at commit time...
    let recorded = read_manifest_json(dir.path(), 2)["parent_hash"].as_str().unwrap().to_string();
    // ...vs. independently recomputed here, the same way verify would.
    let recomputed = sha256_hex(&fs::read(manifest_path(dir.path(), 1)).unwrap());

    assert_eq!(recorded, recomputed);
    verify(dir.path()).success();
}
