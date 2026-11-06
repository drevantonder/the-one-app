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

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::Answers;
    use crate::device::Device;

    /// What a native app keeps of its screens' calls, to show at once next time (one-client's `Answers`), is its person's
    /// alone: only what tools that only read answered, sealed under that device's own key, so another person's
    /// sign-in on the same computer (a device with its own key, even in the same folder) opens none of it; and it's gone
    /// once the device signs out.
    #[tokio::test]
    async fn what_a_native_app_keeps_of_its_calls_is_its_persons_alone() {
        let dir = std::env::temp_dir().join(format!("one-answers-{}", std::process::id()));
        let (andre, sam) = ([1; 32], [2; 32]);
        // On the wire an app's tools are `ops`, and a call's tool `op`.
        let apps = serde_json::to_vec(&serde_json::json!([{ "name": "tally", "ops": [{ "name": "topics", "reads": true }, { "name": "post", "reads": false }] }])).unwrap();
        let call = |op: &str, input: serde_json::Value| serde_json::to_vec(&serde_json::json!({ "id": "1", "app": "tally", "op": op, "input": input, "client": "mac" })).unwrap();
        let mine = Answers::new(&dir, &SigningKey::from_bytes(&andre));
        mine.learn(&apps);
        mine.keep(&call("topics", serde_json::json!({ "scope": "user", "format": "json" })), br#"{"ok":[{"path":"kitchen-reno"}]}"#);
        mine.keep(&call("post", serde_json::json!({ "text": "hi" })), br#"{"ok":{"n":1}}"#);
        mine.keep(&call("topics", serde_json::json!({ "scope": "global" })), br#"{"error":"no"}"#);

        // The same call, its input's fields in any order, gets what it answered; a write, or a failure, was never kept.
        let (answer, at) = mine.kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).unwrap();
        assert_eq!(answer, br#"[{"path":"kitchen-reno"}]"#);
        assert!(at > 0);
        assert!(mine.kept("tally", "post", br#"{"text":"hi"}"#).is_none());
        assert!(mine.kept("tally", "topics", br#"{"scope":"global"}"#).is_none());
        // Nothing of it is in the clear.
        for file in std::fs::read_dir(dir.join("answers")).unwrap().flatten() {
            assert!(!String::from_utf8_lossy(&std::fs::read(file.path()).unwrap()).contains("kitchen-reno"));
        }

        // Another person's device, in the same folder, opens nothing.
        assert!(Answers::new(&dir, &SigningKey::from_bytes(&sam)).kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).is_none());
        // Only a native app's device keeps them; once it starts again, before it's read the apps, it finds them, and once
        // it signs out, they're gone.
        let cli = Device::new(andre, "https://one.invalid", dir.to_str().unwrap());
        assert!(cli.kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).is_none());
        let app = Device::new(andre, "https://one.invalid", dir.to_str().unwrap()).keeping();
        assert!(app.kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).is_some());
        app.sign_out().await;
        assert!(app.kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).is_none());
        assert!(!dir.join("answers").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
