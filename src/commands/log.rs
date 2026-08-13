//! `historia log` - list snapshots: number, timestamp, message (newest first).
//! Read-only: never writes, never takes the lock (CLAUDE.md §5, Rule 6).

use std::env;

use crate::core::{snapshot, store};
use crate::format::manifest::Manifest;

pub fn run(_args: &[String]) -> Result<(), String> {
    let cwd = env::current_dir()
        .map_err(|e| format!("historia log: cannot read current directory: {e}"))?;
    let store_dir = store::locate_store(&cwd).ok_or_else(|| {
        "historia log: not a historia store (run 'historia init' first)".to_string()
    })?;

    let manifests = snapshot::list_manifests(&store_dir).map_err(|e| format!("historia log: {e}"))?;

    print!("{}", format_log(&manifests));
    Ok(())
}

/// Render `manifests` (oldest first, as returned by `snapshot::list_manifests`)
/// as `log` output: newest first, one line each - number (right-aligned to the
/// widest), timestamp, message (§5: number is the primary handle).
fn format_log(manifests: &[Manifest]) -> String {
    if manifests.is_empty() {
        return "no snapshots yet\n".to_string();
    }

    let width = manifests
        .iter()
        .map(|m| m.number.to_string().len())
        .max()
        .unwrap_or(1);

    let mut out = String::new();
    for m in manifests.iter().rev() {
        out.push_str(&format!("{:>width$}  {}  {}\n", m.number, m.timestamp, m.message, width = width));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::manifest::Manifest;

    fn manifest(number: u64, timestamp: &str, message: &str) -> Manifest {
        Manifest {
            number,
            timestamp: timestamp.to_string(),
            message: message.to_string(),
            parent: number.saturating_sub(1),
            entries: vec![],
        }
    }

    #[test]
    fn empty_store_message() {
        assert_eq!(format_log(&[]), "no snapshots yet\n");
    }

    #[test]
    fn lines_are_newest_first_and_include_number_timestamp_message() {
        let manifests = vec![
            manifest(1, "1970-01-01T00:00:00Z", "first"),
            manifest(2, "1970-01-02T00:00:00Z", "second"),
        ];

        let out = format_log(&manifests);
        let lines: Vec<&str> = out.lines().collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains('2') && lines[0].contains("second"));
        assert!(lines[1].contains('1') && lines[1].contains("first"));
    }

    #[test]
    fn numbers_are_right_aligned_to_the_widest_number() {
        let manifests = vec![manifest(1, "t", "a"), manifest(10, "t", "b")];

        let out = format_log(&manifests);
        let lines: Vec<&str> = out.lines().collect();

        // Right-aligned to a fixed width: shorter numbers get leading padding, so
        // the separator before the timestamp column lands at the same index on
        // every line (digits themselves do not start at the same column).
        let sep = |line: &str| line.find("  ").unwrap();
        assert_eq!(sep(lines[0]), sep(lines[1]));
    }
}
