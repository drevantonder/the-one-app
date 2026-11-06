//! Building and publishing: the platforms a release is for, a service built for them, the WebAssembly the node carries, and
//! the one way commands are run. `update`, `deploy`, `services try`, `publish` and `throwaway` all build through here.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use one::config::{CLI, TARGET};
use one::release::Version;
use serde_json::{Value, json};

/// A platform a release is for: what installers and nodes name it, and what Go and a person do.
pub struct Platform {
    /// Rust's target triple, which a node and a release name it by.
    pub target: &'static str,
    /// Go's `GOOS` and `GOARCH`.
    pub os: &'static str,
    pub arch: &'static str,
    pub said: &'static str,
}

/// Platforms the installer knows. A release has each one some device here can build.
pub static PLATFORMS: [Platform; 3] = [
    Platform { target: "x86_64-unknown-linux-gnu", os: "linux", arch: "amd64", said: "Linux" },
    Platform { target: "aarch64-unknown-linux-gnu", os: "linux", arch: "arm64", said: "Linux on ARM" },
    Platform { target: "aarch64-apple-darwin", os: "darwin", arch: "arm64", said: "Mac" },
];

impl Platform {
    /// The platform of a device that says `uname -sm` (`Linux x86_64`).
    pub fn of_uname(uname: &str) -> Option<&'static Platform> {
        let (os, arch) = uname.split_once(' ')?;
        let arch = match arch.trim() {
            "x86_64" | "amd64" => "amd64",
            "aarch64" | "arm64" => "arm64",
            _ => return None,
        };
        PLATFORMS.iter().find(|p| p.os == os.to_lowercase() && p.arch == arch)
    }

    pub fn is_mac(target: &str) -> bool {
        target.ends_with("-apple-darwin")
    }

    /// What install.sh downloads on it: the program, on a Mac the app, which holds it.
    pub fn asset(&self) -> String {
        if Platform::is_mac(self.target) { format!("{CLI}-macos.zip") } else { format!("{CLI}-{}", self.target) }
    }
}

/// How a service's program is built: its folder has a Go module or a Rust package.
fn is_rust(source: &Path) -> Option<bool> {
    if source.join("Cargo.toml").is_file() {
        Some(true)
    } else {
        source.join("go.mod").is_file().then_some(false)
    }
}

/// Whether the app in `dir` has a service (`service/go.mod` or `service/Cargo.toml`).
pub fn has_service(dir: &Path) -> bool {
    is_rust(&dir.join("service")).is_some()
}

/// What install.sh sets as `name` (`repo`, `cli`, `key`): it is shell, which can't read config.toml, so it says it again.
fn installer_says(name: &str) -> Option<&'static str> {
    include_str!("../install.sh").lines().find_map(|l| l.strip_prefix(&format!("{name}="))).map(|v| v.trim_matches('"'))
}

/// Whether install.sh, which gets releases from GitHub, installs this custom build. Only that custom build is released there.
pub fn installer_is_ours() -> bool {
    installer_says("cli") == Some(CLI) && installer_says("repo") == Some(one::config::REPO)
}

