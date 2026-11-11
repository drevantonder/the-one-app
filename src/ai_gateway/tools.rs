//! AI Gateway's tools, each its own route, served by the node (src/node/ai_gateway.rs).

use ::api::Feature;

pub fn feature() -> Feature {
    Feature {
        name: ::ai_gateway::NAME,
        title: ::ai_gateway::TITLE,
        description: ::ai_gateway::DESCRIPTION.into(),
        guide: ::ai_gateway::GUIDE.into(),
        tools: ::ai_gateway::tools(),
    }
}

/// What serves each tool, in this runtime.
#[cfg(feature = "node")]
pub fn runs() -> ::api::Runs {
    use crate::node::runtime::node_serves;
    ::api::Runs {
        tools: vec![("run", node_serves!(serve::run)), ("models", node_serves!(serve::models)), ("set", node_serves!(serve::set))],
        ..::api::Runs::named(::ai_gateway::NAME)
    }
}

#[cfg(feature = "node")]
mod serve {
    use anyhow::Result;
    use serde_json::{Map, Value};

    use crate::node::Node;
    use ::api::Caller;

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
