//! A network's API as an OpenAPI 3.2 document, made from what's defined already: the core's tools, each app's as
//! `GET /api/apps` gives them to this caller, the routes that say what they are (`says`) and what a client answers
//! itself (`settings`). What OpenAPI has no word for goes in `x-one-` extensions, which `ABOUT` says. Every host makes it
//! the same way, a node for those it serves and onet for its own client; what lists tools reads it back (`sets`). Not in it:
//! a client's keys and signed steps (`/api/keys`, `/api/steps`), and what isn't the product's API, an app's own requests
//! (`/<app>/…`, Repos' git among them).

use serde_json::{Map, Value, json};

use crate::feature::Feature;
use crate::handed::Route;
use crate::says::{Op, Said, Who};
use scope::settings;

const ABOUT: &str = "Your network's API, as you can use it now: your apps, the core's tools and routes. Its extensions: `x-one-who`, who may ask (clients, this-machine); `x-one-runs: client`, the caller's client answers it itself, with its own keys, and no app runtime runs it; `x-one-keys`, which keys a client may hand an app runtime with it; `x-one-call`, an app's tool, with no route of its own yet: a client asks POST /api/call with `{ app, tool, input }`, answered `{ ok: <this operation's answer> }`; `x-one-device`, the call can name the device it runs on; `x-one-confirm`, it sends or can't be undone: ask your user first, as nothing enforces it; `x-one-reads`, it only reads; `x-one-accounts`, it lists the accounts signed in to the app; `x-one-words`, the inputs a command line takes as bare words; an input's `x-one-file`, the file it takes, which it's sent as; a tag's `x-one-guide`, how to use it, and `x-one-app`, what a client needs to say of the app: how it runs (`runs`), the services it needs (`uses`) and what setting those up takes (`needs`), `tagline`, and for one with no operations here, `deleted` or `not_added`.";

/// What OpenAPI calls what a tool says of itself, as a tool (`GET /api/apps`) and an operation say it.
const TOOL_AS_OPERATION: [(&str, &str); 6] = [("title", "summary"), ("description", "description"), ("words", "x-one-words"), ("reads", "x-one-reads"), ("confirm", "x-one-confirm"), ("accounts", "x-one-accounts")];

/// The document of `features` and the routes that say what they are (`said`), for a caller who has `apps`, as
/// `GET /api/apps` gives them. Only this machine's own programs see what only they may ask.
pub fn document(features: &[Feature], said: Vec<Said>, apps: &[Value], this_machine: bool) -> Value {
    let mut doc = tools(features, apps);
    let mut add = |path: &str, method: &str, op: Value| {
        doc["paths"][path][method] = op;
    };
    let tool_paths: Vec<String> = features.iter().flat_map(|f| f.tools.iter().map(|t| f.path(t)).collect::<Vec<_>>()).collect();

    for (at, by, op) in said {
        if by != Who::ThisMachine || this_machine {
            let method = op.method;
            let mut op = operation(at, op);
            (op["x-one-who"], op["x-one-keys"]) = (who(by), hands(at));
            if at == crate::path::CALL {
                keys_and_routing(&mut op);
            }
            add(at, method, op);
        }
    }

    for (method, at, summary, body) in settings::answered_routes().filter(|(_, at, ..)| !tool_paths.iter().any(|t| t == at)) {
        let mut op = operation(at, Op::answered(method, summary, body));
        op["x-one-runs"] = json!("client");
        add(at, &method.to_lowercase(), op);
    }

    doc
}

