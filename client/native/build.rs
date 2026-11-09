// The window's files are the web client's (one::web), written where Tauri takes its frontend from (`gen/web`). The app is named by the custom build (config.toml), as
// Tauri's own config says it (`TAURI_CONFIG`, which tauri-build and generate_context! both read). The app's own commands
// (src/commands.rs) are listed, so each needs a permission, which the config's one capability grants the app's own
// windows: left unlisted, Tauri would let any page call them.
use std::path::Path;

include!("src/commands.rs");

macro_rules! names {
    ($($command:ident),*) => {
        &[$(stringify!($command)),*]
    };
}

const COMMANDS: &[&str] = commands!(names);

fn main() {
    let dist = Path::new("gen/web");
    let _ = std::fs::remove_dir_all(dist);
    for (path, bytes) in one::web::with_client() {
        let to = dist.join(&path);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::write(to, bytes).unwrap();
    }
    // A test's build (`one-dev throwaway client mac`) is another app beside the real one: its own id, so its own folder,
    // instance and Keychain items, never the user's.
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
    // The Mac's menu bar mark, the custom build's, which the app draws as a template.
    if platform == "mac" {
        let mark = one::config::MENUBAR;
        assert!(!mark.is_empty(), "the custom build's config.toml names no menubar");
        println!("cargo:rerun-if-changed={mark}");
        std::fs::copy(mark, Path::new(&std::env::var("OUT_DIR").unwrap()).join("menubar.png")).unwrap_or_else(|e| panic!("{mark}: {e}"));
        mac_swift();
    }
    // The iPhone's Swift (iphone/), a plugin of the app's own (src/platform/iphone.rs), built and linked into it as Tauri's own
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
        // The window's own page, and the tray's panel (the same page), alone may ask the app's
        // commands. Screens are sandboxed frames in it with no origin of their own, which reach none: they ask the page,
        // which answers each as its app.
        "app": { "windows": [], "security": { "csp": null, "capabilities": [{
            "identifier": "window",
            "windows": ["main", "panel"],
            // And Tauri's own: what `data-tauri-drag-region` moves the window by.
            "permissions": COMMANDS.iter().map(|c| format!("allow-{}", c.replace('_', "-"))).chain(["core:window:allow-start-dragging".to_string()]).collect::<Vec<_>>(),
        }] } },
        "bundle": { "icon": icons },
        // `<cli>://open?app=&screen=&params=&scope=`, as the address's /back passes a sign-in elsewhere on to the app; on
        // Android also the links to the network's address, which its manifest lets open the app (android/app/build.gradle.kts).
        "plugins": { "deep-link": { "desktop": { "schemes": [one::config::CLI] }, "mobile": mobile_links() } },
    });
    // SAFETY: nothing else runs in this build script.
    unsafe { std::env::set_var("TAURI_CONFIG", config.to_string()) };
    println!("cargo:rustc-env=TAURI_CONFIG={config}");
    println!("cargo:rustc-env=IDENTIFIER={}", identifier(platform));
    println!("cargo:rerun-if-changed=src/commands.rs");
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS))).expect("the app's build")
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

/// Off a Mac there is no Swift to build and no linker for the app: `cargo check` (tests/tree.rs) is all that gets here.
#[cfg(not(target_os = "macos"))]
fn iphone_swift() {}

/// Builds the Mac's Swift package (mac/Package.swift), its menu bar, and links it in.
#[cfg(target_os = "macos")]
fn mac_swift() {
    // Its APIs are macOS 26's, as the app's least.
    // SAFETY: nothing else runs in this build script.
    unsafe { std::env::set_var("MACOSX_DEPLOYMENT_TARGET", "26.0") };
    tauri_utils::build::link_apple_library("one-native-mac", std::fs::canonicalize("mac").unwrap());
    // swift-rs links Swift's system libraries by @rpath and names no path to them.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rerun-if-changed=mac/Sources");
    println!("cargo:rerun-if-changed=mac/Package.swift");
    // What it draws widgets with, the iPhone's, symlinked in.
    println!("cargo:rerun-if-changed=iphone/Widget");
}

/// Elsewhere the Mac's Rust is only checked, which needs none of its Swift.
#[cfg(not(target_os = "macos"))]
fn mac_swift() {}

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
