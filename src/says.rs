//! What each route of the node's API says of itself in the OpenAPI document (`api::openapi`): who may ask it, what it
//! takes and what it answers. Plain data, so a Worker or a browser's onet makes the document a node does. Each feature's
//! is beside its routes (`says.rs`), with the structs its handlers read; they're under `node/`, which a Worker or a browser
//! doesn't build, so this is where they're named.

use ::api::says::Said;

#[path = "api/says.rs"]
pub mod api;
#[path = "node/apps/says.rs"]
pub mod apps;
#[path = "node/backups/says.rs"]
pub mod backups;
#[path = "node/blobs/says.rs"]
pub mod blobs;
#[path = "node/devices/says.rs"]
pub mod devices;
#[path = "insights/says.rs"]
pub mod insights;
#[path = "node/screens/says.rs"]
pub mod screens;
#[path = "node/widgets/says.rs"]
pub mod widgets;

/// Every route that says what it is.
pub fn listed() -> Vec<Said> {
    [api::listed(), apps::listed(), backups::listed(), blobs::listed(), devices::listed(), insights::listed(), screens::listed(), ::vault::says::listed(), widgets::listed()].into_iter().flatten().collect()
}
