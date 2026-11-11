//! What serves `add` and `remove` (deploy/src/tools.rs).

use anyhow::Result;
use api::Caller;
use api::feature::text;
use api::runtime::prelude::*;
use api::runtime::Serves;
use api::serves;
use scope::scopes::Scope;
use serde_json::{Map, Value, json};

pub(crate) fn add() -> Vec<(&'static str, Serves)> {
    vec![("add", serves!(add_app))]
}

pub(crate) fn remove() -> Vec<(&'static str, Serves)> {
    vec![("remove", serves!(remove_app))]
}

/// A scope's page, as a user says it.
fn page_said(rt: &Rt, scope: &Scope) -> Result<String> {
    Ok(match scope {
        Scope::User(_) => "your page".into(),
        Scope::Global => "Everyone's page".into(),
        Scope::Group(g) => format!("{}'s page", rt.read(|r| r.db().group(g))?.map_or_else(|| g.clone(), |g| g.name)),
    })
}

/// Adds `app` and the apps it uses in `scope` (user, `group:<id>` or a group's name, or global), its services on
/// `node` (a device by name).
async fn add_app(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let (app, scope) = (text(&input, "app")?, rt.read(|r| r.scope_for(&me.user, text(&input, "scope")?))?);
    crate::add(&rt, &me, app, &scope, true, input.get("node").and_then(Value::as_str)).await?;
    Ok(json!({ "said": format!("{app} is on {}, on every device.", page_said(&rt, &scope)?) }))
}

async fn remove_app(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let (app, scope) = (text(&input, "app")?, rt.read(|r| r.scope_for(&me.user, text(&input, "scope")?))?);
    crate::add(&rt, &me, app, &scope, false, None).await?;
    Ok(json!({ "said": format!("{app} is off {}. What it kept there stays: adding it back brings it back.", page_said(&rt, &scope)?) }))
}
