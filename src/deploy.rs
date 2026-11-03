//! `one deploy <dir>` and the deploy tool: build an app with a pinned Bun, npm packages and all, into a bundle that
//! holds the built module and the files it was built from, as written.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};

/// The Bun every deploy builds with, and each platform's npm package of it with its sha512.
const BUN: &str = "1.4.4";
const PACKAGES: &[(&str, &str, &str)] = &[
    ("x86_64-unknown-linux-gnu", "linux-x64", "CHbbkmHVM28T1GFq+6GL4Ao4ujwwYwj1CxFFwEFzLanPfNsId9AhbIP0Q4reWuWBkGimUkoQygrK7Rgjtj3vog=="),
    ("aarch64-unknown-linux-gnu", "linux-aarch64", "ZTVtpf9zmHSBVPX1sBwe5agGb/rN34yPGg/Hby8e2Ax2uCUA2URZKlZH7qLntK7qaFJCMXh9v0GWNYN2zINz1w=="),
    ("aarch64-apple-darwin", "darwin-aarch64", "zW/hxwc8sJAILGwB5eTC4i48ouGXf0V4lrxmEOk3JIE6JkYVW1TZh1E2pIempqsezgW1sckTIIDudFON8bJ2Mw=="),
    ("x86_64-apple-darwin", "darwin-x64", "oKGcxLsvh4oHjAo28nPm8c/xN0o/OmGyj1IHYc94vprkWsW2hNg2keEZt5f40cI5Hvu4jDlSRTR0UNpv1UIAxw=="),
];

/// The files an app's build starts from, in this order.
const ENTRIES: &[&str] = &["app.ts", "app.js", "index.ts", "index.js"];

/// The pinned Bun, downloaded and checked the first time.
pub fn bun() -> Result<PathBuf> {
    let (_, package, integrity) =
        PACKAGES.iter().find(|(target, _, _)| *target == crate::config::TARGET).ok_or_else(|| anyhow!("deploying from {} isn't possible yet: Bun has no build for it", crate::config::TARGET))?;
    let url = format!("https://registry.npmjs.org/@oven/bun-{package}/-/bun-{package}-{BUN}.tgz");
    crate::apps::pinned(&format!("bun-{BUN}"), "bun", &url, integrity)
}

/// A browser with no window on this machine, for drawing screens (one try's web client, the tests), started as lean as
/// it still works: CHROME's, Playwright's headless shell (a third of Chrome's memory), the usual Chromes, or
/// Playwright's Chromium. Each app's frame keeps a process of its own, as in people's browsers, which the headless shell
/// doesn't do by itself and one try's script needs to reach the frames.
pub fn chrome() -> Option<std::process::Command> {
    let playwright = |prefix: &str, names: &[&str]| -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("HOME")?).join(".cache/ms-playwright");
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix))).collect();
        found.sort();
        found.into_iter().rev().flat_map(|p| names.iter().map(move |n| p.join(n))).find(|p| p.is_file())
    };
    let named = || {
        ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"]
            .iter()
            .find_map(|name| std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file()))
    };
    let mac = PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    let path = std::env::var_os("CHROME")
        .map(PathBuf::from)
        .or_else(|| playwright("chromium_headless_shell-", &["chrome-headless-shell-linux64/chrome-headless-shell", "chrome-linux/headless_shell"]))
        .or_else(named)
        .or_else(|| mac.is_file().then_some(mac))
        .or_else(|| playwright("chromium-", &["chrome-linux64/chrome", "chrome-linux/chrome"]))?;
    let mut command = std::process::Command::new(path);
    command.args([
        "--headless=new",
        "--site-per-process",
        "--disable-gpu",
        "--disable-features=SpareRendererForSitePerProcess,BackForwardCache",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-breakpad",
        "--mute-audio",
        "--no-first-run",
        "--no-default-browser-check",
    ]);
    Some(command)
}

