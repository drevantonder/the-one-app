//! The features every client lists, one line each: add's and remove's, the Vault's, Backups', Groups', Devices',
//! Widgets', Notifications', logs', Insights' and AI Gateway's, each defined in its own crate (`api::Feature`). They're
//! not apps, so no list of apps has them. What serves each is its `Runs` (app-runtime/src/runtime.rs's `features()`).
//! And the API's document made of them and of what every route says (`api::openapi`).

use api::{Feature, Said};
use serde_json::Value;

/// Every feature, in the order they're listed.
pub fn features() -> Vec<Feature> {
    vec![
        deploy::tools::add_feature(),
        deploy::tools::remove_feature(),
        vault::tools::feature(),
        backups::tools::feature(),
        groups::tools::feature(),
        devices::tools::feature(),
        widgets::tools::feature(),
        notifications::tools::feature(),
        device_logs::tools::feature(),
        insights::tools::feature(),
        ai_gateway::feature(),
    ]
}

/// The feature with the tool `name` (as MCP names it), and the tool's place in it.
pub fn find(name: &str) -> Option<(Feature, usize)> {
    features().into_iter().find_map(|f| {
        let at = f.tools.iter().position(|t| f.mcp_name(t) == name)?;
        Some((f, at))
    })
}

/// Every route that says what it is: the app runtime's own, then each feature's.
pub fn said() -> Vec<Said> {
    use api::says::{apps, blobs, network, screens};
    [network::listed(), apps::listed(), deploy::says::listed(), backups::says::listed(), blobs::listed(), devices::says::listed(), insights::says::listed(), screens::listed(), vault::says::listed(), widgets::says::listed()].into_iter().flatten().collect()
}

/// The API's document for a caller who has `apps`, as `GET /api/apps` gives them. Only this machine's own programs see
/// what only they may ask.
pub fn document(apps: &[Value], this_machine: bool) -> Value {
    api::openapi::document(&features(), said(), apps, this_machine)
}
