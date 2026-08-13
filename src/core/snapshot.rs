//! Manifest read/write, snapshot numbering, and `HEAD` tracking. A snapshot is
//! identified by a sequential integer (1, 2, 3, ...); manifests live under
//! `.historia/snapshots/<n>.json` per CLAUDE.md §9.

/// Contents `init` writes to a fresh `.historia/HEAD`.
///
/// HEAD holds the number of the most recent snapshot as plain ASCII decimal text,
/// so it stays trivially parseable by hand or a one-line script even without this
/// binary (CLAUDE.md §8). "0" means the store has no snapshots yet - the next
/// `commit` creates snapshot 1. CP3 (`commit`) reads and increments this value;
/// CP4 (`log`) reads it to know the newest snapshot to list.
pub const INITIAL_HEAD: &str = "0\n";

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::fsutil;
use crate::format::manifest::{Entry, Manifest};

/// Read `.historia/HEAD`: the number of the most recent snapshot, or `0` if the
/// store has none yet (CP1's `INITIAL_HEAD` convention).
pub fn read_head(store_dir: &Path) -> io::Result<u64> {
    let contents = fs::read_to_string(store_dir.join("HEAD"))?;
    contents
        .trim()
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "HEAD does not contain a valid snapshot number"))
}

/// Atomically advance `.historia/HEAD` to `number` (write-then-rename, Rule 5).
pub fn write_head(store_dir: &Path, number: u64) -> io::Result<()> {
    fsutil::write_atomic(&store_dir.join("HEAD"), format!("{number}\n").as_bytes())
}

/// Path to a snapshot's manifest file, `.historia/snapshots/<number>.json`
/// (CLAUDE.md §9).
pub fn manifest_path(store_dir: &Path, number: u64) -> PathBuf {
    store_dir.join("snapshots").join(format!("{number}.json"))
}

