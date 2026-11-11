//! What serves Notifications' tools (their specs are notifications/src/tools.rs).

use anyhow::Result;
use serde_json::{Map, Value};

use api::runtime::prelude::*;
use api::{Caller, serves};

pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![("send", serves!(send)), ("devices", serves!(devices)), ("status", serves!(status))]
}

/// A note to the caller's devices: → how many it went to, and how many of theirs get notifications.
async fn send(rt: Rt, me: Caller, input: Map<String, Value>) -> Result<Value> {
    let send: notifications::Send = serde_json::from_value(Value::Object(input))?;
    crate::send(&rt, &me.user, &send.note("")?).await
}

async fn devices(rt: Rt, me: Caller, _: Map<String, Value>) -> Result<Value> {
    crate::devices(&rt, &me.user).await
}

async fn status(rt: Rt, _: Caller, _: Map<String, Value>) -> Result<Value> {
    crate::status(&rt).await
}
