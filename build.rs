// What the program carries (`one::carried`): the built-in apps, the Workers' and web client's files, and the WebAssembly.
// config.toml's constants are custom-build's (custom-build/build.rs).
#[path = "src/ui_files.rs"]
mod ui_files;

fn main() {
    let path = std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".into());
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed=CONFIG");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let config: toml::Table = text.parse().expect("parse config file");
    let get = |key: &str| config.get(key).unwrap_or_else(|| panic!("config file lacks {key}")).clone();
    let optional = |key: &str| config.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let out = String::new();
    // Every app in this repo's apps/ folder ships built in: each folder's files. The config's `apps` are the ones a new
    // network deploys and installs in `global`, by folder name.
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("apps");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut dirs: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("app.js").is_file()).collect();
    dirs.sort();
    let mut apps = String::new();
    for dir in &dirs {
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files = Vec::new();
        files.extend(ui_files::collect(dir).into_iter().map(|(rel, abs)| (rel, abs.to_string_lossy().to_string())));
        // Its service's source is built apart (`one-dev update services`), and its tests run with ours: neither ships in its bundle.
        files.retain(|(rel, _)| !rel.starts_with("service/") && !rel.starts_with("tests/"));
        files.sort();
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        // A React screen ships as its `dist/` has it, which `one deploy apps/<name>` builds, so each has one.
        for (rel, _) in &files {
            if let Some(screen) = rel.strip_prefix("screens/").and_then(|f| f.strip_suffix(".jsx").or_else(|| f.strip_suffix(".tsx"))) {
                assert!(files.iter().any(|(r, _)| *r == format!("dist/{screen}.html")), "apps/{name}/screens/{screen} has no dist/{screen}.html: run `one deploy apps/{name}` and commit it");
            }
        }
        let files: Vec<String> = files.iter().map(|(rel, abs)| format!("({rel:?}, include_bytes!({abs:?}))")).collect();
        apps.push_str(&format!("({name:?}, &[{}]),", files.join(", ")));
    }
    let mut starters = String::new();
    for name in config.get("apps").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|d| d.as_str()) {
        assert!(dirs.iter().any(|d| d.file_name().is_some_and(|f| f == name)), "the config's app {name} isn't in apps/");
        starters.push_str(&format!("{name:?},"));
    }
    let out = format!("{out}pub const BUILT_IN: &[(&str, &[(&str, &[u8])])] = &[{apps}];\npub const STARTERS: &[&str] = &[{starters}];\n");
    // The Workers' sources, built with the pinned Bun when they're run or deployed (src/cloudflare.rs), so a
    // release deploys exactly the Workers it was signed with. Which files they are is `ui_files`.
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    for watched in ["cloudflare", "relay", "auth", "gateway", "url-signer", "notifications/worker", "insights/worker", "src/screen", "onboarding/pages.js"] {
        println!("cargo:rerun-if-changed={}", root.join(watched).display());
    }
    let mut files: Vec<(String, String)> = ui_files::list(&root).into_iter().map(|(rel, abs)| (rel, abs.to_string_lossy().into())).collect();
    // Its demo's recording (cloudflare/web/demo.js), if it has one.
    if !optional("demo").is_empty() {
        let demo = std::path::Path::new(&path).parent().unwrap().join(optional("demo"));
        println!("cargo:rerun-if-changed={}", demo.display());
        files.push(("web/demo.json".into(), std::fs::canonicalize(&demo).unwrap_or_else(|e| panic!("{}: {e}", demo.display())).to_string_lossy().into()));
    }
    // The custom build's logo, which the web client shows where the apps show it.
    let logo = std::path::Path::new(&path).parent().unwrap().join(get("logo").as_str().unwrap());
    println!("cargo:rerun-if-changed={}", logo.display());
    files.push(("web/logo.png".into(), std::fs::canonicalize(&logo).unwrap_or_else(|e| panic!("{}: {e}", logo.display())).to_string_lossy().into()));
    // And its app icon, which the web client is installed with (its manifest) and shows on its notifications.
    let icon = std::path::Path::new(&path).parent().unwrap().join(get("icon").as_str().unwrap());
    println!("cargo:rerun-if-changed={}", icon.display());
    files.push(("web/icon.png".into(), std::fs::canonicalize(&icon).unwrap_or_else(|e| panic!("{}: {e}", icon.display())).to_string_lossy().into()));
    // Home screens round an icon and fill what's behind it themselves, so one with margins shows as a small tile in a black
    // or white square: the same icon, drawn out to its edges, for the manifest's maskable icon and the apple-touch-icon.
    let full = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("icon-full.png");
    full_bleed(&icon).save(&full).unwrap_or_else(|e| panic!("{}: {e}", icon.display()));
    files.push(("web/icon-full.png".into(), full.to_string_lossy().into()));
    files.sort();
    let files: Vec<String> = files.iter().map(|(rel, abs)| format!("({rel:?}, include_bytes!({abs:?}))")).collect();
    let out = format!("{out}pub const CLOUDFLARE: &[(&str, &[u8])] = &[{}];\n", files.join(", "));
    // The Cloudflare node's WebAssembly (cloudflare/node/) and onet's, for browsers (onet/src/wasm.rs), which `one-dev update node`
    // builds into CLOUDFLARE_NODE and ONET where Rust builds for WebAssembly, so a release carries them to devices with no
    // Rust to deploy it from, and the address serves the web client. With neither set, the ones `update node`, `throwaway cloudflare
    // node deploy` and `throwaway client web` build into target/, if they're there, so every build of a checkout is the same one,
    // with or without them, but not their own (they're built with `one` in them, which would hold itself). Only a file that
    // exists is watched: cargo reruns a build for one that doesn't.
    let out = format!("{out}{}", wasm_constant("CLOUDFLARE_NODE", "target/cloudflare-node", ["one_cloudflare_node.js", "one_cloudflare_node_bg.wasm"]));
    let out = format!("{out}{}", wasm_constant("ONET", "target/onet", ["onet.js", "onet_bg.wasm"]));
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("carried.rs");
    std::fs::write(dest, out).unwrap();
}

/// `pub const <var>: &[(&str, &[u8])]` holding `names`, which are in the folder the environment variable `var` names or, with
/// none, `default` in this package, if all of them are there; `<var>_FILES`, where and what they are; and what cargo watches for them.
fn wasm_constant(var: &str, default: &str, names: [&str; 2]) -> String {
    println!("cargo:rerun-if-env-changed={var}");
    let wasm = std::env::var("TARGET").unwrap().starts_with("wasm32");
    let explicit = std::env::var(var).ok();
    let dir = explicit.clone().map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join(default));
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
