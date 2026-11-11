//! Run priority. Each device, and the Cloudflare node, has a number, 1 first, or is always last: what the network's
//! admins set (the `priority` setting, a JSON object of each device's key, or `cloudflare`, to its number or "last"),
//! or by default 1, and the Cloudflare node always last. Calls, services' functions and new agent devices go to the
//! lowest number that can run them, is up and has room; devices on the same number are equal, and the quickest
//! runs it; a nearer one only when every one before it is far. Pure: settings in, priorities out; `app-runtime/src/calls.rs`,
//! `oned/cloudflare/worker/` and every client (`onet/src/device.rs`, `ui/web/browser.js`) order by it, and the app
//! runtime and onet try devices in `order`'s.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};

/// The network setting that holds them.
pub const SETTING: &str = "priority";
/// The Cloudflare node's name in it.
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

impl schemars::JsonSchema for Priority {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Priority".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "oneOf": [{ "type": "integer", "minimum": 1 }, { "const": "last" }] })
    }
}

impl serde::Serialize for Priority {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Priority {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Priority::from_value(&value).ok_or_else(|| serde::de::Error::custom("a priority is a number from 1 or \"last\""))
    }
}

/// What the admins set, by device key (hex) or `cloudflare`; what isn't a priority is skipped.
pub fn set(settings: &BTreeMap<String, String>) -> BTreeMap<String, Priority> {
    let parsed: BTreeMap<String, Value> = settings.get(SETTING).and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default();
    parsed.into_iter().filter_map(|(k, v)| Some((k, Priority::from_value(&v)?))).collect()
}

/// A device's priority: what was set for it, or 1; the Cloudflare node's, or always last.
pub fn priority(set: &BTreeMap<String, Priority>, id: &str) -> Priority {
    set.get(id).copied().unwrap_or(if id == CLOUDFLARE { Priority::Last } else { Priority::Number(1) })
}

/// The setting's value after `ids` (a device, or `cloudflare`) get `to`, every other kept as set.
pub fn with(settings: &BTreeMap<String, String>, ids: &[String], to: Priority) -> String {
    let mut out: BTreeMap<String, Value> = set(settings).into_iter().map(|(k, p)| (k, p.to_value())).collect();
    for id in ids {
        out.insert(id.clone(), to.to_value());
    }
    serde_json::to_string(&out).unwrap()
}

/// What's known of a device something might run on, for `order`.
#[derive(Clone, Copy, Debug)]
pub struct Standing {
    /// It's this device, or the part of it a call is about.
    pub here: bool,
    /// Its socket is open at the relay.
    pub online: bool,
    pub priority: Priority,
    pub far: bool,
    /// It has no room: its last answer said so.
    pub full: bool,
    /// When it was last seen (ms), if it ever was.
    pub seen: Option<u64>,
    /// How long it took to answer lately.
    pub took: Duration,
}

/// The order devices are tried in: this device; then those online; by run priority; the near before the far; those
/// with room; the most recently seen; the quickest. One that isn't online is still tried, after.
pub fn order<K>(devices: &mut [K], standing: impl Fn(&K) -> Standing) {
    devices.sort_by_cached_key(|k| {
        let s = standing(k);
        (!s.here, !s.online, s.priority, s.far, s.full, std::cmp::Reverse(s.seen), s.took)
    });
}
