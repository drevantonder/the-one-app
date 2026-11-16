//! What a client keeps: every answer a server gave to something that only reads, as a file in the client's home.
//! A view (an app, or the home screen) is shown what was kept at once and asked again behind it, so it opens as
//! fast with no server up as with one. An answer is kept as whoever asked it: the device's person, or who the
//! client says it acts as or for, so nobody is shown what was answered someone else.
//!
//! Something large isn't in an answer: the answer has a short-lived link to it in the bucket. The client fetches it
//! while the link lasts and keeps it, and sends what's large the same way, by a link a server hands it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::{fs, process};

use anyhow::{Context, Result, bail};
use futures_util::future::join_all;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::Client;

impl Client {
    /// Asks the server for `method`, which only reads, and keeps its answer.
    pub async fn fresh(&self, method: &str, params: &Value) -> Result<Value> {
        let answer = self.request(method, params.clone()).await?;
        self.keep(method, params, &answer)?;
        Ok(answer)
    }

    /// Keeps `answer` as what `method` was last answered with.
    pub(crate) fn keep(&self, method: &str, params: &Value, answer: &Value) -> Result<()> {
        write(&self.cache_file(method, params), answer.to_string().as_bytes())
    }

    /// What a server last answered `method` with.
    pub fn cached(&self, method: &str, params: &Value) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.cache_file(method, params)).ok()?).ok()
    }

    fn cache_file(&self, method: &str, params: &Value) -> PathBuf {
        let asked = json!([self.link().who, self.says, method, params]);
        self.home.join("cache").join(crate::hex(&Sha256::digest(asked.to_string())))
    }

    /// Fetches each large file a tool answered with, by its link, and keeps it, unless it's kept already: a blob
    /// answered by link is never replaced. The answer then says where the client keeps the file, in the link's
    /// place: `/.file/<name>/<type>`, which is what a view asks this client for it by (`file`). The name is one
    /// that only what was given this answer knows.
    pub(crate) async fn keep_files(&self, result: &mut Value) -> Result<()> {
        let Some(parts) = result.get_mut("content").and_then(Value::as_array_mut) else { return Ok(()) };
        for part in parts.iter_mut().filter(|part| part["type"] == "resource_link") {
            let (Some(link), Some(blob)) = (part["uri"].as_str(), part["name"].as_str()) else { continue };
            let secret = self.key.to_bytes();
            let name = crate::hex(&Sha256::digest([secret.as_slice(), blob.as_bytes()].concat()));
            let file = self.home.join("files").join(&name);
            if !file.exists() {
                let web = crate::web()?;
                let fetched = async { web.get(link).send().await?.error_for_status()?.bytes().await };
                write(&file, &fetched.await.context("The file couldn't be fetched. Check this device is online.")?)?;
            }
            let kind = part["mimeType"].as_str().unwrap_or("application/octet-stream");
            part["uri"] = format!("/.file/{name}/{kind}").into();
        }
        Ok(())
    }

    /// A large file this client keeps, by what follows `/.file/` in where an answer said it is: what kind of file
    /// it is, and the file.
    pub fn file(&self, path: &str) -> Option<(String, Vec<u8>)> {
        let (name, kind) = path.split_once('/')?;
        let named = !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_hexdigit());
        Some((kind.to_owned(), fs::read(self.home.join("files").join(name).as_path()).ok().filter(|_| named)?))
    }

    /// Sends a file that's too large to go in a call to where the network keeps it, for a tool of `plugin`: a
    /// server gives a short-lived link to send it by. Gives the name that tool takes for it.
    pub async fn upload(&self, plugin: &str, file: Vec<u8>) -> Result<String> {
        let answer = self.request("tools/call", json!({"name": "upload", "arguments": {"plugin": plugin}})).await?;
        let said = answer["content"][0]["text"].as_str().unwrap_or_default();
        if answer["isError"] == true {
            bail!("{said}")
        }
        let to: Value = serde_json::from_str(said).context("the server didn't say where to send the file")?;
        let (Some(method), Some(url), Some(name)) = (to["method"].as_str(), to["url"].as_str(), to["upload"].as_str())
        else {
            bail!("the server didn't say where to send the file")
        };
        let mut send = crate::web()?.request(method.parse()?, url).body(file);
        for (header, value) in to["headers"].as_object().into_iter().flatten() {
            send = send.header(header, value.as_str().unwrap_or_default());
        }
        let sent = async { send.send().await?.error_for_status() };
        sent.await.context("The file couldn't be sent. Check this device is online, then try again.")?;
        Ok(name.to_owned())
    }
}

