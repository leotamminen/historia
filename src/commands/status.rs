//! `historia status` - show what changed since HEAD: added/modified/deleted.
//! Read-only: never writes, never takes the lock (CLAUDE.md §5, Rule 6).

use std::env;

use crate::core::snapshot::EntryDiff;
use crate::core::{hash, snapshot, store, walk};
use crate::format::manifest::Entry;

pub fn run(_args: &[String]) -> Result<(), String> {
    let cwd = env::current_dir()
        .map_err(|e| format!("historia status: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia status: not a historia store (run 'historia init' first)".to_string()
    })?;

    let head = snapshot::read_head(&store_dir).map_err(|e| format!("historia status: {e}"))?;
    if head == 0 {
        println!("no snapshots yet");
        return Ok(());
    }
    let head_manifest =
        snapshot::read_manifest(&store_dir, head).map_err(|e| format!("historia status: {e}"))?;

    // The tracked folder is `.historia`'s parent, not necessarily the current
    // directory - status always reports on the whole folder (CLAUDE.md §5),
    // mirroring `commit`.
    let root = store_dir
        .parent()
        .expect(".historia is always created inside the tracked folder");
    let walked = walk::walk(root)
        .map_err(|e| format!("historia status: failed to walk '{}': {e}", root.display()))?;

    let mut entries = Vec::with_capacity(walked.files.len());
    for file in &walked.files {
        // Hash only - never `store::write_blob` here. `status` is read-only and
        // must never modify the store (CLAUDE.md §5, this checkpoint's constraint).
        let content_hash = hash::hash_file(&file.absolute_path)
            .map_err(|e| format!("historia status: failed to read '{}': {e}", file.relative_path))?;
        entries.push(Entry { path: file.relative_path.clone(), hash: content_hash, mode: file.mode });
    }

    let diff = snapshot::diff_entries(&entries, &head_manifest.entries);
    print!("{}", format_status(head, &diff));
    Ok(())
}

/// Render a diff against `head` as `status` output: a clean "matches snapshot"
/// message when nothing changed, otherwise one labeled, path-sorted group per
/// change kind present (empty groups omitted).
fn format_status(head: u64, diff: &EntryDiff) -> String {
    if diff.is_empty() {
        return format!("working folder matches snapshot {head}\n");
    }

    let mut blocks = Vec::new();
    if !diff.added.is_empty() {
        blocks.push(format_group("Added", &diff.added));
    }
    if !diff.modified.is_empty() {
        blocks.push(format_group("Modified", &diff.modified));
    }
    if !diff.deleted.is_empty() {
        blocks.push(format_group("Deleted", &diff.deleted));
    }
    blocks.join("\n")
}

fn format_group(label: &str, paths: &[String]) -> String {
    let mut out = format!("{label}:\n");
    for path in paths {
        out.push_str(&format!("  {path}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::snapshot::EntryDiff;

    #[test]
    fn no_changes_prints_matches_snapshot() {
        let out = format_status(3, &EntryDiff::default());
        assert_eq!(out, "working folder matches snapshot 3\n");
    }

    #[test]
    fn groups_are_labeled_and_paths_sorted() {
        let diff = EntryDiff {
            added: vec!["b.txt".to_string(), "a.txt".to_string()],
            modified: vec!["c.txt".to_string()],
            deleted: vec!["d.txt".to_string()],
        };
        // Simulate diff_entries's own sorting contract (it always returns
        // already-sorted lists); format_status must not need to re-sort.
        let mut sorted_diff = diff.clone();
        sorted_diff.added.sort();

        let out = format_status(1, &sorted_diff);

        assert!(out.contains("Added:\n  a.txt\n  b.txt\n"));
        assert!(out.contains("Modified:\n  c.txt\n"));
        assert!(out.contains("Deleted:\n  d.txt\n"));
    }

    #[test]
    fn empty_groups_are_omitted() {
        let diff = EntryDiff {
            added: vec!["a.txt".to_string()],
            modified: vec![],
            deleted: vec![],
        };

        let out = format_status(1, &diff);

        assert!(out.contains("Added:"));
        assert!(!out.contains("Modified:"));
        assert!(!out.contains("Deleted:"));
    }
}
