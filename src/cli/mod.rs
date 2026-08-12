//! Command registry and dispatch: resolves argv against `COMMANDS` and routes to
//! its handler. Adding a command later means one new file in `commands/` plus one
//! new line in `COMMANDS` below - nothing else changes (CLAUDE.md §8).

mod args;
mod help;

use crate::commands;

/// One entry in the command table: canonical name, aliases, one-line help, and the
/// handler to call. Both clap's subcommand list (`args::build`) and `help` read
/// this table - it is the single source of truth for "what commands exist".
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub about: &'static str,
    pub run: fn(&[String]) -> Result<(), String>,
}

/// The command table. To add a command: create `commands/<name>.rs` with a `run`
/// function, then add one line here.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "init",
        aliases: &[],
        about: "Create a store (.historia/) for a folder",
        run: commands::init::run,
    },
    CommandSpec {
        name: "commit",
        aliases: &["snapshot"],
        about: "Snapshot the whole folder",
        run: commands::commit::run,
    },
    CommandSpec {
        name: "log",
        aliases: &[],
        about: "List snapshots: number, time, message",
        run: commands::log::run,
    },
    CommandSpec {
        name: "status",
        aliases: &[],
        about: "Show what changed since the last snapshot",
        run: commands::status::run,
    },
    CommandSpec {
        name: "restore",
        aliases: &[],
        about: "Restore the folder, or one file, to a past snapshot",
        run: commands::restore::run,
    },
    CommandSpec {
        name: "verify",
        aliases: &[],
        about: "Re-hash all blobs and check store integrity",
        run: commands::verify::run,
    },
];

/// Find a registered command by its canonical name or an alias.
fn find(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}

/// A clean, non-panicking message for an unrecognized command name, listing every
/// valid command.
fn unknown_command_message(name: &str) -> String {
    let mut names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    names.push("help");
    format!(
        "historia: unknown command '{name}'\nvalid commands: {}\nrun 'historia help' for details",
        names.join(", ")
    )
}

/// Parse argv (excluding the program name) and run the matching command.
/// Returns `Err(message)` for `main` to print to stderr and exit non-zero; clap's
/// own `--help`/`--version` output is treated as success, not an error.
pub fn dispatch(argv: &[String]) -> Result<(), String> {
    let app = args::build();
    let full = std::iter::once("historia".to_string()).chain(argv.iter().cloned());

    match app.try_get_matches_from(full) {
        Ok(matches) => match matches.subcommand() {
            Some(("help", sub)) => match sub.get_one::<String>("target") {
                Some(target) => help::print_for(target),
                None => {
                    help::print_general();
                    Ok(())
                }
            },
            Some((name, sub)) => match find(name) {
                Some(cmd) => {
                    let args: Vec<String> = sub
                        .get_many::<String>("args")
                        .map(|vals| vals.cloned().collect())
                        .unwrap_or_default();
                    (cmd.run)(&args)
                }
                None => unreachable!("clap only matches names present in COMMANDS"),
            },
            None => {
                help::print_general();
                Ok(())
            }
        },
        Err(err) => {
            use clap::error::ErrorKind;
            match err.kind() {
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayVersion
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    print!("{err}");
                    Ok(())
                }
                _ => Err(unknown_command_message(
                    argv.first().map(String::as_str).unwrap_or(""),
                )),
            }
        }
    }
}
