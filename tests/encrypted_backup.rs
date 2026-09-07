//! CP15 integration tests: `historia backup <path> --encrypt` produces a
//! single opaque encrypted file; `historia backup --decrypt <file>
//! <destination>` reverses it. Builds on CP11's backup and CP14's key
//! lifecycle pattern. Drives the compiled binary as a subprocess in an
//! isolated temp dir.

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

fn encrypt_backup(dir: &Path, dest_file: &Path) -> assert_cmd::assert::Assert {
    historia()
        .args(["backup", dest_file.to_str().unwrap(), "--encrypt"])
        .current_dir(dir)
        .assert()
}

fn decrypt_backup(dir: &Path, source_file: &Path, dest: &Path) -> assert_cmd::assert::Assert {
    historia()
        .args(["backup", "--decrypt", source_file.to_str().unwrap(), dest.to_str().unwrap()])
        .current_dir(dir)
        .assert()
}

fn log_output(dir: &Path) -> String {
    let assert = historia().arg("log").current_dir(dir).assert().success();
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

fn store_bytes(store_dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    fn collect(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, out);
            } else if path.is_file() {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    collect(store_dir, store_dir, &mut out);
    out
}

#[test]
fn encrypt_produces_one_file_and_decrypt_produces_a_store_that_verifies_and_matches_the_log() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"hello").unwrap();
    commit(source.path(), "first");
    fs::write(source.path().join("b.txt"), b"world").unwrap();
    commit(source.path(), "second");

    let backup_dir = tempdir().unwrap();
    let encrypted_file = backup_dir.path().join("backup.age");

    encrypt_backup(source.path(), &encrypted_file)
        .success()
        .stdout(predicate::str::contains("2 snapshot(s)"));

    assert!(encrypted_file.is_file(), "the encrypted backup must be a single file");
    assert!(!encrypted_file.is_dir());
    // Not a directory containing a .historia at all.
    assert!(!encrypted_file.join(".historia").exists());

    let source_log = log_output(source.path());

    let dest = tempdir().unwrap();
    decrypt_backup(source.path(), &encrypted_file, dest.path())
        .success()
        .stdout(predicate::str::contains("2 snapshot(s)"));

    historia()
        .arg("verify")
        .current_dir(dest.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("store OK"));

    let dest_log = log_output(dest.path());
    // Timestamps make lines identical anyway since content round-trips
    // byte-for-byte; compare directly.
    assert_eq!(dest_log, source_log, "decrypted store's log must match the source's");
}

#[test]
fn key_auto_generates_on_first_encrypted_backup() {
    let dir = tempdir().unwrap();
    init(dir.path());
    assert!(!dir.path().join(".historia").join("age_key").exists());

    let backup_dir = tempdir().unwrap();
    encrypt_backup(dir.path(), &backup_dir.path().join("backup.age")).success();

    assert!(dir.path().join(".historia").join("age_key").is_file());
    assert!(dir.path().join(".historia").join("age_key.pub").is_file());
}

#[test]
fn key_missing_after_having_existed_fails_the_next_encrypt_cleanly_and_keygen_recovers() {
    let dir = tempdir().unwrap();
    init(dir.path());
    let backup_dir = tempdir().unwrap();
    encrypt_backup(dir.path(), &backup_dir.path().join("first.age")).success();
    assert!(dir.path().join(".historia").join("age_key").is_file());

    // Simulate the identity having been deleted after it once existed.
    fs::remove_file(dir.path().join(".historia").join("age_key")).unwrap();

    let assert = encrypt_backup(dir.path(), &backup_dir.path().join("second.age")).failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("age encryption key missing") && stderr.contains("keygen --encryption"),
        "expected a clean 'key missing... keygen --encryption' message, got: {stderr}"
    );
    assert!(
        !backup_dir.path().join("second.age").exists(),
        "a failed encrypt must not leave a partial output file"
    );

    // `historia keygen --encryption` recovers (regenerates both files);
    // requires --force since the public key file is still present.
    historia()
        .args(["keygen", "--encryption"])
        .current_dir(dir.path())
        .assert()
        .failure();
    historia()
        .args(["keygen", "--encryption", "--force"])
        .current_dir(dir.path())
        .assert()
        .success();
    assert!(dir.path().join(".historia").join("age_key").is_file());

    // Encrypted backups now succeed again.
    encrypt_backup(dir.path(), &backup_dir.path().join("second.age")).success();
    assert!(backup_dir.path().join("second.age").is_file());
}

