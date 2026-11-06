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
