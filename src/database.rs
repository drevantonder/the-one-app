//! Databases: each scope's SQLite databases. For now only the scope database (`scope`), its signed steps (`steps`),
//! and what a device asks of it (`scopes`); apps' databases are still `src/db.rs`.

pub mod migrate;
pub mod scope;
pub mod scopes;
pub mod steps;

/// What this build reads of who's who: the scope database's tables, types, roles and rules of this number. A device
/// that reads less is told to update, by nodes (`HEADER`), the relay, sign-in and the URL signer (`reads` in what it
/// asks them, `cloudflare/shared/crypto.js`'s `READS`, which `tests/contracts.rs` holds to this). A release that changes
/// `scope::SCHEMA`, `permissions::SCHEMA` or `permissions::defaults`, or takes away a route older devices use, reads one more.
pub const READS: u32 = 5;

/// The header a client says what it reads in.
pub const HEADER: &str = "x-one-reads";

/// What a client said it reads, in `HEADER`'s value: its first number, 0 for nothing.
pub fn reads_of(said: Option<&str>) -> u32 {
    said.and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}
