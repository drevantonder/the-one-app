//! What no commit should have.

/// A merge's markers, left in a tracked file, which `one update` finds only when it ships that file.
#[test]
fn no_file_has_a_conflict_marker_left() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let out = std::process::Command::new("git").args(["ls-files", "--cached", "--exclude-standard"]).current_dir(root).output().expect("git ls-files");
    let left: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|path| {
            let Ok(text) = std::fs::read_to_string(root.join(path)) else { return false };
            text.lines().any(|line| line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "<<<<<<<" || line == ">>>>>>>")
        })
        .map(str::to_string)
        .collect();
    assert!(left.is_empty(), "a merge's conflict markers are left in: {}", left.join(", "));
}

/// The Cloudflare node and the web client build for wasm, without the node feature, which only `one update` and
/// `one throwaway` build otherwise: a `#[cfg]` that hides what they use shows here, not when an update fails.
#[test]
fn what_runs_in_wasm_builds() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args(["check", "--quiet", "--target", "wasm32-unknown-unknown", "-p", "one-cloudflare-node", "-p", "one-client"])
        .current_dir(root)
        .output()
        .expect("cargo check");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// The native app (`client/native`, its own workspace that `cargo test` and `cargo build` never build) compiles for this
/// machine's platform (Linux's Tauri build here) and, where the Android NDK is, for Android's arm64. Apple's targets only
/// build on a Mac, so the iPhone's Rust and Swift are left to `iphone/build.sh`. It has its own target folder, so this
/// neither waits on nor rebuilds the main workspace; warm, with nothing changed, it takes under a second. Android's check
/// has a folder of its own too: `tauri`'s build script writes the Kotlin that `android/build.sh` compiles, only when it
/// runs with `build.sh`'s environment, and doesn't run again when that changes, so a check without it first would leave
/// the build in `target/` with no `TauriActivity`.
#[test]
fn the_native_app_builds() {
    let native = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../client/native"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let check = |args: &[&str], envs: &[(String, std::path::PathBuf)]| {
        let out = std::process::Command::new(&cargo).args(["check", "--quiet"]).args(args).envs(envs.iter().map(|(k, v)| (k, v))).current_dir(native).output().expect("cargo check");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    check(&[], &[]);

    // The NDK's compilers, which the C in the app's dependencies is built with, as `android/build.sh` finds them.
    let sdk = std::env::var_os("ANDROID_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Android/Sdk"));
    let Some(bin) = std::fs::read_dir(sdk.join("ndk")).ok().and_then(|ndks| ndks.filter_map(Result::ok).map(|e| e.path()).max()).and_then(|ndk| std::fs::read_dir(ndk.join("toolchains/llvm/prebuilt")).ok()?.next()?.ok().map(|e| e.path().join("bin"))) else {
        return;
    };
    let target = "aarch64-linux-android";
    check(&["--lib", "--target", target], &[("CC_aarch64_linux_android".into(), bin.join(format!("{target}29-clang"))), ("AR_aarch64_linux_android".into(), bin.join("llvm-ar")), ("CARGO_TARGET_DIR".into(), native.join("target/check-android"))]);
}

