//! This checkout's own debug build, made with the web client and the Cloudflare node it's otherwise built without.

use std::path::Path;

/// Builds the web client's and the Cloudflare node's WebAssembly into target/, at dev speed, and this checkout's debug
/// build with them in: build.rs takes them from there, so every build of this checkout holds them from then on.
pub(crate) fn with_web_client_and_cloudflare_node(checkout: &Path) -> anyhow::Result<()> {
    let config = checkout.join(one::config::CONFIG_FILE);
    eprintln!("Building the web client's and the Cloudflare node's WebAssembly (the first time takes minutes)…");
    one::build::client(checkout, &config)?;
    one::build::cloudflare_node(checkout, &config, "dev")?;
    // cargo watches only files that exist: the first time, build.rs was watching none of these.
    std::fs::File::options().write(true).open(checkout.join("build.rs"))?.set_modified(std::time::SystemTime::now())?;
    one::build::quietly(one::build::nice_cargo().current_dir(checkout).env_remove("CLIENT").env_remove("CLOUDFLARE_NODE").arg("build"))
}
