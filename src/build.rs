//! Building and publishing: the platforms a release is for, a service built for them, the WebAssembly the node carries, and
//! the one way commands are run. `update`, `deploy` and `throwaway` all build through here.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail, ensure};
use crate::config::{CLI, TARGET};
use crate::release::Version;
use serde_json::{Value, json};

/// A platform a release is for: what installers and nodes name it, and what Go and a user do.
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
    for platform in platforms {
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

/// Has the node publish the program at `path` as this node's build (`service` none) or that service's, in the record
/// at the network's address. → what it said.
pub fn publish(node: &crate::api::Client, path: &Path, key: &ssh_key::PrivateKey, version: &Version, service: Option<&str>) -> Result<Value> {
    if version.commit.ends_with("+dirty") {
        bail!("that build has uncommitted changes; commit, build again, then publish");
    }
    let name = match service {
        Some(service) => crate::latest::Thing::service_name(service, &version.target),
        None => crate::latest::Thing::node_name(&version.target),
    };
    publish_latest(node, key, &[(name, Some(path.to_path_buf()), version.clone(), "-".to_string())])?;
    Ok(json!({ "commit": version.commit, "target": version.target }))
}

/// Puts `things`' files where devices take them, and adds them to the record at the network's address, signed with the
/// release key: each `(name, file, version, config commit)`, with no file for what's built, not fetched (`source`).
pub fn publish_latest(node: &crate::api::Client, key: &ssh_key::PrivateKey, things: &[(String, Option<PathBuf>, Version, String)]) -> Result<()> {
    use crate::latest::{Latest, Thing};
    let mut made = Vec::new();
    for (name, path, version, config) in things {
        let (sha256, size) = match path {
            None => ("-".to_string(), 0),
            Some(path) => {
                let put = node.post("/v1/latest/put", json!({ "path": path }))?;
                (put["sha256"].as_str().context("no hash")?.to_string(), put["size"].as_u64().unwrap_or(0))
            }
        };
        made.push(Thing { name: name.clone(), commit: version.commit.clone(), committed: version.committed, sha256, size, config: config.clone() });
    }
    for _ in 0..16 {
        let now = node.get("/v1/latest")?;
        let here = now["text"].as_str().and_then(|t| Latest::read(crate::config::RELEASE_KEY, t).ok()).unwrap_or_default();
        let text = here.merged(made.iter().cloned()).signed(key)?;
        if node.post("/v1/latest", json!({ "text": text, "tag": now["tag"] }))?["replaced"] == true {
            return Ok(());
        }
    }
    bail!("couldn't update the record: another publisher kept changing it")
}

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
    cargo: &'a [&'a str],
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
    /// holds it (`CLOUDFLARE_NODE`, `ONET`) watches these files, so rewriting them would have that build done again.
    fn build(&self, src: &Path, config: &Path) -> Result<PathBuf> {
        quietly(cargo().current_dir(src).env("CONFIG", config).args(self.cargo)).with_context(|| format!("build the {} (rustup target add wasm32-unknown-unknown)", self.what))?;
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

/// Builds onet (`onet/`) into WebAssembly for browsers, as small as it builds, and its JavaScript glue: → the
/// folder they're in.
pub fn onet(src: &Path, config: &Path) -> Result<PathBuf> {
    let (wasm, out) = (src.join("target/wasm32-unknown-unknown/web/onet.wasm"), src.join("target/onet"));
    Wasm { what: "onet", cargo: &["rustc", "-p", "onet", "--profile", "web", "--target", "wasm32-unknown-unknown", "--crate-type", "cdylib"], wasm, out, glued: "onet_bg.wasm", shrink: true }.build(src, config)
}

/// A tool by its name, or where it's kept when a shell doesn't have it on its PATH: `probe` is an argument that makes it
/// answer, and `kept` where it is under the home folder.
fn tool(name: &str, probe: &str, kept: &str) -> PathBuf {
    let kept = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(kept);
    if succeeds(Command::new(name).arg(probe)) || !kept.exists() { name.into() } else { kept }
}

/// Cargo, also where rustup puts it, with sccache findable too: `.cargo/config.toml` names it bare, and a shell without it on
/// its PATH (the ssh that runs a Linux device's update) gets it where mise's shims put it.
pub fn cargo() -> Command {
    cargo_behind(None)
}

/// `cargo()` at a lower priority.
pub fn nice_cargo() -> Command {
    cargo_behind(Some("nice"))
}

fn cargo_behind(wrapper: Option<&str>) -> Command {
    let cargo = tool("cargo", "--version", ".cargo/bin/cargo");
    let mut command = match wrapper {
        Some(wrapper) => {
            let mut command = Command::new(wrapper);
            command.arg(cargo);
            command
        }
        None => Command::new(cargo),
    };
    if let Some(dir) = tool("sccache", "--version", ".local/share/mise/shims/sccache").parent().filter(|dir| !dir.as_os_str().is_empty()) {
        let path = std::env::var_os("PATH").unwrap_or_default();
        command.env("PATH", std::env::join_paths(std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&path))).unwrap_or(path));
    }
    command
}

