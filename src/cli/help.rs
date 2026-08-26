//! Help text: `help` (general and per-command) is generated entirely from the
//! `cli::COMMANDS` registry, so it is always in sync with what clap parses and
//! with each command's actual usage (CLAUDE.md §8, CP9).

use super::{find, unknown_command_message, CommandSpec, COMMANDS};

/// `historia help` with no argument (also reached via bare `-h`/`--help`,
/// `man`, `info` - CP9): every MVP command, one line each.
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
    println!("\nRun 'historia help <command>' (or '<command> --help') for detail on one command.");
}

/// `historia help <target>` (also reached via `<command> -h`/`--help` - CP9):
/// that command's one-liner plus its usage, or a clean error listing valid
/// commands if `target` isn't one (never panics).
pub fn print_for(target: &str) -> Result<(), String> {
    if matches!(target, "help" | "man" | "info") {
        println!("help - show this help, or detail on one command");
        println!();
        println!("Usage: historia help [command]");
        println!();
        println!("Aliases: -h, --help, man, info");
        return Ok(());
    }
    match find(target) {
        Some(cmd) => {
            print_detail(cmd);
            Ok(())
        }
        None => Err(unknown_command_message(target)),
    }
}

fn print_detail(cmd: &CommandSpec) {
    println!("{} - {}", cmd.name, cmd.about);
    println!();
    println!("Usage: {}", cmd.usage);
    if !cmd.usage_detail.is_empty() {
        println!();
        println!("{}", cmd.usage_detail);
    }
    if !cmd.aliases.is_empty() {
        println!();
        println!("Aliases: {}", cmd.aliases.join(", "));
    }
}