/// The relay Worker runs a WebAssembly build of iroh-relay's protocol that `cloudflare/relay/build.sh` makes by hand from
/// `relay/protos/` and commits: editing the sources without building, or moving the nodes to another iroh-relay than the
/// files were copied from, leaves a relay that speaks something else than the nodes do.
#[test]
fn the_relays_wasm_is_built_from_the_sources_beside_it() {
    use sha2::{Digest, Sha256};
    let relay = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("cloudflare/relay");
    let mut files = Vec::new();
    for entry in ["Cargo.toml", "Cargo.lock", "src"] {
        let mut folders = vec![relay.join("protos").join(entry)];
        while let Some(path) = folders.pop() {
            match path.is_dir() {
                true => folders.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path())),
                false => files.push(path),
            }
        }
    }
    files.sort_by_key(|f| f.to_string_lossy().to_string());
    let listing: String = files.iter().map(|f| format!("{}  {}\n", data_encoding::HEXLOWER.encode(&Sha256::digest(std::fs::read(f).unwrap())), f.strip_prefix(relay.join("protos")).unwrap().display())).collect();
    let built = std::fs::read_to_string(relay.join("protos.sha256")).unwrap();
    assert_eq!(built.trim(), data_encoding::HEXLOWER.encode(&Sha256::digest(listing)), "relay/protos changed since relay/build.sh built the committed wasm: run it");

    let version = |lock: std::path::PathBuf, name: &str| -> String {
        let lock = std::fs::read_to_string(lock).unwrap();
        let after = lock.split_once(&format!("name = \"{name}\"\nversion = \"")).unwrap_or_else(|| panic!("{name} isn't locked")).1;
        after.split('"').next().unwrap().to_string()
    };
    let nodes = version(relay.join("../../Cargo.lock"), "iroh-relay");
    let wasm = version(relay.join("protos/Cargo.lock"), "iroh-base");
    assert_eq!(nodes, wasm, "the nodes link iroh-relay {nodes}, but relay/protos holds its files at {wasm}: copy them again, then run relay/build.sh");
}

/// The Cloudflare node's WebAssembly holds its own exports and one-client's (a class `Foo`'s methods are exported as
/// `foo_<method>`, so a `foo_bar` function of the node's is the same name): wasm-bindgen links either one in place of the
/// other, and the node answers with the wrong code. Read from the sources, which is cheaper than building the wasm.
#[test]
fn the_cloudflare_nodes_wasm_has_each_export_name_once() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let out = std::process::Command::new("git").args(["ls-files", "--cached", "--exclude-standard", "client/src", "cloudflare/node/src"]).current_dir(root).output().expect("git ls-files");
    let snake = |name: &str| {
        let mut out = String::new();
        for (i, c) in name.chars().enumerate() {
            if c.is_uppercase() && i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        }
        out
    };
    let fn_name = |line: &str| line.trim().strip_prefix("pub ")?.trim_start_matches("async ").strip_prefix("fn ")?.split(['(', '<']).next().map(str::to_string);
    let mut seen = std::collections::BTreeMap::<String, String>::new();
    let mut twice = Vec::new();
    for path in String::from_utf8(out.stdout).unwrap().lines().filter(|p| p.ends_with(".rs")) {
        let text = std::fs::read_to_string(root.join(path)).unwrap();
        let mut exported = false;
        let mut class = None;
        for line in text.lines() {
            let found = if line.starts_with("#[wasm_bindgen") {
                exported = true;
                None
            } else if let Some(class) = class.as_deref().filter(|_| line.starts_with("    pub ")) {
                fn_name(line).map(|f| format!("{}_{f}", snake(class)))
            } else if line.starts_with("pub ") && exported {
                fn_name(line)
            } else {
                None
            };
            if let Some(name) = found {
                let here = format!("{path}: {line}");
                if let Some(before) = seen.insert(name.clone(), here.clone()) {
                    twice.push(format!("{name}, in {before} and {here}"));
                }
            }
            if line.starts_with("impl ") {
                class = exported.then(|| line.trim_start_matches("impl ").split([' ', '<']).next().unwrap().to_string());
                exported = false;
            } else if line == "}" {
                class = None;
            } else if !line.starts_with('#') && !line.starts_with("///") && !line.starts_with("//") {
                exported = false;
            }
        }
    }
    assert!(twice.is_empty(), "exported twice from the Cloudflare node's WebAssembly (rename one):\n{}", twice.join("\n"));
    assert!(seen.contains_key("keyring_fetch") && seen.contains_key("device_fetch"), "the sources' wasm exports weren't found: {:?}", seen.keys().collect::<Vec<_>>());
}
