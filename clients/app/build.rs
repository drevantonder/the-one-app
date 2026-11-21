fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let phone = target == "android" || target == "ios";
    println!("cargo::rustc-check-cfg=cfg(desktop)");
    println!("cargo::rustc-check-cfg=cfg(mobile)");
    // A phone's app is Tauri's, whose build makes it ready and says it's a phone's (`mobile`).
    #[cfg(feature = "window")]
    if phone {
        tauri_build::build();
    }
    if !phone {
        println!("cargo::rustc-cfg=desktop");
    }
    #[cfg(feature = "window")]
    if !phone {
        name();
    }
    #[cfg(feature = "window")]
    ui();
    if target == "windows" {
        embed_manifest::embed_manifest_file("one.manifest").expect("the manifest is embedded");
    }
}

/// A computer's app's name (`NAME` in lib.rs), as `tauri.conf.json` has it.
#[cfg(feature = "window")]
fn name() {
    let folder = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo says"));
    println!("cargo::rerun-if-changed=tauri.conf.json");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(folder.join("tauri.conf.json")).expect("tauri.conf.json")).expect("it's JSON");
    let name = config["productName"].as_str().expect("the app has a name");
    let to = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo says")).join("name.rs");
    std::fs::write(to, format!("pub(crate) const NAME: &str = {name:?};\n")).expect("OUT_DIR takes it");
}

/// The UI library, as the client's own pages draw with it (`hosts::file`): built for this program from `core/ui` as it is
/// (`bun build.js <folder>`), so they show before the device is in its network, and while its network's is newer than
/// this program draws, as a system's own screens come with it. What a native host loads of it: Solid, the UI library
/// built for one, and its tables.
#[cfg(feature = "window")]
fn ui() {
    use std::fmt::Write;
    let folder = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo says"));
    let ui = folder.join("../../core/ui").canonicalize().expect("the UI library is in the repo");
    for made_from in ["src", "build.js", "tables.ts", "package.json", "bun.lock"] {
        println!("cargo::rerun-if-changed={}", ui.join(made_from).display());
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo says")).join("ui");
    let bun = |args: &[&std::ffi::OsStr]| {
        let status = std::process::Command::new("bun").args(args).current_dir(&ui).status().expect("bun builds the UI library: `mise install` has it");
        assert!(status.success(), "the UI library didn't build: bun {args:?} in {}", ui.display());
    };
    bun(&["install".as_ref(), "--frozen-lockfile".as_ref()]);
    bun(&["build.js".as_ref(), out.as_os_str()]);
    // Its JS: Solid's (`solid.js`, `internal.js` and their chunks), and the UI library (`ui.js`, `web.js` and theirs) from
    // `native/`, where this app's renderer loads it (`Resolve` in the renderer's js/quickjs.rs), and not again beside it.
    let files = |folder: &'static str| std::fs::read_dir(out.join(folder)).expect("the UI library built").map(move |file| format!("{folder}{}", file.expect("a file").file_name().to_string_lossy()));
    let files = files("").filter(|file| file.ends_with(".js") && !file.starts_with("ui") && !file.starts_with("web")).chain(files("native/").filter(|file| file.ends_with(".js"))).chain(["tokens.json".to_owned(), "styles.json".to_owned()]);
    let mut table = "/// The UI library's files the client's own pages load, by the path each is served at.\npub const UI: &[(&str, &[u8])] = &[\n".to_owned();
    for file in files {
        writeln!(table, "    (\"/ui/client/{file}\", include_bytes!({:?})),", out.join(&file).display().to_string()).expect("a String takes it");
    }
    table.push_str("];\n");
    std::fs::write(out.with_file_name("ui.rs"), table).expect("OUT_DIR takes it");
}