#[test]
fn keygen_encryption_without_force_refuses_if_a_key_exists_and_with_force_overwrites() {
    let dir = tempdir().unwrap();
    init(dir.path());
    historia().args(["keygen", "--encryption"]).current_dir(dir.path()).assert().success();
    let original_pub = fs::read_to_string(dir.path().join(".historia").join("age_key.pub")).unwrap();

    historia()
        .args(["keygen", "--encryption"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    assert_eq!(
        fs::read_to_string(dir.path().join(".historia").join("age_key.pub")).unwrap(),
        original_pub,
        "a refused keygen must not touch the existing key"
    );

    historia()
        .args(["keygen", "--encryption", "--force"])
        .current_dir(dir.path())
        .assert()
        .success();
    let new_pub = fs::read_to_string(dir.path().join(".historia").join("age_key.pub")).unwrap();
    assert_ne!(new_pub, original_pub, "--force must actually generate a new key");
}

#[test]
fn decrypting_with_the_wrong_key_fails_cleanly_and_produces_no_store() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"secret").unwrap();
    commit(source.path(), "first");

    let backup_dir = tempdir().unwrap();
    let encrypted_file = backup_dir.path().join("backup.age");
    encrypt_backup(source.path(), &encrypted_file).success();

    // A second, unrelated store with its own (different) age identity.
    let other = tempdir().unwrap();
    init(other.path());
    historia().args(["keygen", "--encryption"]).current_dir(other.path()).assert().success();

    let dest = tempdir().unwrap();
    let assert = decrypt_backup(other.path(), &encrypted_file, dest.path()).failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("decryption failed"), "expected a clean decryption-failed message, got: {stderr}");

    assert!(!dest.path().join(".historia").exists(), "a failed decrypt must not produce a store");
}

#[test]
fn decrypting_with_no_key_at_all_fails_cleanly() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"secret").unwrap();
    commit(source.path(), "first");

    let backup_dir = tempdir().unwrap();
    let encrypted_file = backup_dir.path().join("backup.age");
    encrypt_backup(source.path(), &encrypted_file).success();

    // A fresh store with no age key at all.
    let other = tempdir().unwrap();
    init(other.path());

    let dest = tempdir().unwrap();
    let assert = decrypt_backup(other.path(), &encrypted_file, dest.path()).failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("no age encryption key found"),
        "expected a clean 'no age encryption key found' message, got: {stderr}"
    );
    assert!(!dest.path().join(".historia").exists());
}

#[test]
fn source_store_is_unchanged_after_an_encrypted_backup() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"hello").unwrap();
    commit(source.path(), "first");
    // Pre-generate the key so the encrypt call below causes no mutation at
    // all (auto-generation is the one expected exception, already covered by
    // its own test above).
    historia().args(["keygen", "--encryption"]).current_dir(source.path()).assert().success();

    let before = store_bytes(&source.path().join(".historia"));

    let backup_dir = tempdir().unwrap();
    encrypt_backup(source.path(), &backup_dir.path().join("backup.age")).success();

    assert_eq!(store_bytes(&source.path().join(".historia")), before, "encrypted backup must not modify the source");
    assert!(
        !source.path().join(".historia").join("lock").exists(),
        "encrypted backup must never take the lock"
    );
}

#[test]
fn encrypt_refuses_an_existing_destination_file_without_force() {
    let dir = tempdir().unwrap();
    init(dir.path());
    let backup_dir = tempdir().unwrap();
    let dest = backup_dir.path().join("backup.age");
    fs::write(&dest, b"not a real backup").unwrap();

    encrypt_backup(dir.path(), &dest)
        .failure()
        .stderr(predicate::str::contains("--force"));

    assert_eq!(fs::read(&dest).unwrap(), b"not a real backup");
}

#[test]
fn decrypt_refuses_an_existing_destination_store_without_force() {
    let source = tempdir().unwrap();
    init(source.path());
    fs::write(source.path().join("a.txt"), b"hello").unwrap();
    commit(source.path(), "first");
    let backup_dir = tempdir().unwrap();
    let encrypted_file = backup_dir.path().join("backup.age");
    encrypt_backup(source.path(), &encrypted_file).success();

    let dest = tempdir().unwrap();
    init(dest.path());

    decrypt_backup(source.path(), &encrypted_file, dest.path())
        .failure()
        .stderr(predicate::str::contains("--force"));
}
