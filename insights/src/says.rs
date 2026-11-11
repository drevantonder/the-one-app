//! What a client's report of what went wrong says of itself (src/says.rs).

use serde::Deserialize;
use serde_json::Value;

use ::api::says::Op;
use ::api::says::Said;
use ::api::says::Who;

pub fn listed() -> Vec<Said> {
    vec![("/api/insights/report", Who::Clients, Op::post("What went wrong on a client, and what it did before that").body::<Report>().done())]
}

/// `events`: what the client's timeline holds (`ui/web/insights.js`), each `{ name, time, … }`.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Report {
    pub platform: String,
    pub events: Vec<Value>,
}
