//! `one deploy <dir>`: builds an app with a pinned Bun, npm packages and all, into a bundle that holds the built module
//! and the files it was built from, as written.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

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

/// Chrome or Chromium on this machine, for drawing screens (one try's web client, the tests): CHROME's, the usual ones, or
/// Playwright's.
pub fn chrome() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CHROME") {
        return Some(path.into());
    }
    let named = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"].iter().find_map(|name| {
        std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
    });
    let mac = PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    let playwright = || -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("HOME")?).join(".cache/ms-playwright");
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("chromium-"))).collect();
        found.sort();
        found.into_iter().rev().flat_map(|p| [p.join("chrome-linux64/chrome"), p.join("chrome-linux/chrome")]).find(|p| p.is_file())
    };
    named.or_else(|| mac.is_file().then_some(mac)).or_else(playwright)
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
    if dir.join("package.json").is_file() {
        run(&["install"], "bun install")?;
    }
    let out = tempdir()?;
    let outfile = out.join("app.js");
    let built = run(
        &["build", &format!("./{entry}"), "--target=browser", "--format=esm", "--external=cloudflare:*", "--external=node:*", &format!("--outfile={}", outfile.display())],
        "bun build",
    );
    let module = built.and_then(|_| Ok(std::fs::read(&outfile)?));
    let _ = std::fs::remove_dir_all(&out);
    let mut files = vec![("app.js".to_string(), module?)];
    files.extend(crate::bundle::sources(dir)?.into_iter().map(|(path, bytes)| (format!("src/{path}"), bytes)));
    crate::bundle::pack(&files)
}

/// A built-in app's bundle, built from its files as `build` builds a folder, once per version of them: the build is
/// kept in the cache, by the files' hash.
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
