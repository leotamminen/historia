//! `historia motd` - a small offline status line: local time, hostname, uptime
//! (if available), and a rotating fun fact embedded in the binary (Rule 2: no
//! network, ever). Works anywhere - never requires a store, never takes the
//! lock, never writes anything.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::format::manifest;

/// Fun facts, one per line, embedded into the binary at compile time
/// (`include_str!`, resolved relative to this source file) - no runtime file
/// read, no network, ever (Rule 2). Never empty in practice (this file ships
/// with the repo), but every caller still treats an empty list as harmless.
const FACTS: &str = include_str!("../../assets/facts.txt");

pub fn run(_args: &[String]) -> Result<(), String> {
    println!("historia {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("  Time (UTC): {}", manifest::now_iso8601_utc());

    // Best-effort (same "never fail motd over a missing field" spirit CP12
    // asks for uptime): if the local hostname lookup fails for any reason,
    // just omit the line rather than erroring the whole command.
    if let Ok(name) = hostname::get() {
        println!("  Host:       {}", name.to_string_lossy());
    }

    if let Some(uptime) = system_uptime() {
        println!("  Uptime:     {}", format_uptime(uptime));
    }

    println!();
    println!("  Did you know? {}", pick_fact(SystemTime::now()));

    Ok(())
}

/// Pick today's fact, rotating by calendar day (UTC) so it varies over time
/// without needing an RNG dependency (Rule 9) - the same fact all day, a
/// different one (usually) tomorrow.
fn pick_fact(now: SystemTime) -> &'static str {
    let facts: Vec<&str> = FACTS.lines().filter(|l| !l.trim().is_empty()).collect();
    if facts.is_empty() {
        return "(no facts embedded)";
    }
    facts[fact_index(now, facts.len())]
}

/// The rotation index for `now` into a list of `len` facts. Split out from
/// [`pick_fact`] so the rotation arithmetic is testable without needing a real
/// facts list; `len == 0` can't happen through `pick_fact` (guarded there) but
/// this still returns `0` rather than panicking if ever called directly.
fn fact_index(now: SystemTime, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let days = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 86_400;
    (days as usize) % len
}

/// System uptime, if easily obtainable without a new dependency or unsafe FFI.
/// Linux only for now: `/proc/uptime` is a plain, always-present text file
/// (`<uptime_seconds> <idle_seconds>`), so this is a safe, ordinary file read -
/// no syscall wrapper needed. Other platforms have no such std-reachable
/// source, so this returns `None` rather than reaching for unsafe platform FFI
/// or another dependency just for an optional field (CLAUDE.md CP12: omit
/// gracefully rather than error).
#[cfg(target_os = "linux")]
fn system_uptime() -> Option<Duration> {
    let contents = std::fs::read_to_string("/proc/uptime").ok()?;
    let seconds: f64 = contents.split_whitespace().next()?.parse().ok()?;
    Some(Duration::from_secs_f64(seconds))
}

#[cfg(not(target_os = "linux"))]
fn system_uptime() -> Option<Duration> {
    None
}

fn format_uptime(uptime: Duration) -> String {
    let total_secs = uptime.as_secs();
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn pick_fact_returns_a_line_from_the_embedded_facts_file() {
        let known: Vec<&str> = FACTS.lines().filter(|l| !l.trim().is_empty()).collect();

        let fact = pick_fact(SystemTime::now());

        assert!(known.contains(&fact), "returned fact not found in the embedded list: {fact:?}");
    }

    #[test]
    fn pick_fact_rotates_by_day() {
        let day_zero = UNIX_EPOCH;
        let day_one = UNIX_EPOCH + Duration::from_secs(86_400);

        // With more than one fact embedded, consecutive days must not always
        // collide (a constant/broken rotation would return the same index
        // every time). Not `assert_ne!` on the *fact itself* - two different
        // days legitimately could land on the same fact by coincidence for a
        // short list - but the index computation itself must vary with time.
        assert_ne!(fact_index(day_zero, 12), fact_index(day_one, 12), "rotation must depend on the day");
    }

    #[test]
    fn pick_fact_never_panics_on_a_single_fact_list() {
        // Defensive: fact_index must not divide by zero even in a degenerate
        // (empty) list - callers guard against this, but the arithmetic itself
        // should stay panic-free.
        assert_eq!(fact_index(SystemTime::now(), 1), 0);
    }

    #[test]
    fn format_uptime_shows_minutes_only_under_an_hour() {
        assert_eq!(format_uptime(Duration::from_secs(59 * 60 + 30)), "59m");
    }

    #[test]
    fn format_uptime_shows_hours_and_minutes_under_a_day() {
        assert_eq!(format_uptime(Duration::from_secs(2 * 3600 + 5 * 60)), "2h 5m");
    }

    #[test]
    fn format_uptime_shows_days_hours_and_minutes() {
        assert_eq!(format_uptime(Duration::from_secs(3 * 86_400 + 4 * 3600 + 7 * 60)), "3d 4h 7m");
    }
}
