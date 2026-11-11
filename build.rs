// What the program carries (`one::carried`): the built-in apps. config.toml's constants are custom-build's
// (custom-build/build.rs); the Workers' and web client's files, and the WebAssembly, ui's (ui/build.rs).
#[path = "ui/src/ui_files.rs"]
#[allow(dead_code)]
mod ui_files;

fn main() {
    let path = std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".into());
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed=CONFIG");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let config: toml::Table = text.parse().expect("parse config file");
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
        files.extend(ui_files::collect(dir).into_iter().map(|(rel, abs)| (rel, abs.to_string_lossy().to_string())));
        // Its service's source is built apart (`one-dev update services`), and its tests run with ours: neither ships in its bundle.
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
    let out = format!("pub const BUILT_IN: &[(&str, &[(&str, &[u8])])] = &[{apps}];\npub const STARTERS: &[&str] = &[{starters}];\n");
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("carried.rs");
    std::fs::write(dest, out).unwrap();
}
