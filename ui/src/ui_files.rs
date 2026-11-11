//! Which files of a checkout the Workers, the web client and the client's own screens are made of: what a build holds
//! (build.rs) and what's read from a checkout instead of it (`ui_folder`), the one list.

use std::path::{Path, PathBuf};

/// The folders of the Workers' and the web client's files that have one of their own, and their names in the source
/// tree they're built from: the web client's are ui/web's, and onet's hosts in a browser (onet/web), beside each other.
const WORKERS: [(&str, &str); 9] = [
    ("relay", "relay"),
    ("auth", "auth"),
    ("gateway", "gateway"),
    ("url-signer", "signer"),
    ("notifications/worker", "push"),
    ("insights/worker", "telemetry"),
    ("onet/worker", "mcp"),
    ("ui/web", "web"),
    ("onet/web", "web"),
];

/// The files of those folders that are among the others of the source tree they're built from: what other Workers import
/// (`../shared/token.js`, `../shared/http.js`, `../shared/s3.js`), and the one package the Workers are installed with (`package.json`, `bun.lock`).
const BESIDE_OTHERS: [(&str, &str); 5] = [("auth/token.js", "shared/token.js"), ("auth/package.json", "package.json"), ("auth/bun.lock", "bun.lock"), ("gateway/http.js", "shared/http.js"), ("url-signer/s3.js", "shared/s3.js")];

/// What stays in the repo: the relay's WebAssembly is built from `relay/protos/` by hand (`relay/build.sh`), so only what it
/// built ships.
const NOT_SHIPPED: [&str; 2] = ["relay/protos/", "relay/build.sh"];

/// @one/ui as `bun run build` in ui/kit made it, which the gateway serves beside the web client as `web/screen/ui/`.
const KIT: &str = "ui/kit/dist";

/// The bridge every screen has, and what a web page that holds screens does for them, served as `web/screen/<name>`.
const SCREEN_SCRIPTS: [&str; 2] = ["ui/bridge.js", "ui/host.js"];

/// Onboarding's auth pages, which the auth Worker imports as ../onboarding/pages.js.
const AUTH_PAGES: &str = "onboarding/pages.js";

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
    files.extend(collect(&root.join(KIT)).into_iter().map(|(rel, abs)| (format!("web/screen/ui/{rel}"), abs)));
    for script in SCREEN_SCRIPTS {
        files.push((format!("web/screen/{}", script.rsplit('/').next().unwrap_or(script)), root.join(script)));
    }
    files.push((AUTH_PAGES.into(), root.join(AUTH_PAGES)));
    // And the formats devices hold keys in, which the web client imports as ../shared/crypto.js.
    files.push(("web/shared/crypto.js".into(), cloudflare.join("shared/crypto.js")));
    files
}

/// What `list` reads, for a build to watch: every folder and file it takes from.
pub fn sources(root: &Path) -> Vec<PathBuf> {
    ["cloudflare", KIT, AUTH_PAGES].into_iter().chain(WORKERS.map(|(folder, _)| folder)).chain(SCREEN_SCRIPTS).map(|f| root.join(f)).collect()
}

/// Every folder of the client's own screens in React, each built into its `dist/`: the frame's (ui/home, ui/settings) and
/// each feature's (`<feature>/screens/`). An app's (apps/) are its own, deployed with it.
pub fn screen_folders(root: &Path) -> Vec<PathBuf> {
    let mut folders = vec![root.join("ui/home"), root.join("ui/settings")];
    let mut features: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && !["apps", "target", "node_modules"].contains(&&*e.file_name().to_string_lossy()))
        .map(|e| e.path().join("screens"))
        .filter(|p| p.is_dir())
        .collect();
    features.sort();
    folders.extend(features);
    folders
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
