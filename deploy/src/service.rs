//! An app's service built where it's deployed from, for each platform this machine can build it for: in Go for all of
//! them, in Rust for this machine's own. Nodes never build.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};
use custom_build::config::{CLI, TARGET};
use oned_update::platform::{PLATFORMS, Platform};
use platform::os::{cargo, git, go, quietly};
use serde_json::Value;

/// How a service's program is built: its folder has a Go module or a Rust package.
fn is_rust(source: &Path) -> Option<bool> {
    if source.join("Cargo.toml").is_file() {
        Some(true)
    } else {
        source.join("go.mod").is_file().then_some(false)
    }
}

/// What `build_service` made: each program by its platform, and the platforms it couldn't build for.
pub struct Built {
    pub made: Vec<(&'static Platform, PathBuf)>,
    pub missed: Vec<&'static Platform>,
}

/// The commit of the checkout `dir` is in, as services say what they were built from.
pub fn built_from(dir: &Path) -> Result<String> {
    git(dir, &["rev-parse", "--short=12", "HEAD"])
}

/// Builds the service of the app in `app` (its `service/`), saying it was built from `built`, at `out`/<target> for each
/// of `platforms` this machine can: a service in Go for all of them, one in Rust for this machine's. Nodes never build.
/// None if the app has no service.
pub fn build_service(app: &Path, built: &str, platforms: impl IntoIterator<Item = &'static Platform>, out: &Path) -> Result<Option<Built>> {
    let source = std::fs::canonicalize(app).with_context(|| format!("{} isn't here", app.display()))?.join("service");
    let Some(rust) = is_rust(&source) else { return Ok(None) };
    std::fs::create_dir_all(out)?;
    let out = &std::fs::canonicalize(out)?;
    let mut done = Built { made: Vec::new(), missed: Vec::new() };
    for platform in platforms.into_iter().filter(|p| p.runs_services()) {
        if rust && platform.target != TARGET {
            done.missed.push(platform);
            continue;
        }
        let program = out.join(platform.target);
        if rust {
            let o = cargo()
                .current_dir(&source)
                .env("ONE_BUILD", built)
                .args(["build", "--release", "--message-format=json-render-diagnostics", "--manifest-path"])
                .arg(source.join("Cargo.toml"))
                .output()?;
            ensure!(o.status.success(), "its service didn't build:\n{}", String::from_utf8_lossy(&o.stderr));
            let said = String::from_utf8_lossy(&o.stdout);
            let executable = said.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|m| m["executable"].as_str().map(PathBuf::from)).last();
            std::fs::copy(executable.context("its service's crate builds no program")?, &program)?;
        } else {
            let mut go = Command::new(go());
            go.current_dir(&source).env("CGO_ENABLED", "0").env("GOOS", platform.os).env("GOARCH", platform.arch);
            go.args(["build", "-trimpath", "-ldflags", &format!("-X main.built={built}"), "-o"]).arg(&program).arg(".");
            quietly(&mut go).with_context(|| format!("build its service for {} (Go: go.dev/doc/install)", platform.said))?;
        }
        done.made.push((platform, program));
    }
    ensure!(!done.made.is_empty(), "its service is in Rust, which this machine ({TARGET}) builds for no platform a node runs: deploy it from Linux or a Mac");
    Ok(Some(done))
}

/// The app in `dir` built into a bundle, as a blob of `node`'s, with its service built here for each platform this
/// machine can build it for, each build a blob its bundle names: → the bundle, and what was built of its service.
pub fn built_with_service(node: &::api::Client, dir: &Path) -> Result<(String, String)> {
    let mut builds = std::collections::BTreeMap::new();
    let mut service = String::new();
    let out = std::env::temp_dir().join(format!("{CLI}-service-{}", std::process::id()));
    let built = build_service(dir, &built_from(&dir.join("service")).unwrap_or_default(), PLATFORMS.iter(), &out).and_then(|built| {
        if let Some(built) = &built {
            for (platform, program) in &built.made {
                builds.insert(platform.target.to_string(), node.put_blob(&std::fs::read(program)?)?);
            }
        }
        Ok(built)
    });
    let _ = std::fs::remove_dir_all(&out);
    if let Some(built) = built? {
        let said = |platforms: Vec<&Platform>| platforms.iter().map(|p| p.said).collect::<Vec<_>>().join(", ");
        service = format!(" Its service is built for {}.", said(built.made.iter().map(|(p, _)| *p).collect()));
        if !built.missed.is_empty() {
            service += &format!(" Not for {}: this machine can't build it for them, so deploy it again from one that can.", said(built.missed));
        }
    }
    Ok((node.put_blob(&crate::build::build(dir, &builds)?)?, service))
}

/// MCP's deploy tool's files built into a bundle on this machine, as a blob of `node`'s: for React screens and a
/// service in Go, which only the machine that deploys builds. → the bundle, and what was built of its service.
pub fn built_here(node: &::api::Client, files: &serde_json::Map<String, Value>) -> Result<(String, String)> {
    ensure!(!files.contains_key("service/Cargo.toml"), "its service is in Rust, whose build would run its build scripts here: deploy it with {CLI} deploy <its folder>");
    let files = crate::build::given(files)?;
    if !files.iter().any(|(p, _)| p == "service/go.mod") {
        return Ok((node.put_blob(&crate::build::build_files(&files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect::<Vec<_>>())?)?, String::new()));
    }
    let dir = std::env::temp_dir().join(format!("{CLI}-deploy-{}", std::process::id()));
    let built = (|| {
        for (path, bytes) in &files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap_or(&dir))?;
            std::fs::write(path, bytes)?;
        }
        built_with_service(node, &dir)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    built
}
