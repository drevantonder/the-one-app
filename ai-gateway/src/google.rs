//! Gemini's generateContent, from and to OpenAI's chat completions. Not its OpenAI-compatible endpoint, which drops
//! what apps use: PDFs, the flex tier and thinking level.

use serde_json::{Map, Value, json};

use crate::{Failed, data_url, parts, text_of};

pub fn request(input: &Map<String, Value>) -> Result<Value, Failed> {
    let mut system = Vec::new();
    let mut contents = Vec::new();
    for m in input.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let role = match m["role"].as_str() {
            Some("system" | "developer") => {
                system.push(json!({ "text": text_of(&m["content"]) }));
                continue;
            }
            Some("assistant") => "model",
            Some("user") => "user",
            other => return Err(Failed::Refused(format!("a message's role is system, user or assistant, not {}", other.unwrap_or("missing")))),
        };
        let parts = parts(&m["content"]).iter().map(part).collect::<Result<Vec<_>, _>>()?;
        contents.push(json!({ "role": role, "parts": parts }));
    }
    let mut config = Map::new();
    let mut set = |key: &str, value: Option<Value>| {
        if let Some(v) = value {
            config.insert(key.into(), v);
        }
    };
    set("maxOutputTokens", input.get("max_completion_tokens").cloned());
    set("temperature", input.get("temperature").cloned());
    set("topP", input.get("top_p").cloned());
    set("seed", input.get("seed").cloned());
    set("stopSequences", input.get("stop").map(|s| if s.is_string() { json!([s]) } else { s.clone() }));
    set("thinkingConfig", input.get("reasoning_effort").and_then(Value::as_str).map(|e| json!({ "thinkingLevel": if e == "minimal" { "low" } else { e } })));
    match input.get("response_format") {
        Some(f) if f["type"] == "json_schema" => {
            set("responseMimeType", Some(json!("application/json")));
            set("responseJsonSchema", Some(f["json_schema"]["schema"].clone()));
        }
        Some(f) if f["type"] == "json_object" => set("responseMimeType", Some(json!("application/json"))),
        _ => {}
    }
    let mut body = json!({ "contents": contents, "generationConfig": config });
    if !system.is_empty() {
        body["systemInstruction"] = json!({ "parts": system });
    }
    if let Some(tier) = input.get("service_tier") {
        body["service_tier"] = tier.clone();
    }
    Ok(body)
}

/// One part of a message: text, an image (a `data:` or `https:` URL) or a PDF (`data:`).
fn part(p: &Value) -> Result<Value, Failed> {
    match p["type"].as_str() {
        Some("text") => Ok(json!({ "text": p["text"] })),
        Some("image_url") => {
            let url = p["image_url"]["url"].as_str().unwrap_or("");
            match data_url(url) {
                Some((mime, data)) => Ok(json!({ "inline_data": { "mime_type": mime, "data": data } })),
                None if url.starts_with("https://") => Ok(json!({ "file_data": { "file_uri": url, "mime_type": image_mime(url) } })),
                None => Err(Failed::Refused("an image_url is a data: or https: URL".into())),
            }
        }
        Some("file") => match data_url(p["file"]["file_data"].as_str().unwrap_or("")) {
            Some((mime, data)) => Ok(json!({ "inline_data": { "mime_type": mime, "data": data } })),
            None => Err(Failed::Refused("a file's file_data is a data: URL, like data:application/pdf;base64,…".into())),
        },
        other => Err(Failed::Refused(format!("a message part is text, image_url or file, not {}", other.unwrap_or("missing")))),
    }
}

/// Gemini needs an image's type with its URL: by its name's ending.
fn image_mime(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or("").to_lowercase();
    [(".png", "image/png"), (".webp", "image/webp"), (".gif", "image/gif"), (".heic", "image/heic")].into_iter().find(|(end, _)| path.ends_with(end)).map_or("image/jpeg", |(_, m)| m)
}

pub fn answer(model: &str, got: Value) -> Value {
    let candidate = &got["candidates"][0];
    let text: String = candidate["content"]["parts"].as_array().into_iter().flatten().filter(|p| p["thought"] != true).filter_map(|p| p["text"].as_str()).collect();
    let finish = match candidate["finishReason"].as_str() {
        Some("MAX_TOKENS") => "length",
        Some("STOP") | None if candidate.is_object() => "stop",
        _ => "content_filter",
    };
    let mut message = json!({ "role": "assistant", "content": if candidate.is_object() { json!(text) } else { Value::Null } });
    if let Some(blocked) = got["promptFeedback"]["blockReason"].as_str() {
        message["refusal"] = json!(format!("Google blocked it: {blocked}"));
    }
    let u = &got["usageMetadata"];
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let (prompt, out, thoughts) = (n(&u["promptTokenCount"]), n(&u["candidatesTokenCount"]), n(&u["thoughtsTokenCount"]));
    json!({
        "object": "chat.completion",
        "model": got["modelVersion"].as_str().unwrap_or(model),
        "choices": [{ "index": 0, "message": message, "finish_reason": finish }],
        "usage": {
            "prompt_tokens": prompt, "completion_tokens": out + thoughts, "total_tokens": prompt + out + thoughts,
            "prompt_tokens_details": { "cached_tokens": n(&u["cachedContentTokenCount"]) },
            "completion_tokens_details": { "reasoning_tokens": thoughts },
        },
    })
}
