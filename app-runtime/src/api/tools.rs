//! The core's tools, each their own route (`api::Feature`), each at `POST /api/<feature>/<tool>`: its
//! body is held to the tool's input schema before it runs, and its answer to its output schema before it goes.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use serde_json::{Map, Value};

use ::api::runtime::{Rt, Serves};
use ::api::{Caller, Error, Invalid, Me, Reply, Who};

use super::Routes;

/// Every one, asked by a client device; but one a client answers itself (`settings::answered`) only by this
/// machine's own programs, which its node answers as its own device. What serves each is its feature's `Runs`.
pub(super) fn routes() -> Routes {
    let runs = crate::runtime::features();
    let mut routes = Routes::default();
    for feature in one::core_tools::all() {
        for tool in &feature.tools {
            let name = feature.mcp_name(tool);
            let serves = served(&runs, feature.name, tool).unwrap_or_else(|| panic!("{name} is listed and nothing serves it (`every_core_tool_is_served_once`)"));
            let path = feature.path(tool);
            let who = if scope::settings::answered(&path) { Who::ThisMachine } else { Who::Clients };
            let spec = tool.clone();
            routes = routes.route(&path, who, post(move |rt: State<Rt>, me: Me, headers: HeaderMap, input: Json<Map<String, Value>>| serve(rt, me, headers, input, serves, spec, name)));
        }
    }
    routes
}

/// What serves `tool` of `feature`, of what each feature runs.
fn served(runs: &[::api::Runs], feature: &str, tool: &Value) -> Option<Serves> {
    runs.iter().filter(|r| r.feature == feature).flat_map(|r| &r.tools).find(|(name, _)| tool["name"] == *name).map(|(_, serves)| *serves)
}

async fn serve(State(rt): State<Rt>, me: Me, headers: HeaderMap, Json(input): Json<Map<String, Value>>, serves: Serves, spec: Value, name: String) -> Reply {
    ::api::tools::check(&spec["input"], &Value::Object(input.clone())).map_err(|e| Invalid(format!("{e:#}")))?;
    let agent = headers.get(custom_build::protocol::header::AGENT).and_then(|a| data_encoding::BASE64URL_NOPAD.decode(a.as_bytes()).ok()).and_then(|a| String::from_utf8(a).ok());
    let me = Caller { agent: agent.or(me.caller.agent), ..me.caller };
    let answer = serves(rt, me, input).await?;
    ::api::tools::check(&spec["output"], &answer).map_err(|e| Error(e.context(format!("{name} answered what its output schema doesn't say"))))?;
    Ok(Json(answer))
}

#[cfg(test)]
mod tests {
    use one::core_tools::all;

    /// What's generated from a tool (the CLI, MCP, later OpenAPI) has only its definition to go
    /// by, and a schema left out holds its body or answer to nothing.
    #[test]
    fn a_tool_that_is_a_route_says_what_it_takes_and_answers() {
        for feature in all() {
            for tool in feature.tools.iter() {
                let name = feature.mcp_name(tool);
                assert!(tool["input"]["type"] == "object", "{name} says what it takes");
                assert!(!tool["output"]["type"].is_null(), "{name} says what it answers");
                assert!(tool["title"].is_string() && tool["description"].is_string(), "{name} says what it is");
            }
        }
    }

    /// Every tool a client lists is served by exactly one feature's `Runs`, and no `Runs` serves one nobody lists: the two
    /// lists, the features every client lists and what this runtime runs, agree.
    #[test]
    fn every_core_tool_is_served_once() {
        let runs = crate::runtime::features();
        let listed: Vec<(String, String)> = all().iter().flat_map(|f| f.tools.iter().map(|t| (f.name.to_string(), t["name"].as_str().unwrap_or("").to_string())).collect::<Vec<_>>()).collect();
        for (feature, tool) in &listed {
            let serving = runs.iter().filter(|r| r.feature == feature).flat_map(|r| &r.tools).filter(|(name, _)| name == tool).count();
            assert_eq!(serving, 1, "{feature}_{tool} is served once");
        }
        for r in &runs {
            for (name, _) in &r.tools {
                assert!(listed.iter().any(|(f, t)| f == r.feature && t == name), "{}_{name} is served and no feature lists it", r.feature);
            }
        }
    }

    /// A tool's input schema is all its handler can get (`serve` refuses anything else), so an input it reads and the
    /// schema leaves out is one that never reaches it, the same as a misspelled one.
    #[test]
    fn a_tool_reads_only_the_inputs_its_schema_takes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut served: std::collections::HashMap<String, (String, std::collections::VecDeque<String>)> = Default::default();
        for feature in all() {
            let folder = if matches!(feature.name, "add" | "remove") { "deploy".to_string() } else { feature.name.replace('-', "_") };
            let (source, queue) = served.entry(folder.clone()).or_insert_with(|| {
                // A feature's runtime crate (`<feature>/runtime/src/tools.rs`), or one whose runtime half is still this program's.
                let source = runtime_tools(root, feature.name).unwrap_or_else(|| std::fs::read_to_string(root.join("src").join(&folder).join("tools.rs")).unwrap());
                let queue = source.split("serves!(").skip(1).map(|s| s.split(')').next().unwrap().rsplit("::").next().unwrap().to_string()).collect();
                (source, queue)
            });
            for tool in &feature.tools {
                let name = feature.mcp_name(tool);
                let f = queue.pop_front().unwrap_or_else(|| panic!("{name}: a tool for each serves! in {folder}/tools.rs"));
                let (mut reads, mut seen) = (Default::default(), Default::default());
                inputs_read(source, &f, &mut reads, &mut seen);
                for input in reads {
                    assert!(::api::tools::takes(&tool["input"], &input), "{name}'s handler reads {input}, which its input schema doesn't take");
                }
            }
        }
        assert!(served.values().all(|(_, queue)| queue.is_empty()), "a tool for each serves!");
    }

    /// The `runtime/src/tools.rs` of the folder whose runtime registers `feature` (`Runs::named`), whatever the folder is called.
    fn runtime_tools(root: &std::path::Path, feature: &str) -> Option<String> {
        std::fs::read_dir(root).ok()?.flatten().find_map(|folder| {
            let runs = std::fs::read_to_string(folder.path().join("runtime/src/lib.rs")).ok()?;
            (folder.file_name() == feature || runs.contains(&format!("Runs::named(\"{feature}\")"))).then(|| std::fs::read_to_string(folder.path().join("runtime/src/tools.rs")).ok())?
        })
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
