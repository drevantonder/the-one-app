//! src/screen/bridge.js's messages, answered for one screen, as every shell does (and as the MyGo spike's bridge.go):
//! what a screen asks of its app goes to the node as the person, and whatever a person touches (menus, alerts, files,
//! links) goes to the platform (`Natives`). Screens are untrusted: each acts only as its own app, and only the client's
//! own screens (`Route::own`) as the person.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use data_encoding::BASE64;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::node;

/// A screen to show: an app's (`app`, `screen`, its JSON `params`, the `scope` it's opened for) or one of the client's
/// own (`own`, by `screen`).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Route {
    #[serde(default)]
    pub app: String,
    pub screen: String,
    pub params: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub own: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct MenuItem {
    pub title: String,
    pub danger: bool,
    pub checked: bool,
    pub row: bool,
    pub section: bool,
}

/// What a platform does natively for a screen. Each may wait until the person answers.
#[allow(async_fn_in_trait)]
pub trait Natives {
    async fn menu(&self, items: Vec<MenuItem>, x: f64, y: f64) -> Option<usize>;
    async fn alert(&self, title: String, text: String, action: String, danger: bool) -> bool;
    fn copy(&self, text: &str) -> Result<(), String>;
    fn link(&self, url: &str);
    fn title(&self, text: &str, name: &str);
    async fn open(&self, to: Route, sheet: bool) -> Result<Value, String>;
    fn close(&self, value: Value);
    async fn notify(&self) -> String;
    async fn save(&self, name: String, data: Vec<u8>) -> Result<(), String>;
    fn preview(&self, name: &str, data: &[u8]) -> Result<(), String>;
    async fn sign_out(&self) -> Result<(), String>;
    fn device(&self) -> &'static str;
}

/// The messages only the client's own screens may send.
const OWN_ONLY: [&str; 7] = ["api", "signOut", "appearance", "device", "request", "cache", "developer"];

fn s<'a>(m: &'a Value, k: &str) -> &'a str {
    m[k].as_str().unwrap_or_default()
}

fn first<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.is_empty() { b } else { a }
}

/// A message's reply, or the error the screen's promise fails with.
pub async fn answer(device: &one_client::Device, natives: &impl Natives, route: &Route, kept: &std::path::Path, m: Value) -> Result<Value, String> {
    let kind = s(&m, "type");
    let own = route.own;
    if OWN_ONLY.contains(&kind) && !own {
        return Err("Only Settings can do that.".into());
    }
    match kind {
        "call" if own => Err("Settings has no app to call.".into()),
        "call" => node::call(device, &route.app, s(&m, "op"), m["input"].clone()).await,
        "kept" if own => Ok(Value::Null),
        "kept" => {
            let input = if m["input"].is_null() { json!({}) } else { m["input"].clone() };
            let input = serde_json::to_vec(&input).unwrap();
            Ok(match device.kept(&route.app, s(&m, "op"), &input) {
                Some((answer, at)) => json!({ "answer": serde_json::from_slice::<Value>(&answer).unwrap_or_default(), "at": at }),
                None => Value::Null,
            })
        }
        "open" => {
            let params = if m["params"].is_null() { "{}".to_string() } else { m["params"].to_string() };
            let mut to = Route {
                app: route.app.clone(),
                screen: first(s(&m, "screen"), "main").into(),
                params,
                scope: first(s(&m, "scope"), &route.scope).into(),
                own: false,
            };
            if !s(&m, "app").is_empty() {
                to.app = s(&m, "app").into();
            } else if own {
                to = Route { own: true, screen: s(&m, "screen").into(), params: to.params, ..Default::default() };
            }
            natives.open(to, m["sheet"].as_bool().unwrap_or(false)).await
        }
        "close" => {
            natives.close(m["value"].clone());
            Ok(Value::Null)
        }
        "title" => {
            natives.title(s(&m, "text"), s(&m, "name"));
            Ok(Value::Null)
        }
        "menu" => {
            let items: Vec<MenuItem> = serde_json::from_value(m["items"].clone()).unwrap_or_default();
            if items.is_empty() {
                return Ok(Value::Null);
            }
            Ok(natives.menu(items, m["x"].as_f64().unwrap_or(0.0), m["y"].as_f64().unwrap_or(0.0)).await.map_or(Value::Null, |i| json!(i)))
        }
        "alert" => Ok(json!(natives.alert(s(&m, "title").into(), s(&m, "text").into(), s(&m, "action").into(), m["danger"].as_bool().unwrap_or(false)).await)),
        "copy" => natives.copy(s(&m, "text")).map(|_| Value::Null),
        "link" => {
            natives.link(s(&m, "url"));
            Ok(Value::Null)
        }
        "save" | "preview" => {
            let (name, data) = file_of(device, &m["file"]).await?;
            if kind == "save" { natives.save(name, data).await } else { natives.preview(&name, &data) }.map(|_| Value::Null)
        }
        "notify" => Ok(json!(natives.notify().await)),
        "scan" => Err("Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.".into()),
        "vault" | "request" => Err("The Vault's sheet isn’t in this app yet.".into()),
        "icons" => {
            let all = &*ICONS;
            let names = m["names"].as_array().cloned().unwrap_or_default();
            Ok(Value::Object(names.iter().filter_map(|n| n.as_str()).filter_map(|n| Some((n.to_string(), json!(all.get(n)?)))).collect()))
        }
        "icon" => Ok(json!(tile(&m))),
        "haptic" => Ok(Value::Null),
        "traces" => Ok(device.traces().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)),
        "api" => node::api(device, first(s(&m, "method"), "GET"), s(&m, "path"), m["body"].clone()).await,
        "signOut" => natives.sign_out().await.map(|_| Value::Null),
        "appearance" => Ok(json!("automatic")),
        "device" => Ok(json!({ "name": natives.device(), "signOut": true })),
        "developer" => {
            if let Some(on) = m["on"].as_bool() {
                device.set_developer(on);
            }
            Ok(json!(device.developer()))
        }
        "cache" => cache(kept, &m),
        _ => Err("Unknown message".into()),
    }
}

