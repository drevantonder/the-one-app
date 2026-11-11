// What the UI carries (`ui::carried`): the Workers' and web client's files, the client's own screens, and the WebAssembly.
// config.toml is at the repo's root, the folder above this package, and a relative CONFIG is from there too.
#[path = "src/ui_files.rs"]
mod ui_files;

use std::path::{Path, PathBuf};

fn main() {
    let root = std::fs::canonicalize(std::env::var("CARGO_MANIFEST_DIR").unwrap()).unwrap().parent().unwrap().to_path_buf();
    let config_path = root.join(std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".into()));
    println!("cargo:rerun-if-changed={}", config_path.display());
    println!("cargo:rerun-if-env-changed=CONFIG");
    let text = std::fs::read_to_string(&config_path).unwrap_or_else(|e| panic!("read {}: {e}", config_path.display()));
    let config: toml::Table = text.parse().expect("parse config file");
    let get = |key: &str| config.get(key).unwrap_or_else(|| panic!("config file lacks {key}")).clone();
    let optional = |key: &str| config.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let beside_config = |file: &str| config_path.parent().unwrap().join(file);
    // The Workers' sources, built with the pinned Bun when they're run or deployed (oned/cloudflare/), so a
    // release deploys exactly the Workers it was signed with. Which files they are is `ui_files`.
    for watched in ui_files::sources(&root) {
        println!("cargo:rerun-if-changed={}", watched.display());
    }
    let mut files: Vec<(String, PathBuf)> = ui_files::list(&root);
    // Its demo's recording (ui/web/demo.js), if it has one.
    if !optional("demo").is_empty() {
        let demo = beside_config(&optional("demo"));
        println!("cargo:rerun-if-changed={}", demo.display());
        files.push(("web/demo.json".into(), std::fs::canonicalize(&demo).unwrap_or_else(|e| panic!("{}: {e}", demo.display()))));
    }
    // The custom build's logo, which the web client shows where the apps show it.
    let logo = beside_config(get("logo").as_str().unwrap());
    println!("cargo:rerun-if-changed={}", logo.display());
    files.push(("web/logo.png".into(), std::fs::canonicalize(&logo).unwrap_or_else(|e| panic!("{}: {e}", logo.display()))));
    // And its app icon, which the web client is installed with (its manifest) and shows on its notifications.
    let icon = beside_config(get("icon").as_str().unwrap());
    println!("cargo:rerun-if-changed={}", icon.display());
    files.push(("web/icon.png".into(), std::fs::canonicalize(&icon).unwrap_or_else(|e| panic!("{}: {e}", icon.display()))));
    // Home screens round an icon and fill what's behind it themselves, so one with margins shows as a small tile in a black
    // or white square: the same icon, drawn out to its edges, for the manifest's maskable icon and the apple-touch-icon.
    let full = Path::new(&std::env::var("OUT_DIR").unwrap()).join("icon-full.png");
    full_bleed(&icon).save(&full).unwrap_or_else(|e| panic!("{}: {e}", icon.display()));
    files.push(("web/icon-full.png".into(), full));
    files.sort();
    let files: Vec<String> = files.iter().map(|(rel, abs)| format!("({rel:?}, include_bytes!({:?}))", abs.to_string_lossy())).collect();
    let mut out = format!("pub const CLOUDFLARE: &[(&str, &[u8])] = &[{}];\n", files.join(", "));
    out.push_str(&client_screens(&root));
    // The Cloudflare node's WebAssembly (oned/cloudflare/node/) and onet's, for browsers (onet/src/wasm.rs), which `one-dev update program`
    // builds into CLOUDFLARE_NODE and ONET where Rust builds for WebAssembly, so a release carries them to devices with no
    // Rust to deploy it from, and the address serves the web client. With neither set, the ones `update program`, `throwaway cloudflare
    // oned deploy` and `throwaway client web` build into target/, if they're there, so every build of a checkout is the same one,
    // with or without them, but not their own (they're built with `ui` in them, which would hold itself). Only a file that
    // exists is watched: cargo reruns a build for one that doesn't.
    out.push_str(&wasm_constant(&root, "CLOUDFLARE_NODE", "target/cloudflare-node", ["one_cloudflare_node.js", "one_cloudflare_node_bg.wasm"]));
    out.push_str(&wasm_constant(&root, "ONET", "target/onet", ["onet.js", "onet_bg.wasm"]));
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("carried.rs"), out).unwrap();
}

