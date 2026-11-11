//! Anthropic's Messages, from and to OpenAI's chat completions. Not its OpenAI-compatible endpoint, which drops typed
//! answers (`output_config.format`).

use serde_json::{Map, Value, json};

use crate::{Failed, data_url, parts, text_of};

/// The most it may write when the caller didn't say: Messages needs one, and this answers in one request's time.
const MAX_TOKENS: u64 = 16_000;

pub fn request(model: &str, input: &Map<String, Value>) -> Result<Value, Failed> {
    if input.contains_key("seed") || input.get("service_tier").is_some_and(|t| t == "flex") {
        return Err(Failed::Refused("Anthropic's models take no seed and have no flex tier".into()));
    }
    let mut system = Vec::new();
    let mut messages = Vec::new();
    for m in input.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let role = match m["role"].as_str() {
            Some("system" | "developer") => {
                system.push(text_of(&m["content"]));
                continue;
            }
            Some(role @ ("user" | "assistant")) => role,
            other => return Err(Failed::Refused(format!("a message's role is system, user or assistant, not {}", other.unwrap_or("missing")))),
        };
        let content = parts(&m["content"]).iter().map(block).collect::<Result<Vec<_>, _>>()?;
        messages.push(json!({ "role": role, "content": content }));
    }
    let mut body = json!({ "model": model, "max_tokens": input.get("max_completion_tokens").and_then(Value::as_u64).unwrap_or(MAX_TOKENS), "messages": messages });
    if !system.is_empty() {
        body["system"] = json!(system.join("\n\n"));
    }
    for (from, to) in [("temperature", "temperature"), ("top_p", "top_p")] {
        if let Some(v) = input.get(from) {
            body[to] = v.clone();
        }
    }
    if let Some(stop) = input.get("stop") {
        body["stop_sequences"] = if stop.is_string() { json!([stop]) } else { stop.clone() };
    }
    let mut output = Map::new();
    if let Some(effort) = input.get("reasoning_effort").and_then(Value::as_str) {
        output.insert("effort".into(), json!(if effort == "minimal" { "low" } else { effort }));
    }
    match input.get("response_format") {
        Some(f) if f["type"] == "json_schema" => {
            output.insert("format".into(), json!({ "type": "json_schema", "schema": f["json_schema"]["schema"] }));
        }
        Some(f) if f["type"] == "json_object" => return Err(Failed::Refused("Anthropic's models take a json_schema response_format, not json_object: give the schema".into())),
        _ => {}
    }
    if !output.is_empty() {
        body["output_config"] = Value::Object(output);
    }
    Ok(body)
}

/// One content block: text, an image (a `data:` or `https:` URL) or a PDF (`data:`).
fn block(p: &Value) -> Result<Value, Failed> {
    match p["type"].as_str() {
        Some("text") => Ok(json!({ "type": "text", "text": p["text"] })),
        Some("image_url") => {
            let url = p["image_url"]["url"].as_str().unwrap_or("");
            match data_url(url) {
                Some((mime, data)) => Ok(json!({ "type": "image", "source": { "type": "base64", "media_type": mime, "data": data } })),
                None if url.starts_with("https://") => Ok(json!({ "type": "image", "source": { "type": "url", "url": url } })),
                None => Err(Failed::Refused("an image_url is a data: or https: URL".into())),
            }
        }
        Some("file") => match data_url(p["file"]["file_data"].as_str().unwrap_or("")) {
            Some((mime, data)) => Ok(json!({ "type": "document", "source": { "type": "base64", "media_type": mime, "data": data } })),
            None => Err(Failed::Refused("a file's file_data is a data: URL, like data:application/pdf;base64,…".into())),
        },
        other => Err(Failed::Refused(format!("a message part is text, image_url or file, not {}", other.unwrap_or("missing")))),
    }
}

pub fn answer(got: Value) -> Value {
    let text: String = got["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
    let refused = got["stop_reason"] == "refusal";
    let finish = match got["stop_reason"].as_str() {
        Some("max_tokens") => "length",
        Some("refusal") => "content_filter",
        _ => "stop",
    };
    let mut message = json!({ "role": "assistant", "content": text });
    if refused {
        message["refusal"] = json!(got["stop_details"]["explanation"].as_str().unwrap_or("Anthropic's model declined"));
    }
    let u = &got["usage"];
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let read = n(&u["cache_read_input_tokens"]);
    let prompt = n(&u["input_tokens"]) + read + n(&u["cache_creation_input_tokens"]);
    let out = n(&u["output_tokens"]);
    json!({
        "object": "chat.completion",
        "model": got["model"],
        "choices": [{ "index": 0, "message": message, "finish_reason": finish }],
        "usage": { "prompt_tokens": prompt, "completion_tokens": out, "total_tokens": prompt + out, "prompt_tokens_details": { "cached_tokens": read } },
    })
}
