//! `one deploy <dir>` and the deploy tool: build an app with a pinned Bun, npm packages and all, into a bundle that
//! holds the built module and the files it was built from, as written. An app with React screens has them built first,
//! with the deploying machine's own Bun or Node (`screens`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

/// The files an app's build starts from, in this order.
const ENTRIES: &[&str] = &["app.ts", "app.js", "index.ts", "index.js"];

/// Builds the app in `dir` into a bundle: `app.js`, the ES module Bun makes from its entry and npm packages, and
/// `src/`, the folder's files as written (without `node_modules` and dot files; what its links go to is in it), and
/// `services`, its service's builds by platform, as blobs (`bundle::SERVICES`).
pub fn build(dir: &Path, services: &std::collections::BTreeMap<String, String>) -> Result<Vec<u8>> {
    screens(dir)?;
    let sources = crate::app::bundle::sources(dir)?;
    let mut files = crate::app::bundle::unpack(&compile(&sources, false)?)?;
    if !services.is_empty() {
        files.insert(1, (crate::app::bundle::SERVICES.to_string(), serde_json::to_vec(services)?));
    }
    crate::app::bundle::pack(&files)
}

/// A folder's files built into a bundle, in a folder of their own: so only the files given are what it's built from,
/// never what's beside the app's folder, and it builds the same wherever the files came from.
fn compile(files: &[(String, Vec<u8>)], build_screens: bool) -> Result<Vec<u8>> {
    let dir = tempdir()?;
    let built = (|| {
        for (path, bytes) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, bytes)?;
        }
        // The screens it builds are among its files, as they're built.
        if build_screens {
            screens(&dir)?;
            return bundle(&dir, &crate::app::bundle::sources(&dir)?);
        }
        bundle(&dir, files)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    built
}

fn bundle(dir: &Path, files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let entry = ENTRIES.iter().find(|e| dir.join(e).is_file()).with_context(|| format!("{} has none of {}", dir.display(), ENTRIES.join(", ")))?;
    let bun = crate::build::bun()?;
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
        // Every file it read is the folder's, and its links are files of it by now (`bundle::sources`): never /etc/… or ../….
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(&meta)?)?;
        for input in meta["inputs"].as_object().into_iter().flatten().map(|(path, _)| path) {
            let path = Path::new(input);
            ensure!(path.is_relative() && path.components().all(|c| matches!(c, std::path::Component::Normal(_))), "the app imports {input}, outside its folder: an app is built only from its own files and npm packages");
        }
        Ok(std::fs::read(&outfile)?)
    });
    let _ = std::fs::remove_dir_all(&out);
    let mut packed = vec![("app.js".to_string(), module?)];
    packed.extend(files.iter().map(|(path, bytes)| (format!("src/{path}"), bytes.clone())));
    crate::app::bundle::pack(&packed)
}

/// The tools of a described app that don't say what they are (`title`) or what they answer (`output`, a JSON Schema),
/// each by name: the ones for its own screens (`_`) aren't asked to.
pub fn lacking(described: &serde_json::Value) -> Vec<String> {
    let tools = described["tools"].as_array().into_iter().flatten();
    tools.filter(|t| t["title"].is_null() || t["output"].is_null()).filter_map(|t| t["name"].as_str()).filter(|n| !n.starts_with('_')).map(str::to_string).collect()
}

/// What a deploy says when `lacking` names any, or nothing.
fn lacking_said(names: &[String]) -> Option<String> {
    (!names.is_empty()).then(|| format!("{} {} no title or no output: give each a title, and an output, the JSON Schema of its answer ({{ \"type\": \"string\" }} for text). The API's document and agents read them, and a tool that answers what its output doesn't say fails.", names.join(", "), if names.len() == 1 { "has" } else { "have" }))
}