/// `CLIENT_SCREENS`, each of the client's own screens as @one/ui's build made it (`<folder>/dist/<name>.html`): its name,
/// its path in the repo and its HTML; and `CLIENT_MAPS`, each one's source map. Found in `ui_files::screen_folders`, so
/// the UI names no feature.
fn client_screens(root: &Path) -> String {
    let mut screens: Vec<(String, String, PathBuf)> = Vec::new();
    for folder in ui_files::screen_folders(root) {
        let dist = folder.join("dist");
        println!("cargo:rerun-if-changed={}", dist.display());
        for entry in std::fs::read_dir(&dist).into_iter().flatten().flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".html")) else { continue };
            assert!(!screens.iter().any(|(n, ..)| n == name), "two of the client's own screens are named {name} (served as screens/{name}): rename one");
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            screens.push((name.to_string(), rel, path));
        }
    }
    screens.sort();
    let html: Vec<String> = screens.iter().map(|(name, rel, abs)| format!("({name:?}, {rel:?}, include_str!({:?}))", abs.to_string_lossy())).collect();
    let maps: Vec<String> = screens.iter().map(|(name, _, abs)| format!("({name:?}, include_str!({:?}))", format!("{}.map", abs.to_string_lossy()))).collect();
    format!("pub const CLIENT_SCREENS: &[(&str, &str, &str)] = &[{}];\npub const CLIENT_MAPS: &[(&str, &str)] = &[{}];\n", html.join(", "), maps.join(", "))
}

/// `pub const <var>: &[(&str, &[u8])]` holding `names`, which are in the folder the environment variable `var` names or, with
/// none, `default` in the repo, if all of them are there; `<var>_FILES`, where and what they are; and what cargo watches for them.
fn wasm_constant(root: &Path, var: &str, default: &str, names: [&str; 2]) -> String {
    println!("cargo:rerun-if-env-changed={var}");
    let wasm = std::env::var("TARGET").unwrap().starts_with("wasm32");
    let explicit = std::env::var(var).ok();
    let dir = explicit.clone().map(PathBuf::from).unwrap_or_else(|| root.join(default));
    for name in names.iter().filter(|_| explicit.is_some() || !wasm) {
        if dir.join(name).exists() {
            println!("cargo:rerun-if-changed={}", dir.join(name).display());
        }
    }
    let held: Vec<String> = if explicit.is_some() || (!wasm && names.iter().all(|name| dir.join(name).exists())) {
        names
            .iter()
            .map(|name| {
                let path = dir.join(name);
                format!("({name:?}, include_bytes!({:?}))", std::fs::canonicalize(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
            })
            .collect()
    } else {
        vec![]
    };
    format!("pub const {var}: &[(&str, &[u8])] = &[{}];\npub const {var}_FILES: (&str, [&str; 2]) = ({default:?}, {names:?});\n", held.join(", "))
}

/// `icon` without its margins: the plate (what's opaque) cut out, its rounded corners filled with the colour next to them in
/// their row, and the whole at 1024 px.
fn full_bleed(icon: &std::path::Path) -> image::RgbaImage {
    use image::imageops;
    let img = image::open(icon).unwrap_or_else(|e| panic!("{}: {e}", icon.display())).to_rgba8();
    let solid: Vec<(u32, u32)> = img.enumerate_pixels().filter(|(_, _, p)| p[3] == 255).map(|(x, y, _)| (x, y)).collect();
    let (Some(x0), Some(x1), Some(y0), Some(y1)) = (solid.iter().map(|p| p.0).min(), solid.iter().map(|p| p.0).max(), solid.iter().map(|p| p.1).min(), solid.iter().map(|p| p.1).max()) else {
        panic!("{} has no opaque pixel", icon.display());
    };
    let mut plate = imageops::crop_imm(&img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    for y in 0..plate.height() {
        let row: Vec<u32> = (0..plate.width()).filter(|&x| plate.get_pixel(x, y)[3] == 255).collect();
        let (Some(&first), Some(&last)) = (row.first(), row.last()) else { continue };
        for x in (0..first).chain(last + 1..plate.width()) {
            let mut from = *plate.get_pixel(if x < first { first } else { last }, y);
            from[3] = 255;
            plate.put_pixel(x, y, from);
        }
    }
    imageops::resize(&plate, 1024, 1024, imageops::FilterType::Lanczos3)
}
