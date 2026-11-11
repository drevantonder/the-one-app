//! Backups, as every build has them, WebAssembly too: what a place to back up to is in the Vault, the tools' specs and
//! what its route says. What runs them, the format and the command that opens a backup from nothing are the runtime's
//! (runtime/).

pub mod says;
pub mod tools;

/// The Vault item kind a place is: an item in Everyone, with a folder's `device` and `path`, or S3's `endpoint`,
/// `bucket`, `region`, `key_id` and `secret`.
pub const KIND: &str = "Backup place";
