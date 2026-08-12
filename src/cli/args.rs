//! clap command-line definition, built from the `cli::COMMANDS` registry so the
//! parser's subcommand list (and its generated `--help` text) can never drift from
//! what `cli::help` prints - both read the same table (CLAUDE.md §8).

use clap::{Arg, ArgAction, Command};

use super::COMMANDS;

/// Build the top-level clap `Command`, adding one subcommand per registry entry
/// plus a `help` subcommand (aliased `man`/`info`) for `cli::dispatch` to handle.
pub fn build() -> Command {
    let mut app = Command::new("historia")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Minimal, git-style version control for backing up one folder.")
        .disable_help_subcommand(true)
        .subcommand_required(false)
        .arg_required_else_help(false);

    for spec in COMMANDS {
        app = app.subcommand(
            Command::new(spec.name)
                .visible_aliases(spec.aliases)
                .about(spec.about),
        );
    }

    app.subcommand(
        Command::new("help")
            .visible_aliases(["man", "info"])
            .about("Show this help, or detail on one command")
            .arg(Arg::new("target").action(ArgAction::Set)),
    )
}
