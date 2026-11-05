//! Databases: each scope's SQLite databases. For now only the scope database (`scope`), built from the log's replay;
//! apps' databases are still `src/db.rs`.

pub mod scope;