/// What a deploy did, and what's next, as `reader` would do it. `node` says whether the app is on a page of the caller's.
pub fn said(node: &crate::api::Client, app: &str, r: &serde_json::Value, reader: crate::core_tools::Reader) -> String {
    use crate::config::CLI;
    use crate::core_tools::Reader;
    let short = |b: &serde_json::Value| b.as_str().unwrap_or("").chars().take(12).collect::<String>();
    let added = node.get("/v1/apps").ok().and_then(|a| a.as_array()?.iter().find(|a| a["name"] == app).map(|a| a["added"].as_array().is_some_and(|i| !i.is_empty()))) == Some(true);
    let tools: Vec<String> = r["tools"].as_array().into_iter().flatten().filter_map(|t| t.as_str()).map(|t| match reader { Reader::Cli => format!("{CLI} {app} {t}"), _ => format!("{app}_{t}") }).collect();
    let add = match reader { Reader::Cli => format!("{CLI} add {app}"), _ => format!("add {{ app: \"{app}\", scope: \"user\" }}") };
    let mut out = vec![format!("Deployed {app} ({}){}.", short(&r["bundle"]), if r["made"] == true { ", a new app of yours" } else { "" })];
    match (r["failed"].as_str(), r["here"] == true) {
        (Some(why), _) => out.push(format!("{why}. Fix it and deploy again.")),
        (_, false) => out.push("Your devices take it within a few seconds: until then, its calls say they don't have its latest version yet.".into()),
        _ if !added => out.push(format!("Nobody sees it until it's on a page: {add} puts it on yours.")),
        _ if tools.is_empty() => out.push("It's running, and your other devices take it within a few seconds.".into()),
        _ => out.push(format!("It's running, and your other devices take it within a few seconds. Its tools: {}.", tools.join(", "))),
    }
    let lacking: Vec<String> = r["lacking"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(str::to_string)).collect();
    out.extend(lacking_said(&lacking));
    out.push(match (&r["before"], reader) {
        (b, _) if *b == r["bundle"] => "It ran this version already.".into(),
        (serde_json::Value::String(_), Reader::Cli) => format!("To undo: {CLI} deploy {app} --version {}.", short(&r["before"])),
        (serde_json::Value::String(_), _) => format!("To undo: deploy {{ app: \"{app}\", version: \"{}\" }}.", short(&r["before"])),
        (_, Reader::Cli) => format!("To undo: {CLI} apps delete {app}."),
        _ => "To undo: apps_delete.".into(),
    });
    out.join("\n")
}

/// Whether an app's files, by path, have a React screen: `screens/<name>.jsx` or `.tsx`, which `screens` builds.
pub fn has_screens<'a>(paths: impl IntoIterator<Item = &'a str>) -> bool {
    paths.into_iter().any(|p| p.strip_prefix("screens/").is_some_and(screen_name))
}

