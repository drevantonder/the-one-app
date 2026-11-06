//! An MCP session with a Linux device's server, over streamable HTTP. The browsers its tools launch belong to the
//! session that launched them, so a device keeps one for as long as this runs. A connection's calls share a session
//! label of its own unless they give theirs: the browser tools refuse to act without one, and a label stays bound to
//! the connection that made it, so a new connection (as after this program restarted) can't take an old one over.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::linux::Device;
use super::{Devices, Input, NAME};

/// A device's tool, as its server lists it, kept in tools.json.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
}

#[derive(Debug, Default, Deserialize)]
pub struct Reply {
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(rename = "structuredContent", default)]
    pub structured: Value,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

impl Reply {
    fn said(&self, code: &str) -> bool {
        (!self.structured.is_null() && self.structured.to_string().contains(code)) || self.content.iter().any(|c| c["text"].as_str().is_some_and(|t| t.contains(code)))
    }

    /// Its structured content goes as text too, since that often holds what its text leaves out.
    pub fn content(self) -> Result<Value> {
        let mut items = self.content;
        if !self.structured.is_null() {
            items.push(json!({ "type": "text", "text": self.structured.to_string() }));
        }
        if self.is_error {
            let texts: Vec<&str> = items.iter().filter_map(|c| c["text"].as_str()).collect();
            bail!("{}", texts.join("\n"));
        }
        Ok(json!({ "content": items }))
    }
}

/// A session the server no longer knows, as after it restarted.
#[derive(Debug)]
struct Gone;

impl std::fmt::Display for Gone {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("the device's session ended")
    }
}

impl std::error::Error for Gone {}

pub struct Conn {
    url: String,
    token: String,
    id: String,
    pub label: String,
    next: AtomicI64,
    agent: ureq::Agent,
}

impl Conn {
    pub fn dial(d: &Device) -> Result<Conn> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(590))).build().into();
        let mut c = Conn { url: format!("{}/mcp", d.local()), token: d.token.clone(), id: String::new(), label: format!("agents-{}", super::random_hex(3)), next: AtomicI64::new(0), agent };
        let init = json!({ "jsonrpc": "2.0", "id": c.next.fetch_add(1, Ordering::SeqCst) + 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": NAME, "version": "1" } } });
        let (body, id) = c.send(&init)?;
        c.id = id;
        Self::result("initialize", &body)?;
        c.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok(c)
    }

    pub fn tools(&self) -> Result<Vec<Tool>> {
        let r = self.rpc("tools/list", None)?;
        Ok(serde_json::from_value(r["tools"].clone())?)
    }

    pub fn call(&self, name: &str, args: &Input) -> Result<Reply> {
        let r = self.rpc("tools/call", Some(json!({ "name": name, "arguments": args })))?;
        Ok(serde_json::from_value(r)?)
    }

    fn rpc(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let mut msg = json!({ "jsonrpc": "2.0", "id": self.next.fetch_add(1, Ordering::SeqCst) + 1, "method": method });
        if let Some(p) = params {
            msg["params"] = p;
        }
        let (body, _) = self.send(&msg)?;
        Self::result(method, &body)
    }

    fn result(method: &str, body: &[u8]) -> Result<Value> {
        let reply: Value = serde_json::from_slice(body).map_err(|err| anyhow!("{method}: {err}"))?;
        if let Some(m) = reply.get("error").filter(|e| !e.is_null()) {
            bail!("{method}: {}", m["message"].as_str().unwrap_or_default());
        }
        Ok(reply["result"].clone())
    }

    fn send(&self, msg: &Value) -> Result<(Vec<u8>, String)> {
        let mut req = self.agent.post(&self.url).header("authorization", format!("Bearer {}", self.token)).header("content-type", "application/json").header("accept", "application/json, text/event-stream");
        if !self.id.is_empty() {
            req = req.header("mcp-session-id", &self.id);
        }
        let mut resp = req.send(msg.to_string())?;
        let id = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let stream = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("text/event-stream"));
        let status = resp.status();
        let body = resp.body_mut().with_config().limit(256 << 20).read_to_vec()?;
        if status == 404 && !self.id.is_empty() {
            return Err(Gone.into());
        }
        if status.as_u16() >= 300 {
            bail!("the device's server: {status}: {}", String::from_utf8_lossy(&body).trim());
        }
        if msg.get("id").is_none() {
            return Ok((Vec::new(), id));
        }
        if stream {
            return Ok((event(&body)?, id));
        }
        Ok((body, id))
    }
}

/// The reply in a stream of server-sent events: the first message with an id, skipping notifications.
fn event(stream: &[u8]) -> Result<Vec<u8>> {
    let text = String::from_utf8_lossy(stream).replace("\r\n", "\n");
    for ev in text.split("\n\n") {
        let data: Vec<&str> = ev.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| d.strip_prefix(' ').unwrap_or(d)).collect();
        let msg = data.join("\n");
        if serde_json::from_str::<Value>(&msg).is_ok_and(|m| m.get("id").is_some_and(|id| !id.is_null())) {
            return Ok(msg.into_bytes());
        }
    }
    bail!("the device's server sent no reply")
}

impl Devices {
    /// The device's session, made the first time it's needed. A browser an earlier session launched can't be used
    /// or closed by this one, and holds its profile, so those are closed first.
    pub(super) fn conn(&self, d: &Device) -> Result<std::sync::Arc<Conn>> {
        let mut conns = self.conns.lock().unwrap();
        if let Some(c) = conns.get(&d.name) {
            return Ok(c.clone());
        }
        self.close_browsers(d, false);
        let c = std::sync::Arc::new(Conn::dial(d)?);
        conns.insert(d.name.clone(), c.clone());
        Ok(c)
    }

    pub(super) fn drop_conn(&self, name: &str) {
        self.conns.lock().unwrap().remove(name);
    }

    /// Runs one of a device's tools, dialing again once if the connection is gone, as after the device restarted.
    pub(super) fn tool_call(&self, d: &Device, name: &str, mut args: Input) -> Result<Reply> {
        let ours = !args.contains_key("session") && self.takes_session(name);
        let mut last = anyhow!("no answer");
        for attempt in 0..2 {
            if attempt > 0 {
                self.drop_conn(&d.name);
            }
            let c = match self.conn(d) {
                Ok(c) => c,
                Err(err) => {
                    last = err;
                    continue;
                }
            };
            if ours {
                args.insert("session".into(), json!(c.label));
            }
            let mut r = match c.call(name, &args) {
                Ok(r) => r,
                Err(err) => {
                    last = err;
                    continue;
                }
            };
            if ours && r.said("session_ended") {
                // It ended, idle; only start_session revives a name.
                c.call("start_session", &Map::from_iter([("session".to_string(), json!(c.label))]))?;
                r = c.call(name, &args)?;
            }
            if ours && r.said("session_unavailable") && attempt == 0 {
                last = anyhow!("session_unavailable");
                continue; // a label of another connection's: dial again, with a new one
            }
            if name == "browser_prepare" && r.said("browser_endpoint_owner_mismatch") {
                // The profile is open in a browser no session of ours owns.
                self.close_browsers(d, false);
                r = c.call(name, &args)?;
            }
            return Ok(r);
        }
        Err(last)
    }

    fn takes_session(&self, name: &str) -> bool {
        self.tools.lock().unwrap().iter().find(|t| t.name == name).is_some_and(|t| t.input_schema["properties"].get("session").is_some())
    }
}
