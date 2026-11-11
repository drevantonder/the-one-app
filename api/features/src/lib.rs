//! Every feature's API as clients get it: the core's features, listed (`core_tools`), what each route says of itself
//! (`says`) and the OpenAPI document made of them (`openapi`), on every build, WebAssembly too; and on a machine (its
//! `machine` feature), what agents get of it: MCP (`mcp`), code mode (`code`), the skills (`skills`) and each tool as a
//! command (`tools`).

pub mod core_tools;
pub mod openapi;
pub mod says;
#[cfg(feature = "machine")]
pub mod code;
#[cfg(feature = "machine")]
pub mod mcp;
#[cfg(feature = "machine")]
pub mod skills;
#[cfg(feature = "machine")]
pub mod tools;
