//! Where things run first. Each device, and the serverless node, is Use First, When Needed or Last Resort: what the
//! install's admins set (the `use` setting, a JSON object of each device's key, or `cloudflare`, to its place), or by
//! default what that kind of device is. Calls, sidecars' operations and new agent devices go to the first in that
//! order that can run them, is up and has room, a nearer one only when every one before it is far. Pure: settings and
//! kinds in, places out; `src/node.rs` and `serverless/` both order by it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The install setting that holds the order.
pub const SETTING: &str = "use";
/// The serverless node's name in it.
pub const CLOUDFLARE: &str = "cloudflare";

/// A device's place, first first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Use {
    First,
    Needed,
    Last,
}

impl Use {
    pub fn parse(s: &str) -> Option<Use> {
        serde_json::from_value(serde_json::json!(s)).ok()
    }
}

/// What a device is, as `src/load.rs` tells: `always_on`, `laptop` (it sleeps), `cloud` (rented by the hour) or
/// `cloudflare` (the serverless node, which costs a little each time it wakes). Anything else counts as `always_on`.
pub fn default_use(kind: &str) -> Use {
    match kind {
        "laptop" | CLOUDFLARE => Use::Needed,
        "cloud" => Use::Last,
        _ => Use::First,
    }
}

/// Within a place, the cheaper kind first.
fn cost(kind: &str) -> u8 {
    match kind {
        "laptop" => 1,
        CLOUDFLARE => 2,
        "cloud" => 3,
        _ => 0,
    }
}

/// What the admins set, by device key (hex) or `cloudflare`; what isn't a place is skipped.
pub fn set(settings: &BTreeMap<String, String>) -> BTreeMap<String, Use> {
    let parsed: BTreeMap<String, String> = settings.get(SETTING).and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default();
    parsed.into_iter().filter_map(|(k, v)| Some((k, Use::parse(&v)?))).collect()
}

/// A device's place: what was set for it, or its kind's.
pub fn place(set: &BTreeMap<String, Use>, id: &str, kind: &str) -> Use {
    set.get(id).copied().unwrap_or_else(|| default_use(kind))
}

/// What devices sort by: place, then cost.
pub fn order(set: &BTreeMap<String, Use>, id: &str, kind: &str) -> (Use, u8) {
    (place(set, id, kind), cost(kind))
}

/// The setting's value after one device's place changes, every other device's kept as it is now (`now`, each
/// device's key and kind, its place written down too, so a node that can't see its kind still knows it).
pub fn with(settings: &BTreeMap<String, String>, now: &[(String, String)], id: &str, place_: Use) -> String {
    let had = set(settings);
    let mut out: BTreeMap<String, Use> = now.iter().map(|(k, kind)| (k.clone(), place(&had, k, kind))).collect();
    out.insert(id.into(), place_);
    serde_json::to_string(&out).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_what_admins_set() {
        let mut s = BTreeMap::new();
        let none = set(&s);
        // An idle machine that's always on comes before Cloudflare, a laptop before Cloudflare, Cloudflare before a
        // rented server, whatever their OS.
        let mut kinds = vec![("c", "cloud"), ("w", CLOUDFLARE), ("l", "laptop"), ("a", "always_on"), ("x", "")];
        kinds.sort_by_key(|(id, k)| (order(&none, id, k), *id));
        assert_eq!(kinds.iter().map(|k| k.0).collect::<String>(), "axlwc");
        s.insert(SETTING.into(), with(&s, &[("a".into(), "always_on".into())], CLOUDFLARE, Use::First));
        let set_ = set(&s);
        assert_eq!(place(&set_, "a", "always_on"), Use::First);
        assert_eq!(place(&set_, CLOUDFLARE, CLOUDFLARE), Use::First);
        assert!(order(&set_, "a", "always_on") < order(&set_, CLOUDFLARE, CLOUDFLARE));
        // What an older or newer device wrote that isn't a place is skipped.
        s.insert(SETTING.into(), r#"{"a":"last","b":"soon"}"#.into());
        assert_eq!(set(&s).len(), 1);
    }
}
