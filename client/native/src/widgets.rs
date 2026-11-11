//! Every page's widgets, as the app last had them from the user's devices, kept where the platform's own widgets
//! read them (the iPhone's home screen, the Windows Widgets board, Android's home screen, the Mac's menu bar, which lists the devices too), since
//! those can't ask a node themselves: `widgets.json`, with what each would otherwise work out itself (its `id`, `kind`,
//! `headline`; `fetched` in seconds since 1970), which they only draw; `icons.json` with the Phosphor icons they draw, and `images/<first 12 bytes of the URL's SHA-256,
//! hex>` for the pictures they show, which they can't download.

use std::path::Path;

use serde_json::{Map, Value, json};

/// The most a widget's picture may be.
const IMAGE_MOST: usize = 2 << 20;

/// Asks the user's devices for every page's widgets and keeps them in `dir`. Pages a device didn't answer for keep
/// what they had; `fetched` says when all last answered.
///
/// One at a time: each writes the same files (through the same `.part` name), and the window's events, the five-minute
/// timer and the platform's own asking while the app runs all start one.
pub async fn refresh(device: &crate::Reach, dir: &Path) -> Result<(), String> {
    static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = ONE.lock().await;
    let state: one::state::State = serde_json::from_value(crate::node(device, "GET", "/api/state", None).await?).map_err(|e| e.to_string())?;
    let apps = crate::node(device, "GET", "/api/apps", None).await?;
    let apps: Vec<Value> = apps.as_array().cloned().unwrap_or_default();
    if state.user.is_none() {
        return Ok(());
    }
    let me = state.user.as_ref().map(|u| u.id.clone());
    // The user's devices, which a Mac's menu bar lists.
    let devices: Vec<Value> = state.network.iter().flat_map(|n| &n.devices).filter(|d| d.user.is_empty() || Some(&d.user) == me.as_ref()).map(|d| json!({ "key": d.key, "name": d.name.trim_end_matches(".local"), "online": d.now.online })).collect();
    let pages: Vec<(String, String)> = state.pages.into_iter().map(|p| (p.scope, p.title)).collect();

    let kept: Value = std::fs::read(dir.join("widgets.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut answered = true;
    let mut items = Vec::new();
    for (scope, title) in &pages {
        // An app's widgets show on a page it's added to.
        let here: Vec<&Value> = apps.iter().filter(|a| a["added"].as_array().is_some_and(|s| s.iter().any(|s| in_scope(s.as_str().unwrap_or(""), scope)))).collect();
        let query = |extra: &str| format!("/api/widgets?scope={}{extra}", url::form_urlencoded::byte_serialize(scope.as_bytes()).collect::<String>());
        let widgets = match crate::node(device, "GET", &query(""), None).await {
            Ok(Value::Array(w)) => w,
            _ => {
                answered = false;
                // What it had for this page.
                items.extend(kept["widgets"].as_array().into_iter().flatten().filter(|i| i["scope"] == scope.as_str()).cloned());
                continue;
            }
        };
        for mut w in widgets {
            let name = w["app"].as_str().unwrap_or_default().to_string();
            let name = name.as_str();
            // Too slow to come with the rest: asked for alone, and if that fails too, as it was last kept.
            if w["late"] == true {
                let kind = format!("{name}.{}", w["widget"].as_str().unwrap_or_default());
                match crate::node(device, "GET", &query(&format!("&kind={}", url::form_urlencoded::byte_serialize(kind.as_bytes()).collect::<String>())), None).await {
                    Ok(Value::Array(alone)) if !alone.is_empty() => w = alone.into_iter().next().unwrap_or_default(),
                    _ => {
                        answered = false;
                        items.extend(kept["widgets"].as_array().into_iter().flatten().find(|i| i["scope"] == scope.as_str() && i["kind"] == kind.as_str()).cloned());
                        continue;
                    }
                }
            }
            let Some(app) = here.iter().find(|a| a["name"] == name) else { continue };
            let named = w["title"].as_str().or(app["title"].as_str()).or(w["widget"].as_str()).unwrap_or_default();
            let widget = w["widget"].as_str().unwrap_or_default();
            let kind = format!("{name}.{widget}");
            let mut item = json!({
                // What a home-screen widget's choice is found again by, and a menu bar item's kind.
                "id": format!("{scope}/{kind}"), "kind": kind,
                "app": name, "widget": widget, "scope": scope, "page": title, "appTitle": app["title"],
                "title": named, "icon": app["icon"].as_str().unwrap_or_default(), "root": w["root"],
            });
            if w["error"].is_null() {
                item["headline"] = json!(headline(&w["root"]));
            }
            for (from, to) in [("image", "image"), ("image_kind", "imageKind"), ("color", "color")] {
                if let Some(v) = app[from].as_str() {
                    item[to] = json!(v);
                }
            }
            // Whether its app has a screen to open (a `main`), which Android's widget opens on a tap.
            item["opens"] = json!(app["screens"].as_array().is_some_and(|s| s.iter().any(|s| s["name"] == "main")));
            for field in ["error", "sizes"] {
                if !w[field].is_null() {
                    item[field] = w[field].clone();
                }
            }
            items.push(item);
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or_default();
    let fetched = if answered { json!(now) } else { kept["fetched"].clone() };
    let mut cache = json!({ "network": state.network.as_ref().map(|n| &n.name), "devices": devices, "widgets": items, "fetched": fetched });
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

/// Signed out: none of the user's widgets or pictures are left, and the platform's widgets say to sign in.
pub fn forget(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir.join("images"));
    let _ = write(&dir.join("widgets.json"), br#"{"widgets":[],"signedOut":true}"#);
}

/// A tree's most prominent text (the biggest, then the first that isn't muted): what a one-line place shows (an iPhone's
/// Lock Screen widget, a Mac's menu bar item).
fn headline(tree: &Value) -> Option<String> {
    fn texts(n: &Value, out: &mut Vec<(String, i32, bool)>) {
        match n["t"].as_str() {
            Some("text") => {
                let size = match n["size"].as_str() {
                    Some("sm") => 0,
                    Some("lg") => 2,
                    Some("xl") => 3,
                    _ => 1,
                };
                out.push((n["text"].as_str().unwrap_or_default().to_string(), size, n["tone"] == "muted"));
            }
            Some("stat") => {
                let value = match &n["value"] {
                    Value::String(s) => s.clone(),
                    Value::Number(v) => v.as_f64().map(|f| if f.fract() == 0.0 { format!("{f:.0}") } else { format!("{}", (f * 100.0).round() / 100.0) }).unwrap_or_default(),
                    _ => String::new(),
                };
                let mut text = [value, n["unit"].as_str().unwrap_or_default().to_string()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                if let Some(label) = n["label"].as_str() {
                    text = format!("{text} {label}");
                }
                out.push((text, 4, false));
            }
            Some("item") => out.push((n["title"].as_str().unwrap_or_default().to_string(), 1, false)),
            _ => n["c"].as_array().into_iter().flatten().for_each(|c| texts(c, out)),
        }
    }
    let mut all = Vec::new();
    texts(tree, &mut all);
    let all: Vec<_> = all.into_iter().filter(|t| !t.0.is_empty()).collect();
    // The biggest; of those, one that isn't muted; of those, the first.
    let best = all.iter().enumerate().max_by_key(|(i, t)| (t.1, !t.2, -(*i as i64)))?;
    Some(best.1.0.clone())
}

/// Whether an app added at `added` (`user:<id>`, `group:<id>`, `global`) is on the page `scope`.
fn in_scope(added: &str, scope: &str) -> bool {
    added == scope || (scope == "user" && added.starts_with("user:"))
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
    // And the two a widget draws itself: its problem box's, and a chosen row's.
    let mut wanted: Vec<&str> = vec!["warning-fill", "check"];
    wanted.extend(items.iter().filter_map(|i| i["icon"].as_str()));
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
