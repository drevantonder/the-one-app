//! What a change has to send out: the parts it's made of (`Parts::since`).

use super::*;

/// What a change to the repo has to send out.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Parts {
    /// The node: built, and taken by every device.
    pub(super) node: bool,
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
    /// This program (dev/), which no device takes.
    pub(super) dev: bool,
}

impl Parts {
    pub(super) fn everything() -> Parts {
        Parts { node: true, mac: true, dev: true, ..Default::default() }
    }

    /// What changed from the commit `last` says (and its config's) to `this`'s.
    pub(super) fn since(src: &Path, last: &str, this: &str) -> Parts {
        let (Some((was, was_config)), Some((now, now_config))) = (last.split_once(' '), this.split_once(' ')) else { return Parts::everything() };
        // Without renames, so a renamed app's old folder is in it too.
        let (Ok(files), Ok(renames)) = (git(src, &["diff", "--name-status", "--no-renames", was, now]), git(src, &["diff", "--name-status", "-M", "--diff-filter=R", was, now, "--", "apps/*/app.js"])) else {
            return Parts::everything();
        };
        let mut parts = Parts::of(files.lines().filter_map(|l| l.split_once('\t')).map(|(_, f)| f));
        // A new built-in app is in the node's next build, which adds it.
        if files.lines().any(|l| l.strip_prefix("A\tapps/").is_some_and(|f| f.ends_with("/app.js") && f.matches('/').count() == 1)) {
            (parts.node, parts.mac) = (true, true);
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
            (parts.node, parts.mac) = (true, true);
        }
        parts
    }

    /// What changing `files` (paths in the repo) has to send out. Anything not known to be outside the node is in it.
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
                    // One in Swift isn't published: the Mac app builds it into its helpers (service/mac-helpers.sh).
                    if [".swift", "Info.plist", ".entitlements"].iter().any(|end| file.ends_with(end)) {
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
                // What the Mac's services in Swift are built on, which the Mac app builds (service/mac-helpers.sh).
                Some("service") if file.starts_with("service/swift/") || file == "service/mac-helpers.sh" => parts.mac = true,
                Some("service") => parts.service = true,
                Some("dev") if file.starts_with("dev/skills/") => {}
                Some("dev") => parts.dev = true,
                // The Workers' sources and the Cloudflare node's are in the node, which deploys them; the Mac's needn't.
                Some("cloudflare") if !file.starts_with("cloudflare/test/") => parts.node = true,
                // The native app (client/native) is its own workspace, not in the node: the Mac app, but for Windows' and
                // Android's own (client/native/windows/, client/native/android/), which no update sends out.
                Some("client") if file.starts_with("client/native/windows/") || file.starts_with("client/native/android/") => {}
                Some("client") if file.starts_with("client/native/") => parts.mac = true,
                // The client is the Macs', and in the node too: its WebAssembly is the web client's, and the Cloudflare node's.
                Some("client") => (parts.node, parts.mac) = (true, true),
                Some("docs" | "tests" | "tools" | "cloudflare") => {}
                None if file.ends_with(".md") || file == "LICENSE" || file == ".gitignore" => {}
                _ => (parts.node, parts.mac) = (true, true),
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
        assert_eq!(of(&["docs/x.md", "README.md", "tests/e2e.rs", "tools/iteration/report.py", "apps/repos/tests/repos.rs", ".gitignore"]), Parts::default());
        assert_eq!(of(&["apps/board/app.js", "apps/board/screen.js"]), Parts { apps: vec!["board".into()], ..Default::default() });
        assert_eq!(of(&["apps/shell/service/main.go"]), Parts { services: vec!["shell".into()], ..Default::default() });
        assert!(of(&["service/src/lib.rs"]).service);
        let swift = of(&["apps/computer/service/Computer.swift"]);
        assert!(swift.mac && !swift.node, "a service in Swift is built into the Mac app");
        assert!(!of(&["apps/shell/service/main.go"]).mac);
        let helpers = of(&["service/swift/Helper.swift"]);
        assert!(helpers.mac && !helpers.service, "what the Swift services are built on is the Mac app's to build");
        assert!(of(&["src/node.rs"]).node && of(&["src/node.rs"]).mac);
        let workers = of(&["cloudflare/relay/worker.js"]);
        assert!(workers.node && !workers.mac, "the Workers are deployed by the node, not built by a Mac");
        assert_eq!(of(&["cloudflare/test/x.js"]), Parts::default());
        let native = of(&["client/native/mac/build.sh"]);
        assert!(!native.node && native.mac);
        assert_eq!(of(&["client/native/windows/build.sh"]), Parts::default());
        let client = of(&["client/src/lib.rs"]);
        assert!(client.node && client.mac, "the client's WebAssembly is in the node");
        assert_eq!(of(&["dev/src/ship.rs"]), Parts { dev: true, ..Default::default() }, "no device takes one-dev");
    }
}
