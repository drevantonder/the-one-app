//! The node's API as one OpenAPI 3.2 document, for whoever asks, at `/api/openapi.json`: its tools (src/openapi.rs), the
//! routes that say what they are (`Routes::says`), and what a client answers itself (`settings`). Not in it: a client's keys and signed steps (`/v1/keys`, `/v1/steps`), and what isn't the
//! product's API, an app's own requests (`/<app>/…`, Repos' git among them).

use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use schemars::JsonSchema;
use serde_json::{Value, json};

use crate::node::Node;
use crate::protocol::{path, schema_of};
use crate::settings;

use super::reply::{Error, tagged};
use super::{Me, Routes, Who};

pub(super) fn routes() -> Routes {
    Routes::default().route(path::OPENAPI, Who::Clients, get(document_for))
}

async fn document_for(State(node): State<Node>, me: Me, headers: axum::http::HeaderMap) -> Result<Response, Error> {
    let apps = node.apps(&me.caller).await?;
    let body = serde_json::to_string(&document(&apps, me.client.is_none()))?;
    let tag = crate::screen::tag(&body);
    Ok(tagged(&headers, tag, ([(axum::http::header::CONTENT_TYPE, "application/json")], body)))
}

/// A route as the document lists it: its method, a line for whoever reads it, what it takes and answers.
pub struct Op {
    method: &'static str,
    summary: &'static str,
    query: Option<Value>,
    body: Option<Value>,
    answer: Value,
}

impl Op {
    fn new(method: &'static str, summary: &'static str) -> Op {
        Op { method, summary, query: None, body: None, answer: Op::unsaid() }
    }

    /// What an answer is until its route says: anything.
    fn unsaid() -> Value {
        json!({ "application/json": {} })
    }

    pub fn get(summary: &'static str) -> Op {
        Op::new("get", summary)
    }

