//! The last answer of each call a native app's screens make that only reads (`reads: true`), for bridge.js's `kept`:
//! a page opened before shows at once while it asks again. Each is a file in the device's folder (one per person at
//! each address), sealed under a key from the device's own key (`keys::answers_key`), which the app keeps in the
//! Keychain, the keystore or the Credential Locker and forgets with the sign-in: another person's device, with its own
//! key, opens none. Forgotten as it signs out or is taken out. cloudflare/web/answers.js is the same in a browser.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ed25519_dalek::SigningKey;
use one::keys::hex;
use one::keys;
use serde_json::{Value, json};

use crate::device::now;

/// The most answers kept, the oldest going first, and the most one may be (its JSON).
const MOST: usize = 300;
const BIGGEST: usize = 1_000_000;

pub struct Answers {
    dir: PathBuf,
    key: [u8; 32],
    /// Each app's tools that only read, as `/v1/apps` last said, once it has.
    reads: Mutex<Option<HashMap<String, Vec<String>>>>,
}

impl Answers {
    /// A device's, kept in `dir` (its folder) under its `key`.
    pub fn new(dir: &Path, key: &SigningKey) -> Answers {
        Answers { dir: dir.join("answers"), key: keys::answers_key(key), reads: Mutex::default() }
    }

    /// What `app`'s `tool` answered last for `input` (JSON), if it only reads: its answer's JSON, and when (ms).
    pub fn kept(&self, app: &str, tool: &str, input: &[u8]) -> Option<(Vec<u8>, u64)> {
        if self.reads(app, tool) == Some(false) {
            return None;
        }
        let (path, name) = self.at(app, tool, &serde_json::from_slice(input).ok()?);
        let plain = keys::open(&self.key, &name, &std::fs::read(path).ok()?).ok()?;
        let at = u64::from_be_bytes(plain.get(..8)?.try_into().ok()?);
        Some((plain[8..].to_vec(), at))
    }

    /// Learns which tools only read from the whole list of apps (`GET /v1/apps`'s answer).
    pub fn learn(&self, apps: &[u8]) {
        let Ok(Value::Array(apps)) = serde_json::from_slice::<Value>(apps) else { return };
        let reads = apps
            .iter()
            .filter_map(|a| Some((a["name"].as_str()?.to_string(), a["ops"].as_array()?.iter().filter(|o| o["reads"] == true).filter_map(|o| Some(o["name"].as_str()?.to_string())).collect())))
            .collect();
        *self.reads.lock().unwrap() = Some(reads);
    }

    /// Keeps what a call (`sent`, `/v1/call`'s body) answered, if its tool only reads, as `learn` last said, and
    /// it answered rather than failed.
    pub fn keep(&self, sent: &[u8], answer: &[u8]) {
        let Ok(sent) = serde_json::from_slice::<Value>(sent) else { return };
        let (Some(app), Some(tool)) = (sent["app"].as_str(), sent["op"].as_str()) else { return };
        if self.reads(app, tool) != Some(true) {
            return;
        }
        let Some(ok) = serde_json::from_slice::<Value>(answer).ok().and_then(|a| a.get("ok").cloned()) else { return };
        let ok = ok.to_string();
        if ok.len() > BIGGEST {
            return;
        }
        let (path, name) = self.at(app, tool, &sent["input"]);
        let mut plain = now().to_be_bytes().to_vec();
        plain.extend(ok.as_bytes());
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(path, keys::seal(0, &self.key, &name, &plain));
        let Ok(kept) = std::fs::read_dir(&self.dir) else { return };
        let mut kept: Vec<_> = kept.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
        if kept.len() > MOST {
            kept.sort();
            for (_, p) in &kept[..kept.len() - MOST] {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    /// Every answer kept goes.
    pub fn forget(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Whether `app`'s `tool` only reads, as `learn` last said; None until it has (an answer was kept only if it did then).
    fn reads(&self, app: &str, tool: &str) -> Option<bool> {
        self.reads.lock().unwrap().as_ref().map(|r| r.get(app).is_some_and(|tools| tools.iter().any(|o| o == tool)))
    }

    /// Where a call's last answer is, by what it asked, and the name its seal is bound to.
    fn at(&self, app: &str, tool: &str, input: &Value) -> (PathBuf, String) {
        let name = hex(&keys::sha256(&keys::canonical(&json!([app, tool, input]))));
        (self.dir.join(&name), name)
    }
}
