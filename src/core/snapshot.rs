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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_head_parses_as_zero() {
        let n: u64 = INITIAL_HEAD.trim().parse().unwrap();
        assert_eq!(n, 0);
    }
}
