//! The One App's program, every machine's: the node, oned, the CLI and MCP, and what the Cloudflare node and clients
//! share with them. The domain's words are GLOSSARY.md's; each module's doc says what it is.

/// What the program carries: the built-in apps, the Workers' and web client's files, and the WebAssembly (build.rs).
pub mod carried {
    include!(concat!(env!("OUT_DIR"), "/carried.rs"));
}


// What the Cloudflare node (cloudflare/node/) and the native apps' onet (onet/) share with nodes on machines; the rest
// is only theirs (the `node` feature).
pub mod ai_gateway;
pub mod backups;
#[cfg(feature = "node")]
pub mod skills;
#[cfg(feature = "node")]
pub mod harnesses;
#[cfg(feature = "node")]
pub mod api;
#[cfg(feature = "node")]
pub mod build;
#[cfg(feature = "node")]
pub mod code;
#[cfg(feature = "node")]
pub mod mcp;
#[cfg(feature = "node")]
pub mod tools;
pub mod core_tools;
pub mod app;
#[cfg(feature = "node")]
pub mod git_wire;
#[cfg(feature = "node")]
pub mod watchdog;
#[cfg(not(target_family = "wasm"))]
pub mod loopback;
#[cfg(feature = "node")]
#[cfg(feature = "node")]
pub mod node;
#[cfg(feature = "node")]
pub mod release;
#[cfg(feature = "web-bundle")]
pub mod latest;
#[cfg(feature = "node")]
pub mod update;
#[cfg(feature = "node")]
pub mod schedules;
#[cfg(feature = "node")]
pub mod mode;
#[cfg(feature = "node")]
pub mod oned;
pub mod cloudflare;
pub mod openapi;
pub mod says;
pub mod screen;
pub mod ui_files;
pub mod ui_folder;
pub mod web;
