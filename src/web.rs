//! The web client's files (cloudflare/web, @one/ui and the client's own screens), as this build has them: what the
//! gateway serves browsers (`cloudflare::Settings::web`, which adds what's the network's own) and what the native app
//! shows its window from (client/native), so both draw the same window from the same files.

use serde_json::json;

#[cfg(feature = "web-bundle")]
pub mod bundle;

/// What the shell (client/native) gives the page: its commands (client/native/src/commands.rs) and `window.__oneNative`
/// (client/native/src/native.js). A web bundle says the shell it needs by this: one more whenever those change, and
/// tests/contracts.rs pins each number to them, so a bundle never reaches a shell that can't run it.
pub const SHELL: u32 = 4;

/// Every file of the web client, by its path, its page (`index.html`) naming the rest by their content.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> =
        crate::ui_folder::cloudflare().into_iter().filter_map(|(path, bytes)| Some((path.strip_prefix("web/")?.to_string(), bytes))).collect();
    // What the client says that Rust says too, imported by the formats it holds keys in (`protocol`).
    files.push(("shared/protocol.js".into(), crate::protocol::javascript().into_bytes()));
    // The page says whose it is before anyone signs in: the custom build's name.
    let attr = |v: &str| v.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    files.push(("icons.json".into(), crate::screen::ICONS.to_vec()));
    // Installed from Chrome or Edge, it's an app of its own: the desktop's, where there's no native one.
    let manifest = json!({
        "name": crate::config::NAME, "short_name": crate::config::NAME, "id": "/", "start_url": "/", "scope": "/", "display": "standalone",
        "background_color": "#ffffff", "theme_color": "#ffffff",
        "icons": [{ "src": "/icon.png", "sizes": "192x192 512x512 1024x1024", "type": "image/png", "purpose": "any" }],
    });
    files.push(("manifest.webmanifest".into(), manifest.to_string().into_bytes()));
    // The service worker shows a note it can't open with the custom build's name.
    if let Some((_, sw)) = files.iter_mut().find(|(path, _)| path == "sw.js") {
        *sw = String::from_utf8_lossy(sw)
            .replacen("const NAME = \"\";", &format!("const NAME = {};", json!(crate::config::NAME)), 1)
            .into_bytes();
    }
    // The client's own screens, which it shows from here and never from a node (`screen::client_template`).
    files.extend(crate::screen::client_screens().map(|name| (format!("screens/{name}.html"), crate::screen::client_template(name).into_bytes())));
    // Each file the page loads is named by its content (`?v=`, which the gateway checks), so a browser keeps it for
    // good and asks again only once it changes: those it names itself, and those web.js asks for (`one-files`).
    let named = |path: &str, bytes: &[u8]| {
        use sha2::Digest;
        format!("/{path}?v={}", &data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(bytes))[..16])
    };
    // A script names the scripts it imports by their content too (each after its own imports), so a reopen fetches
    // none of them: web.js's, and the worker's that reaches nodes. What it's named by is what it is with them.
    let scripts: Vec<String> = files.iter().map(|(path, _)| path.clone()).filter(|path| path.ends_with(".js") && !["sw.js", "start.js"].contains(&path.as_str())).collect();
    let mut done: std::collections::HashMap<String, (Vec<u8>, String)> = Default::default();
    for path in &scripts {
        name_script(path, &files, &scripts, &mut done, &mut vec![]);
    }
    for (path, bytes) in files.iter_mut() {
        if let Some((rewritten, _)) = done.remove(path.as_str()) {
            *bytes = rewritten;
        }
    }
    let names: Vec<(String, String)> = files.iter().filter(|(path, _)| !["index.html", "sw.js"].contains(&path.as_str())).map(|(path, bytes)| (path.clone(), named(path, bytes))).collect();
    let start = files.iter().find(|(path, _)| path == "start.js").map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned());
    if let Some((_, page)) = files.iter_mut().find(|(path, _)| path == "index.html") {
        let mut text = String::from_utf8_lossy(page).into_owned();
        // start.js runs inline, as the page arrives, before its styles and scripts are in: its hash is in the
        // page's policy, so it's the one inline script that may.
        if let Some(start) = start {
            use sha2::Digest;
            let hash = data_encoding::BASE64.encode(&sha2::Sha256::digest(start.as_bytes()));
            text = text.replacen("<script src=\"/start.js\"></script>", &format!("<script>{start}</script>"), 1).replacen("script-src 'self'", &format!("script-src 'self' 'sha256-{hash}'"), 1);
        }
        for (path, name) in &names {
            text = text.replace(&format!("=\"/{path}\""), &format!("=\"{name}\""));
        }
        // The scripts web.js imports are asked for beside it, not a round trip after each, on a first visit.
        let preload: String = names.iter().filter(|(path, _)| scripts.contains(path) && !["web.js", "net-worker.js"].contains(&path.as_str())).map(|(_, name)| format!("<link rel=\"modulepreload\" href=\"{name}\">\n")).collect();
        text = text.replacen("<script type=\"module\"", &format!("{preload}<script type=\"module\""), 1);
        let asked = names.iter().filter(|(path, _)| ["frame.html", "icons.json", "net-worker.js"].contains(&path.as_str()) || path.starts_with("screens/")).map(|(path, name)| format!("{path}={}", &name[name.len() - 16..])).collect::<Vec<_>>().join(" ");
        let label = format!(
            "<title>{}</title>\n<meta name=\"application-name\" content=\"{}\">\n<meta name=\"one-cli\" content=\"{}\">\n<meta name=\"one-files\" content=\"{asked}\">",
            attr(crate::config::NAME),
            attr(crate::config::NAME),
            attr(crate::config::CLI)
        );
        *page = text.replacen("<title></title>", &label, 1).into_bytes();
    }
    files
}

