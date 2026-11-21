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

    /// Keeps `answer` as what `method` was last answered with. A tool's error isn't kept, as HTTP's `stale-if-error`
    /// has it: it says why it failed then, so a view opened later shows what was answered before it, and asks again,
    /// where it showed a day-old "couldn't be reached" as if it were now.
    pub(crate) fn keep(&self, method: &str, params: &Value, response: &Value) -> Result<()> {
        if response["isError"] == true {
            return Ok(());
        }
        write(&self.cache_file(method, params), response.to_string().as_bytes())
    }

    /// What a server last answered `method` with.
    pub fn cached(&self, method: &str, params: &Value) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.cache_file(method, params)).ok()?).ok()
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

/// One cached read in a `Subscription`, as stale-while-revalidate has it (SWR, TanStack Query, Apollo): the view is
/// always answered `response` at once, and a write, or a server saying something changed, asks again behind it
/// (`fetching`), telling the view when the answer comes back different.
struct Query {
    method: String,
    params: Value,
    /// The last answer the view was given.
    response: Value,
    /// How many times it was asked again after a write: an answer asked before the latest isn't shown over it.
    invalidated: u64,
    /// The generation `response` was fetched at.
    fetched: u64,
    /// The refetch a write started, which a refresh joins rather than asking again.
    fetching: Option<(u64, watch::Receiver<bool>)>,
}

/// What one view has been answered since it opened. It asks through this, and every view gets the same, with
/// nothing to write: what only reads is answered at once from what was kept and asked again behind it, as
/// stale-while-revalidate is, so a view opens as fast while its servers restart as with them up; a write resolves as
/// soon as its own answer arrives and has the tools the view was answered asked again in the background; and reads
/// are also asked again when the user returns to the view or a server says something changed. When an answer comes
/// back different, the view is told to ask again, and is answered at once.
pub struct Subscription {
    client: Arc<Client>,
    /// What only reads, by what was asked, with the answer the view was last given and when it was asked again.
    responses: Mutex<HashMap<String, Query>>,
    /// Tells the view that what it was answered has changed.
    changed: Box<dyn Fn() + Send + Sync>,
}

impl Subscription {
    pub fn new(client: Arc<Client>, changed: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { client, responses: Mutex::default(), changed: Box::new(changed) })
    }

    /// Answers what the view asks: the result of `method`. A write resolves as soon as its own answer arrives.
    pub async fn request(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        if self.reads(method, &params).await {
            return self.read(method, params).await;
        }
        let response = self.client.request(method, params.clone()).await?;
        if changes_tools(method, &params) {
            self.client.refresh_listed();
        }
        self.invalidate();
        Ok(response)
    }

    /// Asks again, in the background, for each tool the view was answered, as a write may have changed what they
    /// answer (TanStack Query's `invalidateQueries`). An app's files aren't asked again for a tool's write: what
    /// changes them, an install or a deploy, says so (`notifications/one/changed`).
    fn invalidate(self: &Arc<Self>) {
        let mut spawned = Vec::new();
        {
            let mut responses = self.responses.lock().expect("no view panics");
            for (key, query) in responses.iter_mut().filter(|(_, query)| query.method == "tools/call") {
                query.invalidated += 1;
                let generation = query.invalidated;
                let (tx, rx) = watch::channel(false);
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
                // A write's refetch is already on its way to the server; join it rather than sending a duplicate
                // RPC when `notifications/one/changed` arrives for the same write.
                match &query.fetching {
                    Some((_, rx)) => waiting.push(rx.clone()),
                    None => asks.push((key.clone(), query.method.clone(), query.params.clone(), query.invalidated)),
                }
            }
        }
        let fetches = asks.into_iter().map(|(key, method, params, generation)| async move {
            self.fetch(key, method, params, generation, None).await;
        });
        let joins = waiting.into_iter().map(|mut rx| async move {
            let _ = rx.wait_for(|done| *done).await;
        });
        tokio::join!(join_all(fetches), join_all(joins));
    }

    /// What the view was last answered, at once, or what was kept, asked again behind it; only what was never
    /// answered waits for a server.
    async fn read(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        let key = json!([method, params]).to_string();
        if let Some(query) = self.responses.lock().expect("no view panics").get(&key) {
            return Ok(query.response.clone());
        }
        let (response, revalidate) = match self.client.cached(method, &params) {
            Some(cached) => (cached, true),
            None => {
                // Said in the app's log, as a read a window waits on: one a server holds, as it starts, holds the window.
                eprintln!("{} had nothing kept, so it waits for a server", asked(method, &params));
                (self.client.fresh(method, &params).await?, false)
            }
        };
        let mut responses = self.responses.lock().expect("no view panics");
        // Another read of the same may have been answered meanwhile, and asked again behind it.
        if let Some(query) = responses.get(&key) {
            return Ok(query.response.clone());
        }
        responses.insert(key.clone(), Query {
            method: method.to_owned(),
            params: params.clone(),
            response: response.clone(),
            invalidated: 0,
            fetched: 0,
            fetching: None,
        });
        drop(responses);
        if revalidate {
            let shown = self.clone();
            let method = method.to_owned();
            tokio::spawn(async move { shown.fetch(key, method, params, 0, None).await });
        }
        Ok(response)
    }

    async fn fetch(
        &self,
        key: String,
        method: String,
        params: Value,
        generation: u64,
        done: Option<watch::Sender<bool>>,
    ) {
        let fresh = self.client.fresh(&method, &params).await;
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
            let _ = tx.send(true);
        }
        if notify {
            (self.changed)();
        }
    }

    /// Whether asking `method` only reads. A tool does when the server's list of tools says so.
    async fn reads(&self, method: &str, params: &Value) -> bool {
        match method {
            method if crate::resolve::READS.contains(&method) => true,
            "tools/call" => match self.client.reads(&json!({"method": method, "params": params})).await {
                Ok(reads) => reads,
                // Asked as a write, which waits for a server, as the app's log says.
                Err(error) => {
                    eprintln!("{} is asked as a write, since this client can't say whether it only reads: {error:#}", asked(method, params));
                    false
                }
            },
            _ => false,
        }
    }
}

/// What was asked, as the app's log says it: `tools/call settings_badge`, `resources/list`.
fn asked(method: &str, params: &Value) -> String {
    params["name"].as_str().map_or_else(|| method.to_owned(), |tool| format!("{method} {tool}"))
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
