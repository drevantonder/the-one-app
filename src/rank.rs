//! Run priority. Each device, and the serverless node, has a number, 1 first, or is always last: what the install's
//! admins set (the `priority` setting, a JSON object of each device's key, or `cloudflare`, to its number or "last"),
//! or by default 1, and the serverless node always last. Calls, sidecars' operations and new agent devices go to the
//! lowest number that can run them, is up and has room; devices on the same number are equal, and the quickest
//! runs it; a nearer one only when every one before it is far. Pure: settings in, priorities out; `src/node.rs` and
//! `serverless/` both order by it.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// The install setting that holds them.
pub const SETTING: &str = "priority";
/// The serverless node's name in it.
pub const CLOUDFLARE: &str = "cloudflare";
/// The highest number a device may have.
const MOST: u64 = 99;

/// A device's run priority, first first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Number(u32),
    Last,
}

impl Priority {
    /// As the setting holds it: a number from 1, or "last"; anything else isn't one.
    pub fn from_value(v: &Value) -> Option<Priority> {
        match v {
            Value::Number(n) => n.as_u64().filter(|n| (1..=MOST).contains(n)).map(|n| Priority::Number(n as u32)),
            Value::String(s) if s == "last" => Some(Priority::Last),
            _ => None,
        }
    }

    pub fn to_value(self) -> Value {
        match self {
            Priority::Number(n) => json!(n),
            Priority::Last => json!("last"),
        }
    }
}

/// What the admins set, by device key (hex) or `cloudflare`; what isn't a priority is skipped.
pub fn set(settings: &BTreeMap<String, String>) -> BTreeMap<String, Priority> {
    let parsed: BTreeMap<String, Value> = settings.get(SETTING).and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default();
    parsed.into_iter().filter_map(|(k, v)| Some((k, Priority::from_value(&v)?))).collect()
}

/// A device's priority: what was set for it, or 1; the serverless node's, or always last.
pub fn priority(set: &BTreeMap<String, Priority>, id: &str) -> Priority {
    set.get(id).copied().unwrap_or(if id == CLOUDFLARE { Priority::Last } else { Priority::Number(1) })
}

/// The setting's value after `ids` (one device's endpoints, or `cloudflare`) get `to`, every other kept as set.
pub fn with(settings: &BTreeMap<String, String>, ids: &[String], to: Priority) -> String {
    let mut out: BTreeMap<String, Value> = set(settings).into_iter().map(|(k, p)| (k, p.to_value())).collect();
    for id in ids {
        out.insert(id.clone(), to.to_value());
    }
    serde_json::to_string(&out).unwrap()
}