/// The client's build, as the web client fetches it (`/client/<version>/`): what its files hash to, so a browser
/// keeps one for good. None if this build has no client (`update` builds it, where Rust builds for WebAssembly).
pub fn client_version() -> Option<String> {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    for (name, bytes) in crate::config::CLIENT {
        hash.update(name.as_bytes());
        hash.update(bytes);
    }
    (!crate::config::CLIENT.is_empty()).then(|| data_encoding::HEXLOWER.encode(&hash.finalize()[..8]))
}

/// The web client's files with the client that reaches nodes, at a path named by its build, which the page says (net.js):
/// what the address serves browsers and what the native apps run. Without a client built (`CLIENT`), a page that has none.
pub fn with_client() -> Vec<(String, Vec<u8>)> {
    let mut files = files();
    if let Some(version) = client_version() {
        for (name, bytes) in crate::config::CLIENT {
            files.push((format!("client/{version}/{name}"), bytes.to_vec()));
        }
        if let Some((_, page)) = files.iter_mut().find(|(path, _)| path == "index.html") {
            *page = String::from_utf8_lossy(page).replacen(r#"<meta name="one-client" content="">"#, &format!(r#"<meta name="one-client" content="{version}">"#), 1).into_bytes();
        }
    }
    files
}

/// A script of the web client, its imports of other scripts of it written as their names by content (`/device.js?v=…`),
/// and the hash it's named by (the start of its SHA-256), which those are in turn: so a change to one script renames
/// each that imports it. Memoised in `done`; `path`s in `stack` are being named, so an import cycle (there is none) keeps its
/// plain path.
fn name_script(path: &str, files: &[(String, Vec<u8>)], scripts: &[String], done: &mut std::collections::HashMap<String, (Vec<u8>, String)>, stack: &mut Vec<String>) -> Option<String> {
    use sha2::Digest;
    if let Some((_, v)) = done.get(path) {
        return Some(v.clone());
    }
    if stack.iter().any(|p| p == path) {
        return None;
    }
    stack.push(path.to_string());
    let text = String::from_utf8_lossy(&files.iter().find(|(p, _)| p == path)?.1).into_owned();
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut out = String::new();
    let mut at = 0;
    // Each quoted `./x.js`, `../x.js` or `/x.js` that is another script of the client.
    for (i, _) in text.match_indices('"') {
        if i < at {
            continue;
        }
        let end = text[i + 1..].find(|c: char| !(c.is_ascii_alphanumeric() || "./_-".contains(c))).map(|n| i + 1 + n);
        let Some(end) = end.filter(|&e| text[e..].starts_with('"')) else { continue };
        let spec = &text[i + 1..end];
        if !spec.ends_with(".js") || !(spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/')) {
            continue;
        }
        // Resolved as the browser does, from this script's place: `..` at the top stays there.
        let mut parts: Vec<&str> = if spec.starts_with('/') { vec![] } else { dir.split('/').filter(|p| !p.is_empty()).collect() };
        for part in spec.split('/') {
            match part {
                "" | "." => {}
                ".." => drop(parts.pop()),
                part => parts.push(part),
            }
        }
        let target = parts.join("/");
        if target == path || !scripts.contains(&target) {
            continue;
        }
        if let Some(v) = name_script(&target, files, scripts, done, stack) {
            out.push_str(&text[at..i]);
            out.push_str(&format!("\"/{target}?v={v}\""));
            at = end + 1;
        }
    }
    out.push_str(&text[at..]);
    stack.pop();
    let v = data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(out.as_bytes()))[..16].to_string();
    done.insert(path.to_string(), (out.into_bytes(), v.clone()));
    Some(v)
}