/// Go, also where mise's shims put it.
pub fn go() -> PathBuf {
    tool("go", "version", ".local/share/mise/shims/go")
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").current_dir(dir).args(args).output()?;
    ensure!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn quietly(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("run {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{} failed:\n{}{}", said(cmd), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

pub fn show(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("run {:?}", cmd.get_program()))?;
    ensure!(status.success(), "{} failed", said(cmd));
    Ok(())
}

pub fn succeeds(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn said(cmd: &Command) -> String {
    format!("{:?} {}", cmd.get_program(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mise.toml installs what the build shells out to, and three of its versions are the repo's already: a machine set up from
    /// it must build with the Bun deploys download, the wasm-bindgen the client's WebAssembly links, and the Go the service asks for.
    #[test]
    fn mise_installs_the_versions_the_repo_asks_for() {
        let mise = include_str!("../mise.toml");
        let pinned = |tool: &str| mise.lines().find_map(|l| l.strip_prefix(&format!("{tool} = \""))?.strip_suffix('"')).unwrap_or_else(|| panic!("mise.toml pins no {tool}"));
        assert_eq!(pinned("bun"), BUN);
        let locked = include_str!("../Cargo.lock").split_once("name = \"wasm-bindgen\"\nversion = \"").expect("wasm-bindgen is locked").1.split('"').next().unwrap();
        assert_eq!(pinned("\"github:wasm-bindgen/wasm-bindgen\""), locked, "wasm-bindgen-cli must be the version Cargo.lock locks");
        let go = include_str!("../apps/whatsapp/service/go.mod").lines().find_map(|l| l.strip_prefix("go ")).unwrap();
        assert_eq!(pinned("go"), go);
    }

    /// install.sh is shell, so it can't read the table: it must say what the table does.
    #[test]
    fn the_installer_is_for_the_platforms_here() {
        let installer = include_str!("../install.sh");
        // A custom build's installer is its own: when it's this build's, it names the same repo and release key.
        if crate::state::installer_says("cli") == Some(CLI) {
            assert_eq!(crate::state::installer_says("repo"), Some(crate::config::REPO));
            assert_eq!(crate::state::installer_says("key"), Some(crate::config::RELEASE_KEY));
        }
        for p in &PLATFORMS {
            let (arch, rest) = p.target.split_once('-').unwrap();
            let fetched = if Platform::is_mac(p.target) { "$cli-macos.zip".to_string() } else { format!("$cli-$arch-{}", rest) };
            assert!(installer.contains(&format!("fetch \"{fetched}\"")), "install.sh fetches {fetched} for {}", p.target);
            assert_eq!(p.asset(), fetched.replace("$cli", CLI).replace("$arch", arch));
            assert!(installer.contains(&format!("arch={arch}")), "install.sh knows the processor {arch}");
        }
    }
}

/// The app in `dir` built into a bundle, as a blob of `node`'s, with its service built here for each platform this
/// machine can build it for, each build a blob its bundle names: → the bundle, and what was built of its service.
pub fn built_with_service(node: &crate::api::Client, dir: &Path) -> Result<(String, String)> {
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
    Ok((node.put_blob(&crate::app::deploy::build(dir, &builds)?)?, service))
}

/// MCP's deploy tool's files built into a bundle on this machine, as a blob of `node`'s: for React screens and a
/// service in Go, which only the machine that deploys builds. → the bundle, and what was built of its service.
pub fn built_here(node: &crate::api::Client, files: &serde_json::Map<String, Value>) -> Result<(String, String)> {
    ensure!(!files.contains_key("service/Cargo.toml"), "its service is in Rust, whose build would run its build scripts here: deploy it with {CLI} deploy <its folder>");
    let files = crate::app::deploy::given(files)?;
    if !files.iter().any(|(p, _)| p == "service/go.mod") {
        return Ok((node.put_blob(&crate::app::deploy::build_files(&files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect::<Vec<_>>())?)?, String::new()));
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

/// The Bun every deploy builds with, and each platform's npm package of it with its sha512.
pub(crate) const BUN: &str = "1.4.4";
const PACKAGES: &[(&str, &str, &str)] = &[
    ("x86_64-unknown-linux-gnu", "linux-x64", "CHbbkmHVM28T1GFq+6GL4Ao4ujwwYwj1CxFFwEFzLanPfNsId9AhbIP0Q4reWuWBkGimUkoQygrK7Rgjtj3vog=="),
    ("aarch64-unknown-linux-gnu", "linux-aarch64", "ZTVtpf9zmHSBVPX1sBwe5agGb/rN34yPGg/Hby8e2Ax2uCUA2URZKlZH7qLntK7qaFJCMXh9v0GWNYN2zINz1w=="),
    ("aarch64-apple-darwin", "darwin-aarch64", "zW/hxwc8sJAILGwB5eTC4i48ouGXf0V4lrxmEOk3JIE6JkYVW1TZh1E2pIempqsezgW1sckTIIDudFON8bJ2Mw=="),
    ("x86_64-apple-darwin", "darwin-x64", "oKGcxLsvh4oHjAo28nPm8c/xN0o/OmGyj1IHYc94vprkWsW2hNg2keEZt5f40cI5Hvu4jDlSRTR0UNpv1UIAxw=="),
];

/// The pinned Bun, downloaded and checked the first time.
pub fn bun() -> Result<PathBuf> {
    let (_, package, integrity) =
        PACKAGES.iter().find(|(target, _, _)| *target == crate::config::TARGET).ok_or_else(|| anyhow!("deploying from {} isn't possible yet: Bun has no build for it", crate::config::TARGET))?;
    let url = format!("https://registry.npmjs.org/@oven/bun-{package}/-/bun-{package}-{BUN}.tgz");
    crate::app::runtime::pinned(&format!("bun-{BUN}"), "bun", &url, integrity)
}

/// A browser with no window on this machine, for drawing screens (one-dev throwaway's web client, the tests), started as lean as
/// it still works: CHROME's, Playwright's headless shell (a third of Chrome's memory), the usual Chromes, or
/// Playwright's Chromium. Each app's frame keeps a process of its own, as in users' browsers, which the headless shell
/// doesn't do by itself and one-dev throwaway's script needs to reach the frames.
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

/// The release key's private half, from the publisher's password manager by way of the config's release_signer, kept
/// only in memory. It must be the key every build checks releases against.
pub fn release_key() -> Result<ssh_key::PrivateKey> {
    use crate::config::{RELEASE_KEY, RELEASE_SIGNER};
    anyhow::ensure!(!RELEASE_KEY.is_empty() && !RELEASE_SIGNER.is_empty(), "this custom build has no release key, so nothing it builds can be released");
    let out = Command::new("sh").args(["-c", RELEASE_SIGNER]).stdin(std::process::Stdio::null()).output().context("run release_signer")?;
    anyhow::ensure!(
        out.status.success(),
        "couldn't get the release key with `{RELEASE_SIGNER}`: {}\nIs the password manager it reads reachable, and unlocked?",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let key = ssh_key::PrivateKey::from_openssh(&out.stdout).map_err(|_| anyhow::anyhow!("`{RELEASE_SIGNER}` didn't give an OpenSSH private key"))?;
    anyhow::ensure!(key.public_key().key_data() == ssh_key::PublicKey::from_openssh(RELEASE_KEY)?.key_data(), "`{RELEASE_SIGNER}` gave a key, but not the release key in config.toml");
    Ok(key)
}

/// What publishes a Mac's built app (dev/skills/keep-mac-current), on a Mac with the release key.
pub fn commands() -> crate::cli::Command {
    crate::cli::Command {
        help: vec![("publish-app --zip <zip> [--config <commit>]".into(), "publish this Mac's built app, so every Mac downloads it instead of building it".into())],
        run: |g| {
            if g.words().as_slice() != ["publish-app"] {
                return Ok(false);
            }
            let zip = g.flag("zip").context("give the app with --zip")?;
            publish_latest(g.node, &release_key()?, &[(crate::latest::Thing::app_name(TARGET), Some(PathBuf::from(zip)), Version::this(), g.flag("config").unwrap_or("-").to_string())])?;
            Ok(true)
        },
    }
}
