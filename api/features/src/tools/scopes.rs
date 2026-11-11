//! The `scopes` a call may name (`--scopes global,user`, MCP `scopes: ["global", "user"]`), read one way for both.

use anyhow::{Result, bail};
use serde_json::{Value, json};

const ABOUT: &str = "the scopes the call may touch, by name: global, user or group:<id> (default: every one it may reach)";
const HOW: &str = "write them as --scopes global,user or --scopes '[\"global\",\"user\"]' (MCP: [\"global\", \"user\"])";

/// `scopes` as MCP lists it.
pub fn schema() -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": ABOUT })
}

/// `scopes` as `--help` lists it.
pub fn flag() -> (String, String) {
    ("--scopes <names>".into(), format!("{ABOUT}; global,user or '[\"global\",\"user\"]'"))
}

/// The scopes named by values given (each a flag's, or an item of MCP's list): names separated by commas, or a JSON
/// list of them. A value that is neither says how to write one.
pub fn read<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for value in values {
        let value = value.trim();
        let names: Vec<String> = if value.starts_with('[') {
            match serde_json::from_str(value) {
                Ok(names) => names,
                Err(_) => bail!("scopes {value} isn't a list of names: {HOW}"),
            }
        } else {
            value.split(',').map(|s| s.trim().to_string()).collect()
        };
        for name in names {
            if name.is_empty() || name.contains(|c: char| c.is_whitespace() || "[]\"',".contains(c)) {
                bail!("scopes has {name:?}, which isn't a name: {HOW}");
            }
            out.push(name);
        }
    }
    if out.is_empty() {
        bail!("scopes names none: {HOW}");
    }
    Ok(out)
}
