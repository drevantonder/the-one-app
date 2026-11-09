//! This checkout's own debug build, made with the web client and the Cloudflare node it's otherwise built without.

use std::path::Path;

/// Builds the web client's and the Cloudflare node's WebAssembly into target/, at dev speed (cargo keeps what hasn't
/// changed), and, unless this build already holds them as they are, this checkout's debug build with them in: build.rs
/// takes them from there, so every build of this checkout holds them from then on. A program that began before it keeps
/// the files it was built with, so a throwaway network starts again from the new build. → whether it made a new build, which
/// the program running this (an older build of this checkout) isn't.
pub(crate) fn with_web_client_and_cloudflare_node(checkout: &Path) -> anyhow::Result<bool> {
    let config = checkout.join(one::config::CONFIG_FILE);
    eprintln!("Building the web client's and the Cloudflare node's WebAssembly (the first time takes minutes)…");
    let client = one::build::client(checkout, &config)?;
    let node = one::build::cloudflare_node(checkout, &config, "dev")?;
    let held = |built: &Path, files: &[(&str, &[u8])]| !files.is_empty() && files.iter().all(|(name, bytes)| std::fs::read(built.join(name)).is_ok_and(|file| file == *bytes));
    if held(&client, one::config::CLIENT) && held(&node, one::config::CLOUDFLARE_NODE) {
        return Ok(false);
    }
    // cargo watches only files that exist: the first time, build.rs was watching none of these.
    std::fs::File::options().write(true).open(checkout.join("build.rs"))?.set_modified(std::time::SystemTime::now())?;
    one::build::quietly(one::build::nice_cargo().current_dir(checkout).env_remove("CLIENT").env_remove("CLOUDFLARE_NODE").arg("build"))?;
    Ok(true)
}
