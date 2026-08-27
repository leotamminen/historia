//! Per-command implementations. Each file is a thin adapter with a
//! `run(args: &[String]) -> Result<(), String>` that `cli::dispatch` calls through
//! the `CommandSpec` registry in `cli::mod` - see CLAUDE.md §8 for how to add one.

pub mod backup;
pub mod commit;
pub mod init;
pub mod log;
pub mod motd;
pub mod restore;
pub mod status;
pub mod verify;
