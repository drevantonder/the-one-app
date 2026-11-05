// The custom build's CLI name, from config.toml (or the one CONFIG names), as the node's build reads it: what its
// clients' ALPN is named after.
fn main() {
    // A relative CONFIG is from the workspace's root, where the node's build reads it.
    let path = std::path::Path::new("..").join(std::env::var("CONFIG").unwrap_or_else(|_| "config.toml".into())).display().to_string();
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed=CONFIG");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let config: toml::Table = text.parse().expect("parse config file");
    let cli = config.get("cli").and_then(|c| c.as_str()).expect("config file lacks cli");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("config.rs");
    std::fs::write(out, format!("pub const CLI: &str = {cli:?};\n")).unwrap();
}
