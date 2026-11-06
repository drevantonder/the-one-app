//! The network's bucket as a client device reads it: through the URL signer at the network's address, which signs read
//! URLs for a request this device signs (`one/urls`). It only reads: what's there is sealed and signed, and is checked
//! by what opens it, so whoever passes it on can withhold it but not change it.
//!
//! A log's steps are read one after another until one's missing, and every request to a place far away costs a round
//! trip, a new connection more, so reading a step also reads the ones after it, all at once (one signing, their
//! downloads side by side), and the next reads are answered from those.

use one::config::YOURS;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result, bail, ensure};
use one::bucket::{Bucket, Object};
use serde_json::json;

/// How many positions after a step a read of it reads too.
const AHEAD: u64 = 31;

/// One agent for everything this device asks over HTTP, so connections are kept for the next request.
pub(crate) static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(std::time::Duration::from_secs(30))).build().into());

pub struct Signed {
    pub address: String,
    pub secret: [u8; 32],
    /// The steps read ahead and not yet asked for, by name: dropped once a read finds no step, the end of a log.
    ahead: Mutex<HashMap<String, Option<Object>>>,
}

impl Signed {
    pub fn new(address: String, secret: [u8; 32]) -> Signed {
        Signed { address, secret, ahead: Mutex::default() }
    }
}

/// An object, from its signed `url`.
fn download(url: &str) -> Result<Option<Object>> {
    let mut r = AGENT.get(url).call().with_context(|| format!("Couldn't reach {YOURS}'s storage"))?;
    match r.status().as_u16() {
        404 => Ok(None),
        200 => {
            let tag = r.headers().get("etag").and_then(|t| t.to_str().ok()).unwrap_or_default().to_string();
            Ok(Some(Object { bytes: r.body_mut().with_config().limit(256 << 20).read_to_vec()?, tag }))
        }
        status => bail!("{YOURS}'s storage said {status}"),
    }
}

impl Bucket for Signed {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        // In a block of its own, so the lock is never held across an await: a future that holds it can't move between
        // threads, as the native app's do.
        {
            let mut ahead = self.ahead.lock().unwrap();
            if let Some(found) = ahead.remove(name) {
                if found.is_none() {
                    ahead.clear();
                }
                return Ok(found);
            }
        }
        let (address, secret, asked) = (self.address.clone(), self.secret, name.to_string());
        let (found, next) = tokio::task::spawn_blocking(move || -> Result<_> {
            let name = &asked;
            // A database's step: the positions after it too, which come next.
            let item = match name.rsplit_once('/').and_then(|(prefix, at)| Some((prefix, at.parse::<u64>().ok()?))) {
                Some((prefix, at)) if one::db::step_name(prefix, at) == *name && at > 0 => json!({ "steps": prefix, "from": at, "to": at + AHEAD }),
                _ => json!(name),
            };
            let key = ed25519_dalek::SigningKey::from_bytes(&secret);
            let body = one::keys::signed_request(&key, "one/urls", json!({ "read": [item], "reads": one::database::READS }), crate::device::now());
            let mut r = AGENT.post(format!("{address}/urls")).send_json(body).with_context(|| format!("Couldn't reach {YOURS}"))?;
            let status = r.status().as_u16();
            let out: serde_json::Value = r.body_mut().read_json().unwrap_or_default();
            ensure!(status == 200, "{YOURS} didn't let this device read its data ({status}): {}", out["error"].as_str().unwrap_or(""));
            let urls: Vec<(String, String)> = out["read"].as_object().into_iter().flatten().filter_map(|(n, u)| Some((n.clone(), u.as_str()?.to_string()))).collect();
            let url = urls.iter().find(|(n, _)| n == name).map(|(_, u)| u.clone()).with_context(|| format!("{YOURS} didn't sign that"))?;
            // The ones after it, side by side with it; what they come to is for later, and a failure of theirs only theirs.
            Ok(std::thread::scope(|scope| {
                let rest: Vec<_> = urls.iter().filter(|(n, _)| n != name).map(|(n, u)| scope.spawn(move || (n.clone(), download(u).ok()))).collect();
                let found = download(&url);
                let next: Vec<(String, Option<Object>)> = rest.into_iter().filter_map(|t| t.join().ok()).filter_map(|(n, got)| Some((n, got?))).collect();
                (found, next)
            }))
        })
        .await??;
        let found = found?;
        if found.is_some() {
            self.ahead.lock().unwrap().extend(next);
        } else {
            self.ahead.lock().unwrap().clear();
        }
        Ok(found)
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
