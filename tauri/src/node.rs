//! The person's nodes, as screens and the home page ask them: one_client's `Device` itself, no FFI.

use one_client::Device;
use serde_json::{Value, json};

/// What `call`s say they came from, as each shell's does.
pub fn client_name() -> String {
    format!("tauri-{}", std::env::consts::OS)
}

/// A request to the person's nodes, its body JSON (or none) → its response, once it's ok. A call carries an id, so
/// sent again it runs once.
pub async fn send(d: &Device, method: &str, path: &str, body: Option<Value>, accept: Option<&str>) -> Result<one_client::Response, String> {
    let mut headers = Vec::new();
    let raw = match body {
        Some(mut b) => {
            if path.starts_with("/v1/call") && b.get("id").is_none() {
                b["id"] = json!(crate::hex(&rand::random::<[u8; 8]>()));
            }
            headers.push(("content-type".to_string(), "application/json".to_string()));
            serde_json::to_vec(&b).unwrap()
        }
        None => Vec::new(),
    };
    if let Some(a) = accept {
        headers.push(("accept".into(), a.into()));
    }
    let r = d.fetch(method, path, headers, raw).await.map_err(|e| e.to_string())?;
    if (200..300).contains(&r.status) {
        return Ok(r);
    }
    let status = r.status;
    let text = r.body.bytes().await.unwrap_or_default();
    let said: Value = serde_json::from_slice(&text).unwrap_or_default();
    Err(match said["error"].as_str() {
        Some(e) => e.into(),
        None if status >= 500 => "Your devices aren’t answering.".into(),
        None => "That didn’t work. Try again.".into(),
    })
}

/// JSON in and out: a failure the node says (`{ error }`) is returned as one.
pub async fn json(d: &Device, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let r = send(d, method, path, body, None).await?;
    let b = r.body.bytes().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Err(e.into());
    }
    Ok(v)
}

/// Runs an app's tool as this person → its result.
pub async fn call(d: &Device, app: &str, op: &str, input: Value) -> Result<Value, String> {
    let input = if input.is_null() { json!({}) } else { input };
    let out = json(d, "POST", "/v1/call", Some(json!({ "app": app, "op": op, "input": input, "client": client_name() }))).await?;
    Ok(out.get("ok").cloned().unwrap_or(Value::Null))
}

/// An app's screen, as its page.
pub async fn screen(d: &Device, app: &str, screen: &str, params: &str, scope: Option<&str>) -> Result<String, String> {
    let mut u = url::Url::parse("http://x/v1/screens/").unwrap();
    u.path_segments_mut().unwrap().pop_if_empty().push(app).push(screen);
    u.query_pairs_mut().append_pair("params", params);
    if let Some(s) = scope.filter(|s| !s.is_empty()) {
        u.query_pairs_mut().append_pair("scope", s);
    }
    let path = format!("{}?{}", u.path(), u.query().unwrap_or_default());
    let r = send(d, "GET", &path, None, None).await?;
    let b = r.body.bytes().await.map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&b).into())
}

/// An own screen's `one.api`: the node's /v1, as this person; nothing else.
pub async fn api(d: &Device, method: &str, path: &str, body: Value) -> Result<Value, String> {
    let u = url::Url::parse("http://x").unwrap().join(path).map_err(|e| e.to_string())?;
    if u.host_str() != Some("x") || !u.path().starts_with("/v1/") {
        return Err("That isn’t on your devices.".into());
    }
    let method = method.to_uppercase();
    let path = match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => u.path().into(),
    };
    let body = (method != "GET").then(|| {
        let mut b = if body.is_object() { body } else { json!({}) };
        if u.path() == "/v1/call" {
            b["client"] = json!(client_name());
        }
        b
    });
    json(d, &method, &path, body).await
}

/// The node's events as they come (server-sent events), until they end.
pub async fn events(d: &Device, mut each: impl FnMut(Value)) -> Result<(), String> {
    let mut r = send(d, "GET", "/v1/events", None, Some("text/event-stream")).await?;
    let mut rest = String::new();
    let mut data = String::new();
    loop {
        let Some(part) = r.body.chunk().await.map_err(|e| e.to_string())? else {
            return Err("the node's events ended".into());
        };
        rest.push_str(&String::from_utf8_lossy(&part));
        while let Some(i) = rest.find('\n') {
            let line: String = rest.drain(..=i).collect();
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(v) = line.strip_prefix("data:") {
                data.push_str(v.trim());
            } else if line.is_empty() && !data.is_empty() {
                if let Ok(v) = serde_json::from_str(&data) {
                    each(v);
                }
                data.clear();
            }
        }
    }
}
