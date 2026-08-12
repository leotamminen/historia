//! On-disk format definitions: the durability contract that must stay recoverable
//! even if this binary and Rust itself disappear (CLAUDE.md §8, §9).

pub mod manifest;
