//! The API (GLOSSARY): what every client asks a device through, at `/api/*`: the CLI, MCP, the screens, the native apps
//! and outside programs. Each route says who may call it (`says`), and MCP's tools and the CLI's commands are made from
//! what they say (`openapi`). Here: what a feature defines itself with (`Feature`, its routes' `says`, its CLI's
//! `cli::Command`), the interface a feature's runtime crate reaches its app runtime through (`runtime::AppRuntime`)
//! and registers with (`runtime::Runs`), and serving: who asked (`Me`), answers (`reply`), traces and the client.

pub mod call;
pub mod caller;
pub mod feature;
pub mod handed;
pub mod openapi;
pub mod protocol_js;
pub mod says;
pub mod traceparent;

pub use caller::{Caller, Credential};
pub use feature::{Feature, Reader, SENDS};
pub use says::{Op, Said, Who};

// Serving and asking a device's API, and running in an app runtime: what a machine does, never a browser or a Worker.
#[cfg(not(target_family = "wasm"))]
pub mod cli;
#[cfg(not(target_family = "wasm"))]
pub mod client;
#[cfg(not(target_family = "wasm"))]
pub mod local;
#[cfg(not(target_family = "wasm"))]
pub mod reply;
#[cfg(not(target_family = "wasm"))]
mod routes;
#[cfg(not(target_family = "wasm"))]
pub mod runtime;
#[cfg(not(target_family = "wasm"))]
mod serve;
#[cfg(not(target_family = "wasm"))]
pub mod tools;
#[cfg(not(target_family = "wasm"))]
pub mod trace;

#[cfg(not(target_family = "wasm"))]
pub use caller::Me;
#[cfg(not(target_family = "wasm"))]
pub use client::Client;
#[cfg(not(target_family = "wasm"))]
pub use reply::{Body, Elsewhere, Error, Invalid, Reply, RouteFailed, done, given, tagged};
#[cfg(not(target_family = "wasm"))]
pub use routes::Routes;
#[cfg(not(target_family = "wasm"))]
pub use runtime::{AppRuntime, Rt, Runs};
#[cfg(not(target_family = "wasm"))]
pub use serve::serve;

/// The paths of the local API that more than one place names: what is kept apart by what a request runs (`keys::Route`),
/// and what is the API's own, not an app's (`/<app>/…` is an app's, reached by git).
pub mod path {
    pub const CALL: &str = "/api/call";
    pub const SCREENS: &str = "/api/screens/";
    pub const MCP: &str = "/mcp";
    pub const WIDGETS: &str = "/api/widgets";
    pub const BADGES: &str = "/api/badges";
    pub const APPS: &str = "/api/apps";
    pub const VAULT: &str = "/api/vault";
    pub const OPENAPI: &str = "/api/openapi.json";
    pub const HOOKS_PROMPT: &str = "/hooks/prompt";
    /// The first segments of the API's own routes with more under them (`routes()` holds to it: `api::tests`).
    pub const ROOTS: [&str; 4] = ["api", "mcp", "hooks", "skills"];

    /// The app an app's own request (`/<app>/…`, with something after) is for: none for the API's own.
    pub fn app_request(path: &str) -> Option<&str> {
        let (app, rest) = path.strip_prefix('/')?.split_once('/')?;
        (!app.is_empty() && !rest.is_empty() && !ROOTS.contains(&app)).then_some(app)
    }
}
