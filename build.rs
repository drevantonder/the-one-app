// Turns config.toml (or the one CONFIG names) into constants, so the code never names the product.
fn main() {
    // Builds that still say BRAND= would otherwise build The One App under another edition's name.
    assert!(std::env::var_os("BRAND").is_none(), "BRAND= is CONFIG= now, naming a config.toml");
    let path = std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".into());
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed=CONFIG");
    println!("cargo:rerun-if-env-changed=BRAND");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let brand: toml::Table = text.parse().expect("parse config file");
    let get = |key: &str| brand.get(key).unwrap_or_else(|| panic!("config file lacks {key}")).clone();
    let out = format!(
        "pub const NAME: &str = {:?};\npub const CLI: &str = {:?};\npub const PORT: u16 = {};\npub const ACCENT: &str = {:?};\npub const REPO: &str = {:?};\n",
        get("name").as_str().unwrap(),
        get("cli").as_str().unwrap(),
        get("port").as_integer().unwrap(),
        get("accent").as_str().unwrap(),
        // The GitHub repo releases come from; a brand without one isn't released.
        brand.get("repo").and_then(|r| r.as_str()).unwrap_or(""),
    );
    let optional = |key: &str| brand.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    // The config file, inside the repo if it is, so `update` builds the same edition from a fresh checkout.
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let file = std::fs::canonicalize(&path).unwrap();
    let file = file.strip_prefix(std::fs::canonicalize(&manifest).unwrap()).unwrap_or(&file).to_string_lossy().to_string();
    let out = format!("{out}pub const CONFIG_FILE: &str = {file:?};\n");
    let out = format!("{out}pub const RELEASE_KEY: &str = {:?};\npub const RELEASE_SIGNER: &str = {:?};\npub const STORE: &str = {:?};\npub const PUSH: &str = {:?};\n", optional("release_key"), optional("release_signer"), optional("store"), optional("push_driver"));
    // Every app in this repo's apps/ folder ships built in: each folder's files. The brand's `apps` are the ones a new
    // install deploys and installs in `global`; they must be among them.
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
    let base = std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    let mut starters = String::new();
    for dir in brand.get("apps").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|d| d.as_str()) {
        let dir = std::fs::canonicalize(base.join(dir)).unwrap_or_else(|_| panic!("brand app {dir} isn't there"));
        assert!(dirs.iter().any(|d| std::fs::canonicalize(d).is_ok_and(|d| d == dir)), "brand app {} isn't in apps/", dir.display());
        starters.push_str(&format!("{:?},", dir.file_name().unwrap().to_string_lossy()));
    }
    let out = format!("{out}pub const BUILT_IN: &[(&str, &[(&str, &[u8])])] = &[{apps}];\npub const STARTERS: &[&str] = &[{starters}];\n");
    // What the App Store calls the apps made in the install rather than built in.
    let out = format!("{out}pub const MADE_HERE: &str = {:?};\n", brand.get("made_here").and_then(|v| v.as_str()).unwrap_or("Made here"));
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
    let commit = if dirty { format!("{commit}+dirty") } else { commit };
    // Rebuild when HEAD moves; in a worktree .git is a file, so ask git where HEAD lives.
    for what in ["HEAD", "refs/heads", "index"] {
        if let Ok(o) = std::process::Command::new("git").args(["rev-parse", "--git-path", what]).output() {
            println!("cargo:rerun-if-changed={}", String::from_utf8_lossy(&o.stdout).trim());
        }
    }
    // When that commit was made, so releases only ever move devices forward.
    let committed: u64 = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0);
    let out = format!("{out}pub const COMMITTED: u64 = {committed};\npub const BUILD: &str = {commit:?};\npub const TARGET: &str = {:?};\n", std::env::var("TARGET").unwrap());
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("brand.rs");
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
