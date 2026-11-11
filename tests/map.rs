//! The file map, made from each folder's own doc, so nothing describes the code but the code. `cargo test map --
//! --nocapture` prints it.
//!
//! A folder is a part with a doc of its own when it's a Rust module (beside `<folder>.rs`, or with `mod.rs`), holds an
//! app (`app.js`), a Worker (`worker.js`), a package (`Cargo.toml`, `package.json`, `go.mod`, `Package.swift`) or a file
//! named like it (`web/web.js`); any other folder of code is part of the nearest one above it. Its doc is the top
//! comment (`//!` in Rust, `//` otherwise) of the first of these it has: `<folder>.rs` beside it, `mod.rs`, `app.js`,
//! `worker.js`, `src/lib.rs`, `src/main.rs`, `lib.rs`, `main.rs`, `main.go`, `Package.swift`, `<folder>.js`, then its
//! `package.json`'s `description`.

use std::collections::BTreeSet;
use std::path::Path;

const ROOTS: [&str; 34] = ["src", "app-runtime", "cli", "onet", "oned", "native-apps", "cloudflare", "ui", "relay", "auth", "gateway", "url-signer", "apps", "custom-build", "platform", "key", "bucket", "database", "scope", "api", "labs", "groups", "device-logs", "widgets", "devices", "deploy", "notifications", "services", "insights", "backups", "app-schedules", "vault", "onboarding", "ai-gateway"];
const CODE: [&str; 9] = ["rs", "js", "jsx", "ts", "tsx", "go", "swift", "cs", "kt"];
const PACKAGES: [&str; 4] = ["Cargo.toml", "package.json", "go.mod", "Package.swift"];
const SKIPPED: [&str; 4] = ["dist", "node_modules", "target", "vendor"];

/// Every folder that's a part, with its doc ("" when it has none), and the folders of code that are in none.
fn parts() -> (Vec<(String, String)>, Vec<String>) {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    let out = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "--"])
        .args(ROOTS)
        .current_dir(root)
        .output()
        .expect("git ls-files");
    let files: BTreeSet<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|f| !f.split('/').any(|p| SKIPPED.contains(&p)) && root.join(f).exists())
        .map(String::from)
        .collect();
    let has = |dir: &str, name: &str| files.contains(&format!("{dir}/{name}"));
    let name = |dir: &str| dir.rsplit('/').next().unwrap().to_string();
    let is_part = |dir: &str| {
        dir == "src"
            || has(dir, "mod.rs")
            || files.contains(&format!("{dir}.rs"))
            || PACKAGES.iter().chain(&["app.js", "worker.js"]).any(|f| has(dir, f))
            || has(dir, &format!("{}.js", name(dir)))
    };
    let code_dirs: BTreeSet<String> = files
        .iter()
        .filter(|f| f.rsplit_once('.').is_some_and(|(_, ext)| CODE.contains(&ext)))
        .filter_map(|f| Some(f.rsplit_once('/')?.0.to_string()))
        .collect();
    let in_crate_src = |dir: &str| dir.strip_suffix("/src").is_some_and(|p| has(p, "Cargo.toml"));
    let mut dirs = BTreeSet::new();
    for dir in &code_dirs {
        let mut d = dir.as_str();
        loop {
            dirs.insert(d.to_string());
            match d.rsplit_once('/') {
                Some((up, _)) => d = up,
                None => break,
            }
        }
    }
    let parts: Vec<(String, String)> = dirs
        .iter()
        .filter(|d| is_part(d) && !in_crate_src(d))
        .map(|d| (d.clone(), doc(root, d, &name(d), &has, &files)))
        .collect();
    let orphans = code_dirs
        .iter()
        .filter(|d| {
            let mut d = d.as_str();
            loop {
                if is_part(d) {
                    return false;
                }
                match d.rsplit_once('/') {
                    Some((up, _)) => d = up,
                    None => return true,
                }
            }
        })
        .cloned()
        .collect();
    (parts, orphans)
}

fn doc(root: &Path, dir: &str, name: &str, has: &dyn Fn(&str, &str) -> bool, files: &BTreeSet<String>) -> String {
    let beside = format!("{dir}.rs");
    let candidates = [
        files.contains(&beside).then(|| beside.clone()),
        Some(format!("{dir}/mod.rs")),
        Some(format!("{dir}/app.js")),
        Some(format!("{dir}/worker.js")),
        Some(format!("{dir}/src/lib.rs")),
        Some(format!("{dir}/src/main.rs")),
        Some(format!("{dir}/lib.rs")),
        Some(format!("{dir}/main.rs")),
        Some(format!("{dir}/main.go")),
        Some(format!("{dir}/Package.swift")),
        Some(format!("{dir}/{name}.js")),
    ];
    for file in candidates.into_iter().flatten() {
        if !files.contains(&file) {
            continue;
        }
        let text = std::fs::read_to_string(root.join(&file)).unwrap();
        let marker = if file.ends_with(".rs") { "//!" } else { "//" };
        let lines: Vec<&str> = text
            .lines()
            .skip_while(|l| (l.starts_with("#!") && !l.starts_with("#![")) || l.starts_with("// swift-tools-version"))
            .take_while(|l| l.starts_with(marker))
            .map(|l| l[marker.len()..].trim())
            .collect();
        return lines.join(" ").trim().to_string();
    }
    if has(dir, "package.json") {
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join(dir).join("package.json")).unwrap()).unwrap();
        return json["description"].as_str().unwrap_or("").to_string();
    }
    String::new()
}

/// Its first sentence.
fn first_sentence(doc: &str) -> &str {
    doc.find(". ").map_or(doc, |end| &doc[..=end])
}

#[test]
fn every_folder_says_what_it_is() {
    let (parts, orphans) = parts();
    let missing: Vec<&str> = parts.iter().filter(|(_, doc)| doc.is_empty()).map(|(dir, _)| dir.as_str()).collect();
    assert!(missing.is_empty(), "these folders have no doc (see tests/map.rs for where it goes): {missing:?}");
    assert!(orphans.is_empty(), "these folders of code are in no part (see tests/map.rs): {orphans:?}");
    for (dir, doc) in &parts {
        let depth = dir.matches('/').count();
        println!("{}{dir}/  {}", "  ".repeat(depth), first_sentence(doc));
    }
}
