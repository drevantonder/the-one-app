//! The last answer of each call an app's screens make that only reads (`reads: true`), for bridge.js's `kept`: a page
//! opened before shows at once while it asks again. Each client keeps them sealed where it keeps things (`Shelf`), for
//! its user alone, and forgets them as it signs out or is taken out: a native app as files in its folder, sealed under
//! a key from the device's own (`keys::answers_key`); a browser in IndexedDB, under a WebCrypto key of its own that
//! can't be read (cloudflare/web/net-worker.js).

use std::collections::HashMap;
use std::sync::Mutex;

use one::keys::{self, hex};
use serde_json::{Value, json};

use crate::keyring::now;
use crate::{Body, Error, Response};

/// The most answers kept, the oldest going first, and the most one may be (its JSON).
const MOST: usize = 300;
const BIGGEST: usize = 1_000_000;

/// Where a client keeps answers, sealed for its user: each by its name, the oldest past `most` going as one is put.
pub trait Shelf {
    /// What's kept as `name`, opened: none if it isn't, or doesn't open.
    fn get(&self, name: &str) -> impl Future<Output = Option<Vec<u8>>>;
    fn put(&self, name: &str, plain: &[u8], most: usize) -> impl Future<Output = ()>;
    fn forget(&self) -> impl Future<Output = ()>;
}

pub struct Answers<S> {
    shelf: S,
    /// Each app's tools that only read, as `/v1/apps` last said, once it has.
    reads: Mutex<Option<HashMap<String, Vec<String>>>>,
}

impl<S: Shelf> Answers<S> {
    pub fn new(shelf: S) -> Answers<S> {
        Answers { shelf, reads: Mutex::default() }
    }

    /// What `app`'s `tool` answered last for `input` (JSON), if it only reads: its answer's JSON, and when (ms).
    pub async fn kept(&self, app: &str, tool: &str, input: &[u8]) -> Option<(Vec<u8>, u64)> {
        if self.reads(app, tool) == Some(false) {
            return None;
        }
        let plain = self.shelf.get(&name(app, tool, &serde_json::from_slice(input).ok()?)).await?;
        let at = u64::from_be_bytes(plain.get(..8)?.try_into().ok()?);
        Some((plain[8..].to_vec(), at))
    }

    /// A node's answer to a request, as it passes: the apps' list, to learn which calls only read, and what such a
    /// call answered, kept. Each is read whole first.
    pub async fn passing(&self, method: &str, path: &str, sent: &[u8], response: Response) -> Result<Response, Error> {
        if response.status != 200 || !matches!((method, path), ("GET", "/v1/apps") | ("POST", "/v1/call")) {
            return Ok(response);
        }
        let bytes = response.body.bytes().await?;
        match path {
            "/v1/apps" => self.learn(&bytes),
            _ => self.keep(sent, &bytes).await,
        }
        Ok(Response { body: Body::whole(bytes), ..response })
    }

    fn learn(&self, apps: &[u8]) {
        let Ok(Value::Array(apps)) = serde_json::from_slice::<Value>(apps) else { return };
        let reads = apps
            .iter()
            .filter_map(|a| Some((a["name"].as_str()?.to_string(), a["tools"].as_array()?.iter().filter(|o| o["reads"] == true).filter_map(|o| Some(o["name"].as_str()?.to_string())).collect())))
            .collect();
        *self.reads.lock().unwrap() = Some(reads);
    }

    async fn keep(&self, sent: &[u8], answer: &[u8]) {
        let Ok(sent) = serde_json::from_slice::<Value>(sent) else { return };
        let (Some(app), Some(tool)) = (sent["app"].as_str(), sent["tool"].as_str()) else { return };
        if self.reads(app, tool) != Some(true) {
            return;
        }
        let Some(ok) = serde_json::from_slice::<Value>(answer).ok().and_then(|a| a.get("ok").cloned()) else { return };
        let ok = ok.to_string();
        if ok.len() > BIGGEST {
            return;
        }
        let mut plain = now().to_be_bytes().to_vec();
        plain.extend(ok.as_bytes());
        self.shelf.put(&name(app, tool, &sent["input"]), &plain, MOST).await;
    }

    pub async fn forget(&self) {
        self.shelf.forget().await;
    }

    fn reads(&self, app: &str, tool: &str) -> Option<bool> {
        self.reads.lock().unwrap().as_ref().map(|r| r.get(app).is_some_and(|tools| tools.iter().any(|o| o == tool)))
    }
}

