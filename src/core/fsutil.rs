//! Atomic write-then-rename helpers, and the safe (mirror) restore routine that
//! makes the working folder match a snapshot exactly without ever touching ignored
//! paths (CLAUDE.md Rule 3, Rule 4, Rule 5).
