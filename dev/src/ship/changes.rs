//! What a change has to send out: the parts it's made of (`Parts::since`).

use super::*;

/// What a change to the repo has to send out.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Parts {
    /// The program: built, and taken by every device.
    pub(super) program: bool,
    /// The Mac app, which each Mac builds.
    pub(super) mac: bool,
    /// Built-in apps, by name, each deployed on its own.
    pub(super) apps: Vec<String>,
    /// Built-in apps renamed: (old name, new name).
    pub(super) renamed: Vec<(String, String)>,
    /// Apps' services (`apps/<name>/service/`), each built for every platform and published.
    pub(super) services: Vec<String>,
    /// The `service` crate, which every service in Rust is built on, so each is published again.
    pub(super) service: bool,
    /// This program (dev/), which no device takes, and which the Workers and the web client are put together with: what's
    /// in it from the rest of the repo has to be what's sent.
    pub(super) dev: bool,
    /// The Workers, which this device deploys: `cloudflare/`'s own, and those the web client goes up with.
    pub(super) workers: bool,
    /// The web client, for the native apps, with the gateway, which serves it to browsers.
    pub(super) web: bool,
    /// Nothing says what was last sent, so all of it is due.
    pub(super) all: bool,
}

impl Parts {
    pub(super) fn everything() -> Parts {
        Parts { all: true, ..Parts::rust() }
    }

    /// Whether the change is in what this program is built with, which the Workers and the web client are put together with
    /// (the client's WebAssembly, the config's logo and name): Rust anywhere, or the client. A change to dev/ alone is
    /// only to this program, and oned on Cloudflare's is built for the program; neither is in what it puts together.
    pub(super) fn is_built_in(&self) -> bool {
        self.program && self.dev
    }

    /// What a change to Rust, or to anything not known to be outside the program, sends: all of it.
    fn rust() -> Parts {
        Parts { program: true, mac: true, dev: true, workers: true, web: true, ..Default::default() }
    }

    pub(super) fn add_rust(&mut self) {
        let Parts { program, mac, dev, workers, web, .. } = Parts::rust();
        (self.program, self.mac, self.dev, self.workers, self.web) = (program, mac, dev, workers, web);
    }

    /// What changed from the commit `last` says (and its config's) to `this`'s.
    pub(super) fn since(src: &Path, last: &str, this: &str) -> Parts {
        let (Some((was, was_config)), Some((now, now_config))) = (last.split_once(' '), this.split_once(' ')) else { return Parts::everything() };
        // Without renames, so a renamed app's old folder is in it too.
        let (Ok(files), Ok(renames)) = (git(src, &["diff", "--name-status", "--no-renames", was, now]), git(src, &["diff", "--name-status", "-M", "--diff-filter=R", was, now, "--", "apps/*/app.js"])) else {
            return Parts::everything();
        };
        let mut parts = Parts::of(files.lines().filter_map(|l| l.split_once('\t')).map(|(_, f)| f));
        // A new built-in app is in the program's next build, which adds it.
        if files.lines().any(|l| l.strip_prefix("A\tapps/").is_some_and(|f| f.ends_with("/app.js") && f.matches('/').count() == 1)) {
            parts.add_rust();
        }
        let app = |f: &str| f.split('/').nth(1).unwrap_or("").to_string();
        parts.renamed = renames
            .lines()
            .filter_map(|l| {
                let mut f = l.split('\t').skip(1);
                Some((app(f.next()?), app(f.next()?)))
            })
            .collect();
        if was_config != now_config {
            parts.add_rust();
        }
        parts
    }

