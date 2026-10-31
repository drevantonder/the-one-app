//! `one deploy <dir>`: builds an app with a pinned Bun, npm packages and all, into a bundle that holds the built module
//! and the files it was built from, as written. See SPEC.md, "Apps".

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

fn bun() -> Result<PathBuf> {
    let (_, package, integrity) =
        PACKAGES.iter().find(|(target, _, _)| *target == crate::brand::TARGET).ok_or_else(|| anyhow!("deploying from {} isn't possible yet: Bun has no build for it", crate::brand::TARGET))?;
    let url = format!("https://registry.npmjs.org/@oven/bun-{package}/-/bun-{package}-{BUN}.tgz");
    crate::apps::pinned(&format!("bun-{BUN}"), "bun", &url, integrity)
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

fn tempdir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("{}-build-{}", crate::brand::CLI, rand::random::<u64>()));
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

#[test]
fn shows_a_bundles_files() {
    let bundle = crate::bundle::pack(&[("app.js".into(), b"export default {}".to_vec()), ("src/app.ts".into(), b"export default {} as any".to_vec())]).unwrap();
    assert_eq!(show(&bundle, None).unwrap()["files"][1]["path"], "src/app.ts");
    assert_eq!(show(&bundle, Some("src/app.ts")).unwrap()["text"], "export default {} as any");
    assert!(show(&bundle, Some("nope")).is_err());
}
