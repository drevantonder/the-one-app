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
