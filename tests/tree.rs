//! What no commit should have.

/// A merge's markers, left in a tracked file, which `one-dev update` finds only when it ships that file.
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

/// oned on Cloudflare and the web client build for wasm, without what only a computer's app runtime uses, which only `one-dev update` and
/// `one-dev throwaway` build otherwise: a `#[cfg]` that hides what they use shows here, not when an update fails.
#[test]
fn what_runs_in_wasm_builds() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args(["check", "--quiet", "--target", "wasm32-unknown-unknown", "-p", "oned-cloudflare-worker", "-p", "onet"])
        .current_dir(root)
        .output()
        .expect("cargo check");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// The native app (`native-apps`, its own workspace that `cargo test` and `cargo build` never build) compiles for this
/// machine's platform (Linux's Tauri build here), for the Mac's and the iPhone's arm64 and, where the Android NDK is, for
/// Android's. Only the Rust: the C in their dependencies isn't compiled (a stand-in compiler and an SDK folder that is
/// only a name), and the Swift (`mac/`, `iphone/`) still needs a Mac, so `mac/build.sh` and `iphone/build.sh` are what
/// show a Swift error. Every target has a target folder of its own, so this neither waits on nor rebuilds the main
/// workspace, and a target's check doesn't leave in `target/` what its build then finds changed: Android's matters,
/// because `tauri`'s build script writes the Kotlin that `android/build.sh` compiles only when it runs with `build.sh`'s
/// environment, and doesn't run again when that changes, so a check without it first would leave the build with no
/// `TauriActivity`. Cold the two Apple checks took 2½ minutes on an idle machine and 7½ on a busy one; warm, with nothing
/// changed, the whole test takes 2 seconds. The Apple checks run rustc without sccache (`RUSTC_WRAPPER` empty): cc-rs puts
/// it in front of the C compiler too, and sccache refuses the `/bin/true` stand-in ("Compiler not supported"). The Apple targets' rust-std is a `rustup target add` away, so a missing one fails here, where a missing
/// NDK, a heavy install, only skips Android.
#[test]
fn the_native_app_builds() {
    let native = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../native-apps"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let check = |args: &[&str], envs: &[(String, std::path::PathBuf)]| {
        let out = std::process::Command::new(&cargo).args(["check", "--quiet"]).args(args).envs(envs.iter().map(|(k, v)| (k, v))).current_dir(native).output().expect("cargo check");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    check(&[], &[]);

    // The Mac builds the app's program too (`mac/build.sh`), the iPhone only its library (`iphone/build.sh`).
    for (target, args) in [("aarch64-apple-darwin", &[][..]), ("aarch64-apple-ios", &["--lib"][..])] {
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
        let libs = std::process::Command::new(rustc).args(["--print", "target-libdir", "--target", target]).output().expect("rustc");
        assert!(std::path::Path::new(String::from_utf8(libs.stdout).unwrap().trim()).exists(), "{target}'s rust-std isn't installed: run `rustup target add {target}`");
        let folder = native.join("target").join(format!("check-{target}"));
        std::fs::create_dir_all(&folder).unwrap();
        let var = target.replace('-', "_");
        let stand_in = std::path::PathBuf::from("/bin/true");
        let envs = [(format!("CC_{var}"), stand_in.clone()), (format!("AR_{var}"), stand_in), ("SDKROOT".into(), folder.clone()), ("RUSTC_WRAPPER".into(), "".into()), ("CARGO_TARGET_DIR".into(), folder)];
        check(&[&["--target", target][..], args].concat(), &envs);
    }

    // The NDK's compilers, which the C in the app's dependencies is built with, as `android/build.sh` finds them.
    let sdk = std::env::var_os("ANDROID_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Android/Sdk"));
    let Some(bin) = std::fs::read_dir(sdk.join("ndk")).ok().and_then(|ndks| ndks.filter_map(Result::ok).map(|e| e.path()).max()).and_then(|ndk| std::fs::read_dir(ndk.join("toolchains/llvm/prebuilt")).ok()?.next()?.ok().map(|e| e.path().join("bin"))) else {
        return;
    };
    let target = "aarch64-linux-android";
    check(&["--lib", "--target", target], &[("CC_aarch64_linux_android".into(), bin.join(format!("{target}29-clang"))), ("AR_aarch64_linux_android".into(), bin.join("llvm-ar")), ("CARGO_TARGET_DIR".into(), native.join("target/check-android"))]);
}

/// The relay Worker runs a WebAssembly build of iroh-relay's protocol that `relay/build.sh` makes by hand from
/// `relay/protos/` and commits: editing the sources without building, or moving the app runtimes to another iroh-relay than the
/// files were copied from, leaves a relay that speaks something else than the app runtimes do.
#[test]
fn the_relays_wasm_is_built_from_the_sources_beside_it() {
    use sha2::{Digest, Sha256};
    let relay = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")).join("relay");
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
    let linked = version(relay.join("../Cargo.lock"), "iroh-relay");
    let wasm = version(relay.join("protos/Cargo.lock"), "iroh-base");
    assert_eq!(linked, wasm, "the workspace links iroh-relay {linked}, but relay/protos holds its files at {wasm}: copy them again, then run relay/build.sh");
}

/// oned on Cloudflare's WebAssembly holds its own exports and onet's (a class `Foo`'s methods are exported as
/// `foo_<method>`, so a `foo_bar` function of the app runtime's is the same name): wasm-bindgen links either one in place of the
/// other, and the app runtime answers with the wrong code. Read from the sources, which is cheaper than building the wasm.
#[test]
fn oned_on_cloudflares_wasm_has_each_export_name_once() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let out = std::process::Command::new("git").args(["ls-files", "--cached", "--exclude-standard", "onet/src", "oned/cloudflare/worker/src"]).current_dir(root).output().expect("git ls-files");
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
    assert!(twice.is_empty(), "exported twice from oned on Cloudflare's WebAssembly (rename one):\n{}", twice.join("\n"));
    assert!(seen.contains_key("keyring_fetch") && seen.contains_key("device_fetch"), "the sources' wasm exports weren't found: {:?}", seen.keys().collect::<Vec<_>>());
}
