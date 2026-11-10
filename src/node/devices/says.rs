//! What the devices' routes say of themselves (src/says.rs).

use serde::Deserialize;
use serde_json::json;

use crate::says::{Op, Said, Who};

pub fn listed() -> Vec<Said> {
    vec![
        ("/api/pair", Who::Clients, Op::post("A pairing code for a new device or someone new").body::<Pair>().answers_as(json!({
            "type": "object",
            "required": ["code", "user"],
            "properties": { "code": { "type": "string", "description": "to give the new device" }, "user": { "type": "string", "description": "whose it will be" } }
        }))),
        ("/api/erase", Who::Clients, Op::post("Erase what a device keeps").body::<Device>().answers_as(json!({ "type": "object", "required": ["erasing"], "properties": { "erasing": { "type": "string", "description": "the device's name" } } }))),
    ]
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    pub user: Option<String>,
}

/// A device, by its key.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub key: String,
}
