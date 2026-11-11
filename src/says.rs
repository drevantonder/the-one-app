//! What each route of the node's API says of itself in the OpenAPI document (src/openapi.rs): who may ask it, what it
//! takes and what it answers. Plain data, so a Worker or a browser's onet makes the document a node does. Each feature's
//! is beside its routes (`says.rs`), with the structs its handlers read; they're under `node/`, which a Worker or a browser
//! doesn't build, so this is where they're named.

use schemars::JsonSchema;
use serde_json::{Value, json};

use ::custom_build::protocol::schema_of;

#[path = "api/says.rs"]
pub mod api;
#[path = "node/apps/says.rs"]
pub mod apps;
#[path = "node/backups/says.rs"]
pub mod backups;
#[path = "node/blobs/says.rs"]
pub mod blobs;
#[path = "node/devices/says.rs"]
pub mod devices;
#[path = "insights/says.rs"]
pub mod insights;
#[path = "node/screens/says.rs"]
pub mod screens;
#[path = "node/vault/says.rs"]
pub mod vault;
#[path = "node/widgets/says.rs"]
pub mod widgets;

/// Who may ask a route. This machine's own programs may ask any; the rest only what its route says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// Also the network's client devices over iroh, each as their user, with what that user may do.
    Clients,
    /// Only this machine's own programs.
    ThisMachine,
}

/// A route as the document lists it: its method, a line for whoever reads it, what it takes and answers.
pub struct Op {
    pub(crate) method: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) query: Option<Value>,
    pub(crate) body: Option<Value>,
    pub(crate) answer: Value,
}

impl Op {
    fn new(method: &'static str, summary: &'static str) -> Op {
        Op { method, summary, query: None, body: None, answer: Op::unsaid() }
    }

    /// What an answer is until its route says: anything.
    pub(crate) fn unsaid() -> Value {
        json!({ "application/json": {} })
    }

    pub fn get(summary: &'static str) -> Op {
        Op::new("get", summary)
    }

    pub fn post(summary: &'static str) -> Op {
        Op::new("post", summary)
    }

    /// A route of `method` ("GET" or "POST") that a client answers itself (`settings::answered_routes`), taking `body`.
    pub(crate) fn answered(method: &str, summary: &'static str, body: Value) -> Op {
        let mut op = Op::new(if method == "GET" { "get" } else { "post" }, summary);
        if method != "GET" {
            op.body = Some(json!({ "application/json": { "schema": body } }));
        }
        op
    }

    /// Its JSON body, the struct its handler reads as `Body<T>`.
    pub fn body<T: JsonSchema>(mut self) -> Op {
        self.body = Some(json!({ "application/json": { "schema": schema_of::<T>() } }));
        self
    }

    /// Its query, the struct its handler reads as `Query<T>`.
    pub fn query<T: JsonSchema>(mut self) -> Op {
        self.query = Some(schema_of::<T>());
        self
    }

    /// Its JSON answer, the struct its handler gives.
    pub fn answers<T: JsonSchema>(mut self) -> Op {
        self.answer = json!({ "application/json": { "schema": schema_of::<T>() } });
        self
    }

    /// Its JSON answer, as JSON Schema, for one its handler makes of JSON. `tests/e2e.rs` holds what it answers to it.
    pub fn answers_as(mut self, schema: Value) -> Op {
        self.answer = json!({ "application/json": { "schema": schema } });
        self
    }

    /// It answers `{}`: that it's done.
    pub fn done(self) -> Op {
        self.answers_as(json!({ "type": "object", "additionalProperties": false }))
    }

    /// It takes a file's bytes.
    pub fn takes_file(mut self) -> Op {
        self.body = Some(json!({ "application/octet-stream": {} }));
        self
    }

    /// It answers a file's bytes.
    pub fn file(mut self) -> Op {
        self.answer = json!({ "application/octet-stream": {} });
        self
    }

    /// It answers a page.
    pub fn html(mut self) -> Op {
        self.answer = json!({ "text/html": {} });
        self
    }

    /// It answers Server-Sent Events, each `data` a `T` as JSON.
    pub fn events<T: JsonSchema>(mut self) -> Op {
        self.answer = json!({ "text/event-stream": { "itemSchema": { "type": "object", "required": ["data"], "properties": { "data": { "type": "string", "contentMediaType": "application/json", "contentSchema": schema_of::<T>() } } } } });
        self
    }
}

/// A route that says what it is: its path, who may ask it, and one `Op` for each method it answers.
pub type Said = (&'static str, Who, Op);

/// Every route that says what it is.
pub fn listed() -> Vec<Said> {
    [api::listed(), apps::listed(), backups::listed(), blobs::listed(), devices::listed(), insights::listed(), screens::listed(), vault::listed(), widgets::listed()].into_iter().flatten().collect()
}