/// Read and parse the manifest for snapshot `number`.
pub fn read_manifest(store_dir: &Path, number: u64) -> io::Result<Manifest> {
    let contents = fs::read_to_string(manifest_path(store_dir, number))?;
    serde_json::from_str(&contents).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Atomically write a snapshot's manifest (write-then-rename, Rule 5). Callers
/// must write every blob an entry references *before* calling this (Rule 5's
/// fixed write order: blobs, then manifest, then HEAD).
pub fn write_manifest(store_dir: &Path, manifest: &Manifest) -> io::Result<()> {
    let json = serde_json::to_string_pretty(manifest).map_err(io::Error::other)?;
    fsutil::write_atomic(&manifest_path(store_dir, manifest.number), json.as_bytes())
}

/// The manifest at `HEAD`, or `None` if the store has no snapshots yet.
pub fn read_head_manifest(store_dir: &Path) -> io::Result<Option<Manifest>> {
    let head = read_head(store_dir)?;
    if head == 0 {
        Ok(None)
    } else {
        Ok(Some(read_manifest(store_dir, head)?))
    }
}

/// Read every snapshot's manifest, oldest first (snapshot `1..=HEAD`). There is
/// no deletion yet (§11: `prune`/GC is deferred), so this sequential range is
/// exactly the full history; an empty store (`HEAD == 0`) yields an empty `Vec`.
/// The shared read path for `log` (CP4) - reuses `read_manifest`, never a second
/// way of parsing a manifest.
pub fn list_manifests(store_dir: &Path) -> io::Result<Vec<Manifest>> {
    let head = read_head(store_dir)?;
    let mut manifests = Vec::with_capacity(head as usize);
    for number in 1..=head {
        manifests.push(read_manifest(store_dir, number)?);
    }
    Ok(manifests)
}

/// True if two entry sets are identical - same paths, hashes, and modes,
/// regardless of order. The shared "does the working set match HEAD?" primitive
/// (CLAUDE.md §5): `commit`'s skip-if-unchanged compares a freshly walked+hashed
/// entry set against `HEAD`'s (or `&[]` when there is no `HEAD` snapshot yet);
/// CP5's `status` reuses this same function rather than a second comparison.
pub fn entries_match(a: &[Entry], b: &[Entry]) -> bool {
    let mut a: Vec<&Entry> = a.iter().collect();
    let mut b: Vec<&Entry> = b.iter().collect();
    a.sort_by(|x, y| x.path.cmp(&y.path));
    b.sort_by(|x, y| x.path.cmp(&y.path));
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::store::init_store;
    use crate::format::manifest::{Entry, Manifest};
    use tempfile::tempdir;

    #[test]
    fn initial_head_parses_as_zero() {
        let n: u64 = INITIAL_HEAD.trim().parse().unwrap();
        assert_eq!(n, 0);
    }

    fn sample_manifest(number: u64, parent: u64) -> Manifest {
        Manifest {
            number,
            timestamp: "1970-01-01T00:00:00Z".to_string(),
            message: "test".to_string(),
            parent,
            entries: vec![Entry {
                path: "a.txt".to_string(),
                hash: "deadbeef".to_string(),
                mode: 0o644,
            }],
        }
    }

    #[test]
    fn fresh_store_reads_head_as_zero() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        assert_eq!(read_head(&store_dir).unwrap(), 0);
    }

    #[test]
    fn write_head_then_read_head_round_trips() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        write_head(&store_dir, 7).unwrap();

        assert_eq!(read_head(&store_dir).unwrap(), 7);
    }

    #[test]
    fn write_manifest_then_read_manifest_round_trips() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let manifest = sample_manifest(1, 0);

        write_manifest(&store_dir, &manifest).unwrap();

        assert_eq!(read_manifest(&store_dir, 1).unwrap(), manifest);
    }

    #[test]
    fn read_head_manifest_is_none_when_head_is_zero() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        assert_eq!(read_head_manifest(&store_dir).unwrap(), None);
    }

    #[test]
    fn read_head_manifest_returns_the_manifest_at_head() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let manifest = sample_manifest(1, 0);
        write_manifest(&store_dir, &manifest).unwrap();
        write_head(&store_dir, 1).unwrap();

        assert_eq!(read_head_manifest(&store_dir).unwrap(), Some(manifest));
    }

    // ---- list_manifests (CP4 `log`) ----

    #[test]
    fn list_manifests_is_empty_for_a_fresh_store() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();

        assert_eq!(list_manifests(&store_dir).unwrap(), vec![]);
    }

    #[test]
    fn list_manifests_returns_every_snapshot_oldest_first() {
        let dir = tempdir().unwrap();
        let store_dir = init_store(dir.path()).unwrap();
        let m1 = sample_manifest(1, 0);
        let m2 = sample_manifest(2, 1);
        let m3 = sample_manifest(3, 2);
        write_manifest(&store_dir, &m1).unwrap();
        write_manifest(&store_dir, &m2).unwrap();
        write_manifest(&store_dir, &m3).unwrap();
        write_head(&store_dir, 3).unwrap();

        assert_eq!(list_manifests(&store_dir).unwrap(), vec![m1, m2, m3]);
    }

    // ---- shared "does the working set match HEAD?" comparison (§5) ----
    // Used by commit's skip-if-unchanged and reused by CP5's `status`.

    #[test]
    fn entries_match_ignores_order() {
        let a = vec![
            Entry { path: "a".into(), hash: "1".into(), mode: 0o644 },
            Entry { path: "b".into(), hash: "2".into(), mode: 0o644 },
        ];
        let b = vec![
            Entry { path: "b".into(), hash: "2".into(), mode: 0o644 },
            Entry { path: "a".into(), hash: "1".into(), mode: 0o644 },
        ];

        assert!(entries_match(&a, &b));
    }

    #[test]
    fn entries_match_detects_a_changed_hash() {
        let a = vec![Entry { path: "a".into(), hash: "1".into(), mode: 0o644 }];
        let b = vec![Entry { path: "a".into(), hash: "2".into(), mode: 0o644 }];

        assert!(!entries_match(&a, &b));
    }

    #[test]
    fn entries_match_detects_an_added_or_removed_file() {
        let a = vec![Entry { path: "a".into(), hash: "1".into(), mode: 0o644 }];
        let b: Vec<Entry> = vec![];

        assert!(!entries_match(&a, &b));
    }

    #[test]
    fn two_empty_entry_sets_match() {
        assert!(entries_match(&[], &[]));
    }
}
