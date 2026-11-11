//! How a machine's iroh endpoint is made and reaches the network's relay, the same for its app runtime, its services
//! and a computer joining: what devices speak (`alpn`), the preset every endpoint binds with, and the relay at the
//! network's address.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::presets;
use iroh::{RelayConfig, RelayMap, RelayUrl};

/// How long connecting to another device may take.
pub const CONNECT: Duration = Duration::from_secs(10);
/// The most an ask or its answer may carry: an email of a claim's receipts, base64, fits.
pub const MESSAGE_LIMIT: usize = 64 << 20;

/// What devices of this custom build speak: another's never meet them.
pub fn alpn() -> Vec<u8> {
    format!("{}/1", ::custom_build::config::CLI).into_bytes()
}

/// iroh's minimal endpoint without its captive portal check, a request to the relay's address on each full net report
/// that tells a device nothing it acts on. Every endpoint of an app runtime and its services binds with it; onet's own is the
/// same (onet/src/lib.rs, `Endpoints`).
pub struct Endpoints;

impl presets::Preset for Endpoints {
    fn apply(self, builder: iroh::endpoint::Builder) -> iroh::endpoint::Builder {
        let mut report = iroh::NetReportConfig::default();
        report.captive_portal_check = false;
        presets::Minimal.apply(builder).net_report_config(report)
    }
}

/// The relay at the network's address, which serves no QUIC address discovery, a Worker having no UDP; with `ticket`,
/// a pairing code's, which lets a computer joining in.
pub fn relay(address: &str, ticket: Option<&str>) -> Result<(RelayUrl, RelayMap)> {
    let url: RelayUrl = address.trim_end_matches('/').parse().context("the network's address isn't a URL")?;
    let mut config = RelayConfig::from(url.clone());
    config.quic = None;
    if let Some(ticket) = ticket {
        config = config.with_auth_token(ticket);
    }
    Ok((url, config.into()))
}
