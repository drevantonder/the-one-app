//! Databases: each scope's SQLite databases, every one kept in the bucket as signed steps and copies (`stored`): its
//! apps' databases, and the scope database (`scope`): its signed steps (`steps`), what each says (`content`), each
//! kind's rule (`rules`), and what a device asks of it (`scopes`).

pub mod content;
pub mod migrate;
pub mod rules;
pub mod scope;
pub mod scopes;
pub mod steps;
pub mod stored;

/// What this build reads of who's who: the scope database's tables, types, roles and rules of this number. A device
/// that reads less is told to update, by nodes (`protocol::header::READS`), the relay, auth and the URL signer (`reads` in what it
/// asks them, `cloudflare/shared/protocol.js`'s `READS`, which `protocol::javascript` writes from this). A release that changes
/// `scope::SCHEMA`, `permissions::SCHEMA` or `permissions::DEFAULTS`, or takes away a route older devices use, or writes a
/// sealed format older devices can't open (`keys::FORMAT`), reads one more.
pub const READS: u32 = 14;

/// What a client said it reads, in `protocol::header::READS`' value: its first number, 0 for nothing.
pub fn reads_of(said: Option<&str>) -> u32 {
    said.and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}
