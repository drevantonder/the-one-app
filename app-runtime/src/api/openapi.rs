//! The node's route for its API's document, `/api/openapi.json`, made as every host makes it (`api::openapi`).

use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use crate::Host;
use ::api::path;

use ::api::reply::{Error, tagged};
use super::Routes;
use ::api::Me;
use ::api::Who;

pub(super) fn routes() -> Routes {
    Routes::default().route(path::OPENAPI, Who::Clients, get(document_for))
}

async fn document_for(State(host): State<Host>, me: Me, headers: axum::http::HeaderMap) -> Result<Response, Error> {
    let apps = host.apps(&me.caller).await?;
    let body = serde_json::to_string(&api_features::openapi::document(&apps, me.client.is_none()))?;
    let tag = ::ui::screen::tag(&body);
    Ok(tagged(&headers, tag, ([(axum::http::header::CONTENT_TYPE, "application/json")], body)))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    /// Each caller is told only what they can use: their own apps' tools, never a tool an app keeps to itself, and what only
    /// this machine's own programs may ask only to them.
    #[test]
    fn a_callers_document_lists_only_what_they_can_use() {
        let apps = [json!({ "name": "notes", "added": ["user:a"], "tools": [{ "name": "add", "input": { "type": "object", "properties": {} } }, { "name": "_http" }] }), json!({ "name": "gone", "deleted": { "time": 1 }, "tools": [{ "name": "run" }] })];
        let theirs = api_features::openapi::document(&apps, false);
        let paths = theirs["paths"].as_object().unwrap();
        assert!(paths.contains_key("/api/call/notes/add") && !paths.contains_key("/api/call/notes/_http"), "their app's tools, not its own");
        assert!(!paths.keys().any(|p| p.starts_with("/api/call/gone/")), "a deleted app's");
        assert!(!paths.contains_key("/api/join") && api_features::openapi::document(&apps, true)["paths"].get("/api/join").is_some(), "only this machine's programs join");
    }

    /// What a command line and MCP read of a document is what the app lists: each tool whole, a file it takes still a
    /// file, and an app that can't be used yet still named, to say why.
    #[test]
    fn the_sets_a_document_lists_are_what_was_listed() {
        let tool = json!({ "name": "send", "title": "Send", "description": "Sends", "input": { "type": "object", "required": ["to"], "properties": { "to": { "type": "string" }, "file": { "type": "object", "format": "file", "description": "it" }, "more": { "type": "array", "items": { "type": "object", "format": "file", "required": ["name"] } } } }, "output": { "type": "object" }, "confirm": true, "accounts": true, "words": ["to"] });
        let apps = [json!({ "name": "notes", "title": "Notes", "description": "d", "instructions": "i", "tagline": "t", "needs": "a Mac", "runs": "service", "uses": { "services": ["x"] }, "added": ["user:a"], "tools": [tool, { "name": "_http", "input": {} }] }), json!({ "name": "gone", "title": "Gone", "deleted": { "time": 1 }, "tools": [{ "name": "run" }] }), json!({ "name": "new", "title": "New" })];
        let sets = ::api::openapi::sets(&api_features::openapi::document(&apps, false));
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
        assert_eq!(list["route"], "/api/vault/list");
    }

    /// A route in the document says what it answers: one left to say anything is one a client can't read the shape of.
    #[test]
    fn each_route_says_what_it_answers() {
        for (path, _, op) in api_features::says::listed() {
            assert!(op.answer != ::api::says::Op::unsaid(), "{} {path} says what it answers (api/features/src/says.rs: `answers`, `answers_as`, `done`)", op.method);
        }
    }

    /// The last word of a type's path: `Body<body::Role>` and `.body::<Role>()` name the same struct.
    fn last(ty: &str) -> &str {
        ty.rsplit("::").next().unwrap()
    }

    /// A route's schema is what its handler reads, in the only place one could differ from the other: which struct each
    /// names. What a route says (api/features/src/says.rs) names the struct of its handler's `Body` or `Query`, and what a client answers
    /// itself reads the body `settings::about` says.
    #[test]
    fn each_routes_schema_is_what_its_handler_reads() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = repo.join("src");
        // This program's, and each feature's with a runtime crate (`<feature>/src`, `<feature>/runtime/src`).
        let features = std::fs::read_dir(repo).unwrap().flatten().map(|e| e.path()).filter(|d| d.join("runtime/Cargo.toml").exists());
        let mut stack: Vec<_> = std::iter::once(root.clone()).chain(features.flat_map(|d| [d.join("src"), d.join("runtime/src")])).collect();
        let (mut routes, mut says) = (Vec::new(), Vec::new());
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let source = std::fs::read_to_string(&path).unwrap();
                    for route in source.split(".route(").skip(1) {
                        routes.push((path.clone(), source.clone(), route.to_string()));
                    }
                    if path.file_name().is_some_and(|n| n == "says.rs") {
                        says.extend(source.split("(\"/").skip(1).map(|s| s.to_string()));
                    }
                }
            }
        }
        let mut said = 0;
        for says in says {
            let path = format!("/{}", says.split('"').next().unwrap());
            let method = says.split("Op::").nth(1).and_then(|s| s.split('(').next()).unwrap_or_default();
            let Some((extractor, ty)) = [("Body", ".body::<"), ("Query", ".query::<")].into_iter().find_map(|(e, form)| Some((e, says.split(form).nth(1)?.split('>').next()?))) else { continue };
            let (file, source, route) = routes.iter().find(|(_, _, r)| r.split('"').nth(1) == Some(path.as_str()) && r.split(',').nth(1).is_some_and(|who| who.contains("Clients") || who.contains("ThisMachine"))).unwrap_or_else(|| panic!("{path} says what it takes, and no route is it"));
            let handler = route.split(&format!("{method}(")).nth(1).and_then(|s| s.split([')', ',']).next()).unwrap_or_default();
            let signature = source.split(&format!("fn {handler}(")).nth(1).and_then(|s| s.split("->").next()).unwrap_or_else(|| panic!("no handler {handler} in {file:?}"));
            let read = signature.split(&format!("{extractor}<")).nth(1).and_then(|s| s.split('>').next()).unwrap_or_else(|| panic!("{handler} in {file:?} doesn't read a {extractor}"));
            assert_eq!(last(read), last(ty), "{file:?}: {handler} reads {read}, {path} says {ty}");
            said += 1;
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
        for feature in api_features::core_tools::all() {
            for tool in &feature.tools {
                let at = feature.path(tool);
                let Some((.., body)) = scope::settings::answered_routes().find(|(_, path, ..)| *path == at) else { continue };
                let names = |schema: &Value| schema["properties"].as_object().map(|p| p.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
                assert_eq!(names(&tool["input"]), names(&body), "{at}'s tool and what its client reads");
            }
        }
    }
}
