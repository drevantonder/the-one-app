//! The core's tools, each their own route (`core_tools::Tool`), each at `POST /v1/<feature>/<tool>`: its
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

/// Every one, asked by whoever signed in; but one a client answers itself (`settings::answered`) only by this
/// machine's own programs, which its node answers as its own device.
pub(super) fn routes() -> Routes {
    let mut routes = Routes::default();
    for feature in crate::core_tools::all() {
        for tool in &feature.tools {
            let serves = tool.serves;
            let path = feature.path(&tool.spec);
            let who = if crate::settings::answered(&path) { Who::ThisMachine } else { Who::SignedIn };
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
}