    pub fn post(summary: &'static str) -> Op {
        Op::new("post", summary)
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

/// The document for a caller who has `apps`, as `GET /v1/apps` gives them. Only this machine's own programs see what only
/// they may ask.
pub fn document(apps: &[Value], this_machine: bool) -> Value {
    let mut doc = crate::openapi::document(apps);
    let mut add = |path: &str, method: &str, op: Value| {
        doc["paths"][path][method] = op;
    };
    let tool_paths: Vec<String> = crate::core_tools::all().iter().flat_map(|f| f.tools.iter().map(|t| f.path(&t.spec)).collect::<Vec<_>>()).collect();

    for (at, by, op) in super::routes().listed {
        if by != Who::ThisMachine || this_machine {
            let method = op.method;
            let mut op = operation(&at, op);
            (op["x-one-who"], op["x-one-keys"]) = (who(by), crate::openapi::hands(&at));
            if at == path::CALL {
                crate::openapi::keys_and_routing(&mut op);
            }
            add(&at, method, op);
        }
    }

    for (method, at, summary, body) in settings::answered_routes().filter(|(_, at, ..)| !tool_paths.iter().any(|t| t == at)) {
        let mut op = Op::new(if method == "GET" { "get" } else { "post" }, summary);
        if method != "GET" {
            op.body = Some(json!({ "application/json": { "schema": body } }));
        }
        let mut op = operation(at, op);
        op["x-one-runs"] = json!("client");
        add(at, &method.to_lowercase(), op);
    }

    doc
}

/// A tool as an app lists it (`GET /v1/apps`), and the core's as `core_tools` do.
pub fn tool_spec() -> Value {
    json!({
        "type": "object",
        "required": ["name", "description", "input"],
        "properties": {
            "name": { "type": "string" },
            "title": { "type": "string", "description": "a few words for what it does" },
            "description": { "type": "string" },
            "input": { "type": "object", "description": "JSON Schema of what it takes" },
            "output": { "description": "JSON Schema of what it answers" },
            "reads": { "type": "boolean" },
            "confirm": { "type": "boolean" },
            "accounts": { "type": "boolean" },
            "words": { "type": "array", "items": { "type": "string" } },
            "lab": { "type": "string" }
        }
    })
}

fn who(by: Who) -> Value {
    json!(match by {
        Who::Clients => "clients",
        Who::ThisMachine => "this-machine",
    })
}

fn operation(at: &str, op: Op) -> Value {
    let mut parameters: Vec<Value> = at.split('/').filter_map(|s| s.strip_prefix('{')?.strip_suffix('}')).map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } })).collect();
    if let Some(query) = &op.query {
        let required: Vec<&str> = query["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        parameters.extend(query["properties"].as_object().into_iter().flatten().map(|(name, schema)| json!({ "name": name, "in": "query", "required": required.contains(&name.as_str()), "schema": schema })));
    }
    let mut out = json!({ "summary": op.summary, "responses": crate::openapi::responses(op.answer) });
    if !parameters.is_empty() {
        out["parameters"] = json!(parameters);
    }
    if let Some(body) = op.body {
        out["requestBody"] = json!({ "required": true, "content": body });
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    /// Each caller is told only what they can use: their own apps' tools, never a tool an app keeps to itself, and what only
    /// this machine's own programs may ask only to them.
    #[test]
    fn a_callers_document_lists_only_what_they_can_use() {
        let apps = [json!({ "name": "notes", "added": ["user:a"], "tools": [{ "name": "add", "input": { "type": "object", "properties": {} } }, { "name": "_http" }] }), json!({ "name": "gone", "deleted": { "time": 1 }, "tools": [{ "name": "run" }] })];
        let theirs = super::document(&apps, false);
        let paths = theirs["paths"].as_object().unwrap();
        assert!(paths.contains_key("/v1/call/notes/add") && !paths.contains_key("/v1/call/notes/_http"), "their app's tools, not its own");
        assert!(!paths.keys().any(|p| p.starts_with("/v1/call/gone/")), "a deleted app's");
        assert!(!paths.contains_key("/v1/join") && super::document(&apps, true)["paths"].get("/v1/join").is_some(), "only this machine's programs join");
    }

    /// What a command line and MCP read of a document is what the app lists: each tool whole, a file it takes still a
    /// file, and an app that can't be used yet still named, to say why.
    #[test]
    fn the_sets_a_document_lists_are_what_was_listed() {
        let tool = json!({ "name": "send", "title": "Send", "description": "Sends", "input": { "type": "object", "required": ["to"], "properties": { "to": { "type": "string" }, "file": { "type": "object", "format": "file", "description": "it" }, "more": { "type": "array", "items": { "type": "object", "format": "file", "required": ["name"] } } } }, "output": { "type": "object" }, "confirm": true, "accounts": true, "words": ["to"] });
        let apps = [json!({ "name": "notes", "title": "Notes", "description": "d", "instructions": "i", "tagline": "t", "needs": "a Mac", "runs": "service", "uses": { "services": ["x"] }, "added": ["user:a"], "tools": [tool, { "name": "_http", "input": {} }] }), json!({ "name": "gone", "title": "Gone", "deleted": { "time": 1 }, "tools": [{ "name": "run" }] }), json!({ "name": "new", "title": "New" })];
        let sets = crate::openapi::sets(&super::document(&apps, false));
        let of = |name: &str| sets.iter().find(|s| s["name"] == name).unwrap();
        let notes = of("notes");
        for (key, want) in [("title", "Notes"), ("description", "d"), ("instructions", "i"), ("tagline", "t"), ("needs", "a Mac"), ("runs", "service")] {
            assert_eq!(notes[key], want, "{key}");
        }
        assert_eq!(notes["uses"]["services"], json!(["x"]));
        assert_eq!(notes["tools"].as_array().unwrap().len(), 1, "not what it keeps to itself");
        let mut read = notes["tools"][0].clone();
        assert_eq!(read["id"], "notes_send");
        assert!(read["route"].is_null(), "an app's tool is a call");
        read.as_object_mut().unwrap().remove("id");
        assert_eq!(read, tool);
        assert!(!of("gone")["deleted"].is_null() && of("gone")["tools"].is_null());
        assert!(of("new")["deleted"].is_null() && of("new")["tools"].is_null(), "one to add");
        let vault = of("vault");
        assert_eq!(vault["runs"], "client");
        let list = vault["tools"].as_array().unwrap().iter().find(|t| t["id"] == "vault_list").unwrap();
        assert_eq!(list["route"], "/v1/vault/list");
    }

    /// A route in the document says what it answers: one left to say anything is one a client can't read the shape of.
    #[test]
    fn each_route_says_what_it_answers() {
        for (path, _, op) in crate::api::routes().listed {
            assert!(op.answer != super::Op::unsaid(), "{} {path} says what it answers (src/api/openapi.rs: `answers`, `answers_as`, `done`)", op.method);
        }
    }

    /// The last word of a type's path: `Body<body::Role>` and `.body::<Role>()` name the same struct.
    fn last(ty: &str) -> &str {
        ty.rsplit("::").next().unwrap()
    }

    /// A route's schema is what its handler reads, in the only place one could differ from the other: which struct each
    /// names. A route's `says` names the struct of its handler's `Body` or `Query`, and what a client answers itself reads
    /// the body `settings::about` says.
    #[test]
    fn each_routes_schema_is_what_its_handler_reads() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root.clone()];
        let mut said = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let source = std::fs::read_to_string(&path).unwrap();
                    for route in source.split(".route(").skip(1) {
                        for says in route.split(".says(").skip(1) {
                            let says = says.split(".route(").next().unwrap();
                            let method = says.split("Op::").nth(1).and_then(|s| s.split('(').next()).unwrap_or_default();
                            let Some((extractor, ty)) = [("Body", ".body::<"), ("Query", ".query::<")].into_iter().find_map(|(e, form)| Some((e, says.split(form).nth(1)?.split('>').next()?))) else { continue };
                            let handler = route.split(&format!("{method}(")).nth(1).and_then(|s| s.split([')', ',']).next()).unwrap_or_default();
                            let signature = source.split(&format!("fn {handler}(")).nth(1).and_then(|s| s.split("->").next()).unwrap_or_else(|| panic!("no handler {handler} in {path:?}"));
                            let read = signature.split(&format!("{extractor}<")).nth(1).and_then(|s| s.split('>').next()).unwrap_or_else(|| panic!("{handler} in {path:?} doesn't read a {extractor}"));
                            assert_eq!(last(read), last(ty), "{path:?}: {handler} reads {read}, its route says {ty}");
                            said += 1;
                        }
                    }
                }
            }
        }
        assert!(said > 10, "the routes that say what they read were found");

        let settings = std::fs::read_to_string(root.join("settings.rs")).unwrap();
        let about = settings.split("fn about(").nth(1).unwrap().split("\n}\n").next().unwrap();
        let answered = settings.split("async fn answered_by(").nth(1).unwrap().split("\n}\n").next().unwrap();
        for arm in about.split("Route::").skip(1) {
            let (route, rest) = arm.split_once(" =>").unwrap();
            let ty = rest.split("of::<").nth(1).unwrap().split('>').next().unwrap();
            let reads = answered.split(&format!("Route::{route} =>")).nth(1).unwrap_or_else(|| panic!("{route} isn't answered")).split("\n        Route::").next().unwrap();
            assert!(reads.contains(ty), "{route}: answered_by doesn't read {ty}");
        }
    }

    /// What a client answers itself of a tool is that tool's body: nothing else holds the two together.
    #[test]
    fn a_tool_a_client_answers_takes_what_the_client_reads() {
        for feature in crate::core_tools::all() {
            for tool in &feature.tools {
                let at = feature.path(&tool.spec);
                let Some((.., body)) = crate::settings::answered_routes().find(|(_, path, ..)| *path == at) else { continue };
                let names = |schema: &Value| schema["properties"].as_object().map(|p| p.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
                assert_eq!(names(&tool.spec["input"]), names(&body), "{at}'s tool and what its client reads");
            }
        }
    }
}