/// Builds the app in `dir` into a bundle: `app.js`, the ES module Bun makes from its entry and npm packages, and
/// `src/`, the folder's files as written (without `node_modules` and dot files).
pub fn build(dir: &Path) -> Result<Vec<u8>> {
    let entry = ENTRIES.iter().find(|e| dir.join(e).is_file()).with_context(|| format!("{} has none of {}", dir.display(), ENTRIES.join(", ")))?;
    let bun = bun()?;
    let cache = bun.parent().unwrap().join("cache");
    let run = |args: &[&str], what: &str| -> Result<()> {
        let out = std::process::Command::new(&bun).args(args).current_dir(dir).env("BUN_INSTALL_CACHE_DIR", &cache).env("DO_NOT_TRACK", "1").output()?;
        if !out.status.success() {
            bail!("{what} failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        }
        Ok(())
    };
    // Nothing in the folder runs while it builds: no packages' scripts, no macros. What the deploy tool builds is
    // someone else's, maybe from elsewhere, on this machine.
    if dir.join("package.json").is_file() {
        run(&["install", "--ignore-scripts"], "bun install")?;
    }
    let out = tempdir()?;
    let outfile = out.join("app.js");
    let meta = out.join("meta.json");
    let built = run(
        &[
            "build",
            &format!("./{entry}"),
            "--target=browser",
            "--format=esm",
            "--no-macros",
            "--external=cloudflare:*",
            "--external=node:*",
            &format!("--outfile={}", outfile.display()),
            &format!("--metafile={}", meta.display()),
        ],
        "bun build",
    );
    let module = built.and_then(|_| {
        // Every file it read is the folder's (Bun names them from it, links followed): never /etc/… or ../….
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(&meta)?)?;
        for input in meta["inputs"].as_object().into_iter().flatten().map(|(path, _)| path) {
            let path = Path::new(input);
            ensure!(path.is_relative() && path.components().all(|c| matches!(c, std::path::Component::Normal(_))), "the app imports {input}, outside its folder: an app is built only from its own files and npm packages");
        }
        Ok(std::fs::read(&outfile)?)
    });
    let _ = std::fs::remove_dir_all(&out);
    let mut files = vec![("app.js".to_string(), module?)];
    files.extend(crate::bundle::sources(dir)?.into_iter().map(|(path, bytes)| (format!("src/{path}"), bytes)));
    crate::bundle::pack(&files)
}

/// An app's bundle, built from its files as `build` builds a folder, once per version of them: the build is kept in
/// the cache, by the files' hash.
pub fn build_files(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    for (path, bytes) in files {
        hash.update(format!("{path}\0{}\0", bytes.len()));
        hash.update(bytes);
    }
    let kept = crate::cache().join("built-in").join(format!("{}-{}.tar", data_encoding::HEXLOWER.encode(&hash.finalize()), BUN));
    if let Ok(bundle) = std::fs::read(&kept) {
        return Ok(bundle);
    }
    let dir = tempdir()?;
    let built = (|| {
        for (path, bytes) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, bytes)?;
        }
        build(&dir)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    let bundle = built?;
    std::fs::create_dir_all(kept.parent().unwrap())?;
    let part = kept.with_extension(format!("{}.part", rand::random::<u64>()));
    std::fs::write(&part, &bundle)?;
    std::fs::rename(&part, &kept)?;
    Ok(bundle)
}

/// An app's files as the deploy tool is given them, checked before they're built on this machine: paths inside the
/// folder, no dot files, lockfiles or Bun's settings, and npm packages only from npm.
pub fn given(files: &serde_json::Map<String, serde_json::Value>) -> Result<Vec<(String, Vec<u8>)>> {
    ensure!(!files.is_empty(), "give the app's files: {{ \"app.js\": \"…\" }}");
    let mut out = Vec::new();
    for (path, text) in files {
        let text = text.as_str().with_context(|| format!("{path}: give each file's text"))?;
        let p = Path::new(path);
        ensure!(
            p.is_relative() && p.components().all(|c| matches!(c, std::path::Component::Normal(n) if !n.to_string_lossy().starts_with('.') && n != "node_modules")),
            "{path}: a file's path is inside the app's folder, like app.js or screens/main.html, with no dot files or node_modules"
        );
        ensure!(!["bunfig.toml", "bun.lock", "bun.lockb", "package-lock.json", "yarn.lock", "pnpm-lock.yaml"].contains(&path.as_str()), "{path}: leave it out; packages come from package.json");
        if path == "package.json" {
            let package: serde_json::Value = serde_json::from_str(text).context("package.json isn't JSON")?;
            for key in ["overrides", "resolutions", "workspaces", "patchedDependencies", "catalog", "catalogs"] {
                ensure!(package.get(key).is_none(), "package.json: leave out {key}; give dependencies with versions from npm");
            }
            for key in ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"] {
                for (name, version) in package[key].as_object().into_iter().flatten() {
                    let version = version.as_str().unwrap_or("/");
                    ensure!(!version.contains([':', '/', '\\']), "package.json: {name} is {version}: packages come from npm, by version, like \"^2.1.0\"");
                }
            }
        }
        out.push((path.clone(), text.as_bytes().to_vec()));
    }
    ensure!(out.iter().any(|(p, _)| ENTRIES.contains(&p.as_str())), "give the app's module, one of {}", ENTRIES.join(", "));
    Ok(out)
}

