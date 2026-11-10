//! What the network's own routes say of themselves: its state, joining it, and running a call (src/says.rs).

use serde_json::json;

use crate::protocol::CallRequest;
use crate::says::{Op, Said, Who};

pub fn listed() -> Vec<Said> {
    vec![
        ("/api/state", Who::Clients, Op::get("Who's who and what this device reads of the network").answers::<crate::state::State>()),
        ("/api/join", Who::ThisMachine, Op::post("Join a network with a pairing code").body::<Join>().done()),
        ("/api/call", Who::Clients, Op::post("Run one call of an app's tool or a service's function").body::<CallRequest>().answers_as(json!({ "type": "object", "required": ["ok"], "properties": { "ok": { "description": "the tool's or function's answer, as its output schema says" } } }))),
    ]
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Join {
    pub code: String,
}
