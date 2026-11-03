//! The install's bucket as a client device reads it: through the URL signer at the install's address, which signs read
//! URLs for a request this device signs (`one/urls`). It only reads: what's there is sealed and signed, and is checked
//! by what opens it, so whoever passes it on can withhold it but not change it.

use anyhow::{Context, Result, bail, ensure};
use one::bucket::{Bucket, Object};
use serde_json::json;

pub struct Signed {
    pub address: String,
    pub secret: [u8; 32],
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(std::time::Duration::from_secs(30))).build().into()
}

impl Bucket for Signed {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        let (address, secret, name) = (self.address.clone(), self.secret, name.to_string());
        tokio::task::spawn_blocking(move || {
            let key = ed25519_dalek::SigningKey::from_bytes(&secret);
            let asked = one::keys::signed_request(&key, "one/urls", json!({ "read": [name] }), crate::device::now());
            let mut r = agent().post(format!("{address}/urls")).send_json(asked).context("Couldn't reach your One App")?;
            let status = r.status().as_u16();
            let out: serde_json::Value = r.body_mut().read_json().unwrap_or_default();
            ensure!(status == 200, "your One App didn't let this device read its data ({status}): {}", out["error"].as_str().unwrap_or(""));
            let url = out["read"][&name].as_str().context("your One App didn't sign that")?;
            let mut r = agent().get(url).call().context("Couldn't reach your One App's storage")?;
            match r.status().as_u16() {
                404 => Ok(None),
                200 => {
                    let tag = r.headers().get("etag").and_then(|t| t.to_str().ok()).unwrap_or_default().to_string();
                    Ok(Some(Object { bytes: r.body_mut().with_config().limit(256 << 20).read_to_vec()?, tag }))
                }
                status => bail!("your One App's storage said {status}"),
            }
        })
        .await?
    }

    async fn create(&self, _: &str, _: &[u8]) -> Result<bool> {
        bail!("a client device only reads the bucket")
    }

    async fn replace(&self, _: &str, _: &[u8], _: Option<&str>) -> Result<bool> {
        bail!("a client device only reads the bucket")
    }

    async fn wait(&self, ms: u64) {
        n0_future::time::sleep(n0_future::time::Duration::from_millis(ms)).await
    }
}
