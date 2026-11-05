//! What the page polls, kept. It asks for the list every few seconds and for each running device's picture now and
//! then, and asking the devices takes a while (the other devices' services over the relay, Podman, simctl, adb, a
//! picture taken). So the last answer is kept and given at once, and asked again in the background once it's older
//! than fresh. One older than stale, or none yet, is waited for. A change made here forgets the list, so whoever made
//! it sees it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};

use super::Devices;

pub struct Latest {
    fresh: Duration,
    stale: Duration,
    now: Mutex<Kept>,
}

#[derive(Default)]
struct Kept {
    val: Option<Value>,
    at: Option<Instant>,
    /// Which forget it's from, so an answer asked before one isn't kept.
    generation: u64,
    asking: bool,
}

type Ask = Box<dyn FnOnce() -> Result<Value> + Send>;

impl Latest {
    pub fn new(fresh: Duration, stale: Duration) -> Latest {
        Latest { fresh, stale, now: Mutex::new(Kept::default()) }
    }

    /// The answer kept, asked again in the background if it isn't fresh, or asked now if there's none or it's stale:
    /// with when it was asked.
    fn get(self: &Arc<Self>, ask: Ask) -> Result<(Value, Instant)> {
        let mut k = self.now.lock().unwrap();
        if let (Some(v), Some(at)) = (k.val.clone(), k.at)
            && at.elapsed() < self.stale
        {
            if at.elapsed() > self.fresh && !k.asking {
                k.asking = true;
                let (me, generation) = (self.clone(), k.generation);
                std::thread::spawn(move || {
                    let _ = me.ask(ask, generation);
                });
            }
            return Ok((v, at));
        }
        let generation = k.generation;
        drop(k);
        self.ask(ask, generation)
    }

    fn ask(&self, ask: Ask, generation: u64) -> Result<(Value, Instant)> {
        let v = ask();
        let now = Instant::now();
        let mut k = self.now.lock().unwrap();
        k.asking = false;
        let v = v?;
        if generation == k.generation {
            (k.val, k.at) = (Some(v.clone()), Some(now));
        }
        Ok((v, now))
    }

    /// The answer it has, if any.
    fn kept(&self) -> Option<Value> {
        self.now.lock().unwrap().val.clone()
    }

    pub fn forget(&self) {
        let mut k = self.now.lock().unwrap();
        (k.val, k.at) = (None, None);
        k.generation += 1;
    }
}

/// The devices' pictures, kept by device and width.
#[derive(Default)]
pub struct Pictures(Mutex<HashMap<String, Arc<Latest>>>);

impl Pictures {
    /// Drops a device's pictures, when it stops or goes.
    pub fn forget(&self, name: &str) {
        self.0.lock().unwrap().retain(|key, _| !key.starts_with(&format!("{name}/")));
    }
}

/// The image in a screen's answer, if any.
fn picture_in(v: &Value) -> Option<image::DynamicImage> {
    let item = v["content"].as_array()?.iter().find(|c| c["type"] == "image")?;
    let raw = data_encoding::BASE64.decode(item["data"].as_str()?.as_bytes()).ok()?;
    super::picture::decode(&raw).ok()
}

impl Devices {
    /// This service's state, shared.
    fn arc(&self) -> Arc<Devices> {
        super::DEVICES.get().expect("made before its first call").clone()
    }

    /// list, kept: this device's, or with others, every device's.
    pub(super) fn list_latest(&self, here: bool) -> Result<Value> {
        let ds = self.arc();
        if here {
            return Ok(self.latest_here.get(Box::new(move || Ok(json!({ "devices": ds.list_here()? }))))?.0);
        }
        Ok(self.latest_all.get(Box::new(move || ds.list_all(true)))?.0)
    }

    /// A device's picture, kept for a little while: a page watching it asks again every so often, and gets the one
    /// taken in the background since. Its hash says which it is: given the one it has (have), it's told it's the same
    /// rather than sent it again. A picture that's hardly changed (a phone's clock) counts as the same.
    pub(super) fn screen_latest(&self, name: &str, width: u32, have: &str) -> Result<Value> {
        let k = self.pics.0.lock().unwrap().entry(format!("{name}/{width}")).or_insert_with(|| Arc::new(Latest::new(Duration::from_secs(15), Duration::from_secs(24 * 3600)))).clone();
        let (ds, kept, name) = (self.arc(), k.clone(), name.to_string());
        let (v, at) = k.get(Box::new(move || {
            let r = ds.screen_now(&name, width)?;
            if let Some(was) = kept.kept()
                && let (Some(a), Some(b)) = (picture_in(&was["picture"]), picture_in(&r))
                && super::picture::alike(&a, &b)
            {
                return Ok(was);
            }
            use sha2::Digest;
            let sum = sha2::Sha256::digest(r.to_string().as_bytes());
            Ok(json!({ "picture": r, "hash": data_encoding::BASE64URL_NOPAD.encode(&sum[..9]) }))
        }))?;
        let age = at.elapsed().as_secs();
        if !have.is_empty() && v["hash"] == have {
            return Ok(json!({ "same": true, "hash": v["hash"], "age": age }));
        }
        let mut out = json!({ "hash": v["hash"], "age": age });
        if let Some(p) = v["picture"].as_object() {
            for (k, v) in p {
                out[k] = v.clone();
            }
        }
        Ok(out)
    }
}
