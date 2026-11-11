//! The web client's and the Cloudflare node's WebAssembly, built in this checkout for its throwaway network, which reads
//! them from there.

use std::path::Path;

/// Builds both into target/, quickly (cargo keeps what hasn't changed), with one profile, so they share what they compile.
pub(crate) fn web_client_and_oned_cloudflare(checkout: &Path) -> anyhow::Result<()> {
    let config = checkout.join(custom_build::config::CONFIG_FILE);
    eprintln!("Building the web client's and oned on Cloudflare's WebAssembly (the first time takes minutes)…");
    crate::build::onet(checkout, &config, "web-dev")?;
    crate::build::oned_cloudflare(checkout, &config, "web-dev")?;
    Ok(())
}
