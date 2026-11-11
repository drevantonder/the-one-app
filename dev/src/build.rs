//! What only whoever builds this repo builds: the WebAssembly a release carries (oned on Cloudflare's and onet's), a
//! browser to draw screens with, and the checks that what a machine is set up with is what the repo asks for.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use platform::os::{cargo, quietly, succeeds};

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

/// Builds oned on Cloudflare (`oned/cloudflare/worker/`) into WebAssembly and its JavaScript glue, for the release to carry: → the
/// folder they're in. `profile` is cargo's: a release's is `release`, a throwaway network's `dev`.
pub fn oned_cloudflare(src: &Path, config: &Path, profile: &str) -> Result<PathBuf> {
    let dir = if profile == "dev" { "debug" } else { profile };
    let (wasm, out) = (src.join("target/wasm32-unknown-unknown").join(dir).join("oned_cloudflare_worker.wasm"), src.join("target/oned-cloudflare"));
    Wasm { what: "oned on Cloudflare", cargo: &["build", "--profile", profile, "-p", "oned-cloudflare-worker", "--target", "wasm32-unknown-unknown"], wasm, out, glued: "oned_cloudflare_worker_bg.wasm", shrink: false }.build(src, config)
}

/// Builds onet (`onet/`) into WebAssembly for browsers and its JavaScript glue: → the folder they're in. `profile` is
/// cargo's: a release's `web` is as small as it builds, a throwaway network's `web-dev` as quick.
pub fn onet(src: &Path, config: &Path, profile: &str) -> Result<PathBuf> {
    let (wasm, out) = (src.join(format!("target/wasm32-unknown-unknown/{profile}/onet.wasm")), src.join("target/onet"));
    Wasm { what: "onet", cargo: &["rustc", "-p", "onet", "--profile", profile, "--target", "wasm32-unknown-unknown", "--crate-type", "cdylib"], wasm, out, glued: "onet_bg.wasm", shrink: profile == "web" }.build(src, config)
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

#[cfg(test)]
mod tests {
    use ::custom_build::config::CLI;
    use ::oned_update::platform::{PLATFORMS, WINDOWS};

    /// mise.toml installs what the build shells out to, and three of its versions are the repo's already: a machine set up from
    /// it must build with the Bun deploys download, the wasm-bindgen the client's WebAssembly links, and the Go the service asks for.
    #[test]
    fn mise_installs_the_versions_the_repo_asks_for() {
        let mise = include_str!("../../mise.toml");
        let pinned = |tool: &str| mise.lines().find_map(|l| l.strip_prefix(&format!("{tool} = \""))?.strip_suffix('"')).unwrap_or_else(|| panic!("mise.toml pins no {tool}"));
        assert_eq!(pinned("bun"), ::deploy::build::BUN);
        let locked = include_str!("../../Cargo.lock").split_once("name = \"wasm-bindgen\"\nversion = \"").expect("wasm-bindgen is locked").1.split('"').next().unwrap();
        assert_eq!(pinned("\"github:wasm-bindgen/wasm-bindgen\""), locked, "wasm-bindgen-cli must be the version Cargo.lock locks");
        let go = include_str!("../../apps/whatsapp/service/go.mod").lines().find_map(|l| l.strip_prefix("go ")).unwrap();
        assert_eq!(pinned("go"), go);
    }

    /// install.ps1 is PowerShell, so it says config.toml's repo, cli and key again, as install.sh does, and the asset's name.
    #[test]
    fn the_windows_installer_says_what_the_config_does() {
        assert!(include_str!("../../install.ps1").contains(&format!("$cli-{WINDOWS}.exe\"")), "install.ps1 fetches the asset the table names");
        let says = |name: &str| include_str!("../../install.ps1").lines().find_map(|l| l.strip_prefix(&format!("${name} = '"))?.strip_suffix('\''));
        if scope::state::installer_says("cli") == Some(CLI) {
            assert_eq!(says("cli"), Some(CLI));
            assert_eq!(says("repo"), Some(::custom_build::config::REPO));
            assert_eq!(says("key"), Some(::custom_build::config::RELEASE_KEY));
        }
    }

    /// install.sh is shell, so it can't read the table: it must say what the table does.
    #[test]
    fn the_installer_is_for_the_platforms_here() {
        let installer = include_str!("../../install.sh");
        // A custom build's installer is its own: when it's this build's, it names the same repo and release key.
        if scope::state::installer_says("cli") == Some(CLI) {
            assert_eq!(scope::state::installer_says("repo"), Some(::custom_build::config::REPO));
            assert_eq!(scope::state::installer_says("key"), Some(::custom_build::config::RELEASE_KEY));
        }
        for p in PLATFORMS.iter().filter(|p| p.target != WINDOWS) {
            let (arch, rest) = p.target.split_once('-').unwrap();
            let fetched = if ::oned_update::latest::is_mac(p.target) { "$cli-macos.zip".to_string() } else { format!("$cli-$arch-{}", rest) };
            assert!(installer.contains(&format!("fetch \"{fetched}\"")), "install.sh fetches {fetched} for {}", p.target);
            assert_eq!(p.asset(), fetched.replace("$cli", CLI).replace("$arch", arch));
            assert!(installer.contains(&format!("arch={arch}")), "install.sh knows the processor {arch}");
        }
    }
}

/// What `native-apps/mac/build.sh` made on the Mac `host` in `folder`, copied into `here` (emptied first), and what it is,
/// as its program says: for publishing from this device what a Mac built.
pub fn mac_build_from(host: &str, folder: &str, here: &Path) -> Result<oned_update::release::Version> {
    use crate::throwaway::shell_quote;
    let _ = std::fs::remove_dir_all(here);
    std::fs::create_dir_all(here)?;
    let copied = Command::new("sh").arg("-c").arg(format!("ssh -o BatchMode=yes {host} 'tar -c -C {} .' | tar -x -C {}", shell_quote(folder), shell_quote(&here.to_string_lossy()))).status()?;
    anyhow::ensure!(copied.success(), "couldn't copy what {host} built from {folder}");
    let said = Command::new("ssh").args(["-o", "BatchMode=yes", host, &format!("{}/{} version --json", shell_quote(folder), ::custom_build::config::CLI)]).output()?;
    serde_json::from_slice(&said.stdout).with_context(|| format!("ask what {host} built what it is"))
}
