// Turns config.toml (or the one CONFIG names) into constants, so the code never names the product. config.toml is at the
// repo's root, the folder above this package, and a relative CONFIG is from there too.

fn main() {
    let manifest = std::fs::canonicalize(std::env::var("CARGO_MANIFEST_DIR").unwrap()).unwrap();
    let manifest = manifest.parent().unwrap().to_path_buf();
    let path = manifest.join(std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".into())).to_string_lossy().into_owned();
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
        // The git address the code comes from; a custom build without one has nothing to build from.
        config.get("repo").and_then(|r| r.as_str()).unwrap_or(""),
    );
    let optional = |key: &str| config.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    // Where the config file is, so `update` builds the same custom build from fresh checkouts: inside this repo, or inside
    // the git repo it's in (another custom build's own, `CONFIG_REPO`), or else wherever it is on this machine.
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
    let out = format!("{out}pub const JOIN_DOMAIN: &str = {:?};\n", optional("join_domain"));
    let out = format!("{out}pub const CHECK: &str = {:?};\n", optional("check"));
    let relay_location = optional("relay_location");
    assert!(["", "wnam", "enam", "sam", "weur", "eeur", "apac", "oc", "afr", "me"].contains(&relay_location.as_str()), "relay_location is one of Cloudflare's location hints");
    let out = format!("{out}pub const RELAY_LOCATION: &str = {relay_location:?};\n");
    // The iPhone app's bundle id (its extensions are `<id>.widgets` and `<id>.notifications`, its App Group and Keychain
    // group `group.<id>`); native-apps/apple.sh says the same.
    let iphone = match optional("iphone") {
        id if id.is_empty() => format!("app.{}.phone", get("cli").as_str().unwrap()),
        id => id,
    };
    let out = format!("{out}pub const IPHONE: &str = {iphone:?};\n");
    // The Android app's package (its applicationId); native-apps/android/app/build.gradle.kts says the same.
    let android = match optional("android") {
        id if id.is_empty() => format!("app.{}.android", get("cli").as_str().unwrap()),
        id => id,
    };
    let out = format!("{out}pub const ANDROID: &str = {android:?};\n");
    // What a user joins, in their words: "your One App network"; a workplace's (one with a join_domain) is shared, "the PTC App network".
    // native-apps/apple.sh, native-apps/android/app/build.gradle.kts and native-apps/windows/build.sh say the same.
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
