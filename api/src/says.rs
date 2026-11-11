//! What each route of the API says of itself in the OpenAPI document (`openapi`): who may ask it, what it takes and what
//! it answers. Plain data, so a Worker or a browser's onet makes the document a device does. Each feature says its own,
//! beside its routes, with the structs its handlers read.

use schemars::JsonSchema;
use serde_json::{Value, json};

use custom_build::protocol::schema_of;

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
    pub method: &'static str,
    pub summary: &'static str,
    pub query: Option<Value>,
    pub body: Option<Value>,
    pub answer: Value,
}

impl Op {
    fn new(method: &'static str, summary: &'static str) -> Op {
        Op { method, summary, query: None, body: None, answer: Op::unsaid() }
    }

    /// What an answer is until its route says: anything.
    pub fn unsaid() -> Value {
        json!({ "application/json": {} })
    }

    pub fn get(summary: &'static str) -> Op {
        Op::new("get", summary)
    }

    pub fn post(summary: &'static str) -> Op {
        Op::new("post", summary)
    }

    /// A route of `method` ("GET" or "POST") that a client answers itself (`settings::answered_routes`), taking `body`.
    pub fn answered(method: &str, summary: &'static str, body: Value) -> Op {
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
