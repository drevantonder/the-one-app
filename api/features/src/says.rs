//! What each route of the node's API says of itself in the OpenAPI document (`api::openapi`): who may ask it, what it
//! takes and what it answers. Plain data, so a Worker or a browser's onet makes the document a node does. Each feature's
//! is beside its routes (`says.rs`), with the structs its handlers read; they're under `node/`, which a Worker or a browser
//! doesn't build, so this is where they're named.

use ::api::says::Said;

#[path = "../../../app-runtime/src/api/says.rs"]
pub mod api;
#[path = "../../../app-runtime/src/apps/says.rs"]
pub mod apps;
#[path = "../../../app-runtime/src/blobs/says.rs"]
pub mod blobs;
#[path = "../../../app-runtime/src/screens/says.rs"]
pub mod screens;

/// Every route that says what it is.
pub fn listed() -> Vec<Said> {
    [api::listed(), apps::listed(), ::deploy::says::listed(), ::backups::says::listed(), blobs::listed(), ::devices::says::listed(), ::insights::says::listed(), screens::listed(), ::vault::says::listed(), ::widgets::says::listed()].into_iter().flatten().collect()
}
