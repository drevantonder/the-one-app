//! The core's tools, each their own route (`core_tools::Tool`), each at `POST /api/<feature>/<tool>`: its
//! body is held to the tool's input schema before it runs, and its answer to its output schema before it goes.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use serde_json::{Map, Value};

use crate::core_tools::Serves;
use crate::node::{Caller, Node};

use super::guard::Me;
use super::reply::{Error, Reply};
use super::{Routes, Who};

/// Every one, asked by a client device; but one a client answers itself (`settings::answered`) only by this
/// machine's own programs, which its node answers as its own device.
pub(super) fn routes() -> Routes {
    let mut routes = Routes::default();
    for feature in crate::core_tools::all() {
        for tool in &feature.tools {
            let serves = tool.serves;
            let path = feature.path(&tool.spec);
            let who = if crate::settings::answered(&path) { Who::ThisMachine } else { Who::Clients };
            let (spec, name) = (tool.spec.clone(), feature.mcp_name(&tool.spec));
            routes = routes.route(&path, who, post(move |node: State<Node>, me: Me, headers: HeaderMap, input: Json<Map<String, Value>>| serve(node, me, headers, input, serves, spec, name)));
        }
    }
    routes
}

async fn serve(State(node): State<Node>, me: Me, headers: HeaderMap, Json(input): Json<Map<String, Value>>, serves: Serves, spec: Value, name: String) -> Reply {
    crate::tools::check(&spec["input"], &Value::Object(input.clone()))?;
    let agent = headers.get(crate::protocol::header::AGENT).and_then(|a| data_encoding::BASE64URL_NOPAD.decode(a.as_bytes()).ok()).and_then(|a| String::from_utf8(a).ok());
    let me = Caller { agent: agent.or(me.caller.agent), ..me.caller };
    let answer = serves(node, me, input).await?;
    crate::tools::check(&spec["output"], &answer).map_err(|e| Error(e.context(format!("{name} answered what its output schema doesn't say"))))?;
    Ok(Json(answer))
}

#[cfg(test)]
mod tests {
    use crate::core_tools::all;

    /// What's generated from a tool (the CLI, MCP, later OpenAPI) has only its definition to go
    /// by, and a schema left out holds its body or answer to nothing.
    #[test]
    fn a_tool_that_is_a_route_says_what_it_takes_and_answers() {
        for feature in all() {
            for tool in feature.tools.iter() {
                let name = feature.mcp_name(&tool.spec);
                assert!(tool.spec["input"]["type"] == "object", "{name} says what it takes");
                assert!(tool.spec["output"]["type"].is_string(), "{name} says what it answers");
                assert!(tool.spec["title"].is_string() && tool.spec["description"].is_string(), "{name} says what it is");
            }
        }
    }

    /// A tool's input schema is all its handler can get (`serve` refuses anything else), so an input it reads and the
    /// schema leaves out is one that never reaches it, the same as a misspelled one.
    #[test]
    fn a_tool_reads_only_the_inputs_its_schema_takes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut served: std::collections::HashMap<&str, (String, std::collections::VecDeque<String>)> = Default::default();
        for feature in all() {
            let folder = if matches!(feature.name, "add" | "remove") { "app" } else { feature.name };
            let (source, queue) = served.entry(folder).or_insert_with(|| {
                let source = std::fs::read_to_string(root.join(folder).join("tools.rs")).unwrap();
                let queue = source.split("serves!(serve::").skip(1).map(|s| s.split(')').next().unwrap().to_string()).collect();
                (source, queue)
            });
            for tool in &feature.tools {
                let name = feature.mcp_name(&tool.spec);
                let f = queue.pop_front().unwrap_or_else(|| panic!("{name}: a tool for each serves! in {folder}/tools.rs"));
                let (mut reads, mut seen) = (Default::default(), Default::default());
                inputs_read(source, &f, &mut reads, &mut seen);
                for input in reads {
                    assert!(crate::tools::takes(&tool.spec["input"], &input), "{name}'s handler reads {input}, which its input schema doesn't take");
                }
            }
        }
        assert!(served.values().all(|(_, queue)| queue.is_empty()), "a tool for each serves!");
    }

    /// The inputs `f` reads in `source`, and the functions of `source` it hands its input to read.
    fn inputs_read(source: &str, f: &str, reads: &mut std::collections::BTreeSet<String>, seen: &mut std::collections::BTreeSet<String>) {
        let Some(at) = source.find(&format!("fn {f}(")) else { return };
        if !seen.insert(f.to_string()) {
            return;
        }
        let body = source[at..].split("\n    }\n").next().unwrap();
        for form in ["input.get(\"", "input[\"", "text(&input, \"", "text(input, \""] {
            reads.extend(body.match_indices(form).map(|(i, _)| body[i + form.len()..].split('"').next().unwrap().to_string()));
        }
        for line in body.lines().filter(|l| l.contains("input")) {
            for (i, _) in line.match_indices('(') {
                let word = line[..i].rsplit(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap();
                if !word.is_empty() {
                    inputs_read(source, word, reads, seen);
                }
            }
        }
    }
}