/// Writes a file whole, under a name no other write has, then gives it its name: another ask, or another program
/// of this client, may be reading it or keeping the same thing.
pub(crate) fn write(file: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(file.parent().context("what's kept is kept in a folder")?)?;
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let writing = file.with_extension(format!("{}-{}", process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
    fs::write(&writing, bytes)?;
    fs::rename(writing, file)?;
    Ok(())
}

/// What one view has been answered since it opened. It asks through this, and every view gets the same, with
/// nothing to write: what only reads is answered from what was kept and asked again behind it, and asked again
/// after the view changes something, when its user returns to it, and when a server says something changed. When
/// an answer comes back different, the view is told to ask again, and is answered at once.
pub struct Subscription {
    client: Arc<Client>,
    /// What only reads, by what was asked, with the answer the view was last given.
    answered: Mutex<HashMap<String, (String, Value, Value)>>,
    /// Tells the view that what it was answered has changed.
    changed: Box<dyn Fn() + Send + Sync>,
}

impl Subscription {
    pub fn new(client: Arc<Client>, changed: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { client, answered: Mutex::default(), changed: Box::new(changed) })
    }

    /// Answers what the view asks: the result of `method`.
    pub async fn request(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        if self.reads(method, &params).await {
            return self.read(method, params).await;
        }
        let answer = self.client.request(method, params).await?;
        self.refresh().await;
        Ok(answer)
    }

    /// Asks the server again for everything the view was answered, and tells the view if any of it changed.
    pub async fn refresh(&self) {
        let asked: Vec<String> = self.answered.lock().expect("no view panics").keys().cloned().collect();
        self.refresh_for(asked).await
    }

    async fn read(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        let asked = json!([method, params]).to_string();
        if let Some((_, _, answer)) = self.answered.lock().expect("no view panics").get(&asked) {
            return Ok(answer.clone());
        }
        let (answer, cached) = match self.client.cached(method, &params) {
            Some(answer) => (answer, true),
            None => (self.client.fresh(method, &params).await?, false),
        };
        self.answered
            .lock()
            .expect("no view panics")
            .insert(asked.clone(), (method.to_owned(), params, answer.clone()));
        if cached {
            let shown = self.clone();
            tokio::spawn(async move { shown.refresh_for(vec![asked]).await });
        }
        Ok(answer)
    }

    async fn refresh_for(&self, asked: Vec<String>) {
        let asks = asked.into_iter().filter_map(|asked| {
            let (method, params, _) = self.answered.lock().expect("no view panics").get(&asked)?.clone();
            Some(async move { (asked, self.client.fresh(&method, &params).await) })
        });
        let mut changed = false;
        for (asked, fresh) in join_all(asks).await {
            // With no server answering, what was kept stays what's shown.
            let Ok(fresh) = fresh else { continue };
            if let Some((_, _, answer)) = self.answered.lock().expect("no view panics").get_mut(&asked)
                && *answer != fresh
            {
                *answer = fresh;
                changed = true;
            }
        }
        if changed {
            (self.changed)()
        }
    }

    /// Whether asking `method` only reads. A tool does when the server's list of tools says so.
    async fn reads(self: &Arc<Self>, method: &str, params: &Value) -> bool {
        match method {
            method if crate::resolve::READS.contains(&method) => true,
            "tools/call" => {
                let Ok(listed) = self.read("tools/list", json!({})).await else { return false };
                let tools = listed["tools"].as_array().map(Vec::as_slice).unwrap_or_default();
                tools.iter().any(|tool| tool["name"] == params["name"] && tool["annotations"]["readOnlyHint"] == true)
            }
            _ => false,
        }
    }
}