/// The core's tools and each app's, with their tags.
fn tools(features: &[Feature], apps: &[Value]) -> Value {
    let (mut paths, mut tags) = (Map::new(), Vec::new());
    let mut add = |path: &str, method: &str, op: Value| {
        paths.entry(path.to_string()).or_insert_with(|| json!({}))[method] = op;
    };

    for feature in features {
        tags.push(tag(feature.name, feature.title, &feature.description, &feature.guide, json!({ "runs": "client" })));
        for tool in &feature.tools {
            let at = feature.path(tool);
            let mut op = tool_op(feature.mcp_name(tool), feature.name, tool);
            match settings::answered(&at) {
                true => op["x-one-runs"] = json!("client"),
                false => (op["x-one-who"], op["x-one-keys"]) = (json!("clients"), hands(&at)),
            }
            add(&at, "post", op);
        }
    }

    for app in apps {
        let name = app["name"].as_str().unwrap_or_default();
        tags.push(tag(name, app["title"].as_str().unwrap_or(name), app["description"].as_str().unwrap_or(""), app["instructions"].as_str().unwrap_or(""), about_app(app)));
        if !app["deleted"].is_null() {
            continue;
        }
        let tools: Vec<&Value> = app["tools"].as_array().into_iter().flatten().filter(|t| !t["name"].as_str().unwrap_or("").starts_with('_')).collect();
        for tool in tools {
            let tool_name = tool["name"].as_str().unwrap_or_default();
            let mut op = tool_op(format!("{name}_{tool_name}"), name, tool);
            op["x-one-keys"] = json!(crate::handed::hands(&Route::Call(name.to_string())));
            op["x-one-call"] = json!({ "app": name, "tool": tool_name, "added": app["added"] });
            if runs_on_devices(app) && !own_device(tool) {
                op["x-one-device"] = json!(true);
            }
            keys_and_routing(&mut op);
            add(&format!("{}/{name}/{tool_name}", crate::path::CALL), "post", op);
        }
    }

    json!({
        "openapi": "3.2.0",
        "info": { "title": custom_build::config::NAME, "version": custom_build::config::BUILD, "description": ABOUT },
        "tags": tags,
        "paths": paths,
        "components": { "responses": error_responses() },
    })
}

fn tag(name: &str, title: &str, description: &str, guide: &str, app: Value) -> Value {
    json!({ "name": name, "summary": title, "description": description, "x-one-guide": guide, "x-one-app": app })
}

/// A tag's `x-one-app`, of an app as `GET /api/apps` gives it.
fn about_app(app: &Value) -> Value {
    let mut about = Map::new();
    for key in ["runs", "needs", "tagline", "deleted"].into_iter().filter(|k| !app[*k].is_null()) {
        about.insert(key.to_string(), app[key].clone());
    }
    if !app["uses"]["services"].is_null() {
        about.insert("uses".to_string(), json!({ "services": app["uses"]["services"] }));
    }
    if app["tools"].is_null() {
        about.insert("not_added".to_string(), json!(true));
    }
    Value::Object(about)
}

/// The sets of tools a document lists, each as `GET /api/apps` shapes an app, for whoever makes commands or MCP tools of
/// them: each tool also with the name MCP calls it (`id`) and, when it's its own route, that route (`route`). A set the
/// caller can't use yet has no `tools`.
pub fn sets(doc: &Value) -> Vec<Value> {
    let operations = || doc["paths"].as_object().into_iter().flatten().filter_map(|(path, item)| Some((path.as_str(), item.get("post")?))).filter(|(_, op)| op["operationId"].is_string());
    let set_of = |tag: &Value| {
        let mut set = if tag["x-one-app"].is_object() { tag["x-one-app"].clone() } else { json!({}) };
        for (key, from) in [("name", "name"), ("title", "summary"), ("description", "description"), ("instructions", "x-one-guide")] {
            set[key] = tag[from].clone();
        }
        if set["not_added"] != true && set["deleted"].is_null() {
            set["tools"] = json!(operations().filter(|(_, op)| op["tags"][0] == tag["name"]).map(|(path, op)| tool_of(path, op)).collect::<Vec<_>>());
        }
        set.as_object_mut().map(|s| s.remove("not_added"));
        set
    };
    doc["tags"].as_array().into_iter().flatten().map(set_of).collect()
}

/// An operation with an `operationId` as the tool it was made from (`tool_op`).
fn tool_of(path: &str, op: &Value) -> Value {
    let name = op["x-one-call"]["tool"].as_str().unwrap_or_else(|| path.rsplit('/').next().unwrap_or(""));
    let mut input = op["requestBody"]["content"]["application/json"]["schema"].clone();
    for prop in input["properties"].as_object_mut().into_iter().flat_map(|p| p.values_mut()) {
        if prop["x-one-file"].is_object() {
            *prop = prop["x-one-file"].take();
        } else if prop["items"]["x-one-file"].is_object() {
            prop["items"] = prop["items"]["x-one-file"].take();
        }
    }
    let mut tool = json!({ "name": name, "id": op["operationId"], "input": input });
    for (key, said) in TOOL_AS_OPERATION {
        if let Some(value) = op.get(said) {
            tool[key] = value.clone();
        }
    }
    let output = &op["responses"]["200"]["content"]["application/json"]["schema"];
    if !output.is_null() {
        tool["output"] = output.clone();
    }
    if op["x-one-call"].is_null() {
        tool["route"] = json!(path);
    }
    tool
}

