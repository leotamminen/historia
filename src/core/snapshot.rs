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

/// Paths added, modified, or deleted between a working entry set and a snapshot's
/// entry set (CP5 `status`'s detailed report). Each list is sorted by path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryDiff {
    /// In `working` but not in the snapshot.
    pub added: Vec<String>,
    /// In both, but the content hash differs.
    pub modified: Vec<String>,
    /// In the snapshot but not in `working`.
    pub deleted: Vec<String>,
}

impl EntryDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.deleted.is_empty()
    }
}

/// Compare a working entry set against a snapshot's entry set (or `&[]` when
/// there is no `HEAD` snapshot yet) by path, then by content hash - never by
/// mode, which is best-effort and platform-dependent (Rule 7) and so is never a
/// meaningful "this file changed" signal on its own.
///
/// The single shared primitive behind two views of the same question: `status`
/// (CP5) wants the detailed added/modified/deleted breakdown; `commit`'s
/// skip-if-unchanged (§5, via [`entries_match`]) only wants a yes/no. Both read
/// this one function rather than comparing entry sets two different ways.
pub fn diff_entries(working: &[Entry], snapshot_entries: &[Entry]) -> EntryDiff {
    use std::collections::BTreeMap;

    // BTreeMap keys iterate in sorted order, so `added`/`modified`/`deleted`
    // come out path-sorted for free.
    let working_map: BTreeMap<&str, &str> =
        working.iter().map(|e| (e.path.as_str(), e.hash.as_str())).collect();
    let snapshot_map: BTreeMap<&str, &str> =
        snapshot_entries.iter().map(|e| (e.path.as_str(), e.hash.as_str())).collect();

    let mut diff = EntryDiff::default();
    for (path, hash) in &working_map {
        match snapshot_map.get(path) {
            None => diff.added.push((*path).to_string()),
            Some(snapshot_hash) if snapshot_hash != hash => diff.modified.push((*path).to_string()),
            Some(_) => {}
        }
    }
    for path in snapshot_map.keys() {
        if !working_map.contains_key(path) {
            diff.deleted.push((*path).to_string());
        }
    }
    diff
}

/// True if two entry sets are equivalent by path + content hash (mode-only
/// differences don't count - see [`diff_entries`]). The boolean form of the
/// "does the working set match HEAD?" question (CLAUDE.md §5): `commit`'s
/// skip-if-unchanged uses this; `status` uses [`diff_entries`] directly for the
/// detailed breakdown. Both share this one comparison, never a second way.
pub fn entries_match(a: &[Entry], b: &[Entry]) -> bool {
    diff_entries(a, b).is_empty()
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
    fn diff_entries_finds_added_modified_and_deleted_together() {
        let working = vec![
            Entry { path: "kept.txt".into(), hash: "same".into(), mode: 0o644 },
            Entry { path: "changed.txt".into(), hash: "new-hash".into(), mode: 0o644 },
            Entry { path: "new.txt".into(), hash: "n".into(), mode: 0o644 },
        ];
        let head = vec![
            Entry { path: "kept.txt".into(), hash: "same".into(), mode: 0o644 },
            Entry { path: "changed.txt".into(), hash: "old-hash".into(), mode: 0o644 },
            Entry { path: "gone.txt".into(), hash: "g".into(), mode: 0o644 },
        ];

        let diff = diff_entries(&working, &head);

        assert_eq!(diff.added, vec!["new.txt".to_string()]);
        assert_eq!(diff.modified, vec!["changed.txt".to_string()]);
        assert_eq!(diff.deleted, vec!["gone.txt".to_string()]);
    }

    #[test]
    fn diff_entries_paths_are_sorted_within_each_group() {
        let working = vec![
            Entry { path: "z_new.txt".into(), hash: "1".into(), mode: 0o644 },
            Entry { path: "a_new.txt".into(), hash: "2".into(), mode: 0o644 },
        ];

        let diff = diff_entries(&working, &[]);

        assert_eq!(diff.added, vec!["a_new.txt".to_string(), "z_new.txt".to_string()]);
    }

    #[test]
    fn diff_entries_ignores_mode_only_differences() {
        let working = vec![Entry { path: "a".into(), hash: "1".into(), mode: 0o755 }];
        let head = vec![Entry { path: "a".into(), hash: "1".into(), mode: 0o644 }];

        let diff = diff_entries(&working, &head);

        assert!(diff.is_empty(), "a mode-only difference must not be reported as a change");
    }

    #[test]
    fn diff_entries_is_empty_when_nothing_changed() {
        let entries = vec![Entry { path: "a".into(), hash: "1".into(), mode: 0o644 }];

        assert!(diff_entries(&entries, &entries).is_empty());
    }

    #[test]
    fn entries_match_is_defined_in_terms_of_diff_entries() {
        let a = vec![Entry { path: "a".into(), hash: "1".into(), mode: 0o644 }];
        let b = vec![Entry { path: "a".into(), hash: "2".into(), mode: 0o644 }];

        assert_eq!(entries_match(&a, &b), diff_entries(&a, &b).is_empty());
    }

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