    /// What changing `files` (paths in the repo) has to send out. Anything not known to be outside the program is in it.
    pub(super) fn of<'a>(files: impl IntoIterator<Item = &'a str>) -> Parts {
        let mut parts = Parts::default();
        for file in files {
            let top = file.split_once('/').map(|(top, _)| top);
            match top {
                // An app's service is built apart from its app (`service_dir`), which its deploy leaves out.
                Some("apps") if file.split('/').nth(2) == Some("service") && file.matches('/').count() >= 3 => {
                    let app = file.split('/').nth(1).unwrap_or("").to_string();
                    if !parts.services.contains(&app) {
                        parts.services.push(app);
                    }
                    // One in Swift or Rust a Mac builds and publishes for Macs (native-apps/mac/build.sh): a helper app
                    // of its own (services/sdk/mac-helpers.sh), or a program.
                    if [".swift", "Info.plist", ".entitlements", ".rs", "Cargo.toml"].iter().any(|end| file.ends_with(end)) {
                        parts.mac = true;
                    }
                }
                // An app's tests aren't in its deploy.
                Some("apps") if file.split('/').nth(2) == Some("tests") => {}
                Some("apps") => {
                    let app = file.split('/').nth(1).unwrap_or("").to_string();
                    if !parts.apps.contains(&app) {
                        parts.apps.push(app);
                    }
                }
                // What the Mac's services in Swift are built on, which a Mac builds (services/sdk/mac-helpers.sh).
                Some("services") if file.starts_with("services/sdk/swift/") || file == "services/sdk/mac-helpers.sh" => parts.mac = true,
                // What every service in Rust is built on, a Mac's too.
                Some("services") if file.starts_with("services/sdk/") => (parts.service, parts.mac) = (true, true),
                // Services' tests stay in the repo.
                Some("services") if file.starts_with("services/tests/") => {}
                // What only whoever builds this repo reads or runs by hand.
                Some("dev") if file.starts_with("dev/skills/") || file.starts_with("dev/tools/") || file.starts_with("dev/plans/") => {}
                Some("dev") => parts.dev = true,
                // The Workers' sources but for the web client's: the Workers alone. The relay's protos only make its
                // WebAssembly by hand.
                Some("relay") if file.starts_with("relay/protos/") || file == "relay/build.sh" => {}
                // What every Worker answers HTTP with, which oned on Cloudflare carries too.
                Some("gateway") if file == "gateway/http.js" => (parts.program, parts.workers) = (true, true),
                Some("relay" | "auth" | "gateway" | "url-signer") => parts.workers = true,
                // Notifications' Worker; the rest of the folder is Rust.
                Some("notifications") if file.starts_with("notifications/worker/") => parts.workers = true,
                // The telemetry Worker; the rest of the folder is Rust.
                Some("insights") if file.starts_with("insights/worker/") => parts.workers = true,
                // oned on Cloudflare is Rust, built with the program; the remote MCP server runs that build, so the Workers
                // go out with it.
                Some("oned") if file.starts_with("oned/cloudflare/node/") => (parts.program, parts.workers) = (true, true),
                // The update path's tests stay in the repo.
                Some("oned") if file.starts_with("oned/update/tests/") => {}
                // What the web client is made of: the native apps' bundle, and the Workers that serve it.
                Some("key") if file == "key/crypto.js" => (parts.web, parts.workers) = (true, true),
                Some("onet") if file.starts_with("onet/web/") => (parts.web, parts.workers) = (true, true),
                // The screens' kit, the client's own screens (the frame's and each feature's) and the sign-in pages: devices
                // hold a copy, but what shows is the web client's (`screens.js`). Phosphor's icons are built into the
                // native apps' widgets too.
                Some("ui") if !file.ends_with(".rs") && !file.ends_with("Cargo.toml") && !file.starts_with("ui/vendor/") => (parts.web, parts.workers) = (true, true),
                Some(_) if file.split('/').nth(1) == Some("screens") => (parts.web, parts.workers) = (true, true),
                Some("onboarding") if !file.ends_with(".rs") && !file.ends_with("Cargo.toml") => (parts.web, parts.workers) = (true, true),
                // The native app (native-apps) is its own workspace, not in the program: the Mac app, but for Windows' and
                // Android's own (native-apps/windows/, native-apps/android/), which no update sends out.
                Some("native-apps") if file.starts_with("native-apps/windows/") || file.starts_with("native-apps/android/") => {}
                Some("native-apps") if file.starts_with("native-apps/") => parts.mac = true,
                // The remote MCP server's Worker; the rest of the folder is Rust.
                Some("onet") if file.starts_with("onet/worker/") => parts.workers = true,
                // onet is the Macs', and in the program too: its WebAssembly is the web client's, and oned on Cloudflare's.
                Some("onet") => (parts.program, parts.mac, parts.web, parts.dev) = (true, true, true, true),
                Some("tests") => {}
                None if file.ends_with(".md") || file == "LICENSE" || file == ".gitignore" => {}
                _ => parts.add_rust(),
            }
        }
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(files: &[&str]) -> Parts {
        Parts::of(files.iter().copied())
    }

    #[test]
    fn a_change_sends_out_what_is_made_of_it() {
        assert_eq!(of(&["dev/plans/x.md", "README.md", "tests/e2e.rs", "dev/tools/iteration/report.py", "apps/repos/tests/repos.rs", ".gitignore", "tests/workers/x.js", "relay/protos/a.proto"]), Parts::default());
        assert_eq!(of(&["apps/board/app.js", "apps/board/screen.js"]), Parts { apps: vec!["board".into()], ..Default::default() });
        assert_eq!(of(&["apps/whatsapp/service/main.go"]), Parts { services: vec!["whatsapp".into()], ..Default::default() });
        assert!(of(&["services/sdk/src/lib.rs"]).service && of(&["services/sdk/src/lib.rs"]).mac);
        let swift = of(&["apps/computer/service/Computer.swift"]);
        assert!(swift.mac && !swift.program, "a service in Swift is a Mac's to build");
        let rust = of(&["apps/awake/service/src/main.rs"]);
        assert!(rust.mac && rust.services == ["awake"], "a service in Rust is built on Linux and on a Mac");
        assert!(!of(&["apps/whatsapp/service/main.go"]).mac);
        let helpers = of(&["services/sdk/swift/Helper.swift"]);
        assert!(helpers.mac && !helpers.service, "what the Swift services are built on is the Mac app's to build");
        let native = of(&["native-apps/mac/build.sh"]);
        assert!(!native.program && native.mac);
        assert_eq!(of(&["native-apps/windows/build.sh"]), Parts::default());
        assert_eq!(of(&["dev/src/ship.rs"]), Parts { dev: true, ..Default::default() }, "no device takes one-dev");
        assert!(!of(&["dev/src/ship.rs"]).is_built_in() && !of(&["oned/cloudflare/node/src/lib.rs"]).is_built_in(), "a change to this program alone doesn't change what it puts together");
        assert!(of(&["app-runtime/src/lib.rs"]).is_built_in() && of(&["onet/src/lib.rs"]).is_built_in());
        assert_eq!(of(&["auth/worker.js", "relay/worker.js", "auth/package.json", "notifications/worker/senders.js"]), Parts { workers: true, ..Default::default() }, "the Workers alone");
        let http = of(&["gateway/http.js"]);
        assert!(http.program && http.workers && !http.web, "oned on Cloudflare carries what every Worker answers HTTP with");
        for file in ["ui/web/web.js", "onet/web/net-worker.js", "key/crypto.js", "ui/kit/dist/ui.js", "vault/screens/dist/vault.html", "ui/settings/dist/requests.html", "ui/bridge.js", "onboarding/pages.js", "onboarding/pages.tsx"] {
            assert_eq!(of(&[file]), Parts { web: true, workers: true, ..Default::default() }, "{file}: the web client and the Workers that serve it, with no program");
        }
        let oned_cloudflare = of(&["oned/cloudflare/node/src/lib.rs"]);
        assert!(oned_cloudflare.program && oned_cloudflare.workers && !oned_cloudflare.mac, "oned on Cloudflare is built with the program");
        for rust in ["app-runtime/src/lib.rs", "ui/src/screen.rs", "ui/vendor/phosphor/icons.json", "onboarding/src/lib.rs", "Cargo.toml", "somewhere/new.txt"] {
            let parts = of(&[rust]);
            assert!(parts.program && parts.mac && parts.workers && parts.web && parts.dev, "{rust}: anything not listed sends everything");
        }
        let onet = of(&["onet/src/lib.rs"]);
        assert!(onet.program && onet.mac && onet.web && !onet.workers, "onet's WebAssembly is in the program and the web client");
    }
}
