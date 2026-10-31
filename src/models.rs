//! Models: the local models a device serves with Ollama, for apps and agents on any of the user's devices.

use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::apps::{AppEntry, Me};
use crate::native::Reply;

const OLLAMA: &str = "http://127.0.0.1:11434";

pub fn describe() -> Value {
    json!({
        "name": "models", "title": "Models", "icon": "cpu", "runs": "here",
        "instructions": "Local models run on the device that has them. models_here lists a server's models (give server to ask another). models_chat asks one and returns its answer; use it for private or bulk work instead of a hosted model. models_decide asks a decision model (like clef-flash) typed questions and answers with probabilities, fast.",
        "ops": [
            { "name": "here", "description": "This server's local models (or the server given).", "input": { "type": "object", "properties": {} } },
            { "name": "chat", "description": "Ask a local model and get its answer. Give prompt, or messages as [{role, content}]; system sets its instructions; format \"json\" or a JSON schema makes it answer in JSON.",
              "input": { "type": "object", "required": ["model"], "properties": {
                  "model": { "type": "string", "description": "a model's name as here lists it" },
                  "prompt": { "type": "string" },
                  "images": { "type": "array", "items": { "type": "string" }, "description": "images (JPEG, PNG or WebP) in base64, shown with the prompt" },
                  "messages": { "type": "array", "items": { "type": "object" } },
                  "system": { "type": "string" },
                  "format": { "description": "\"json\", or a JSON schema the answer must match" },
                  "think": { "type": "boolean", "description": "whether a thinking model reasons first" },
                  "options": { "type": "object", "description": "Ollama's options, like { temperature: 0, num_predict: 1024 }" },
              }}},
            { "name": "decide", "description": "Ask a decision model (a model with the decision capability, like clef-flash) questions about something and get typed answers with probabilities: choice (one of criteria, an object of option: description), noul (yes/no) or score (levels, lowest first). Answers { answers: { name: { choice, probabilities, confidence } }, usage }.",
              "input": { "type": "object", "required": ["model", "questions"], "properties": {
                  "model": { "type": "string", "description": "a decision model, like clef-flash" },
                  "state": { "type": "object", "description": "what to decide about: named fields" },
                  "questions": { "type": "object", "description": "named questions, each { type, instructions, criteria | levels }" },
                  "images": { "type": "array", "items": { "type": "object" }, "description": "images as [{ content_type, base64 }], named in state as image 1, 2..." },
              }}},
        ],
        "widgets": [],
    })
}

pub fn call(op: String, input: Value, _caller: Me, _entries: Vec<AppEntry>) -> Reply {
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
                        let mut m = json!({ "role": "user", "content": p });
                        if let Some(images) = input["images"].as_array() {
                            m["images"] = json!(images);
                        }
                        messages.push(m);
                    }
                    if messages.is_empty() {
                        bail!("give a prompt or messages");
                    }
                    if let Some(s) = input["system"].as_str() {
                        messages.insert(0, json!({ "role": "system", "content": s }));
                    }
                    let mut body = json!({ "model": model, "messages": messages, "stream": false });
                    for key in ["format", "think", "options"] {
                        if !input[key].is_null() {
                            body[key] = input[key].clone();
                        }
                    }
                    let reply = post("/api/chat", &body)?;
                    if let Some(err) = reply.get("error") {
                        bail!("{}", err.as_str().unwrap_or("the model failed"));
                    }
                    Ok(reply["message"]["content"].clone())
                }
                "decide" => {
                    let model = input["model"].as_str().context("give a decision model, like clef-flash")?;
                    ensure!(input["questions"].is_object(), "give questions");
                    let mut body = json!({ "model": model, "state": input.get("state").cloned().unwrap_or(json!({})), "questions": input["questions"] });
                    if input["images"].is_array() {
                        body["images"] = input["images"].clone();
                    }
                    let reply = post("/v1/systemone", &body)?;
                    if let Some(err) = reply.get("error") {
                        bail!("{}", err.as_str().map(str::to_string).unwrap_or_else(|| err.to_string()));
                    }
                    Ok(json!({ "answers": reply["answers"], "usage": reply["usage"] }))
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
