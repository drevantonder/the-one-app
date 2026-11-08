// Turns config.toml (or the one CONFIG names) into constants, so the code never names the product.
fn main() {
    let path = std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".into());
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed=CONFIG");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let config: toml::Table = text.parse().expect("parse config file");
    let get = |key: &str| config.get(key).unwrap_or_else(|| panic!("config file lacks {key}")).clone();
    let out = format!(
        "pub const NAME: &str = {:?};\npub const CLI: &str = {:?};\npub const PORT: u16 = {};\npub const REPO: &str = {:?};\n",
        get("name").as_str().unwrap(),
        get("cli").as_str().unwrap(),
        get("port").as_integer().unwrap(),
        // The GitHub repo releases come from; a custom build without one isn't released.
        config.get("repo").and_then(|r| r.as_str()).unwrap_or(""),
    );
    let optional = |key: &str| config.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    // Where the config file is, so `update` builds the same custom build from fresh checkouts: inside this repo, or inside
    // the git repo it's in (another custom build's own, `CONFIG_REPO`), or else wherever it is on this machine.
    let manifest = std::fs::canonicalize(std::env::var("CARGO_MANIFEST_DIR").unwrap()).unwrap();
    let file = std::fs::canonicalize(&path).unwrap();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git").current_dir(file.parent().unwrap()).args(args).output().ok().filter(|o| o.status.success())?;
        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let (repo, file) = match file.strip_prefix(&manifest) {
        Ok(rel) => (String::new(), rel.to_path_buf()),
        Err(_) => match (git(&["rev-parse", "--show-toplevel"]), git(&["remote", "get-url", "origin"])) {
            (Some(top), Some(url)) => (url, file.strip_prefix(std::fs::canonicalize(top).unwrap()).unwrap().to_path_buf()),
            _ => (String::new(), file.clone()),
        },
    };
    // A config in its own repo is part of what this build is: its commit, when it was made, and whether it's dirty.
    let config_commit = if repo.is_empty() {
        None
    } else {
        if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
            for what in ["HEAD", "index"] {
                println!("cargo:rerun-if-changed={dir}/{what}");
            }
        }
        let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
        let at: u64 = git(&["log", "-1", "--format=%ct"]).and_then(|t| t.parse().ok()).unwrap_or(0);
        git(&["rev-parse", "--short=8", "HEAD"]).map(|c| (c, at, dirty))
    };
    let out = format!("{out}pub const CONFIG_FILE: &str = {:?};\npub const CONFIG_REPO: &str = {repo:?};\n", file.to_string_lossy());
    let out = format!("{out}pub const MADE_HERE: &str = {:?};\n", config.get("made_here").and_then(|v| v.as_str()).unwrap_or("Made here"));
    let out = format!("{out}pub const RELEASE_KEY: &str = {:?};\npub const RELEASE_SIGNER: &str = {:?};\n", optional("release_key"), optional("release_signer"));
    let sign_in = config.get("sign_in").and_then(|v| v.as_str()).unwrap_or("passkey");
    assert!(["passkey", "google"].contains(&sign_in), "sign_in is passkey or google");
    assert!(sign_in != "google" || !optional("join_domain").is_empty(), "signing in with Google needs join_domain");
    let variant = if sign_in == "google" { "Google" } else { "Passkey" };
    let out = format!("{out}pub const SIGN_IN: SignIn = SignIn::{variant};\npub const JOIN_DOMAIN: &str = {:?};\n", optional("join_domain"));
    let relay_location = optional("relay_location");
    assert!(["", "wnam", "enam", "sam", "weur", "eeur", "apac", "oc", "afr", "me"].contains(&relay_location.as_str()), "relay_location is one of Cloudflare's location hints");
    let out = format!("{out}pub const RELAY_LOCATION: &str = {relay_location:?};\n");
    // What a user joins, in their words: "your One App network"; a workplace's (one with a join_domain) is shared, "the PTC App network".
    // client/native/apple.sh, client/native/android/app/build.gradle.kts and client/native/windows/build.sh say the same.
    let name = get("name").as_str().unwrap().to_string();
    let yours = if optional("join_domain").is_empty() { format!("your {} network", name.trim_start_matches("The ")) } else { format!("{} network", name.replacen("The ", "the ", 1)) };
    let out = format!("{out}pub const YOURS: &str = {yours:?};\n");
    // A custom build's one network's address, if it serves only one: its native app never asks where.
    let out = format!("{out}pub const ADDRESS: &str = {:?};\n", optional("address").trim_end_matches('/'));
    // The Mac's menu bar mark, its @2x, which the native app's build draws its menu bar item with.
    let menubar = std::path::Path::new(&path).parent().unwrap().join(optional("menubar").replace(".png", "@2x.png"));
    println!("cargo:rerun-if-changed={}", menubar.display());
    let menubar = std::fs::canonicalize(&menubar).map(|m| m.to_string_lossy().into_owned()).unwrap_or_default();
    let out = format!("{out}pub const MENUBAR: &str = {menubar:?};\n");
    // The network settings a network has until its admins set them (`setting` steps).
    let mut settings = String::new();
    for (name, value) in config.get("settings").and_then(|s| s.as_table()).into_iter().flatten() {
        settings.push_str(&format!("({name:?}, {:?}),", value.as_str().unwrap_or_else(|| panic!("the setting {name} isn't text"))));
    }
    let out = format!("{out}pub const SETTINGS: &[(&str, &str)] = &[{settings}];\n");
    // Every app in this repo's apps/ folder ships built in: each folder's files. The config's `apps` are the ones a new
    // network deploys and installs in `global`, by folder name.
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("apps");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut dirs: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("app.js").is_file()).collect();
    dirs.sort();
    let mut apps = String::new();
    for dir in &dirs {
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files = Vec::new();
        collect(dir, dir, &mut files);
        // Its service's source is built apart (`one update`), and its tests run with ours: neither ships in its bundle.
        files.retain(|(rel, _)| !rel.starts_with("service/") && !rel.starts_with("tests/"));
        files.sort();
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        // A React screen ships as its `dist/` has it, which `one deploy apps/<name>` builds, so each has one.
        for (rel, _) in &files {
            if let Some(screen) = rel.strip_prefix("screens/").and_then(|f| f.strip_suffix(".jsx").or_else(|| f.strip_suffix(".tsx"))) {
                assert!(files.iter().any(|(r, _)| *r == format!("dist/{screen}.html")), "apps/{name}/screens/{screen} has no dist/{screen}.html: run `one deploy apps/{name}` and commit it");
            }
        }
        let files: Vec<String> = files.iter().map(|(rel, abs)| format!("({rel:?}, include_bytes!({abs:?}))")).collect();
        apps.push_str(&format!("({name:?}, &[{}]),", files.join(", ")));
    }
    let mut starters = String::new();
    for name in config.get("apps").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|d| d.as_str()) {
        assert!(dirs.iter().any(|d| d.file_name().is_some_and(|f| f == name)), "the config's app {name} isn't in apps/");
        starters.push_str(&format!("{name:?},"));
    }
    // Each built-in app's service built for Cloudflare (`service/cloudflare.js`), which the Cloudflare node carries.
    let mut on_cloudflare = String::new();
    for dir in dirs.iter().filter(|d| d.join("service/cloudflare.js").is_file()) {
        println!("cargo:rerun-if-changed={}", dir.join("service/cloudflare.js").display());
        on_cloudflare.push_str(&format!("({:?}, include_bytes!({:?})),", dir.file_name().unwrap().to_string_lossy(), dir.join("service/cloudflare.js")));
    }
    let out = format!("{out}pub const BUILT_IN: &[(&str, &[(&str, &[u8])])] = &[{apps}];\npub const STARTERS: &[&str] = &[{starters}];\npub const CLOUDFLARE_SERVICES: &[(&str, &[u8])] = &[{on_cloudflare}];\n");
    // The Workers' sources, built with the pinned Bun when they're run or deployed (src/cloudflare.rs), so a
    // release deploys exactly the Workers it was signed with. Their tests stay in the repo.
    let cloudflare = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("cloudflare");
    println!("cargo:rerun-if-changed={}", cloudflare.display());
    let mut files = Vec::new();
    collect(&cloudflare, &cloudflare, &mut files);
    // The relay's WebAssembly is built from relay/protos/ by hand (relay/build.sh), so only what it built ships; the
    // Cloudflare node (node/) is Rust, which ships built (CLOUDFLARE_NODE, below).
    files.retain(|(rel, _)| !rel.starts_with("test/") && !rel.starts_with("relay/protos/") && rel != "relay/build.sh" && !rel.starts_with("node/"));
    // The gateway serves the screens' design kit beside the web client, the same files nodes put in screens.
    let screen = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/screen");
    println!("cargo:rerun-if-changed={}", screen.display());
    let mut kit = Vec::new();
    collect(&screen, &screen, &mut kit);
    let kit = kit.into_iter().filter_map(|(rel, abs)| match rel.strip_prefix("ui/") {
        Some(ui) => ui.strip_prefix("dist/").map(|built| (format!("ui/{built}"), abs)),
        None => Some((rel, abs)),
    });
    files.extend(kit.map(|(rel, abs)| (format!("web/screen/{rel}"), abs)));
    // And the formats devices hold keys in, which the web client imports as ../shared/crypto.js.
    files.push(("web/shared/crypto.js".into(), cloudflare.join("shared/crypto.js").to_string_lossy().into()));
    // The custom build's logo, which the web client shows where the apps show it.
    let logo = std::path::Path::new(&path).parent().unwrap().join(get("logo").as_str().unwrap());
    println!("cargo:rerun-if-changed={}", logo.display());
    files.push(("web/logo.png".into(), std::fs::canonicalize(&logo).unwrap_or_else(|e| panic!("{}: {e}", logo.display())).to_string_lossy().into()));
    // And its app icon, which the web client is installed with (its manifest) and shows on its notifications.
    let icon = std::path::Path::new(&path).parent().unwrap().join(get("icon").as_str().unwrap());
    println!("cargo:rerun-if-changed={}", icon.display());
    files.push(("web/icon.png".into(), std::fs::canonicalize(&icon).unwrap_or_else(|e| panic!("{}: {e}", icon.display())).to_string_lossy().into()));
    files.sort();
    let files: Vec<String> = files.iter().map(|(rel, abs)| format!("({rel:?}, include_bytes!({abs:?}))")).collect();
    let out = format!("{out}pub const CLOUDFLARE: &[(&str, &[u8])] = &[{}];\n", files.join(", "));
    // The Cloudflare node's WebAssembly (cloudflare/node/) and the client's, for browsers (client/src/wasm.rs), which `update`
    // builds into CLOUDFLARE_NODE and CLIENT where Rust builds for WebAssembly, so a release carries them to devices with no
    // Rust to deploy it from, and the address serves the web client. With neither set, the ones `update`, `throwaway cloudflare
    // node deploy` and `throwaway client web` build into target/, if they're there, so every build of a checkout is the same one,
    // with or without them, but not their own (they're built with `one` in them, which would hold itself). Only a file that
    // exists is watched: cargo reruns a build for one that doesn't.
    let out = format!("{out}{}", wasm_constant("CLOUDFLARE_NODE", "target/cloudflare-node", ["one_cloudflare_node.js", "one_cloudflare_node_bg.wasm"]));
    let out = format!("{out}{}", wasm_constant("CLIENT", "target/client", ["one_client.js", "one_client_bg.wasm"]));
    // The apps every agent needs from the start: their guide is in MCP's instructions, and code mode keeps their tools.
    let always_on: String = config.get("always_on").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|d| d.as_str()).map(|n| format!("{n:?},")).collect();
    let out = format!("{out}pub const ALWAYS_ON: &[&str] = &[{always_on}];\n");
    // Which build this is, so a node can tell a release apart from itself.
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "dev".into());
    // A build with uncommitted changes isn't that commit; say so, so it can't be released as it.
    let dirty = std::process::Command::new("git").args(["status", "--porcelain", "--untracked-files=no"]).output().is_ok_and(|o| !o.stdout.is_empty());
    let dirty = dirty || config_commit.as_ref().is_some_and(|c| c.2);
    // A custom build's build is main's commit and its config's, so a change to either is a new build that nodes take.
    let commit = match &config_commit {
        Some((config, ..)) => format!("{commit}-{config}"),
        None => commit,
    };
    let commit = if dirty { format!("{commit}+dirty") } else { commit };
    // Rebuild when HEAD moves: HEAD itself, or the branch it's on, never all of refs/heads, which every worktree shares,
    // so another's commit would rebuild this one. In a worktree .git is a file, so ask git where they live; a branch
    // whose ref is packed has no file of its own until its next commit.
    let git_out = |args: &[&str]| {
        let o = std::process::Command::new("git").args(args).output().ok().filter(|o| o.status.success())?;
        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    // The index too, for "+dirty", in a release build only: `git status` rewrites it (T3 runs it all the time), and a
    // test build isn't published.
    let branch = git_out(&["symbolic-ref", "-q", "HEAD"]);
    let index = (std::env::var("PROFILE").as_deref() == Ok("release")).then_some("index");
    for what in ["HEAD"].into_iter().chain(index).chain(branch.as_deref()) {
        let Some(path) = git_out(&["rev-parse", "--git-path", what]) else { continue };
        let path = if std::path::Path::new(&path).exists() { Some(path) } else { git_out(&["rev-parse", "--git-path", "packed-refs"]) };
        if let Some(path) = path {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    // When that commit (or the config's, if later) was made, so releases only ever move devices forward.
    let committed: u64 = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
        .max(config_commit.map_or(0, |c| c.1));
    let out = format!("{out}pub const COMMITTED: u64 = {committed};\npub const BUILD: &str = {commit:?};\npub const TARGET: &str = {:?};\n", std::env::var("TARGET").unwrap());
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("config.rs");
    std::fs::write(dest, out).unwrap();
}

fn collect(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            out.push((path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), path.to_string_lossy().to_string()));
        }
    }
}

/// `pub const <var>: &[(&str, &[u8])]` holding `names`, which are in the folder the environment variable `var` names or, with
/// none, `default` in this package, if all of them are there; and what cargo watches for them.
fn wasm_constant(var: &str, default: &str, names: [&str; 2]) -> String {
    println!("cargo:rerun-if-env-changed={var}");
    let wasm = std::env::var("TARGET").unwrap().starts_with("wasm32");
    let explicit = std::env::var(var).ok();
    let dir = explicit.clone().map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join(default));
    for name in names.iter().filter(|_| explicit.is_some() || !wasm) {
        if dir.join(name).exists() {
            println!("cargo:rerun-if-changed={}", dir.join(name).display());
        }
    }
    let held: Vec<String> = if explicit.is_some() || (!wasm && names.iter().all(|name| dir.join(name).exists())) {
        names
            .iter()
            .map(|name| {
                let path = dir.join(name);
                format!("({name:?}, include_bytes!({:?}))", std::fs::canonicalize(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
            })
            .collect()
    } else {
        vec![]
    };
    format!("pub const {var}: &[(&str, &[u8])] = &[{}];\n", held.join(", "))
}
