//! Folder traversal for `commit`/`status`, respecting ignore rules. Symlinks are
//! never followed or stored; each one is reported to the caller so it can print
//! `skipped symlink: <path>` instead of silently dropping it (CLAUDE.md §5).
