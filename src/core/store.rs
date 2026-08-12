//! Locate and open `.historia/` (walking up from the current directory, like git);
//! read and write content-addressed blobs under `objects/` by their hash; the
//! `.historia/lock` primitive (acquire/release, fail-fast, stale-lock reporting).
//! CLAUDE.md §8 (core/store.rs), §9 (on-disk layout), Rule 6 (locking).
