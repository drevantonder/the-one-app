//! What a client keeps: every answer a server gave to something that only reads, as a file in the client's home.
//! A view (an app, or the home screen) is shown what was kept at once and asked again behind it, so it opens as
//! fast with no server up as with one. An answer is kept as whoever asked it: the device's person, or who the
//! client says it acts as or for, so nobody is shown what was answered someone else.
//!
//! Something large isn't in an answer: the answer has a short-lived link to it in the bucket. The client fetches it
//! while the link lasts and keeps it, and sends what's large the same way, by a link a server hands it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use std::fs;
use std::io::Write;

use anyhow::{Context, Result, bail};
use futures_util::future::join_all;
use futures_util::stream::{self, StreamExt, TryStreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::Client;

impl Client {
    /// Asks the server for `method`, which only reads, and keeps its answer. A list of files is answered once it's
    /// whole (`keep_whole`).
    pub async fn fresh(&self, method: &str, params: &Value) -> Result<Value> {
        let response = self.request(method, params.clone()).await?;
        if method == "resources/list" {
            return Ok(self.keep_whole(params, response).await);
        }
        self.keep(method, params, &response)?;
        Ok(response)
    }

    /// A list of files a server gave, as a page loads its files from it: each version's files (`ui://<app>/<code>/…`,
    /// which never change) are kept first, so a page and everything it loads are one version's, all here, as a CDN's
    /// hashed assets are, whatever the server keeps after. A server keeps only the last few versions' files, and one
    /// still moving to the latest may list a version whose files are gone: a list whose files couldn't all be kept
    /// isn't taken, and the one kept before it is answered, whose files all are, while Telemetry hears why. With none
    /// kept yet, as on a first start, it's answered at once and kept once it's whole, as its files are fetched anyway.
    /// It's boxed, as asking a server may ask for its list of tools again (`fresh`), which this is in.
    fn keep_whole(&self, params: &Value, listed: Value) -> futures_util::future::BoxFuture<'static, Value> {
        let Some(client) = self.me.upgrade() else { return Box::pin(async { listed }) };
        let kept = client.cached("resources/list", params);
        let runtime = self.runtime.clone();
        let keeping = {
            let (params, listed) = (params.clone(), listed.clone());
            async move {
                let files = listed["resources"].as_array().into_iter().flatten().filter_map(|resource| resource["uri"].as_str());
                // Each read as a page asks it, for whoever the list is for (`_meta.subject`), so it's what it finds.
                let reads = files.filter(|uri| uri.starts_with("ui://")).map(|uri| {
                    let mut read = params.clone();
                    read["uri"] = json!(uri);
                    read
                });
                let missing: Vec<Value> = reads.filter(|read| !client.cache_file("resources/read", read).exists()).collect();
                // What's in each file, by its SHA-256, where the server says: one kept from a version before is kept
                // for this one from that, not read again, as a CDN's content-addressed assets are.
                let hashes: HashMap<String, String> = listed["resources"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|resource| Some((resource["uri"].as_str()?.to_owned(), resource["_meta"]["sha256"].as_str()?.to_owned())))
                    .filter(|(_, hash)| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
                    .collect();
                let fetch = |read: Value| {
                    let client = client.clone();
                    let hash = read["uri"].as_str().and_then(|uri| hashes.get(uri)).cloned();
                    async move {
                        let uri = read["uri"].clone();
                        let kept = hash.as_deref().and_then(|hash| fs::read(client.home.join("cache").join("sha256").join(hash)).ok());
                        if let Some(content) = kept.and_then(|kept| serde_json::from_slice::<Value>(&kept).ok()) {
                            let mut content = content;
                            content["uri"] = uri;
                            return client.keep("resources/read", &read, &json!({"contents": [content]}));
                        }
                        let response = client.request("resources/read", read.clone()).await?;
                        if let Some(hash) = &hash {
                            let mut content = response["contents"][0].clone();
                            if let Some(content) = content.as_object_mut() {
                                content.remove("uri");
                            }
                            write(&client.home.join("cache").join("sha256").join(hash), content.to_string().as_bytes())?;
                        }
                        client.keep("resources/read", &read, &response)
                    }
                };
                let whole = stream::iter(missing).map(fetch).buffer_unordered(8).try_collect::<Vec<_>>().await;
                let whole = whole.and_then(|_| client.keep("resources/list", &params, &listed));
                if let Err(error) = &whole {
                    client.report(json!({"name": "one.client.cache", "error": format!("{error:#}")}));
                }
                whole.is_ok()
            }
        };
        Box::pin(async move {
            match kept {
                Some(kept) => if keeping.await { listed } else { kept },
                None => {
                    runtime.spawn(keeping);
                    listed
                }
            }
        })
    }

    /// Keeps `answer` as what `method` was last answered with.
    pub(crate) fn keep(&self, method: &str, params: &Value, response: &Value) -> Result<()> {
        write(&self.cache_file(method, params), response.to_string().as_bytes())
    }

    /// What a server last answered `method` with.
    pub fn cached(&self, method: &str, params: &Value) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.cache_file(method, params)).ok()?).ok()
    }

    /// What a server last answered `method` with, and when it was written on disk.
    fn cached_entry(&self, method: &str, params: &Value) -> Option<(Value, Option<SystemTime>)> {
        let path = self.cache_file(method, params);
        let modified = fs::metadata(&path).ok().and_then(|meta| meta.modified().ok());
        let value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
        Some((value, modified))
    }

    fn cache_file(&self, method: &str, params: &Value) -> PathBuf {
        let key = json!([self.link().who, self.says, method, params]);
        self.home.join("cache").join(crate::hex(&Sha256::digest(key.to_string())))
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

    /// Sends a file that's too large to go in a call to where the network keeps it, for a tool of `app`: a
    /// server gives a short-lived link to send it by. Gives the name that tool takes for it.
    pub async fn upload(&self, app: &str, file: Vec<u8>) -> Result<String> {
        let response = self.request("tools/call", json!({"name": "upload", "arguments": {"app": app}})).await?;
        let text = response["content"][0]["text"].as_str().unwrap_or_default();
        if response["isError"] == true {
            bail!("{text}")
        }
        let to: Value = serde_json::from_str(text).context("the server didn't say where to send the file")?;
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
    let folder = file.parent().context("what's kept is kept in a folder")?;
    fs::create_dir_all(folder)?;
    let mut writing = tempfile::NamedTempFile::new_in(folder)?;
    writing.write_all(bytes)?;
    // Named by std's rename, not tempfile's `persist`: on Windows, MoveFileEx alone is refused (Access is denied)
    // where another ask has the file open to read it, and std's rename then replaces it as POSIX's does.
    let written = writing.into_temp_path().keep()?;
    fs::rename(&written, file).inspect_err(|_| drop(fs::remove_file(&written)))?;
    Ok(())
}

/// One cached read in a `Subscription`, following TanStack Query's invalidation model: a write marks matching entries
/// stale (`invalidated > fetched`) and starts an independent background refetch (`fetching`). Subsequent reads join
/// the in-flight refetch rather than returning stale pre-write data, and when any refetch changes an answer, the view
/// is notified immediately without waiting for other queries.
struct Query {
    method: String,
    params: Value,
    /// The last answer the view was given.
    response: Value,
    /// Incremented on every write invalidation; `response` is fresh while `fetched >= invalidated`.
    invalidated: u64,
    /// The invalidation generation `response` was fetched at or after.
    fetched: u64,
    /// Active post-write refetch for `generation`, shared by any `read` waiting for a fresh answer.
    fetching: Option<(u64, watch::Receiver<Option<bool>>)>,
}

/// What one view has been answered since it opened. It asks through this, and every view gets the same, with
/// nothing to write: what only reads is answered from what was kept and asked again behind it; writes resolve as
/// soon as their own answer arrives, invalidating the reads they may affect and refetching them in the background;
/// and reads are also asked again when the user returns to the view or a server says something changed. When an
/// answer comes back different, the view is told to ask again, and is answered at once.
pub struct Subscription {
    client: Arc<Client>,
    /// What only reads, by what was asked, with the answer the view was last given and its invalidation state.
    responses: Mutex<HashMap<String, Query>>,
    /// When this view last completed a write, so a cold read doesn't serve a pre-write disk cache entry.
    written: Mutex<Option<SystemTime>>,
    /// Tells the view that what it was answered has changed.
    changed: Box<dyn Fn() + Send + Sync>,
}

impl Subscription {
    pub fn new(client: Arc<Client>, changed: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            client,
            responses: Mutex::default(),
            written: Mutex::default(),
            changed: Box::new(changed),
        })
    }

    /// Answers what the view asks: the result of `method`. Writes resolve as soon as their own answer arrives and
    /// invalidate cached reads in the background (TanStack Query `invalidateQueries` / `onSuccess`).
    pub async fn request(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        if self.reads(method, &params).await {
            return self.read(method, params).await;
        }
        let response = self.client.request(method, params.clone()).await?;
        if changes_tools(method, &params) {
            self.client.refresh_listed();
        }
        self.invalidate(None);
        Ok(response)
    }

    /// Marks matching cached reads as stale and spawns an independent background refetch for each.
    pub fn invalidate(self: &Arc<Self>, app: Option<&str>) {
        *self.written.lock().expect("no view panics") = Some(SystemTime::now());
        let prefix = app.map(|app| format!("{app}_"));
        let mut spawned = Vec::new();
        {
            let mut responses = self.responses.lock().expect("no view panics");
            for (key, query) in responses.iter_mut() {
                if !matches_app(&query.method, &query.params, prefix.as_deref()) {
                    continue;
                }
                query.invalidated += 1;
                let generation = query.invalidated;
                let (tx, rx) = watch::channel(None);
                query.fetching = Some((generation, rx));
                spawned.push((key.clone(), query.method.clone(), query.params.clone(), generation, tx));
            }
        }
        for (key, method, params, generation, tx) in spawned {
            let shown = self.clone();
            tokio::spawn(async move { shown.fetch(key, method, params, generation, Some(tx)).await });
        }
    }

    /// Asks the server again for everything the view was answered, or with `app`, the tools of that app, which is
    /// all a server says changed when it's one's call that did (`notifications/one/changed`), and tells the view as
    /// each answer changes.
    pub async fn refresh(&self, app: Option<&str>) {
        let prefix = app.map(|app| format!("{app}_"));
        let mut asks = Vec::new();
        let mut waiting = Vec::new();
        {
            let responses = self.responses.lock().expect("no view panics");
            for (key, query) in responses.iter() {
                if !matches_app(&query.method, &query.params, prefix.as_deref()) {
                    continue;
                }
                // A post-write invalidation refetch is already on its way to the server; join it rather than
                // sending a duplicate RPC when `notifications/one/changed` arrives for the same write.
                if query.fetched < query.invalidated
                    && let Some((_, rx)) = &query.fetching
                {
                    waiting.push(rx.clone());
                } else {
                    asks.push((key.clone(), query.method.clone(), query.params.clone(), query.invalidated));
                }
            }
        }
        let fetches = asks.into_iter().map(|(key, method, params, generation)| async move {
            self.fetch(key, method, params, generation, None).await;
        });
        let joins = waiting.into_iter().map(|mut rx| async move {
            let _ = rx.wait_for(Option::is_some).await;
        });
        tokio::join!(join_all(fetches), join_all(joins));
    }

    async fn read(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        let key = json!([method, params]).to_string();
        loop {
            let wait = {
                let mut responses = self.responses.lock().expect("no view panics");
                if let Some(query) = responses.get_mut(&key) {
                    if query.fetched >= query.invalidated {
                        return Ok(query.response.clone());
                    }
                    match &query.fetching {
                        Some((at, rx)) if *at == query.invalidated => Some(rx.clone()),
                        _ => {
                            let generation = query.invalidated;
                            let (tx, rx) = watch::channel(None);
                            query.fetching = Some((generation, rx.clone()));
                            let (m, p) = (query.method.clone(), query.params.clone());
                            let shown = self.clone();
                            let k = key.clone();
                            tokio::spawn(async move { shown.fetch(k, m, p, generation, Some(tx)).await });
                            Some(rx)
                        }
                    }
                } else {
                    None
                }
            };
            if let Some(mut rx) = wait {
                let ok = rx.wait_for(Option::is_some).await.ok().and_then(|v| *v).unwrap_or(false);
                let responses = self.responses.lock().expect("no view panics");
                if let Some(query) = responses.get(&key) {
                    if query.fetched >= query.invalidated || !ok {
                        return Ok(query.response.clone());
                    }
                    // A newer write invalidated this query while we awaited an earlier fetch; wait for the newer one.
                    continue;
                }
            }
            let written = *self.written.lock().expect("no view panics");
            let disk = self.client.cached_entry(method, &params);
            let usable = disk.as_ref().is_some_and(|(_, modified)| {
                written.is_none_or(|at| modified.is_some_and(|m| m >= at))
            });
            let (response, revalidate) = if usable {
                (disk.expect("usable implies Some").0, true)
            } else {
                match self.client.fresh(method, &params).await {
                    Ok(fresh) => {
                        if *self.written.lock().expect("no view panics") != written {
                            continue;
                        }
                        (fresh, false)
                    }
                    Err(error) => match disk {
                        Some((cached, _)) => (cached, false),
                        None => return Err(error),
                    },
                }
            };
            self.responses.lock().expect("no view panics").entry(key.clone()).or_insert_with(|| Query {
                method: method.to_owned(),
                params: params.clone(),
                response: response.clone(),
                invalidated: 0,
                fetched: 0,
                fetching: None,
            });
            if revalidate {
                let shown = self.clone();
                let (m, p) = (method.to_owned(), params);
                tokio::spawn(async move { shown.fetch(key, m, p, 0, None).await });
            }
            return Ok(response);
        }
    }

    async fn fetch(
        &self,
        key: String,
        method: String,
        params: Value,
        generation: u64,
        done: Option<watch::Sender<Option<bool>>>,
    ) {
        let fresh = self.client.fresh(&method, &params).await;
        let ok = fresh.is_ok();
        let mut notify = false;
        {
            let mut responses = self.responses.lock().expect("no view panics");
            if let Some(query) = responses.get_mut(&key) {
                if query.fetching.as_ref().is_some_and(|(at, _)| *at == generation) {
                    query.fetching = None;
                }
                // With no server answering, what was kept stays what's shown. Ignore any out-of-order response
                // started before a newer invalidation or fetch.
                if let Ok(fresh) = fresh
                    && generation >= query.invalidated
                    && generation >= query.fetched
                {
                    query.fetched = generation;
                    if query.response != fresh {
                        query.response = fresh;
                        notify = true;
                    }
                }
            }
        }
        if let Some(tx) = done {
            let _ = tx.send(Some(ok));
        }
        if notify {
            (self.changed)();
        }
    }

    /// Whether asking `method` only reads. A tool does when the server's list of tools says so.
    async fn reads(&self, method: &str, params: &Value) -> bool {
        match method {
            method if crate::resolve::READS.contains(&method) => true,
            "tools/call" => self
                .client
                .reads(&json!({"method": method, "params": params}))
                .await
                .unwrap_or(false),
            _ => false,
        }
    }
}

fn matches_app(method: &str, params: &Value, prefix: Option<&str>) -> bool {
    prefix.is_none_or(|prefix| method == "tools/call" && params["name"].as_str().is_some_and(|name| name.starts_with(prefix)))
}

/// Whether a write call may change the server's `tools/list`: One Core's own tools (no `_`) or `apps_*`.
fn changes_tools(method: &str, params: &Value) -> bool {
    method == "tools/call"
        && params["name"]
            .as_str()
            .is_some_and(|name| !name.contains('_') || name.starts_with("apps_"))
}
