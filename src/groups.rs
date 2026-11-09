//! Groups: who's in the network and in each group, as tools (`tools`), and the commands that make and join them (`cli`).

pub mod tools;
#[cfg(feature = "node")]
pub mod cli;
