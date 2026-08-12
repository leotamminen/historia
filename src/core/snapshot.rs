//! Manifest read/write, snapshot numbering, and `HEAD` tracking. A snapshot is
//! identified by a sequential integer (1, 2, 3, ...); manifests live under
//! `.historia/snapshots/<n>.json` per CLAUDE.md §9.
