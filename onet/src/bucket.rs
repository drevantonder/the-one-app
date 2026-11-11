//! The network's bucket as a client device reads it: through the URL signer at the network's address, which signs read
//! URLs for a request this device signs (`one/urls`). It only reads: what's there is sealed and signed, and is checked
//! by what opens it, so whoever passes it on can withhold it but not change it.
//!
//! A log's steps are read one after another until one's missing, and every request to a place far away costs a round
//! trip, a new connection more, so reading a step that's there also reads the ones after it, all at once (one signing,
//! their downloads side by side), and the next reads are answered from those. Past the end, where a device asks most
//! often, only the step asked for is read.

use ::custom_build::config::YOURS;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Context, Result, bail};
use ::bucket::{Bucket, Object, Signer, signer_requests};

/// One agent for everything this device asks over HTTP, so connections are kept for the next request.
pub(crate) static AGENT: LazyLock<ureq::Agent> = LazyLock::new(signer_requests);

/// What a native app downloaded of the bucket, counted as it was.
static READS: ::bucket::Reads = ::bucket::Reads::new();

pub struct Signed {
    signer: Arc<Signer>,
    /// The steps read ahead and not yet asked for, by name: dropped once a read finds no step, the end of a log.
    ahead: Mutex<HashMap<String, Option<Object>>>,
}

impl Signed {
    fn new(address: String, secret: [u8; 32]) -> Signed {
        Signed { signer: Arc::new(Signer::reading(address, ed25519_dalek::SigningKey::from_bytes(&secret))), ahead: Mutex::default() }
    }
}

fn download(url: &str) -> Result<Option<Object>> {
    let request = ureq::http::Request::get(url).body(Vec::new())?;
    let (mut r, _) = platform::hedge::run(&AGENT, "bucket", request, platform::hedge::Twice::Safe).with_context(|| format!("Couldn't reach {YOURS}'s storage"))?;
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
        let (signer, asked, reads) = (self.signer.clone(), name.to_string(), &READS);
        let (found, next) = tokio::task::spawn_blocking(move || -> Result<_> {
            let (url, urls) = signer.reads_from(&asked).map_err(|e| anyhow::anyhow!("Couldn't read from {YOURS}: {e:#}"))?;
            // The ones after it, once it's there, side by side; what they come to is for later, and a failure of theirs
            // only theirs.
            let found = download(&url);
            if let Ok(f) = &found {
                reads.count(&asked, f.is_some());
            }
            if !matches!(found, Ok(Some(_))) {
                return Ok((found, Vec::new()));
            }
            Ok(std::thread::scope(|scope| {
                let rest: Vec<_> = urls.iter().map(|(n, u)| scope.spawn(|| (n.clone(), download(u).inspect(|f| reads.count(n, f.is_some())).ok()))).collect();
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

/// The network's bucket as a device reaches it: through the URL signer, only reading, as a phone or a native app does;
/// or on a computer, the place its home's network file names (`::bucket::Place`), a folder here or S3 through the URL
/// signer, which it may write in too, as anyone in the network may create there what isn't yet. What it reads it counts,
/// and says itself as `bucket.reads` once a minute, to the telemetry Worker at the network's address, if it has one.
pub struct Reached {
    at: Reach,
    reads: &'static ::bucket::Reads,
    key: ed25519_dalek::SigningKey,
    address: Option<String>,
    client: &'static str,
}

enum Reach {
    Signed(Signed),
    #[cfg(feature = "computer")]
    Place(::bucket::Place),
}

impl Reached {
    /// A native app's: through the URL signer at `address`.
    pub fn signed(address: String, secret: [u8; 32]) -> Reached {
        Reached { at: Reach::Signed(Signed::new(address.clone(), secret)), reads: &READS, key: ed25519_dalek::SigningKey::from_bytes(&secret), address: Some(address), client: "app" }
    }

    /// A computer's onet's: `place`, of the network at `address` if it has one yet.
    #[cfg(feature = "computer")]
    pub fn place(place: ::bucket::Place, secret: [u8; 32], address: Option<String>) -> Reached {
        Reached { at: Reach::Place(place), reads: &::bucket::READS, key: ed25519_dalek::SigningKey::from_bytes(&secret), address, client: "onet" }
    }

    /// What it read, if it's been a minute: sent on a thread of its own, as telemetry may be lost and nothing waits for it.
    fn say(&self) {
        let Some(address) = self.address.clone() else { return };
        let Some(rows) = one::insights::bucket_reads(&self.reads, crate::keyring::now(), &key::hex(&self.key.verifying_key().to_bytes()), self.client) else { return };
        let body = key::signed_request(&self.key, ::custom_build::protocol::TELEMETRY, rows, key::next_at(crate::keyring::now()));
        std::thread::spawn(move || {
            let sent = ureq::http::Request::post(format!("{address}/telemetry/write")).header("content-type", "application/json").body(serde_json::to_vec(&body).unwrap_or_default());
            if let Ok(request) = sent {
                let _ = platform::hedge::run(&AGENT, "telemetry", request, platform::hedge::Twice::Never);
            }
        });
    }
}

impl Bucket for Reached {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        let got = match &self.at {
            Reach::Signed(b) => b.get(name).await,
            #[cfg(feature = "computer")]
            Reach::Place(b) => Bucket::get(b, name).await,
        }?;
        self.say();
        Ok(got)
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        match &self.at {
            Reach::Signed(b) => b.create(name, bytes).await,
            #[cfg(feature = "computer")]
            Reach::Place(b) => Bucket::create(b, name, bytes).await,
        }
    }

    async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
        match &self.at {
            Reach::Signed(b) => b.replace(name, bytes, tag).await,
            #[cfg(feature = "computer")]
            Reach::Place(b) => Bucket::replace(b, name, bytes, tag).await,
        }
    }

    async fn wait(&self, ms: u64) {
        n0_future::time::sleep(n0_future::time::Duration::from_millis(ms)).await
    }

    fn now(&self) -> u64 {
        match &self.at {
            Reach::Signed(b) => b.now(),
            #[cfg(feature = "computer")]
            Reach::Place(b) => Bucket::now(b),
        }
    }

    async fn delete(&self, name: &str) -> Result<()> {
        match &self.at {
            Reach::Signed(b) => b.delete(name).await,
            #[cfg(feature = "computer")]
            Reach::Place(b) => Bucket::delete(b, name).await,
        }
    }
}