async fn file_of(device: &one_client::Device, f: &Value) -> Result<(String, Vec<u8>), String> {
    let name = std::path::Path::new(s(f, "name")).file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    if !s(f, "base64").is_empty() {
        return BASE64.decode(s(f, "base64").as_bytes()).map(|b| (name, b)).map_err(|e| e.to_string());
    }
    if !s(f, "blob").is_empty() {
        let r = node::send(device, "GET", &format!("/v1/blobs/{}", s(f, "blob")), None, None).await?;
        return r.body.bytes().await.map(|b| (name, b)).map_err(|e| e.to_string());
    }
    Err("No file to show.".into())
}

fn cache(dir: &std::path::Path, m: &Value) -> Result<Value, String> {
    let key = s(m, "key");
    if key.is_empty() || key.len() > 40 || !key.chars().all(|c| c == '-' || c.is_ascii_alphanumeric()) {
        return Ok(Value::Null);
    }
    let path = dir.join(format!("{key}.json"));
    if m["value"].is_null() {
        return Ok(std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null));
    }
    let _ = std::fs::create_dir_all(dir);
    std::fs::write(path, m["value"].to_string()).map(|_| Value::Null).map_err(|e| e.to_string())
}

/// Phosphor's icons (vendor/phosphor/icons.json), built in.
static ICONS: LazyLock<Arc<HashMap<String, String>>> =
    LazyLock::new(|| Arc::new(serde_json::from_str(include_str!("../../vendor/phosphor/icons.json")).unwrap_or_default()));

/// An app's icon as the shells draw it: a finished icon as it is, a logo inset on white, or its icon filled (else its
/// first letter) in white on its colour.
fn tile(m: &Value) -> String {
    let (image, kind, seed) = (s(m, "image"), s(m, "kind"), s(m, "seed"));
    if !image.is_empty() && kind == "icon" {
        return image.into();
    }
    let esc = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let svg = if !image.is_empty() {
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" rx="58" fill="#fff"/><image href="{}" x="36" y="36" width="184" height="184"/></svg>"##, esc(image))
    } else {
        let palette = ["#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"];
        let color = match s(m, "color") {
            "" => palette[(seed.chars().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32)) % 8) as usize].to_string(),
            c => c.to_string(),
        };
        let face = match ICONS.get(&format!("{}-fill", s(m, "icon"))).filter(|_| !s(m, "icon").is_empty()) {
            Some(d) => format!(r##"<path transform="translate(64 64) scale(0.5)" fill="#fff" d="{}"/>"##, esc(d)),
            None => {
                let letter = seed.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or("?".into());
                format!(r##"<text x="128" y="128" dy="0.35em" text-anchor="middle" font-family="system-ui" font-weight="600" font-size="118" fill="#fff">{}</text>"##, esc(&letter))
            }
        };
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" rx="58" fill="{}"/>{face}</svg>"##, esc(&color))
    };
    format!("data:image/svg+xml;base64,{}", BASE64.encode(svg.as_bytes()))
}

/// What a page needs before bridge.js runs: `window.__oneShell`, as Android's and Windows' web views put one, posting
/// each message to the screen's own origin's `/one`, which the shell answers. Node events come by the shell evaluating
/// `one.__event(…)` in the page. No Tauri IPC: screens have no capability, so `__TAURI_INTERNALS__` reaches nothing.
pub const SHIM: &str = r#"(() => {
  window.__oneShell = { postMessage: async (message) => {
    const r = await fetch("/one", { method: "POST", body: JSON.stringify(message) });
    const m = await r.json();
    if (m.error !== undefined) throw new Error(m.error);
    return m.ok;
  } };
})();"#;
