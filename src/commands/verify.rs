//! `historia verify` - re-hash every blob and check every manifest reference
//! resolves (CLAUDE.md §8, §9). Read-only: never writes, never takes the lock.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;

use crate::core::{hash, signing, snapshot, store};
use crate::format::manifest::FORMAT_MARKER;

/// The result of checking a store: counts, and a list of every problem found
/// (empty means the store is fully intact - see [`Report::is_ok`]).
#[derive(Debug, Default)]
struct Report {
    objects_checked: usize,
    snapshots_checked: usize,
    problems: Vec<String>,
}

impl Report {
    fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub fn run(_args: &[String]) -> Result<(), String> {
    let cwd = env::current_dir()
        .map_err(|e| format!("historia verify: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia verify: not a historia store (run 'historia init' first)".to_string()
    })?;

    let report = check_store(&store_dir);
    print_report(&report);

    if report.is_ok() {
        Ok(())
    } else {
        Err(format!("historia verify: {} problem(s) found", report.problems.len()))
    }
}

fn print_report(report: &Report) {
    if report.is_ok() {
        println!(
            "{} object(s) checked, {} snapshot(s) checked - store OK",
            report.objects_checked, report.snapshots_checked
        );
        return;
    }

    println!(
        "{} object(s) checked, {} snapshot(s) checked",
        report.objects_checked, report.snapshots_checked
    );
    println!("{} problem(s) found:", report.problems.len());
    for problem in &report.problems {
        println!("  - {problem}");
    }
}

/// Check everything CP8 asks for: the format marker, `HEAD`'s validity, every
/// object's content against its claimed hash, and every manifest entry's hash
/// against what's actually stored. Never stops at the first problem - collects
/// all of them, since this is the command a user runs to find out everything
/// that might be wrong before trusting a backup.
fn check_store(store_dir: &Path) -> Report {
    let mut report = Report::default();

    check_format_marker(store_dir, &mut report);

    let numbers = match snapshot::list_snapshot_numbers(store_dir) {
        Ok(numbers) => numbers,
        Err(e) => {
            report.problems.push(format!("cannot list snapshots/: {e}"));
            Vec::new()
        }
    };

    check_head(store_dir, &numbers, &mut report);

    let present_hashes = check_objects(store_dir, &mut report);
    check_manifests(store_dir, &numbers, &present_hashes, &mut report);

    report
}

fn check_format_marker(store_dir: &Path, report: &mut Report) {
    match fs::read_to_string(store_dir.join("format")) {
        Ok(contents) if contents == FORMAT_MARKER => {}
        Ok(contents) => report
            .problems
            .push(format!("format marker mismatch: expected {FORMAT_MARKER:?}, found {contents:?}")),
        Err(e) => report.problems.push(format!("cannot read format marker: {e}")),
    }
}

fn check_head(store_dir: &Path, known_numbers: &[u64], report: &mut Report) {
    let head = match snapshot::read_head(store_dir) {
        Ok(head) => head,
        Err(e) => {
            report.problems.push(format!("cannot read HEAD: {e}"));
            return;
        }
    };
    // 0 is the CP1 "no snapshots yet" sentinel - always valid, regardless of
    // what (if anything) is under snapshots/.
    if head != 0 && !known_numbers.contains(&head) {
        report
            .problems
            .push(format!("HEAD points to snapshot {head}, which does not exist"));
    }
}

/// Re-hash every stored object against its claimed name (Rule 11: streamed, via
/// `hash::hash_file` - never the whole blob in memory). Returns the set of
/// hashes that are actually *present* (by filename), regardless of whether
/// their content is corrupted - `check_manifests` uses this presence set for
/// dangling-reference checks, since a reference to a present-but-corrupted blob
/// is already reported once here, not a second time as "dangling".
fn check_objects(store_dir: &Path, report: &mut Report) -> HashSet<String> {
    let blobs = match store::list_blobs(store_dir) {
        Ok(blobs) => blobs,
        Err(e) => {
            report.problems.push(format!("cannot list objects/: {e}"));
            return HashSet::new();
        }
    };

    let mut present = HashSet::with_capacity(blobs.len());
    for (claimed_hash, path) in &blobs {
        present.insert(claimed_hash.clone());
        match hash::hash_file(path) {
            Ok(actual_hash) if &actual_hash == claimed_hash => {}
            Ok(actual_hash) => report.problems.push(format!(
                "corrupted object {claimed_hash}: content actually hashes to {actual_hash}"
            )),
            Err(e) => report.problems.push(format!("cannot read object {claimed_hash}: {e}")),
        }
        report.objects_checked += 1;
    }
    present
}

/// Check every discovered manifest: its entries resolve to stored objects
/// (CP8), and - CP13 - if it carries a `parent_hash`, that it still matches its
/// parent's actual on-disk bytes. A manifest with no `parent_hash` (pre-chain,
/// or the very first snapshot ever) is not chain-checked at all - CLAUDE.md
/// CP13's explicit "chain starts from new snapshots" decision, not an error.
fn check_manifests(store_dir: &Path, numbers: &[u64], present_hashes: &HashSet<String>, report: &mut Report) {
    for &number in numbers {
        report.snapshots_checked += 1;
        let manifest = match snapshot::read_manifest(store_dir, number) {
            Ok(manifest) => manifest,
            Err(e) => {
                report.problems.push(format!("snapshot {number}: cannot read manifest: {e}"));
                continue;
            }
        };
        for entry in &manifest.entries {
            if !present_hashes.contains(&entry.hash) {
                report.problems.push(format!(
                    "snapshot {number}: '{}' references missing object {}",
                    entry.path, entry.hash
                ));
            }
        }

        if let Some(expected_parent_hash) = &manifest.parent_hash {
            match snapshot::hash_manifest_file(store_dir, manifest.parent) {
                Ok(actual_parent_hash) if &actual_parent_hash == expected_parent_hash => {}
                Ok(actual_parent_hash) => report.problems.push(format!(
                    "snapshot {number}: chain broken - recorded parent_hash {expected_parent_hash} \
                     does not match snapshot {}'s actual hash {actual_parent_hash} (tampered?)",
                    manifest.parent
                )),
                Err(e) => report.problems.push(format!(
                    "snapshot {number}: chain broken - cannot hash parent snapshot {}: {e}",
                    manifest.parent
                )),
            }
        }

        check_signature(store_dir, number, report);
    }
}

/// If snapshot `number` has a signature sidecar file (CLAUDE.md CP14), verify
/// it against the manifest's exact on-disk bytes using the store's current
/// public key. No sidecar file at all means "unsigned" (pre-CP14, or created
/// before any key existed) - not an error, simply not checked. A sidecar file
/// present with no usable public key to check it against IS reported: a claimed
/// signature nobody can confirm is a real integrity concern, not silently
/// ignorable the way "never signed at all" is.
fn check_signature(store_dir: &Path, number: u64, report: &mut Report) {
    let sig_path = signing::signature_path(store_dir, number);
    if !sig_path.is_file() {
        return;
    }

    let signature = match signing::read_signature(store_dir, number) {
        Ok(sig) => sig,
        Err(e) => {
            report.problems.push(format!("snapshot {number}: cannot read signature file: {e}"));
            return;
        }
    };

    let verifying_key = match signing::load_verifying_key(store_dir) {
        Ok(key) => key,
        Err(e) => {
            report
                .problems
                .push(format!("snapshot {number}: cannot verify signature - no usable public key: {e}"));
            return;
        }
    };

    let manifest_bytes = match fs::read(snapshot::manifest_path(store_dir, number)) {
        Ok(bytes) => bytes,
        Err(e) => {
            report
                .problems
                .push(format!("snapshot {number}: cannot verify signature - cannot read manifest: {e}"));
            return;
        }
    };

    if !signing::verify_signature(&verifying_key, &manifest_bytes, &signature) {
        report.problems.push(format!(
            "snapshot {number}: signature invalid - the manifest does not match its recorded \
             signature (tampered, or signed by a different key)"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::store::{init_store, write_blob};
    use crate::format::manifest::{Entry, Manifest, FORMAT_MARKER};
    use std::fs;
    use tempfile::tempdir;

    fn manifest(number: u64, entries: Vec<Entry>) -> Manifest {
        Manifest {
            number,
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            message: "test".to_string(),
            parent: number.saturating_sub(1),
            parent_hash: None,
            entries,
        }
    }

    #[test]
    fn fresh_empty_store_is_ok() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "{report:?}");
        assert_eq!(report.objects_checked, 0);
        assert_eq!(report.snapshots_checked, 0);
    }

    #[test]
    fn a_correctly_committed_store_is_ok_with_correct_counts() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let hash = write_blob(&store_dir, &mut &b"content"[..]).unwrap();
        let m = manifest(1, vec![Entry { path: "a.txt".into(), hash, mode: 0o644 }]);
        crate::core::snapshot::write_manifest(&store_dir, &m).unwrap();
        crate::core::snapshot::write_head(&store_dir, 1).unwrap();

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "{report:?}");
        assert_eq!(report.objects_checked, 1);
        assert_eq!(report.snapshots_checked, 1);
    }