/// A React screen's file name: lowercase words joined by -, then `.jsx` or `.tsx`.
fn screen_name(file: &str) -> bool {
    let stem = file.strip_suffix(".jsx").or_else(|| file.strip_suffix(".tsx")).unwrap_or("");
    !stem.is_empty() && stem.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Builds the app's React screens, if it has any: each `screens/<name>.jsx` or `.tsx` into `dist/<name>.html`, the
/// screen's HTML that app.js imports as text (`with { type: "text" }`), opted in to @one/ui, with its own styles and
/// module in it and React and @one/ui left to the page (src/screen.rs). Built by this machine's Bun, or else Node: the
/// pinned Bun is only for bundling app.js, not for running build tools.
fn screens(dir: &Path) -> Result<()> {
    let some = std::fs::read_dir(dir.join("screens")).into_iter().flatten().flatten().any(|e| screen_name(&e.file_name().to_string_lossy()));
    if !some {
        return Ok(());
    }
    // One that runs: a version manager's shim (mise's, asdf's) can be on PATH with no version behind it.
    let runs = |name: &str| {
        let path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|d| d.join(name)).find(|p| p.is_file())?;
        let ran = std::process::Command::new(&path).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        ran.is_ok_and(|s| s.success()).then_some(path)
    };
    // Bun runs the build and installs its tools; Node runs it, and npm, which comes with it, installs them.
    let (runtime, install): (PathBuf, (PathBuf, &[&str])) = match (runs("bun"), runs("node"), runs("npm")) {
        (Some(bun), _, _) => (bun.clone(), (bun, &["install", "--frozen-lockfile", "--ignore-scripts"])),
        (None, Some(node), Some(npm)) => (node, (npm, &["install", "--ignore-scripts", "--no-audit", "--no-fund"])),
        _ => bail!(
            "{}'s React screens (screens/*.jsx) are built on the machine that deploys it, which needs Bun (bun.sh, recommended) or Node (nodejs.org): install one and deploy again",
            dir.canonicalize().ok().and_then(|d| Some(d.file_name()?.to_string_lossy().to_string())).unwrap_or_else(|| "this app".into())
        ),
    };
    let tools = screen_tools(&install.0, install.1)?;
    let shared: serde_json::Value = serde_json::from_str(include_str!("../screen/ui/dist/importmap.json"))?;
    let shared: Vec<&String> = shared["imports"].as_object().context("@one/ui's import map has no imports")?.keys().collect();
    let out = std::process::Command::new(&runtime)
        .arg(tools.join("screens.mjs"))
        .arg(dir.canonicalize()?.join("screens"))
        .arg(dir.canonicalize()?.join("dist"))
        .arg(dir.canonicalize()?)
        .arg(crate::screen::UI)
        .args(shared)
        .current_dir(&tools)
        .env("DO_NOT_TRACK", "1")
        .output()
        .with_context(|| format!("couldn't run {}", runtime.display()))?;
    if !out.status.success() {
        bail!("building its React screens failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// The tools that build React screens, as @one/ui is built with them (its package.json, and bun.lock where Bun installs
/// them), installed by `install` once per version of them, beside @one/ui's `screens.mjs`, which builds them, and the
/// theme's names it imports, where they are in @one/ui.
fn screen_tools(installer: &Path, install: &[&str]) -> Result<PathBuf> {
    use sha2::Digest;
    let files: [(&str, &str); 4] = [
        ("package.json", include_str!("../screen/ui/package.json")),
        ("bun.lock", include_str!("../screen/ui/bun.lock")),
        ("src/theme.css", include_str!("../screen/ui/src/theme.css")),
        ("screens.mjs", include_str!("../screen/ui/screens.mjs")),
    ];
    let mut hash = sha2::Sha256::new();
    for (path, text) in files {
        hash.update(format!("{path}\0{}\0", text.len()));
        hash.update(text);
    }
    let dir = crate::cache().join(format!("screen-tools-{}", &data_encoding::HEXLOWER.encode(&hash.finalize())[..16]));
    if dir.join("node_modules").is_dir() {
        return Ok(dir);
    }
    // Installed beside it, then moved in whole, so a build never finds it half installed.
    let part = dir.with_extension(format!("part-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&part)?;
    let installed = (|| {
        for (path, text) in files {
            std::fs::create_dir_all(part.join(path).parent().unwrap_or(&part))?;
            std::fs::write(part.join(path), text)?;
        }
        let out = std::process::Command::new(installer).args(install).current_dir(&part).env("DO_NOT_TRACK", "1").output().with_context(|| format!("couldn't run {}", installer.display()))?;
        if !out.status.success() {
            bail!("installing the tools that build React screens failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        }
        Ok(())
    })();
    if installed.is_err() || std::fs::rename(&part, &dir).is_err() {
        let _ = std::fs::remove_dir_all(&part);
    }
    installed.map(|_| dir)
}

/// An app's bundle, built from its files as `build` builds a folder, once per version of them: the build is kept in
/// the cache, by the files' hash.
pub fn build_files(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    build_kept(files, true)
}

/// A built-in app's files built the same way, its React screens as its `dist/` has them: a release is built where they
/// were, and no node needs Bun or Node to run them.
pub fn build_built_in(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    build_kept(files, false)
}

fn build_kept(files: &[(&str, &[u8])], build_screens: bool) -> Result<Vec<u8>> {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    for (path, bytes) in files {
        hash.update(format!("{path}\0{}\0", bytes.len()));
        hash.update(bytes);
    }
    let kept = crate::cache().join("built-in").join(format!("{}-{}{}.tar", data_encoding::HEXLOWER.encode(&hash.finalize()), crate::build::BUN, if build_screens { "" } else { "-prebuilt" }));
    if let Ok(bundle) = std::fs::read(&kept) {
        return Ok(bundle);
    }
    let files: Vec<(String, Vec<u8>)> = files.iter().map(|(path, bytes)| (path.to_string(), bytes.to_vec())).collect();
    let built = compile(&files, build_screens);
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
    let files = crate::app::bundle::unpack(bundle)?;
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
