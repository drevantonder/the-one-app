//! The scope database (GLOSSARY: Scope): its signed steps (`steps`), what each says (`content`), each kind's rule
//! (`rules`), who's who and what a device asks of it (`scopes`, `scope`), permissions, and the settings and state built on it.

pub mod content;
pub mod migrate;
pub mod permissions;
pub mod rank;
pub mod replica;
pub mod rules;
pub mod scope;
pub mod scopes;
pub mod settings;
pub mod sign_in_proof;
pub mod state;
pub mod steps;