    #[test]
    fn a_corrupted_blob_is_detected_and_named() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let hash = write_blob(&store_dir, &mut &b"original content"[..]).unwrap();
        let blob_path = store_dir.join("objects").join(&hash[..2]).join(&hash[2..]);
        fs::write(&blob_path, b"CORRUPTED!!!!!!!").unwrap();

        let report = check_store(&store_dir);

        assert!(!report.is_ok());
        assert!(
            report.problems.iter().any(|p| p.contains(&hash)),
            "expected a problem naming {hash}, got {:?}",
            report.problems
        );
    }

    #[test]
    fn a_missing_blob_referenced_by_a_manifest_is_a_dangling_reference() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let hash = write_blob(&store_dir, &mut &b"will be deleted"[..]).unwrap();
        let blob_path = store_dir.join("objects").join(&hash[..2]).join(&hash[2..]);
        let m = manifest(1, vec![Entry { path: "gone.txt".into(), hash: hash.clone(), mode: 0o644 }]);
        crate::core::snapshot::write_manifest(&store_dir, &m).unwrap();
        crate::core::snapshot::write_head(&store_dir, 1).unwrap();
        fs::remove_file(&blob_path).unwrap();

        let report = check_store(&store_dir);

        assert!(!report.is_ok());
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains('1') && p.contains("gone.txt") && p.contains(&hash)),
            "expected a dangling-reference problem naming snapshot 1, gone.txt, {hash}; got {:?}",
            report.problems
        );
    }

    #[test]
    fn a_bad_format_marker_is_reported() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        fs::write(store_dir.join("format"), "not the right marker\n").unwrap();

        let report = check_store(&store_dir);

        assert!(!report.is_ok());
        assert!(report.problems.iter().any(|p| p.to_lowercase().contains("format")));
    }

    #[test]
    fn a_good_format_marker_is_not_reported() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(store_dir.join("format")).unwrap(), FORMAT_MARKER);

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "{report:?}");
    }

    #[test]
    fn an_out_of_range_head_is_reported() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let m = manifest(1, vec![]);
        crate::core::snapshot::write_manifest(&store_dir, &m).unwrap();
        crate::core::snapshot::write_head(&store_dir, 99).unwrap();

        let report = check_store(&store_dir);

        assert!(!report.is_ok());
        assert!(report.problems.iter().any(|p| p.contains("HEAD") && p.contains("99")));
    }

    // ---- hash chain (CP13) ----

    fn write_chained(store_dir: &std::path::Path, m1: &Manifest) -> Manifest {
        crate::core::snapshot::write_manifest(store_dir, m1).unwrap();
        let parent_hash = crate::core::snapshot::hash_manifest_file(store_dir, m1.number).unwrap();
        let mut m2 = manifest(m1.number + 1, vec![]);
        m2.parent_hash = Some(parent_hash);
        crate::core::snapshot::write_manifest(store_dir, &m2).unwrap();
        m2
    }

    #[test]
    fn a_correct_chain_link_is_not_reported() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let m1 = manifest(1, vec![]);
        write_chained(&store_dir, &m1);
        crate::core::snapshot::write_head(&store_dir, 2).unwrap();

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "{report:?}");
    }

    #[test]
    fn tampering_with_a_past_manifest_breaks_the_chain_link() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let m1 = manifest(1, vec![]);
        write_chained(&store_dir, &m1);
        crate::core::snapshot::write_head(&store_dir, 2).unwrap();

        // Tamper with snapshot 1 after snapshot 2's parent_hash was computed.
        let mut tampered = m1.clone();
        tampered.message = "tampered!".to_string();
        crate::core::snapshot::write_manifest(&store_dir, &tampered).unwrap();

        let report = check_store(&store_dir);

        assert!(!report.is_ok());
        assert!(
            report.problems.iter().any(|p| p.contains('2') && p.to_lowercase().contains("chain")),
            "expected a chain-broken problem naming snapshot 2, got {:?}",
            report.problems
        );
    }

    #[test]
    fn a_pre_chain_manifest_with_no_parent_hash_is_not_chain_checked() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        // No parent_hash at all - simulates a pre-CP13 manifest. Even though
        // its `parent` field points at a nonexistent snapshot 0-adjacent
        // number, that alone is not a chain error since there's no
        // parent_hash to check.
        let m = manifest(1, vec![]);
        crate::core::snapshot::write_manifest(&store_dir, &m).unwrap();
        crate::core::snapshot::write_head(&store_dir, 1).unwrap();

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "pre-chain manifest must not be chain-checked: {report:?}");
    }

    #[test]
    fn a_mix_of_pre_chain_and_chained_manifests_only_checks_the_chained_part() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        // Snapshot 1: pre-chain (no parent_hash).
        let m1 = manifest(1, vec![]);
        crate::core::snapshot::write_manifest(&store_dir, &m1).unwrap();
        // Snapshot 2: also pre-chain - simulates two old manifests in a row.
        let m2 = manifest(2, vec![]);
        crate::core::snapshot::write_manifest(&store_dir, &m2).unwrap();
        // Snapshot 3: chained, anchored to snapshot 2's real on-disk bytes.
        let parent_hash = crate::core::snapshot::hash_manifest_file(&store_dir, 2).unwrap();
        let mut m3 = manifest(3, vec![]);
        m3.parent_hash = Some(parent_hash);
        crate::core::snapshot::write_manifest(&store_dir, &m3).unwrap();
        crate::core::snapshot::write_head(&store_dir, 3).unwrap();

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "{report:?}");
    }

    // ---- signatures (CP14) ----

    fn write_signed(store_dir: &std::path::Path, number: u64) {
        let m = manifest(number, vec![]);
        crate::core::snapshot::write_manifest(store_dir, &m).unwrap();
        let key = signing::ensure_key(store_dir).unwrap();
        let bytes = fs::read(crate::core::snapshot::manifest_path(store_dir, number)).unwrap();
        let sig = signing::sign(&key, &bytes);
        signing::write_signature(store_dir, number, &sig).unwrap();
    }

    #[test]
    fn a_correctly_signed_snapshot_is_not_reported() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        write_signed(&store_dir, 1);
        crate::core::snapshot::write_head(&store_dir, 1).unwrap();

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "{report:?}");
    }

    #[test]
    fn tampering_with_a_signed_manifest_is_reported_as_an_invalid_signature() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        write_signed(&store_dir, 1);
        crate::core::snapshot::write_head(&store_dir, 1).unwrap();

        let mut tampered = manifest(1, vec![]);
        tampered.message = "tampered!".to_string();
        crate::core::snapshot::write_manifest(&store_dir, &tampered).unwrap();

        let report = check_store(&store_dir);

        assert!(!report.is_ok());
        assert!(
            report.problems.iter().any(|p| p.contains('1') && p.contains("signature invalid")),
            "expected an invalid-signature problem naming snapshot 1, got {:?}",
            report.problems
        );
        assert!(
            !report.problems.iter().any(|p| p.to_lowercase().contains("chain")),
            "an invalid-signature problem must read distinctly from a chain-broken one, got {:?}",
            report.problems
        );
    }

    #[test]
    fn an_unsigned_snapshot_is_not_signature_checked() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        // No key, no .sig file at all - simulates a pre-CP14 snapshot.
        let m = manifest(1, vec![]);
        crate::core::snapshot::write_manifest(&store_dir, &m).unwrap();
        crate::core::snapshot::write_head(&store_dir, 1).unwrap();

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "{report:?}");
    }

    #[test]
    fn a_mix_of_unsigned_and_signed_snapshots_only_checks_the_signed_ones() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        // Snapshot 1: unsigned (no .sig file).
        let m1 = manifest(1, vec![]);
        crate::core::snapshot::write_manifest(&store_dir, &m1).unwrap();
        // Snapshot 2: signed.
        write_signed(&store_dir, 2);
        crate::core::snapshot::write_head(&store_dir, 2).unwrap();

        let report = check_store(&store_dir);

        assert!(report.is_ok(), "{report:?}");
    }

    #[test]
    fn a_signature_with_no_public_key_available_is_reported() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        write_signed(&store_dir, 1);
        crate::core::snapshot::write_head(&store_dir, 1).unwrap();

        // Simulate the public key having been lost.
        fs::remove_file(store_dir.join(signing::PUBLIC_KEY_FILE_NAME)).unwrap();

        let report = check_store(&store_dir);

        assert!(!report.is_ok());
        assert!(report.problems.iter().any(|p| p.contains('1') && p.contains("no usable public key")));
    }
}
