//! Models: the local models a device serves with Ollama, for apps and agents on any of the person's devices.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::apps::{Folded, Me};
use crate::native::Reply;

const OLLAMA: &str = "http://127.0.0.1:11434";

pub fn describe() -> Value {
    json!({
        "name": "models", "title": "Models", "icon": "cpu", "runs": "here",
        "instructions": "Local models run on the device that has them. models_here lists a device's models (give device to ask another). models_chat asks one and returns its answer; use it for private or bulk work instead of a hosted model.",
        "ops": [
            { "name": "here", "description": "This device's local models (or the device given).", "input": { "type": "object", "properties": {} } },
            { "name": "chat", "description": "Ask a local model and get its answer. Give prompt, or messages as [{role, content}]; system sets its instructions; format \"json\" or a JSON schema makes it answer in JSON.",
              "input": { "type": "object", "required": ["model"], "properties": {
                  "model": { "type": "string", "description": "a model's name as here lists it" },
                  "prompt": { "type": "string" },
                  "messages": { "type": "array", "items": { "type": "object" } },
                  "system": { "type": "string" },
                  "format": { "description": "\"json\", or a JSON schema the answer must match" },
              }}},
        ],
        "widgets": [],
    })
}

pub fn call(op: String, input: Value, _caller: Me, _entries: Vec<Folded>) -> Reply {
    Box::pin(async move {
        let result = tokio::task::spawn_blocking(move || -> Result<Value> {
            match op.as_str() {
                "here" => {
                    let tags: Value = get("/api/tags")?;
                    Ok(json!(tags["models"].as_array().into_iter().flatten().map(|m| json!({
                        "name": m["name"], "size": m["size"], "family": m["details"]["family"], "parameters": m["details"]["parameter_size"],
                    })).collect::<Vec<_>>()))
                }
                "chat" => {
                    let model = input["model"].as_str().context("give a model; models_here lists them")?;
                    let mut messages: Vec<Value> = input["messages"].as_array().cloned().unwrap_or_default();
                    if let Some(p) = input["prompt"].as_str() {
                        messages.push(json!({ "role": "user", "content": p }));
                    }
                    if messages.is_empty() {
                        bail!("give a prompt or messages");
                    }
                    if let Some(s) = input["system"].as_str() {
                        messages.insert(0, json!({ "role": "system", "content": s }));
                    }
                    let mut body = json!({ "model": model, "messages": messages, "stream": false });
                    if !input["format"].is_null() {
                        body["format"] = input["format"].clone();
                    }
                    let reply = post("/api/chat", &body)?;
                    if let Some(err) = reply.get("error") {
                        bail!("{}", err.as_str().unwrap_or("the model failed"));
                    }
                    Ok(reply["message"]["content"].clone())
                }
                other => bail!("models has no operation {other}"),
            }
        })
        .await??;
        Ok((result, vec![]))
    })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(600))).build().into()
}

fn get(path: &str) -> Result<Value> {
    let mut resp = agent().get(format!("{OLLAMA}{path}")).call().context("this device has no Ollama running")?;
    Ok(resp.body_mut().read_json()?)
}

fn post(path: &str, body: &Value) -> Result<Value> {
    let mut resp = agent().post(format!("{OLLAMA}{path}")).send_json(body).context("this device has no Ollama running")?;
    Ok(resp.body_mut().read_json()?)
}
