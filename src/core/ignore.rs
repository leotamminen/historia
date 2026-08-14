//! Default ignores (`.git`, `node_modules`, `target`, `dist`, `build`, `.historia/`)
//! layered with `.historiaignore`, matched via the `ignore` crate (CLAUDE.md §5, §7).
//!
//! Precedence (CLAUDE.md §5): the default patterns apply first, then
//! `.historiaignore`'s own patterns are layered on top of them in one combined
//! gitignore-style matcher - later patterns win, so a `.historiaignore` negation
//! (`!pattern`) CAN re-include something a default pattern excluded (§5 calls the
//! five-name default list "overridable via .historiaignore"). The one exception is
//! `.historia/` itself: it is never fed into that pattern system at all, so no
//! negation - however written - can ever re-include it. It is always ignored
//! because it is the store, not a matter of taste.

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// The §5 default ignore patterns, in gitignore syntax. A bare name with no
/// slash matches that basename at any depth under gitignore semantics - exactly
/// the "ignored wherever it appears in the tree" behavior CLAUDE.md §5
/// describes, so no special "match at any depth" logic is needed here beyond
/// what the `ignore` crate already does for a plain pattern.
const DEFAULT_IGNORE_PATTERNS: &[&str] = &[".git", "node_modules", "target", "dist", "build"];

/// Name of the user-editable ignore file at the tracked folder's root
/// (CLAUDE.md §5, §7) - gitignore syntax, layered on top of the defaults.
const HISTORIAIGNORE_FILE_NAME: &str = ".historiaignore";

/// The combined ignore rule set for one tracked folder: the hardcoded defaults,
/// `.historiaignore` if present, and the non-negotiable store directory.
pub struct Ignore {
    matcher: Gitignore,
}

impl Ignore {
    /// Build the ignore rule set rooted at `root` (the tracked folder, i.e.
    /// `.historia`'s parent - the same `root` `walk::walk` is called with):
    /// the default patterns, then `.historiaignore`'s patterns layered on top if
    /// the file exists. Never fails - a missing or malformed `.historiaignore`
    /// falls back to "no user patterns", exactly like today's defaults-only
    /// behavior, rather than breaking the whole walk over one bad ignore file.
    pub fn load(root: &Path) -> Ignore {
        let mut builder = GitignoreBuilder::new(root);
        for pattern in DEFAULT_IGNORE_PATTERNS {
            // A hardcoded literal name can't fail to parse as a glob.
            let _ = builder.add_line(None, pattern);
        }

        let historiaignore_path = root.join(HISTORIAIGNORE_FILE_NAME);
        if historiaignore_path.is_file() {
            // `add` tolerates malformed individual lines on its own (matching
            // git's own leniency toward a bad .gitignore line); a genuine read
            // error just means "no .historiaignore patterns this time".
            let _ = builder.add(&historiaignore_path);
        }

        let matcher = builder.build().unwrap_or_else(|_| Gitignore::empty());
        Ignore { matcher }
    }

    /// True if `relative_path` (forward-slash, relative to the tracked root)
    /// should never be walked into or tracked - because it's the store
    /// directory (always, non-negotiable), or because it matches the combined
    /// default + `.historiaignore` pattern set (after negations).
    pub fn is_ignored(&self, relative_path: &str, is_dir: bool) -> bool {
        if is_store_path(relative_path) {
            return true;
        }
        self.matcher.matched(relative_path, is_dir).is_ignore()
    }
}

/// True if `relative_path`'s own basename is the store directory's name,
/// wherever it appears in the tree - checked entirely outside the gitignore
/// pattern system (see the module doc comment) so nothing can ever negate it.
fn is_store_path(relative_path: &str) -> bool {
    relative_path
        .rsplit('/')
        .next()
        .is_some_and(|basename| basename == crate::core::store::STORE_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn default_names_are_ignored_with_no_historiaignore_present() {
        let dir = tempdir().unwrap();
        let ignore = Ignore::load(dir.path());

        for name in [".git", "node_modules", "target", "dist", "build"] {
            assert!(ignore.is_ignored(name, true), "{name} should be ignored by default");
        }
    }

    #[test]
    fn the_store_directory_is_always_ignored() {
        let dir = tempdir().unwrap();
        let ignore = Ignore::load(dir.path());

        assert!(ignore.is_ignored(".historia", true));
    }

    #[test]
    fn ordinary_names_are_not_ignored() {
        let dir = tempdir().unwrap();
        let ignore = Ignore::load(dir.path());

        for name in ["src", "main.rs", "README.md", "Cargo.toml"] {
            assert!(!ignore.is_ignored(name, false), "{name} should not be ignored");
        }
    }

    #[test]
    fn no_historiaignore_file_behaves_exactly_like_defaults_only() {
        let dir = tempdir().unwrap();
        let ignore = Ignore::load(dir.path());

        assert!(ignore.is_ignored("target", true));
        assert!(!ignore.is_ignored("keep.txt", false));
    }

    #[test]
    fn a_historiaignore_pattern_excludes_matching_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".historiaignore"), "*.log\n").unwrap();
        let ignore = Ignore::load(dir.path());

        assert!(ignore.is_ignored("debug.log", false));
        assert!(!ignore.is_ignored("keep.txt", false));
    }

    #[test]
    fn default_ignores_still_apply_with_a_historiaignore_present() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".historiaignore"), "*.log\n").unwrap();
        let ignore = Ignore::load(dir.path());

        assert!(ignore.is_ignored("node_modules", true), "defaults must still apply");
    }

    #[test]
    fn a_negation_re_includes_a_file_a_broader_pattern_excluded() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".historiaignore"), "*.log\n!keep.log\n").unwrap();
        let ignore = Ignore::load(dir.path());

        assert!(ignore.is_ignored("debug.log", false));
        assert!(!ignore.is_ignored("keep.log", false), "the negation must re-include keep.log");
    }

    #[test]
    fn a_negation_can_re_include_a_default_ignored_name() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".historiaignore"), "!dist\n").unwrap();
        let ignore = Ignore::load(dir.path());

        assert!(!ignore.is_ignored("dist", true), "§5: the default list is overridable via .historiaignore");
    }

    #[test]
    fn nothing_can_ever_re_include_the_store_directory() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".historiaignore"), "!.historia\n!.historia/**\n").unwrap();
        let ignore = Ignore::load(dir.path());

        assert!(ignore.is_ignored(".historia", true), ".historia/ must never be re-includable");
    }
}
