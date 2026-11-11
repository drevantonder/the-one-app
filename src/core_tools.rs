//! The core's features, the one list every client lists: add's and remove's, the Vault's, Backups', Groups', Devices',
//! Widgets', Push's, logs', insights' and AI Gateway's, each defined beside its feature's others (`api::Feature`). They're
//! not apps, so no list of apps has them. What serves each is its `Runs` (src/node/runtime.rs's `features()`).

use ::api::Feature;

/// Every feature's, in the order they're listed.
pub fn all() -> Vec<Feature> {
    vec![crate::app::tools::add_feature(), crate::app::tools::remove_feature(), ::vault::tools::feature(), crate::backups::tools::feature(), crate::groups::tools::feature(), crate::devices::tools::feature(), crate::widgets::tools::feature(), crate::push::tools::feature(), crate::logs::tools::feature(), crate::insights::tools::feature(), crate::ai_gateway::tools::feature()]
}

/// The feature with the tool `name` (as MCP names it), and the tool's place in it.
pub fn find(name: &str) -> Option<(Feature, usize)> {
    all().into_iter().find_map(|f| {
        let at = f.tools.iter().position(|t| f.mcp_name(t) == name)?;
        Some((f, at))
    })
}