/// What `build_service` made: each program by its platform, and the platforms it couldn't build for.
pub struct Built {
    pub made: Vec<(&'static Platform, PathBuf)>,
    pub missed: Vec<&'static Platform>,
}

/// The commit of the checkout `dir` is in, as services say what they were built from; `+dirty` after it if `dirty` and it
/// has changes.
pub fn built_from(dir: &Path, dirty: bool) -> Result<String> {
    let commit = git(dir, &["rev-parse", "--short=12", "HEAD"])?;
    let changed = dirty && !succeeds(Command::new("git").current_dir(dir).args(["diff", "--quiet", "HEAD", "--", "."]));
    Ok(if changed { format!("{commit}+dirty") } else { commit })
}

/// Builds the service of the app in `app` (its `service/`), saying it was built from `built`, at `out`/<target> for each
/// of `platforms` this machine can: a service in Go for all of them, one in Rust for this machine's. Nodes never build.
/// None if the app has no service.
pub fn build_service(app: &Path, built: &str, platforms: impl IntoIterator<Item = &'static Platform>, out: &Path) -> Result<Option<Built>> {
    let source = app.join("service");
    let Some(rust) = is_rust(&source) else { return Ok(None) };
    std::fs::create_dir_all(out)?;
    let mut done = Built { made: Vec::new(), missed: Vec::new() };
    for platform in platforms {
        if rust && platform.target != TARGET {
            done.missed.push(platform);
            continue;
        }
        let program = out.join(platform.target);
        if rust {
            // Cargo says where it put the program it built.
            let o = Command::new(cargo())
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

/// Has the node publish the program at `path` as a release, signed with `key`: this node's (`service` none) or that
/// service's. → what it answered.
pub fn publish(path: &Path, key: &ssh_key::PrivateKey, version: &Version, service: Option<&str>) -> Result<Value> {
    if version.commit.ends_with("+dirty") {
        bail!("that build has uncommitted changes; commit, build again, then publish");
    }
    let signature = one::release::sign(key, &std::fs::read(path)?)?;
    let body = json!({ "path": path, "commit": version.commit, "target": version.target, "committed": version.committed, "signature": signature, "service": service });
    crate::post(if service.is_some() { "/v1/service-release" } else { "/v1/node-release" }, body)
}

/// What publishing says, from what the node answered.
pub fn published_said(published: &Value, service: Option<&str>) -> String {
    let (commit, target) = (published["commit"].as_str().unwrap_or(""), published["target"].as_str().unwrap_or(""));
    match service {
        Some(service) => format!("Published {service} {commit} for {target}. Every device on that platform that runs it takes it."),
        None => format!("Published {commit} for {target}. Every device on that platform updates itself now."),
    }
}

/// WebAssembly built by cargo, and the JavaScript glue `wasm-bindgen` makes for it.
struct Wasm<'a> {
    /// What it is, for what to install when it can't build.
    what: &'a str,
    /// Cargo's arguments, after the program.
    cargo: &'a [&'a str],
    /// Where cargo leaves it.
    wasm: PathBuf,
    /// Where the glue and the program go, and what it's called there.
    out: PathBuf,
    glued: &'a str,
    /// Make it smaller still with wasm-opt, where it's installed.
    shrink: bool,
}

impl Wasm<'_> {
    /// → the folder it's in. It needs Rust's wasm32-unknown-unknown target, a clang that builds for it (SQLite is C), and the
    /// wasm-bindgen of the version it's built with. Its glue is only made again for what cargo built again: a build that
    /// holds it (`CLOUDFLARE_NODE`, `CLIENT`) watches these files, so rewriting them would have that build done again.
    fn build(&self, src: &Path, config: &Path) -> Result<PathBuf> {
        quietly(Command::new(cargo()).current_dir(src).env("CONFIG", config).args(self.cargo)).with_context(|| format!("build the {} (rustup target add wasm32-unknown-unknown)", self.what))?;
        let modified = |path: &Path| path.metadata().and_then(|m| m.modified()).ok();
        let glued = self.out.join(self.glued);
        if modified(&glued).is_some_and(|glued| modified(&self.wasm).is_some_and(|built| glued > built)) {
            return Ok(self.out.clone());
        }
        quietly(Command::new("wasm-bindgen").args(["--target", "web", "--out-dir"]).arg(&self.out).arg(&self.wasm)).context("make its glue (cargo install wasm-bindgen-cli, the version in Cargo.lock)")?;
        if self.shrink && succeeds(Command::new("wasm-opt").arg("--version")) {
            let flags = ["-Oz", "--enable-bulk-memory", "--enable-nontrapping-float-to-int", "--enable-sign-ext", "--enable-mutable-globals", "--enable-reference-types", "--enable-multivalue", "-o"];
            quietly(Command::new("wasm-opt").args(flags).arg(&glued).arg(&glued))?;
        }
        Ok(self.out.clone())
    }
}

/// Builds the Cloudflare node (`cloudflare/node/`) into WebAssembly and its JavaScript glue, for the release to carry: → the
/// folder they're in. `profile` is cargo's: a release's is `release`, a throwaway network's `dev`.
pub fn cloudflare_node(src: &Path, config: &Path, profile: &str) -> Result<PathBuf> {
    let dir = if profile == "dev" { "debug" } else { profile };
    let (wasm, out) = (src.join("target/wasm32-unknown-unknown").join(dir).join("one_cloudflare_node.wasm"), src.join("target/cloudflare-node"));
    Wasm { what: "Cloudflare node", cargo: &["build", "--profile", profile, "-p", "one-cloudflare-node", "--target", "wasm32-unknown-unknown"], wasm, out, glued: "one_cloudflare_node_bg.wasm", shrink: false }.build(src, config)
}

/// Builds the client (`client/`) into WebAssembly for browsers, as small as it builds, and its JavaScript glue: → the
/// folder they're in.
pub fn client(src: &Path, config: &Path) -> Result<PathBuf> {
    let (wasm, out) = (src.join("target/wasm32-unknown-unknown/web/one_client.wasm"), src.join("target/client"));
    Wasm { what: "client", cargo: &["rustc", "-p", "one-client", "--profile", "web", "--target", "wasm32-unknown-unknown", "--crate-type", "cdylib"], wasm, out, glued: "one_client_bg.wasm", shrink: true }.build(src, config)
}

/// A tool by its name, or where it's kept when a shell doesn't have it on its PATH: `probe` is an argument that makes it
/// answer, and `kept` where it is under the home folder.
fn tool(name: &str, probe: &str, kept: &str) -> PathBuf {
    let kept = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(kept);
    if succeeds(Command::new(name).arg(probe)) || !kept.exists() { name.into() } else { kept }
}

/// Cargo, also where rustup puts it.
pub fn cargo() -> PathBuf {
    tool("cargo", "--version", ".cargo/bin/cargo")
}

/// Go, also where mise's shims put it.
fn go() -> PathBuf {
    tool("go", "version", ".local/share/mise/shims/go")
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    ensure!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Runs a command, showing its output only if it fails.
pub fn quietly(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("run {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{} failed:\n{}{}", said(cmd), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

/// Runs a command, showing its output.
pub fn show(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("run {:?}", cmd.get_program()))?;
    ensure!(status.success(), "{} failed", said(cmd));
    Ok(())
}

/// Whether a command succeeds, saying nothing.
pub fn succeeds(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn said(cmd: &Command) -> String {
    format!("{:?} {}", cmd.get_program(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// install.sh is shell, so it can't read the table: it must say what the table does.
    #[test]
    fn the_installer_is_for_the_platforms_here() {
        let installer = include_str!("../install.sh");
        // A custom build's installer is its own: when it's this build's, it names the same repo and release key.
        if installer_says("cli") == Some(CLI) {
            assert_eq!(installer_says("repo"), Some(one::config::REPO));
            assert_eq!(installer_says("key"), Some(one::config::RELEASE_KEY));
        }
        for p in &PLATFORMS {
            let (arch, rest) = p.target.split_once('-').unwrap();
            let fetched = if Platform::is_mac(p.target) { "$cli-macos.zip".to_string() } else { format!("$cli-$arch-{}", rest) };
            assert!(installer.contains(&format!("fetch \"{fetched}\"")), "install.sh fetches {fetched} for {}", p.target);
            assert_eq!(p.asset(), fetched.replace("$cli", CLI).replace("$arch", arch));
            assert!(installer.contains(&format!("arch={arch}")), "install.sh knows the processor {arch}");
        }
    }

    #[test]
    fn a_device_is_a_platform_by_what_it_says() {
        assert_eq!(Platform::of_uname("Linux x86_64").unwrap().target, "x86_64-unknown-linux-gnu");
        assert_eq!(Platform::of_uname("Darwin arm64").unwrap().target, "aarch64-apple-darwin");
        assert_eq!(Platform::of_uname("Linux aarch64").unwrap().target, "aarch64-unknown-linux-gnu");
        assert!(Platform::of_uname("Linux riscv64").is_none());
    }
}
