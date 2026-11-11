//! Which files of a checkout the Workers and the web client are made of: what a build holds (build.rs) and what's read
//! from a checkout instead of it (`ui_folder`), the one list.

use std::path::{Path, PathBuf};

/// The folders of the Workers that have one of their own, and their names in the source tree they're built from.
const WORKERS: [(&str, &str); 5] = [("relay", "relay"), ("auth", "auth"), ("gateway", "gateway"), ("url-signer", "signer"), ("notifications/worker", "push")];

/// The files of those folders that are among the others of the source tree they're built from: what other Workers import
/// (`../shared/token.js`, `../shared/http.js`, `../shared/s3.js`), and the one package the Workers are installed with (`package.json`, `bun.lock`).
const BESIDE_OTHERS: [(&str, &str); 5] = [("auth/token.js", "shared/token.js"), ("auth/package.json", "package.json"), ("auth/bun.lock", "bun.lock"), ("gateway/http.js", "shared/http.js"), ("url-signer/s3.js", "shared/s3.js")];

/// What stays in the repo: the relay's WebAssembly is built from `relay/protos/` by hand (`relay/build.sh`), so only what it
/// built ships.
const NOT_SHIPPED: [&str; 2] = ["relay/protos/", "relay/build.sh"];

/// Each file in the checkout at `root` the Workers and the web client are made of: its path among them, and where it
/// is. All but the custom build's logo and icon, which its config names.
pub fn list(root: &Path) -> Vec<(String, PathBuf)> {
    let cloudflare = root.join("cloudflare");
    let mut files = collect(&cloudflare);
    // The Workers' tests stay in the repo, and the Cloudflare node (node/) is Rust, which ships built.
    files.retain(|(rel, _)| !rel.starts_with("test/") && !rel.starts_with("node/"));
    // Each Worker's own folder, in the source tree the Workers are built from, so their imports of each other keep working.
    for (folder, source) in WORKERS {
        for (rel, abs) in collect(&root.join(folder)) {
            let path = format!("{folder}/{rel}");
            if !NOT_SHIPPED.iter().any(|p| path.starts_with(p)) {
                let name = BESIDE_OTHERS.iter().find(|(from, _)| *from == path).map_or_else(|| format!("{source}/{rel}"), |(_, to)| to.to_string());
                files.push((name, abs));
            }
        }
    }
    // The gateway serves the screens' design kit beside the web client, the same files nodes put in screens.
    // Not its source maps: only nodes read those, and every client would download them.
    files.extend(collect(&root.join("src/screen")).into_iter().filter(|(rel, _)| !rel.ends_with(".html.map")).filter_map(|(rel, abs)| {
        let rel = match rel.strip_prefix("ui/") {
            Some(ui) => format!("ui/{}", ui.strip_prefix("dist/")?),
            None => rel,
        };
        Some((format!("web/screen/{rel}"), abs))
    }));
    // Onboarding's auth pages, which the auth Worker imports as ../onboarding/pages.js.
    files.push(("onboarding/pages.js".into(), root.join("onboarding/pages.js")));
    // And the formats devices hold keys in, which the web client imports as ../shared/crypto.js.
    files.push(("web/shared/crypto.js".into(), cloudflare.join("shared/crypto.js")));
    files
}

/// Each file under `dir`, by its path in it, but hidden ones and installed packages.
pub fn collect(dir: &Path) -> Vec<(String, PathBuf)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}
