//! The manifest schema: `{ number, timestamp, message, parent, entries: [ { path,
//! hash, mode } ] }`, plus the `.historia/format` version marker. Kept boring and
//! documented on purpose - see CLAUDE.md §9, the on-disk format contract.

/// Contents `init` writes to a fresh `.historia/format`: the on-disk format version
/// marker, in plain text so the store's format is identifiable without parsing JSON
/// or running this binary (CLAUDE.md §8, §9).
pub const FORMAT_MARKER: &str = "historia format v1\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_marker_is_plain_text_ending_in_newline() {
        assert!(FORMAT_MARKER.ends_with('\n'));
        assert_eq!(FORMAT_MARKER.trim(), "historia format v1");
    }
}
