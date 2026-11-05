//! Databases: each scope's SQLite databases. For now only the scope database (`scope`), built from the log's replay,
//! and what a device asks of it (`scopes`); apps' databases are still `src/db.rs`.

pub mod scope;
pub mod scopes;
pub mod switch;