fn tempdir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("{}-build-{}", crate::config::CLI, rand::random::<u64>()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The files of a bundle, or one of them: `{ files: [{ path, size }] }`, or `{ path, text }` (`{ path, base64 }` if
/// it isn't text).
pub fn show(bundle: &[u8], path: Option<&str>) -> Result<serde_json::Value> {
    let files = crate::bundle::unpack(bundle)?;
    Ok(match path {
        None => serde_json::json!({ "files": files.iter().map(|(p, b)| serde_json::json!({ "path": p, "size": b.len() })).collect::<Vec<_>>() }),
        Some(path) => {
            let (_, bytes) = files.into_iter().find(|(p, _)| p == path).with_context(|| format!("the bundle has no {path}"))?;
            match String::from_utf8(bytes) {
                Ok(text) => serde_json::json!({ "path": path, "text": text }),
                Err(e) => serde_json::json!({ "path": path, "base64": data_encoding::BASE64.encode(e.as_bytes()) }),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// What someone deploys from elsewhere is built on this machine: it reads nothing of it but the files given.
    #[test]
    fn a_build_reads_only_the_apps_own_files() {
        let files = |app_js: &str| super::given(json!({ "app.js": app_js }).as_object().unwrap()).unwrap();
        let build = |app_js: &str| super::build_files(&files(app_js).iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect::<Vec<_>>());
        for import in ["/etc/hostname", "../../../../../../etc/hostname"] {
            let err = build(&format!("import t from {import:?} with {{ type: \"text\" }};\nexport default {{ t }};")).unwrap_err();
            assert!(format!("{err:#}").contains("outside its folder"), "{err:#}");
        }
        let err = build("import { f } from \"./app.js\" with { type: \"macro\" };\nexport function g() { return 1; }\nexport default { a: f() };").unwrap_err();
        assert!(format!("{err:#}").contains("Macros are disabled"), "{err:#}");
        for (path, why) in [("../x.js", "inside the app's folder"), ("/tmp/x.js", "inside the app's folder"), (".npmrc", "no dot files"), ("bunfig.toml", "leave it out")] {
            let err = super::given(json!({ "app.js": "export default {}", path: "" }).as_object().unwrap()).unwrap_err();
            assert!(format!("{err:#}").contains(why), "{path}: {err:#}");
        }
        let package = json!({ "dependencies": { "secret": "file:/home" } }).to_string();
        assert!(super::given(json!({ "app.js": "export default {}", "package.json": package }).as_object().unwrap()).is_err());
        assert!(build("export default { ok: true };").is_ok());
    }
}
