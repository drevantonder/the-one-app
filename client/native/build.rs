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
    // A test's build (`one throwaway client mac`) is another app beside the real one: its own id, so its own folder,
    // instance and Keychain items, never the person's.
    println!("cargo:rerun-if-env-changed=THROWAWAY");
    let throwaway = std::env::var("THROWAWAY").is_ok_and(|v| !v.is_empty());
    let identifier = |platform: &str| format!("app.{}.{platform}{}", one::config::CLI, if throwaway { ".throwaway" } else { "" });
    let platform = match std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        "macos" => "mac",
        "ios" => "phone",
        "windows" => "windows",
        "android" => "android",
        _ => "linux",
    };
    // The Mac's menu bar mark, the custom build's (config.toml's `menubar`, its @2x), which the app draws as a template.
    if platform == "mac" {
        let config = std::env::var("CONFIG").map(std::path::PathBuf::from).unwrap_or_else(|_| Path::new("../../config.toml").to_path_buf());
        let text = std::fs::read_to_string(&config).expect("the custom build's config.toml");
        let named = text.lines().take_while(|l| !l.starts_with('[')).find_map(|l| l.strip_prefix("menubar")?.trim().strip_prefix('=')).expect("config.toml's menubar");
        let mark = config.parent().unwrap().join(named.trim().trim_matches('"').replace(".png", "@2x.png"));
        println!("cargo:rerun-if-changed={}", mark.display());
        std::fs::copy(&mark, Path::new(&std::env::var("OUT_DIR").unwrap()).join("menubar.png")).unwrap_or_else(|e| panic!("{}: {e}", mark.display()));
    }
    // The iPhone's Swift (iphone/), a plugin of the app's own (src/iphone.rs), built and linked into it as Tauri's own
    // plugins are, against Tauri's Swift, copied beside it as theirs is.
    if platform == "phone" {
        iphone_swift();
    }
    // The custom build's app icon, as the web client's files have it; on Windows, the program's own too (its window's,
    // the taskbar's), an .ico.
    let mut icons = vec!["gen/web/icon.png"];
    if platform == "windows" {
        windows_icon(&dist.join("icon.png"), Path::new("gen/icon.ico"));
        icons.push("gen/icon.ico");
    }
    // Android's Kotlin is one package, `app.client` (android/app/src/main/kotlin), which Tauri's web view's own is made in
    // and its library's functions are named by: so Tauri's identifier there, which says that, isn't the app's
    // (`app.<cli>.android`, the Gradle project's applicationId).
    let tauri_identifier = if platform == "android" { "app.client".to_string() } else { identifier(platform) };
    let config = serde_json::json!({
        "productName": one::config::NAME,
        "identifier": tauri_identifier,
        "bundle": { "icon": icons },
        // `<cli>://open?app=&screen=&params=&scope=`, as the address's /back passes a sign-in elsewhere on to the app; on
        // Android also the links to the network's address, which its manifest lets open the app (android/app/build.gradle.kts).
        "plugins": { "deep-link": { "desktop": { "schemes": [one::config::CLI] }, "mobile": mobile_links() } },
    });
    // SAFETY: nothing else runs in this build script.
    unsafe { std::env::set_var("TAURI_CONFIG", config.to_string()) };
    println!("cargo:rustc-env=TAURI_CONFIG={config}");
    println!("cargo:rustc-env=IDENTIFIER={}", identifier(platform));
    println!("cargo:rerun-if-env-changed=CONFIG");
    let commands = tauri_build::AppManifest::new().commands(&["native", "address", "sign_in", "join", "wait", "fetch", "read", "cancel", "signs", "removed", "sign_out", "notify", "copy", "open", "save", "changed", "main", "login", "quit", "panel", "fit", "badge", "computer_use", "pair", "haptic", "scan", "menu", "appearance", "menubar"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands)).expect("the app's build")
}

/// The icon as Windows wants a program's: an .ico of each size it draws, from the icon drawn on Apple's Mac grid (an
/// 824-point tile in a clear 100-point margin in 1024), its tile filling each, its corners clear.
fn windows_icon(png: &Path, to: &Path) {
    let icon = image::open(png).expect("the custom build's icon").into_rgba8();
    let margin = icon.width() * 100 / 1024;
    let tile = image::imageops::crop_imm(&icon, margin, margin, icon.width() - 2 * margin, icon.height() - 2 * margin).to_image();
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 24, 32, 48, 64, 128, 256] {
        let small = image::imageops::resize(&tile, size, size, image::imageops::FilterType::Lanczos3);
        dir.add_entry(ico::IconDirEntry::encode(&ico::IconImage::from_rgba_data(size, size, small.into_raw())).unwrap());
    }
    dir.write(std::fs::File::create(to).unwrap()).unwrap();
}

/// Builds the iPhone's Swift package (iphone/Package.swift) and links it in.
#[cfg(target_os = "macos")]
fn iphone_swift() {
    let tauri = std::path::PathBuf::from(std::env::var("DEP_TAURI_IOS_LIBRARY_PATH").expect("Tauri's Swift, which its build names"));
    let to = Path::new("iphone/.tauri/tauri-api");
    let _ = std::fs::remove_dir_all(to);
    copy(&tauri, to);
    // Its APIs are iOS 26's, as the app's least.
    // SAFETY: nothing else runs in this build script.
    unsafe { std::env::set_var("IPHONEOS_DEPLOYMENT_TARGET", "26.0") };
    tauri_utils::build::link_apple_library("one-native-iphone", std::fs::canonicalize("iphone").unwrap());
    println!("cargo:rerun-if-changed=iphone/Sources");
    println!("cargo:rerun-if-changed=iphone/Package.swift");
}

#[cfg(not(target_os = "macos"))]
fn iphone_swift() {
    panic!("The iPhone's app builds on a Mac.");
}

/// A folder's files, but what Swift's build left in it.
#[cfg(target_os = "macos")]
fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name();
        if [".build", "Package.resolved", "Tests"].iter().any(|n| name == *n) {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to.join(&name));
        } else {
            std::fs::copy(entry.path(), to.join(&name)).unwrap();
        }
    }
}

/// What Android passes the app as links: its own scheme, and `https` links to the hosts of the custom build's address and
/// `LINKS`'s (comma-separated), as the manifest's filters (android/app/build.gradle.kts) let in.
fn mobile_links() -> Vec<serde_json::Value> {
    println!("cargo:rerun-if-env-changed=LINKS");
    let mut links = vec![serde_json::json!({ "scheme": [one::config::CLI] })];
    let more = std::env::var("LINKS").unwrap_or_default();
    for address in std::iter::once(one::config::ADDRESS).chain(more.split(',')) {
        if let Some(host) = address.trim().strip_prefix("https://").map(|h| h.trim_end_matches('/').to_string()).filter(|h| !h.is_empty()) {
            links.push(serde_json::json!({ "scheme": ["https"], "host": host }));
        }
    }
    links
}
