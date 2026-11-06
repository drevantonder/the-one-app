//! Labs: features still being built, which a network's admins switch on for everyone in Settings › Labs. An app
//! declares its own in its module, and a service in what it describes, as `labs: { <name>: { title, description } }`.
//! Each is off until its setting says "on": `lab.app.<app>.<name>` for an app's, `lab.service.<service>.<name>` for a
//! service's, which a service reads from the network's settings as any other. A lab that's done leaves its
//! declaration, and its setting goes with it.

use std::collections::BTreeMap;

/// Whether `name` may be one of the network's settings: lowercase words joined by _, or a lab's, whose parts are an
/// app's or service's name and a lab's. An older device ignores a lab's setting, so to it the lab is off.
pub fn valid_setting(name: &str) -> bool {
    let word = |w: &str| !w.is_empty() && w.len() <= 40 && w.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    match name.strip_prefix("lab.") {
        Some(rest) => {
            let parts: Vec<&str> = rest.split('.').collect();
            match parts.as_slice() {
                [_] => parts.iter().all(|p| word(p)),
                ["app" | "service", _, _] => parts.iter().all(|p| word(p)),
                _ => false,
            }
        }
        None => !name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
    }
}

/// The setting that switches an app's lab on.
pub fn app_key(app: &str, lab: &str) -> String {
    format!("lab.app.{app}.{lab}")
}

/// Whether the lab whose setting is `key` is on, by the network's `settings`.
pub fn on(settings: &BTreeMap<String, String>, key: &str) -> bool {
    settings.get(key).is_some_and(|v| v == "on")
}

/// The settings of every lab that's on, of the network's `settings`.
pub fn on_keys(settings: &BTreeMap<String, String>) -> Vec<String> {
    settings.iter().filter(|(k, v)| k.starts_with("lab.") && *v == "on").map(|(k, _)| k.clone()).collect()
}

/// `app`'s labs that are on, by name, from `on_keys`: what its `ctx.labs` says.
pub fn of_app(on: &[String], app: &str) -> serde_json::Value {
    let prefix = format!("lab.app.{app}.");
    serde_json::Value::Object(on.iter().filter_map(|k| k.strip_prefix(&prefix)).map(|n| (n.to_string(), serde_json::json!(true))).collect())
}