/// What a call's answer is kept by: the hash of its app, tool and input, the input's fields in any order.
fn name(app: &str, tool: &str, input: &Value) -> String {
    hex(&keys::sha256(&keys::canonical(&json!([app, tool, input]))))
}

/// A native app's shelf: files in its device's folder (`answers/`), sealed under a key from the device's own.
#[cfg(not(target_family = "wasm"))]
pub struct Files {
    dir: std::path::PathBuf,
    key: [u8; 32],
}

#[cfg(not(target_family = "wasm"))]
impl Files {
    pub fn new(dir: &std::path::Path, key: &ed25519_dalek::SigningKey) -> Files {
        Files { dir: dir.join("answers"), key: keys::answers_key(key) }
    }
}

#[cfg(not(target_family = "wasm"))]
impl Shelf for Files {
    async fn get(&self, name: &str) -> Option<Vec<u8>> {
        keys::open(&self.key, name, &std::fs::read(self.dir.join(name)).ok()?).ok()
    }

    async fn put(&self, name: &str, plain: &[u8], most: usize) {
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(self.dir.join(name), keys::seal(0, &self.key, name, plain));
        let Ok(kept) = std::fs::read_dir(&self.dir) else { return };
        let mut kept: Vec<_> = kept.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
        if kept.len() > most {
            kept.sort();
            for (_, p) in &kept[..kept.len() - most] {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    async fn forget(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::{Answers, Files};
    use crate::device::Device;

    fn response(body: &[u8]) -> crate::Response {
        crate::Response { node: [0; 32], status: 200, headers: Vec::new(), body: crate::Body::whole(body.to_vec()), trace: None }
    }

    /// What a native app keeps of its screens' calls, to show at once next time (onet's `Answers`), is its user's
    /// alone: only what tools that only read answered, sealed under that device's own key, so another user's
    /// sign-in on the same computer (a device with its own key, even in the same folder) opens none of it; and it's gone
    /// once the device signs out.
    #[tokio::test]
    async fn what_a_native_app_keeps_of_its_calls_is_its_users_alone() {
        let dir = std::env::temp_dir().join(format!("one-answers-{}", std::process::id()));
        let (andre, sam) = ([1; 32], [2; 32]);
        let apps = serde_json::to_vec(&serde_json::json!([{ "name": "tally", "tools": [{ "name": "topics", "reads": true }, { "name": "post", "reads": false }] }])).unwrap();
        let call = |tool: &str, input: serde_json::Value| serde_json::to_vec(&serde_json::json!({ "id": "1", "app": "tally", "tool": tool, "input": input, "client": "mac" })).unwrap();
        let mine = Answers::new(Files::new(&dir, &SigningKey::from_bytes(&andre)));
        mine.passing("GET", "/v1/apps", b"", response(&apps)).await.unwrap();
        for (sent, answer) in [
            (call("topics", serde_json::json!({ "scope": "user", "format": "json" })), &br#"{"ok":[{"path":"kitchen-reno"}]}"#[..]),
            (call("post", serde_json::json!({ "text": "hi" })), br#"{"ok":{"n":1}}"#),
            (call("topics", serde_json::json!({ "scope": "global" })), br#"{"error":"no"}"#),
        ] {
            mine.passing("POST", "/v1/call", &sent, response(answer)).await.unwrap();
        }

        // The same call, its input's fields in any order, gets what it answered; a write, or a failure, was never kept.
        let (answer, at) = mine.kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).await.unwrap();
        assert_eq!(answer, br#"[{"path":"kitchen-reno"}]"#);
        assert!(at > 0);
        assert!(mine.kept("tally", "post", br#"{"text":"hi"}"#).await.is_none());
        assert!(mine.kept("tally", "topics", br#"{"scope":"global"}"#).await.is_none());
        for file in std::fs::read_dir(dir.join("answers")).unwrap().flatten() {
            assert!(!String::from_utf8_lossy(&std::fs::read(file.path()).unwrap()).contains("kitchen-reno"));
        }

        assert!(Answers::new(Files::new(&dir, &SigningKey::from_bytes(&sam))).kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).await.is_none());
        // Only a native app's device keeps them; once it starts again, before it's read the apps, it finds them, and once
        // it signs out, they're gone.
        let cli = Device::new(andre, "https://one.invalid", dir.to_str().unwrap());
        assert!(cli.kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).await.is_none());
        let app = Device::new(andre, "https://one.invalid", dir.to_str().unwrap()).keeping();
        assert!(app.kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).await.is_some());
        app.sign_out().await;
        assert!(app.kept("tally", "topics", br#"{"format":"json","scope":"user"}"#).await.is_none());
        assert!(!dir.join("answers").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
