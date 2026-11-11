//! Databases: each scope's SQLite databases, every one kept in the bucket as signed steps and copies (`stored`),
//! and where the keys that open them are kept (`keys`). The scope database is `scope`'s.

pub mod keys;
pub mod stored;

/// What a client said it reads, in `protocol::header::READS`' value: its first number, 0 for nothing.
pub fn reads_of(said: Option<&str>) -> u32 {
    said.and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}
