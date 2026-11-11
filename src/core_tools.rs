//! The core's features, the one list every client lists: add's and remove's, the Vault's, Backups', Groups', Devices',
//! Widgets', Notifications', logs', insights' and AI Gateway's, each defined beside its feature's others (`api::Feature`). They're
//! not apps, so no list of apps has them. What serves each is its `Runs` (src/node/runtime.rs's `features()`).

use ::api::Feature;

/// Every feature's, in the order they're listed.
pub fn all() -> Vec<Feature> {
    vec![
        ::deploy::tools::add_feature(),
        ::deploy::tools::remove_feature(),
        ::vault::tools::feature(),
        ::backups::tools::feature(),
        ::groups::tools::feature(),
        ::devices::tools::feature(),
        ::widgets::tools::feature(),
        ::notifications::tools::feature(),
        ::device_logs::tools::feature(),
        ::insights::tools::feature(),
        ::ai_gateway::feature(),
    ]
}

/// The feature with the tool `name` (as MCP names it), and the tool's place in it.
pub fn find(name: &str) -> Option<(Feature, usize)> {
    all().into_iter().find_map(|f| {
        let at = f.tools.iter().position(|t| f.mcp_name(t) == name)?;
        Some((f, at))
    })
}
