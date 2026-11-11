//! What serves Devices' tools (devices/src/tools.rs). Each but `versions` changes who can do what, so this runtime
//! answers it as its own device (`Holds::answer_as_device`), which signs what changes.

use anyhow::Result;
use api::Caller;
use api::runtime::prelude::*;
use api::serves;
use serde_json::{Map, Value};

/// What serves each of Devices' tools, in their specs' order.
pub(crate) fn served() -> Vec<(&'static str, api::runtime::Serves)> {
    vec![
        ("requests", serves!(requests)),
        ("approve", serves!(approve)),
        ("remove", serves!(remove)),
        ("versions", serves!(versions)),
        ("update", serves!(update)),
        ("run_schedules", serves!(run_schedules)),
        ("stop_schedules", serves!(stop_schedules)),
        ("invite", serves!(invite)),
    ]
}

/// Asked of this runtime as its own device, which signs what it changes.
async fn as_device(rt: &Rt, path: &str, input: &Map<String, Value>) -> Result<Value> {
    rt.answer_as_device("POST", path, &serde_json::to_vec(input)?).await
}

async fn requests(rt: Rt, _: Caller, input: Map<String, Value>) -> Result<Value> {
    as_device(&rt, "/api/devices/requests", &input).await
}

/// Lets in the device showing `code`, once the user agreed.
async fn approve(rt: Rt, _: Caller, input: Map<String, Value>) -> Result<Value> {
    as_device(&rt, "/api/devices/approve", &input).await
}

async fn remove(rt: Rt, _: Caller, input: Map<String, Value>) -> Result<Value> {
    as_device(&rt, "/api/devices/remove", &input).await
}

async fn versions(rt: Rt, _: Caller, _: Map<String, Value>) -> Result<Value> {
    rt.versions().await
}

async fn update(rt: Rt, _: Caller, input: Map<String, Value>) -> Result<Value> {
    as_device(&rt, "/api/devices/update", &input).await
}

async fn run_schedules(rt: Rt, _: Caller, input: Map<String, Value>) -> Result<Value> {
    as_device(&rt, "/api/devices/run_schedules", &input).await
}

async fn stop_schedules(rt: Rt, _: Caller, input: Map<String, Value>) -> Result<Value> {
    as_device(&rt, "/api/devices/stop_schedules", &input).await
}

async fn invite(rt: Rt, _: Caller, input: Map<String, Value>) -> Result<Value> {
    as_device(&rt, "/api/devices/invite", &input).await
}
