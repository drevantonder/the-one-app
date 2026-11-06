// The window's files are the web client's (one::web), written where Tauri takes its frontend from (`gen/web`), with
// the page's policy letting it reach the app (Tauri's `ipc:`). The app is named by the custom build (config.toml), as
// Tauri's own config says it (`TAURI_CONFIG`, which tauri-build and generate_context! both read). The app's own commands
// are listed, so each needs a permission a capability grants (capabilities/window.json): left unlisted, Tauri would let
// any page call them.
use std::path::Path;

fn main() {
    let dist = Path::new("gen/web");
    let _ = std::fs::remove_dir_all(dist);
    for (path, bytes) in one::web::files() {
        let to = dist.join(&path);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        let bytes = if path == "index.html" {
            String::from_utf8_lossy(&bytes).replacen("connect-src 'self'", "connect-src 'self' ipc: http://ipc.localhost", 1).into_bytes()
        } else {
            bytes
        };
        std::fs::write(to, bytes).unwrap();
    }
    let identifier = |platform: &str| format!("app.{}.{platform}", one::config::CLI);
    let platform = match std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        "macos" => "mac",
        "ios" => "phone",
        "windows" => "windows",
        "android" => "android",
        _ => "linux",
    };
    let config = serde_json::json!({
        "productName": one::config::NAME,
        "identifier": identifier(platform),
        // The custom build's app icon, as the web client's files have it.
        "bundle": { "icon": ["gen/web/icon.png"] },
        // `<cli>://open?app=&screen=&params=&scope=`, as the address's /back passes a sign-in elsewhere on to the app.
        "plugins": { "deep-link": { "desktop": { "schemes": [one::config::CLI] } } },
    });
    // SAFETY: nothing else runs in this build script.
    unsafe { std::env::set_var("TAURI_CONFIG", config.to_string()) };
    println!("cargo:rustc-env=TAURI_CONFIG={config}");
    println!("cargo:rerun-if-env-changed=CONFIG");
    let commands = tauri_build::AppManifest::new().commands(&["native", "address", "sign_in", "join", "wait", "fetch", "read", "cancel", "removed", "sign_out", "notify", "copy", "open", "save", "changed", "main", "login", "quit", "panel"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands)).expect("the app's build")
}
