// Turns config.toml (or the one CONFIG names) into constants, so the code never names the product.
fn main() {
    let path = std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".into());
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed=CONFIG");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let config: toml::Table = text.parse().expect("parse config file");
    let get = |key: &str| config.get(key).unwrap_or_else(|| panic!("config file lacks {key}")).clone();
    let out = format!(
        "pub const NAME: &str = {:?};\npub const CLI: &str = {:?};\npub const PORT: u16 = {};\npub const ACCENT: &str = {:?};\npub const REPO: &str = {:?};\n",
        get("name").as_str().unwrap(),
        get("cli").as_str().unwrap(),
        get("port").as_integer().unwrap(),
        get("accent").as_str().unwrap(),
        // The GitHub repo releases come from; a white label without one isn't released.
        config.get("repo").and_then(|r| r.as_str()).unwrap_or(""),
    );
    let optional = |key: &str| config.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    // Where the config file is, so `update` builds the same white label from fresh checkouts: inside this repo, or inside
    // the git repo it's in (another white label's own, `CONFIG_REPO`), or else wherever it is on this machine.
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
    let out = format!("{out}pub const RELEASE_KEY: &str = {:?};\npub const RELEASE_SIGNER: &str = {:?};\n", optional("release_key"), optional("release_signer"));
    // How people sign in: with a passkey, or with Google at the install's domain.
    let sign_in = config.get("sign_in").and_then(|v| v.as_str()).unwrap_or("passkey");
    assert!(["passkey", "google"].contains(&sign_in), "sign_in is passkey or google");
    assert!(sign_in != "google" || !optional("join_domain").is_empty(), "signing in with Google needs join_domain");
    let out = format!("{out}pub const SIGN_IN: &str = {sign_in:?};\npub const JOIN_DOMAIN: &str = {:?};\n", optional("join_domain"));
    // The install settings an install has until its admins set them (`setting` entries).
    let mut settings = String::new();
    for (name, value) in config.get("settings").and_then(|s| s.as_table()).into_iter().flatten() {
        settings.push_str(&format!("({name:?}, {:?}),", value.as_str().unwrap_or_else(|| panic!("the setting {name} isn't text"))));
    }
    let out = format!("{out}pub const SETTINGS: &[(&str, &str)] = &[{settings}];\n");
    // Every app in this repo's apps/ folder ships built in: each folder's files. The config's `apps` are the ones a new
    // install deploys and installs in `global`, by folder name.
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("apps");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut dirs: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("app.js").is_file()).collect();
    dirs.sort();
    let mut apps = String::new();
    for dir in &dirs {
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files = Vec::new();
        collect(dir, dir, &mut files);
        files.sort();
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let files: Vec<String> = files.iter().map(|(rel, abs)| format!("({rel:?}, include_bytes!({abs:?}))")).collect();
        apps.push_str(&format!("({name:?}, &[{}]),", files.join(", ")));
    }
    let mut starters = String::new();
    for name in config.get("apps").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|d| d.as_str()) {
        assert!(dirs.iter().any(|d| d.file_name().is_some_and(|f| f == name)), "the config's app {name} isn't in apps/");
        starters.push_str(&format!("{name:?},"));
    }
    let out = format!("{out}pub const BUILT_IN: &[(&str, &[(&str, &[u8])])] = &[{apps}];\npub const STARTERS: &[&str] = &[{starters}];\n");
    // The kernel services' sources, built with the pinned Bun when they're run or deployed (src/kernel.rs), so a
    // release deploys exactly the kernel it was signed with. Their tests stay in the repo.
    let kernel = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("kernel");
    println!("cargo:rerun-if-changed={}", kernel.display());
    let mut files = Vec::new();
    collect(&kernel, &kernel, &mut files);
    files.retain(|(rel, _)| !rel.starts_with("test/"));
    // The gateway serves the screens' design kit beside the web client, the same files nodes put in screens.
    let screen = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/screen");
    println!("cargo:rerun-if-changed={}", screen.display());
    let mut kit = Vec::new();
    collect(&screen, &screen, &mut kit);
    files.extend(kit.into_iter().map(|(rel, abs)| (format!("web/screen/{rel}"), abs)));
    // The white label's logo, which the web client shows where the apps show it.
    let logo = std::path::Path::new(&path).parent().unwrap().join(get("logo").as_str().unwrap());
    println!("cargo:rerun-if-changed={}", logo.display());
    files.push(("web/logo.png".into(), std::fs::canonicalize(&logo).unwrap_or_else(|e| panic!("{}: {e}", logo.display())).to_string_lossy().into()));
    files.sort();
    let files: Vec<String> = files.iter().map(|(rel, abs)| format!("({rel:?}, include_bytes!({abs:?}))")).collect();
    let out = format!("{out}pub const KERNEL: &[(&str, &[u8])] = &[{}];\n", files.join(", "));
    // The serverless node's WebAssembly and its glue (serverless/), which `update` builds into SERVERLESS where Rust
    // builds for WebAssembly, so a release carries them to devices with no Rust to deploy it from.
    println!("cargo:rerun-if-env-changed=SERVERLESS");
    let serverless: Vec<String> = match std::env::var("SERVERLESS") {
        Ok(dir) => ["one_serverless.js", "one_serverless_bg.wasm"]
            .iter()
            .map(|name| {
                let path = std::path::Path::new(&dir).join(name);
                println!("cargo:rerun-if-changed={}", path.display());
                format!("({name:?}, include_bytes!({:?}))", std::fs::canonicalize(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
            })
            .collect(),
        Err(_) => vec![],
    };
    let out = format!("{out}pub const SERVERLESS: &[(&str, &[u8])] = &[{}];\n", serverless.join(", "));
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
    // A white label's build is main's commit and its config's, so a change to either is a new build that nodes take.
    let commit = match &config_commit {
        Some((config, ..)) => format!("{commit}-{config}"),
        None => commit,
    };
    let commit = if dirty { format!("{commit}+dirty") } else { commit };
    // Rebuild when HEAD moves; in a worktree .git is a file, so ask git where HEAD lives.
    for what in ["HEAD", "refs/heads", "index"] {
        if let Ok(o) = std::process::Command::new("git").args(["rev-parse", "--git-path", what]).output() {
            println!("cargo:rerun-if-changed={}", String::from_utf8_lossy(&o.stdout).trim());
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
