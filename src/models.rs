//! Models: the local models a device serves with Ollama, for apps and agents on any of the user's devices.

use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::apps::Me;
use crate::sidecars::{Answer, Elsewhere, Record, Reply};

const OLLAMA: &str = "http://127.0.0.1:11434";

pub fn describe() -> Value {
    json!({
        "name": "models", "title": "Models", "icon": "cpu", "color": "#32ade6", "runs": "here", "description": "Runs AI models on your own devices",
        "instructions": "Local models run on the device that has them. models_here lists this server's models; give server \"all\" for every server's, and which are loaded. models_chat asks one and returns its answer, on whichever of your servers has it; use it for private or bulk work instead of a hosted model. models_decide asks a decision model (like clef-flash) typed questions and answers with probabilities, fast.",
        "ops": [
            { "name": "here", "reads": true, "prefer": "here", "description": "This server's local models, and whether each is loaded; give server \"all\" for every server's.", "input": { "type": "object", "properties": {} } },
            { "name": "chat", "reads": true, "description": "Ask a local model and get its answer, on whichever of your servers has it (or the server given). Give prompt, or messages as [{role, content}]; system sets its instructions; format \"json\" or a JSON schema makes it answer in JSON.",
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
            { "name": "decide", "reads": true, "description": "Ask a decision model (a model with the decision capability, like clef-flash) questions about something and get typed answers with probabilities: choice (one of criteria, an object of option: description), noul (yes/no) or score (levels, lowest first). Answers { answers: { name: { choice, probabilities, confidence } }, usage }.",
              "input": { "type": "object", "required": ["model", "questions"], "properties": {
                  "model": { "type": "string", "description": "a decision model, like clef-flash" },
                  "state": { "type": "object", "description": "what to decide about: named fields" },
                  "questions": { "type": "object", "description": "named questions, each { type, instructions, criteria | levels }" },
                  "images": { "type": "array", "description": "images in base64 (or [{ content_type, base64 }]), named in state as image 1, 2..." },
              }}},
        ],
        "widgets": [],
    })
}

pub fn call(op: String, input: Value, _caller: Me, _records: Vec<Record>) -> Reply {
    Box::pin(async move {
        let result = tokio::task::spawn_blocking(move || -> Result<Value> {
            match op.as_str() {
                "here" => {
                    let tags: Value = get("/api/tags")?;
                    let loaded: Vec<Value> = get("/api/ps").map(|ps| ps["models"].as_array().cloned().unwrap_or_default()).unwrap_or_default();
                    Ok(json!(tags["models"].as_array().into_iter().flatten().map(|m| json!({
                        "name": m["name"], "size": m["size"], "family": m["details"]["family"], "parameters": m["details"]["parameter_size"],
                        "loaded": loaded.iter().any(|l| l["name"] == m["name"]),
                    })).collect::<Vec<_>>()))
                }
                "chat" => {
                    let model = input["model"].as_str().context("give a model; models_here lists them")?;
                    here(model)?;
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
                    here(model)?;
                    ensure!(input["questions"].is_object(), "give questions");
                    let mut body = json!({ "model": model, "state": input.get("state").cloned().unwrap_or(json!({})), "questions": input["questions"] });
                    // Ollama takes each image as its base64; [{ content_type, base64 }] (Workers AI's way) works too.
                    if let Some(images) = input["images"].as_array() {
                        body["images"] = json!(images.iter().map(|i| i.get("base64").unwrap_or(i).clone()).collect::<Vec<_>>());
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
        Ok(Answer::new(result))
    })
}

/// Whether this device has `model`; if not, the call goes on to a server that might.
fn here(model: &str) -> Result<()> {
    let tags = get("/api/tags").map_err(|e| Elsewhere(format!("{e:#}")))?;
    let named = |m: &&Value| m["name"] == model || m["name"].as_str() == Some(&format!("{model}:latest"));
    match tags["models"].as_array().into_iter().flatten().find(named) {
        Some(_) => Ok(()),
        None => Err(Elsewhere(format!("it has no model {model}")).into()),
    }
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
