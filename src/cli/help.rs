//! Help text: `help` (general and per-command) is generated entirely from the
//! `cli::COMMANDS` registry, so it is always in sync with what clap parses
//! (CLAUDE.md §8).

use super::{find, unknown_command_message, COMMANDS};

/// `historia help` with no argument: every MVP command, one line each.
pub fn print_general() {
    println!(
        "historia {} - minimal, git-style version control for one folder\n",
        env!("CARGO_PKG_VERSION")
    );
    println!("USAGE:\n    historia <COMMAND>\n");
    println!("COMMANDS:");
    let width = COMMANDS
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(0)
        .max("help".len());
    for cmd in COMMANDS {
        println!("    {:width$}  {}", cmd.name, cmd.about, width = width);
    }
    println!(
        "    {:width$}  Show this help, or detail on one command (aliases: -h, --help, man, info)",
        "help",
        width = width
    );
}

/// `historia help <target>`: that command's one-liner, or a clean error listing
/// valid commands if `target` isn't one (never panics).
pub fn print_for(target: &str) -> Result<(), String> {
    if matches!(target, "help" | "man" | "info") {
        println!("help - show this help, or detail on one command");
        return Ok(());
    }
    match find(target) {
        Some(cmd) => {
            println!("{} - {}", cmd.name, cmd.about);
            Ok(())
        }
        None => Err(unknown_command_message(target)),
    }
}
