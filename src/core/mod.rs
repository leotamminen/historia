//! The engine: reusable, CLI-agnostic logic (blob storage, hashing, manifests,
//! ignore rules, folder walking, atomic filesystem ops). No clap types and no
//! stdout/stderr formatting live here - `commands/` adapts this to the CLI
//! (CLAUDE.md §8).

pub mod fsutil;
pub mod hash;
pub mod ignore;
pub mod snapshot;
pub mod store;
pub mod walk;