pub fn hands(at: &str) -> Value {
    json!(crate::handed::hands(&Route::of(at, || "{app}".to_string())))
}

/// What an operation that runs a call also answers.
pub fn keys_and_routing(op: &mut Value) {
    op["responses"]["421"] = json!({ "$ref": "#/components/responses/Elsewhere" });
    op["responses"]["428"] = json!({ "$ref": "#/components/responses/NeedsKeys" });
}

pub fn responses(answer: Value) -> Value {
    json!({ "200": { "description": "Its answer", "content": answer }, "default": { "$ref": "#/components/responses/Failed" } })
}

fn tool_op(id: String, tag: &str, tool: &Value) -> Value {
    let answer = match tool.get("output") {
        Some(output) => json!({ "application/json": { "schema": output } }),
        None => json!({ "application/json": {} }),
    };
    let mut op = json!({ "operationId": id, "tags": [tag], "requestBody": { "required": true, "content": { "application/json": { "schema": input_schema(&tool["input"]) } } }, "responses": responses(answer) });
    for (from, to) in TOOL_AS_OPERATION {
        if let Some(value) = tool.get(from).filter(|v| !v.is_null() && **v != json!(false)) {
            op[to] = value.clone();
        }
    }
    op
}

/// A tool's input as it's sent: an input that takes a file (`format: "file"`) is its name, type, size and bytes, as a
/// client reads it for the tool. One that lists no inputs takes any.
fn input_schema(input: &Value) -> Value {
    let mut schema = input.clone();
    if !schema["properties"].is_object() {
        return json!({ "type": "object", "additionalProperties": true });
    }
    for prop in schema["properties"].as_object_mut().into_iter().flat_map(|p| p.values_mut()) {
        if prop["format"] == "file" {
            *prop = file_input(prop);
        } else if prop["items"]["format"] == "file" {
            prop["items"] = file_input(&prop["items"]);
        }
    }
    schema
}

fn file_input(given: &Value) -> Value {
    let mut schema = json!({ "type": "object", "required": ["name", "type", "size", "base64"], "properties": { "name": { "type": "string" }, "type": { "type": "string" }, "size": { "type": "integer" }, "base64": { "type": "string", "contentEncoding": "base64" } }, "x-one-file": given });
    if given["description"].is_string() {
        schema["description"] = given["description"].clone();
    }
    schema
}

/// What the API answers when something fails, as the OpenAPI document says it: `failed`'s kinds, 421 and 428 for what a
/// call's client does something about.
fn error_responses() -> Value {
    let error = json!({ "type": "string", "description": "why, for whoever reads it" });
    let mut needs = custom_build::protocol::schema_of::<crate::handed::Needs>();
    needs["properties"]["error"] = error.clone();
    needs["required"].as_array_mut().expect("Needs has fields").push(json!("error"));
    let answer = |description: &str, schema: Value| json!({ "description": description, "content": { "application/json": { "schema": schema } } });
    json!({
        "Failed": answer("Why it failed", json!({ "type": "object", "required": ["error"], "properties": { "error": error, "code": { "type": "string", "description": "the kind of failure callers act on" } } })),
        "Elsewhere": answer("This app runtime can't run it: ask one of these", json!({ "type": "object", "required": ["error", "elsewhere"], "properties": { "error": error, "elsewhere": { "type": "array", "items": { "type": "string" } } } })),
        "NeedsKeys": answer("It needs keys it wasn't handed: send it again with these", needs),
    })
}

/// A tool as an app lists it (`GET /api/apps`), and the core's as onet's `features()` do.
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
    let mut out = json!({ "summary": op.summary, "responses": responses(op.answer) });
    if !parameters.is_empty() {
        out["parameters"] = json!(parameters);
    }
    if let Some(body) = op.body {
        out["requestBody"] = json!({ "required": true, "content": body });
    }
    out
}

/// Whether a tool takes a `device` of its own (an agent device's name, say): then that's what `device` is, and
/// it says itself where it runs, so the CLI's and MCP's `device` (which of your devices runs it) isn't offered.
pub fn own_device(tool: &Value) -> bool {
    tool["input"]["properties"].get("device").is_some()
}

/// Whether an app's calls run on a device of the user's choosing: it runs on every device, or uses a service (Shell
/// on a named device, iMessage on the Mac), so MCP and search offer `device` as the CLI's `--device` does.
pub fn runs_on_devices(app: &Value) -> bool {
    app["runs"] == "here" || app["uses"]["services"].as_array().is_some_and(|s| !s.is_empty())
}
