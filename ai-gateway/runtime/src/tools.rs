//! What serves AI Gateway's tools (ai-gateway/src/lib.rs's `tools`), as the caller: the network's own calls, from no
//! app.

use anyhow::Result;
use api::Caller;
use api::runtime::prelude::*;
use api::serves;
use serde_json::{Map, Value};

use crate::{Call, coded};

pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![("run", serves!(run)), ("models", serves!(models)), ("set", serves!(set))]
}

async fn run(rt: Rt, me: Caller, mut input: Map<String, Value>) -> Result<Value> {
    let current = api::trace::current();
    let call = Call::new(&rt, &me, None, Vec::new(), current.as_ref());
    let retry_key = input.remove("retry_key").and_then(|k| k.as_str().map(str::to_string));
    ai_gateway::run(&call, insights::own(), Value::Object(input), retry_key.as_deref()).await.map_err(coded)
}

async fn models(rt: Rt, me: Caller, _: Map<String, Value>) -> Result<Value> {
    let current = api::trace::current();
    ai_gateway::models(&Call::new(&rt, &me, None, Vec::new(), current.as_ref())).await.map_err(coded)
}

async fn set(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let current = api::trace::current();
    let admin = rt.read(|r| Ok(r.db().may(&me.user, "change-settings", scope::scopes::NETWORK)))?;
    ai_gateway::set(&Call::new(&rt, &me, None, Vec::new(), current.as_ref()), &input, admin).await.map_err(coded)
}
