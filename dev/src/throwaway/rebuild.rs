//! The web client's and the Cloudflare node's WebAssembly, built in this checkout for its throwaway network, which reads
//! them from there.

use std::path::Path;

/// Builds both into target/, quickly (cargo keeps what hasn't changed), with one profile, so they share what they compile.
pub(crate) fn web_client_and_cloudflare_node(checkout: &Path) -> anyhow::Result<()> {
    let config = checkout.join(custom_build::config::CONFIG_FILE);
    eprintln!("Building the web client's and the Cloudflare node's WebAssembly (the first time takes minutes)…");
    one::build::onet(checkout, &config, "web-dev")?;
    one::build::cloudflare_node(checkout, &config, "web-dev")?;
    Ok(())
}
