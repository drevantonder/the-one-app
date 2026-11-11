//! Models' service: the models its device serves with Ollama, for AI Gateway (`ctx.ai.run` with `local/<model>`), which
//! asks it in its own shapes: `chat` takes OpenAI's chat completions input and answers its ChatCompletion (Ollama's
//! OpenAI-compatible endpoint), `decide` takes Cloudflare Clef's and answers `{ answers, usage }`.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use one_service::{Answer, Call, Elsewhere, Host, Service};
use serde_json::{Value, json};

const OLLAMA: &str = "http://127.0.0.1:11434";

fn main() -> Result<()> {
    one_service::serve(Service { name: "models", prefer_here: &["here"], call, start: None })
}

fn call(_: &Host, call: &Call) -> Result<Answer> {
    let input = &call.input;
    let result: Result<Value> = match call.function.as_str() {
        "here" => {
            let tags: Value = get("/api/tags")?;
            let loaded: Vec<Value> = get("/api/ps").map(|ps| ps["models"].as_array().cloned().unwrap_or_default()).unwrap_or_default();
            Ok(json!(tags["models"].as_array().into_iter().flatten().map(|m| {
                // A decision model says so in its capabilities; every other answers chat.
                let shown = post("/api/show", &json!({ "model": m["name"] })).unwrap_or_default();
                let decides = shown["capabilities"].as_array().is_some_and(|c| c.iter().any(|c| c == "decision"));
                json!({
                    "name": m["name"], "kind": if decides { "decide" } else { "chat" }, "size": m["size"], "family": m["details"]["family"],
                    "parameters": m["details"]["parameter_size"], "loaded": loaded.iter().any(|l| l["name"] == m["name"]),
                })
            }).collect::<Vec<_>>()))
        }
        "chat" => {
            let model = input["model"].as_str().context("give a model")?;
            here(model)?;
            if !input["messages"].is_array() {
                one_service::refuse!("give messages");
            }
            let mut body = input.clone();
            body["stream"] = json!(false);
            let reply = post("/v1/chat/completions", &body)?;
            if let Some(err) = reply.get("error") {
                bail!("{}", err["message"].as_str().or(err.as_str()).unwrap_or("the model failed"));
            }
            Ok(reply)
        }
        "decide" => {
            let model = input["model"].as_str().context("give a decision model")?;
            here(model)?;
            if !input["questions"].is_object() {
                one_service::refuse!("give questions");
            }
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
        other => bail!("models has no function {other}"),
    };
    Ok(Answer::new(result?))
}

/// Whether this device has `model`; if not, the call goes on to a device that might.
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
