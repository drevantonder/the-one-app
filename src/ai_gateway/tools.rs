//! AI Gateway's tools, each its own route, served by the node (src/node/ai_gateway.rs).

use crate::core_tools::{Feature, Serves, Tool, serves};

pub fn feature() -> Feature {
    let serves: [Serves; 3] = [serves!(serve::run), serves!(serve::models), serves!(serve::set)];
    Feature {
        name: ::ai_gateway::NAME,
        title: ::ai_gateway::TITLE,
        description: ::ai_gateway::DESCRIPTION.into(),
        guide: ::ai_gateway::GUIDE.into(),
        tools: ::ai_gateway::tools().into_iter().zip(serves).map(|(spec, serves)| Tool::route(spec, serves)).collect(),
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value};

    use crate::node::{Caller, Node};

    pub async fn run(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        node.ai_tool(&me, "run", input).await
    }

    pub async fn models(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        node.ai_tool(&me, "models", input).await
    }

    pub async fn set(node: Node, me: Caller, input: Map<String, Value>) -> Result<Value> {
        node.ai_tool(&me, "set", input).await
    }
}
