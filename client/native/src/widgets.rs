//! Every page's widgets, as the app last had them from the person's devices, kept where the platform's own widgets
//! read them (the iPhone's home screen, the Windows Widgets board, Android's home screen, the Mac's menu bar), since
//! those can't ask a node themselves: `widgets.json` in the iPhone widget's format (apple/Widget/Cache.swift's
//! `WidgetCache`), `icons.json` with the Phosphor icons they draw, and `images/<first 12 bytes of the URL's SHA-256,
//! hex>` for the pictures they show, which they can't download.

use std::path::Path;

use serde_json::{Map, Value, json};

/// Seconds from 1970 to 2001, where a Swift `Date` counts from (JSONEncoder's default).
const SWIFT_EPOCH: f64 = 978_307_200.0;
/// The most a widget's picture may be.
const IMAGE_MOST: usize = 2 << 20;

/// Asks the person's devices for every page's widgets and keeps them in `dir`. Pages a device didn't answer for keep
/// what they had; `fetched` says when all last answered.
///
/// One at a time: each writes the same files (through the same `.part` name), and the window's events, the five-minute
/// timer and the platform's own asking while the app runs all start one.
pub async fn refresh(device: &one_client::Device, dir: &Path) -> Result<(), String> {
    static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = ONE.lock().await;
    let state = crate::node_json(device, "/v1/state").await?;
    let apps = crate::node_json(device, "/v1/apps").await?;
    let apps: Vec<Value> = apps.as_array().cloned().unwrap_or_default();
    #[cfg(target_os = "macos")]
    crate::menubar::named(&apps);
    let Some(me) = state["user"]["id"].as_str() else { return Ok(()) };
    let mut pages = vec![("user".to_string(), "You".to_string())];
    let mut groups: Vec<(String, String)> = state["network"]["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|g| g["members"].as_array().is_some_and(|m| m.iter().any(|m| m["user"] == me)))
        .map(|g| (format!("group:{}", g["id"].as_str().unwrap_or_default()), g["name"].as_str().unwrap_or_default().to_string()))
        .collect();
    groups.sort_by(|a, b| a.1.cmp(&b.1));
    pages.extend(groups);
    pages.push(("global".into(), "Everyone".into()));

    let kept: Value = std::fs::read(dir.join("widgets.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut answered = true;
    let mut items = Vec::new();
    for (scope, title) in &pages {
        // An app's widgets show on a page it's installed on.
        let here: Vec<&Value> = apps.iter().filter(|a| a["installed"].as_array().is_some_and(|s| s.iter().any(|s| in_scope(s.as_str().unwrap_or(""), scope)))).collect();
        let widgets = match crate::node_json(device, &format!("/v1/widgets?scope={}", url::form_urlencoded::byte_serialize(scope.as_bytes()).collect::<String>())).await {
            Ok(Value::Array(w)) => w,
            _ => {
                answered = false;
                // What it had for this page.
                items.extend(kept["widgets"].as_array().into_iter().flatten().filter(|i| i["scope"] == scope.as_str()).cloned());
                continue;
            }
        };
        for w in widgets {
            let name = w["app"].as_str().unwrap_or_default();
            let Some(app) = here.iter().find(|a| a["name"] == name) else { continue };
            let named = w["title"].as_str().or(app["title"].as_str()).or(w["widget"].as_str()).unwrap_or_default();
            let mut item = json!({
                "app": name, "widget": w["widget"], "scope": scope, "page": title,
                "title": named, "icon": app["icon"].as_str().unwrap_or_default(), "root": w["root"],
            });
            for (from, to) in [("image", "image"), ("image_kind", "imageKind")] {
                if let Some(v) = app[from].as_str() {
                    item[to] = json!(v);
                }
            }
            for field in ["error", "sizes"] {
                if !w[field].is_null() {
                    item[field] = w[field].clone();
                }
            }
            items.push(item);
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or_default();
    let fetched = if answered { json!(now - SWIFT_EPOCH) } else { kept["fetched"].clone() };
    let mut cache = json!({ "network": state["network"]["name"], "widgets": items, "fetched": fetched });
    if cache["fetched"].is_null() {
        cache.as_object_mut().unwrap().remove("fetched");
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write(&dir.join("widgets.json"), &serde_json::to_vec(&cache).unwrap())?;
    icons(dir, &items)?;
    let dir = dir.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || images(&dir, &items)).await;
    Ok(())
}

/// Signed out: none of the person's widgets or pictures are left, and the platform's widgets say to sign in.
pub fn forget(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir.join("images"));
    let _ = write(&dir.join("widgets.json"), br#"{"widgets":[],"signedOut":true}"#);
}

/// Whether an app installed at `installed` (`user:<id>`, `group:<id>`, `global`) is on the page `scope`.
fn in_scope(installed: &str, scope: &str) -> bool {
    installed == scope || (scope == "user" && installed.starts_with("user:"))
}

/// Written whole or not at all, as a widget may read it at any moment.
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let part = path.with_extension("part");
    std::fs::write(&part, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&part, path).map_err(|e| e.to_string())
}

/// Every node of a widget's tree.
fn nodes<'a>(tree: &'a Value, out: &mut Vec<&'a Value>) {
    out.push(tree);
    for child in tree["c"].as_array().into_iter().flatten() {
        nodes(child, out);
    }
}

fn trees(items: &[Value]) -> Vec<&Value> {
    let mut out = Vec::new();
    for item in items {
        nodes(&item["root"], &mut out);
        for tree in item["sizes"].as_object().into_iter().flat_map(|s| s.values()) {
            nodes(tree, &mut out);
        }
    }
    out
}

/// The Phosphor icons the widgets may draw (each app's, and each named in a tree, and their filled forms).
fn icons(dir: &Path, items: &[Value]) -> Result<(), String> {
    let all: Map<String, Value> = serde_json::from_slice(one::screen::ICONS).unwrap_or_default();
    let mut wanted: Vec<&str> = items.iter().filter_map(|i| i["icon"].as_str()).collect();
    wanted.extend(trees(items).iter().flat_map(|n| [n["icon"].as_str(), n["name"].as_str()]).flatten());
    let mut out = Map::new();
    for name in wanted {
        for name in [name.to_string(), format!("{name}-fill")] {
            if let Some(path) = all.get(&name) {
                out.insert(name, path.clone());
            }
        }
    }
    write(&dir.join("icons.json"), &serde_json::to_vec(&out).unwrap())
}

/// The pictures the widgets show (https only, at most 2 MB each), named by their URL.
fn images(dir: &Path, items: &[Value]) {
    use sha2::{Digest, Sha256};
    let images = dir.join("images");
    let _ = std::fs::create_dir_all(&images);
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(15))).build().into();
    for node in trees(items).into_iter().filter(|n| n["t"] == "image") {
        let Some(url) = node["url"].as_str().filter(|u| u.starts_with("https://")) else { continue };
        let file = images.join(data_encoding::HEXLOWER.encode(&Sha256::digest(url.as_bytes())[..12]));
        if file.exists() {
            continue;
        }
        if let Ok(mut r) = agent.get(url).call()
            && let Ok(bytes) = r.body_mut().with_config().limit(IMAGE_MOST as u64).read_to_vec()
        {
            let _ = write(&file, &bytes);
        }
    }
}
