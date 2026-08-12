//! Entry point: parse argv, dispatch to a command via the `cli` registry, and map
//! the result to a process exit code. No command logic lives here (CLAUDE.md §8,
//! Rule 10).

mod cli;
mod commands;
mod core;
mod format;

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match cli::dispatch(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
