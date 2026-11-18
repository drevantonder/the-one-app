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
        page();
        // A computer's window on Windows asks for Common Controls 6 and to be drawn at each display's own scale.
        if target == "windows" {
            embed_manifest::embed_manifest(embed_manifest::new_manifest("one")).expect("the manifest is embedded");
        }
    }
    // `one` without its window, on Windows, is a console program that asks for no console window of its own when
    // nothing it's started from has one, as when Task Scheduler starts its server (Windows 11 24H2 and later).
    #[cfg(not(feature = "window"))]
    if target == "windows" {
        embed_manifest::embed_manifest_file("one.manifest").expect("the manifest is embedded");
    }
}

/// A computer's window's page, served from inside the program (`page` in lib.rs): the files `tauri.conf.json` lists as
/// the page's (`frontendDist`), each at its name, and what they may load (`csp`). With them, the app's name, from the
/// same file and whatever a network's build merges over it (`TAURI_CONFIG`), as a phone's Tauri build has it.
#[cfg(feature = "window")]
fn page() {
    use std::fmt::Write;
    let folder = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo says"));
    println!("cargo::rerun-if-changed=tauri.conf.json");
    println!("cargo::rerun-if-env-changed=TAURI_CONFIG");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(folder.join("tauri.conf.json")).expect("tauri.conf.json")).expect("it's JSON");
    let merged: Option<serde_json::Value> = std::env::var("TAURI_CONFIG").ok().and_then(|config| serde_json::from_str(&config).ok());
    let name = merged.as_ref().and_then(|merged| merged["productName"].as_str()).or(config["productName"].as_str()).expect("the app has a name");
    let csp = config["app"]["security"]["csp"].as_str().unwrap_or_default();
    let mut out = format!("/// The app's name.\npub const NAME: &str = {name:?};\n/// What the page may load.\npub const CSP: &str = {csp:?};\n/// The page's files, by the path each is served at.\npub const FILES: &[(&str, &[u8])] = &[\n");
    for file in config["build"]["frontendDist"].as_array().expect("frontendDist lists the page's files") {
        let path = folder.join(file.as_str().expect("a path")).canonicalize().expect("each of the page's files is there");
        println!("cargo::rerun-if-changed={}", path.display());
        let served = format!("/{}", path.file_name().expect("a file").to_string_lossy());
        writeln!(out, "    ({served:?}, include_bytes!({:?})),", path.display().to_string()).expect("a String takes it");
    }
    out.push_str("];\n");
    let to = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo says")).join("page.rs");
    std::fs::write(to, out).expect("OUT_DIR takes it");
}
