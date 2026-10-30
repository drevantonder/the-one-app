// Turns brand.toml into constants, so the code never names the product.
fn main() {
    let path = std::env::var("BRAND").unwrap_or_else(|_| "brand.toml".into());
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed=BRAND");
    let text = std::fs::read_to_string(&path).expect("read brand file");
    let brand: toml::Table = text.parse().expect("parse brand file");
    let get = |key: &str| brand.get(key).unwrap_or_else(|| panic!("brand file lacks {key}")).clone();
    let out = format!(
        "pub const NAME: &str = {:?};\npub const CLI: &str = {:?};\npub const PORT: u16 = {};\npub const ACCENT: &str = {:?};\n",
        get("name").as_str().unwrap(),
        get("cli").as_str().unwrap(),
        get("port").as_integer().unwrap(),
        get("accent").as_str().unwrap(),
    );
    // Which build this is, so a node can tell a release apart from itself.
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "dev".into());
    // Rebuild when HEAD moves; in a worktree .git is a file, so ask git where HEAD lives.
    for what in ["HEAD", "refs/heads"] {
        if let Ok(o) = std::process::Command::new("git").args(["rev-parse", "--git-path", what]).output() {
            println!("cargo:rerun-if-changed={}", String::from_utf8_lossy(&o.stdout).trim());
        }
    }
    let out = format!("{out}pub const BUILD: &str = {commit:?};\npub const TARGET: &str = {:?};\n", std::env::var("TARGET").unwrap());
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("brand.rs");
    std::fs::write(dest, out).unwrap();
}
