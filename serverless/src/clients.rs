//! The MCP Worker's devices (kernel/mcp/), each a client of the install as any app is (one-client, over `<cli>/http/1`),
//! with no socket: one per remote MCP client, in its own Durable Object, which may share this isolate with others'.
//! What one sends goes to its object's `send(dst, segment size, contents)`, and the relay hands it what's sent to its
//! key, by RPC, which comes in at `device_deliver`. It lives while its object does, and holds its key only while it
//! runs.
//!
//! A device trusts no node for who's who or for its keys: it reads `global`'s entries from the bucket itself, through
//! the URL signer, with the epochs sealed to it that those entries made, replays them as every client device does
//! (one-client's `members::Global`, the nodes' own replay), and asks who's who and who may of `global`'s scope
//! database built from that (`one::database::scopes`) (`device_view`); it opens the keys a call needs from the bucket
//! too, each epoch checked against its entry's hash (`device_keys`). So it asks only the compute endpoints the install
//! lists as running its person's calls, and hands each only the keys of scopes it runs.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, ensure};
use js_sys::{Function, Uint8Array};
use one::database::scopes::{ScopeDb, object};
use one::entry::Scope;
use one::members;
use one_client::members::Global;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

type Key = [u8; 32];

/// The most a node's answer to a device may be.
const ANSWER_MOST: usize = 8 << 20;
/// A device not asked for this long is taken to have ended with its object, which didn't close it: its key leaves.
const FORGOTTEN_AFTER: f64 = 10.0 * 60_000.0;

thread_local! {
    /// Each device running here, by its key: its client, where the relay's datagrams for it go, and when it was last
    /// used (ms).
    static CLIENTS: RefCell<HashMap<Key, (one_client::Client, one_client::relayed::Inbox, f64)>> = RefCell::new(HashMap::new());
    /// Each device's `send`, which can't cross into the transport (a JavaScript function is neither Send nor Sync).
    static SENDS: RefCell<HashMap<Key, Function>> = RefCell::new(HashMap::new());
    /// What each device knows of who's who, and the keys it opened.
    static VIEWS: RefCell<HashMap<Key, View>> = RefCell::new(HashMap::new());
}

fn now() -> f64 {
    js_sys::Date::now()
}

/// Ends the devices here that nobody has used for `FORGOTTEN_AFTER`: their objects ended without closing them.
fn sweep() {
    let now = now();
    let gone: Vec<(Key, one_client::Client)> = CLIENTS.with(|c| {
        let mut c = c.borrow_mut();
        let old: Vec<Key> = c.iter().filter(|(_, (_, _, used))| now - *used > FORGOTTEN_AFTER).map(|(k, _)| *k).collect();
        old.into_iter().filter_map(|k| c.remove(&k).map(|(client, _, _)| (k, client))).collect()
    });
    for (key, client) in gone {
        forget(&key);
        wasm_bindgen_futures::spawn_local(async move { client.close().await });
    }
}

fn forget(key: &Key) {
    SENDS.with(|s| s.borrow_mut().remove(key));
    VIEWS.with(|v| v.borrow_mut().remove(key));
}

fn running(me: &Key) -> Option<one_client::Client> {
    CLIENTS.with(|c| {
        let mut c = c.borrow_mut();
        let (client, _, used) = c.get_mut(me)?;
        *used = now();
        Some(client.clone())
    })
}

fn thrown(err: impl std::fmt::Display) -> JsError {
    JsError::new(&err.to_string())
}

fn key(hex: &str) -> Result<Key, JsError> {
    data_encoding::HEXLOWER_PERMISSIVE.decode(hex.as_bytes()).ok().and_then(|k| k.try_into().ok()).ok_or_else(|| thrown(format!("{hex} isn't a key")))
}

fn keys(hexes: &[String]) -> Result<Vec<Key>, JsError> {
    hexes.iter().map(|h| key(h)).collect()
}

/// Starts the device whose key is `seed`, asking `nodes` (hex, in the order things run on them), its datagrams going
/// to `send`: → its key, hex. One already running is given `send` and `nodes`.
#[wasm_bindgen]
pub async fn device_start(seed: Vec<u8>, nodes: Vec<String>, send: Function) -> Result<String, JsError> {
    let seed: Key = seed.try_into().map_err(|_| thrown("a device's key is 32 bytes"))?;
    let me = *iroh::SecretKey::from_bytes(&seed).public().as_bytes();
    let nodes = keys(&nodes)?;
    sweep();
    SENDS.with(|s| s.borrow_mut().insert(me, send));
    if let Some(client) = running(&me) {
        client.set_nodes(&nodes).map_err(thrown)?;
    } else {
        let to_relay: one_client::relayed::Send = Arc::new(move |dst: &Key, segment: u16, contents: &[u8]| {
            SENDS.with(|s| {
                if let Some(send) = s.borrow().get(&me) {
                    let args = [Uint8Array::from(&dst[..]).into(), JsValue::from(segment), Uint8Array::from(contents).into()];
                    let _ = send.apply(&JsValue::NULL, &args.iter().collect());
                }
            })
        });
        let (client, inbox) = one_client::Client::relayed(seed, to_relay, &nodes).await.map_err(thrown)?;
        CLIENTS.with(|c| c.borrow_mut().insert(me, (client, inbox, now())));
    }
    Ok(data_encoding::HEXLOWER.encode(&me))
}

/// Datagrams the relay was given for the device `to` (bytes), from `src`: one, or `segment_size` long each.
#[wasm_bindgen]
pub fn device_deliver(to: &[u8], src: &[u8], segment_size: u32, contents: &[u8]) {
    let Ok(to) = Key::try_from(to) else { return };
    if let Some(inbox) = CLIENTS.with(|c| c.borrow().get(&to).map(|(_, inbox, _)| inbox.clone())) {
        inbox.deliver(src, segment_size, contents);
    }
}

/// The nodes the device `me` (hex) asks, in order.
#[wasm_bindgen]
pub fn device_nodes(me: &str, nodes: Vec<String>) -> Result<(), JsError> {
    let me = key(me)?;
    let client = running(&me).ok_or_else(|| thrown("that device isn't running"))?;
    client.set_nodes(&keys(&nodes)?).map_err(thrown)
}

/// Asks the first of the device `me`'s nodes that answers: `headers` names and values, one after the other. → `{
/// node, status, headers, body }`, `node` the key (hex) of the node that answered, the body whole, at most
/// `ANSWER_MOST`. Throws with `sent: false` if it reached none.
#[wasm_bindgen]
pub async fn device_fetch(me: String, method: String, path: String, headers: Vec<String>, body: Vec<u8>) -> Result<JsValue, JsValue> {
    let failed = |message: String, sent: bool| -> JsValue {
        let err = js_sys::Error::new(&message);
        let _ = js_sys::Reflect::set(&err, &"sent".into(), &sent.into());
        err.into()
    };
    let me = key(&me).map_err(JsValue::from)?;
    let client = running(&me).ok_or_else(|| failed("that device isn't running".into(), false))?;
    let headers = headers.chunks(2).map(|h| (h[0].clone(), h.get(1).cloned().unwrap_or_default())).collect();
    let response = client.fetch(one_client::Request { method, path, headers, body: body.into() }).await.map_err(|e| failed(e.to_string(), e.sent))?;
    let node = data_encoding::HEXLOWER.encode(&response.node);
    let body = response.body.bytes_within(ANSWER_MOST).await.map_err(|e| failed(e.to_string(), true))?;
    let out = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&out, &"node".into(), &node.into());
    let _ = js_sys::Reflect::set(&out, &"status".into(), &response.status.into());
    let said: js_sys::Array = response.headers.into_iter().flat_map(|(n, v)| [n, v]).map(JsValue::from).collect();
    let _ = js_sys::Reflect::set(&out, &"headers".into(), &said);
    let _ = js_sys::Reflect::set(&out, &"body".into(), &Uint8Array::from(&body[..]));
    Ok(out.into())
}

/// Ends the device `me` (hex): its connections close, telling the nodes, and its key leaves this isolate.
#[wasm_bindgen]
pub async fn device_close(me: String) {
    let Ok(me) = key(&me) else { return };
    VIEWS.with(|v| v.borrow_mut().remove(&me));
    let Some((client, _, _)) = CLIENTS.with(|c| c.borrow_mut().remove(&me)) else { return };
    let _ = n0_future::time::timeout(std::time::Duration::from_secs(1), client.close()).await;
    SENDS.with(|s| s.borrow_mut().remove(&me));
}

// Who's who, as the device reads it itself.

/// What a device knows of who's who: `global`'s entries as its epochs open them in the bucket, `global`'s scope
/// database built from what they add up to, and the keys it opened, each epoch's only while it's the key its entry
/// made.
struct View {
    user: String,
    seal: Key,
    global: one::db::Keys,
    at: u64,
    floor: u32,
    entries: Global,
    db: ScopeDb,
    epochs: HashMap<Scope, one::db::Keys>,
    apps: HashMap<(Scope, String, u32), Key>,
}

/// `global`'s scope database, from its entries.
fn built(entries: &mut Global) -> Result<ScopeDb> {
    entries.who()
}

/// `global`'s scope database's steps after those `entries` took, with the log's keys: who's who once it's frozen.
async fn steps(bucket: &crate::Js, keys: &one::db::Keys, entries: &mut Global) -> Result<()> {
    entries.add_read(one::database::steps::read(bucket, keys, &Scope::Global, entries.at()).await?)
}

impl View {
    /// `global`'s entries from scratch: each of its epochs sealed to `me` in the bucket, kept only once the entries
    /// they open say it's theirs (the genesis names the first's hash, each `epoch` entry the next's), as a node's
    /// first read is (`Node::first_global_read`).
    async fn read(bucket: &crate::Js, me: &Key, user: &str, seal: &Key) -> Result<View> {
        let mine = data_encoding::HEXLOWER.encode(me);
        let mut candidates = one::db::Keys::new();
        for n in 1..=one::db::EPOCHS_MAX {
            let Some(o) = one::bucket::Bucket::get(bucket, &one::keys::epoch_name("global", n, &mine)).await? else { break };
            if let Some(k) = one::kernel::open_sealed(seal, &o.bytes).ok().and_then(|k| Key::try_from(k).ok()) {
                candidates.insert(n, k);
            }
        }
        loop {
            ensure!(!candidates.is_empty(), "no keys of your One App are sealed to this device yet");
            let mut log_keys = one::db::Keys::new();
            for (n, epoch) in &candidates {
                if let Ok(Some(app)) = one::db::app_key(bucket, epoch, "global", members::APP, *n, false).await {
                    log_keys.insert(*n, one::kernel::database_key(&app, one::log::NAME));
                }
            }
            let read = one::log::open(bucket, &log_keys, &one::log::prefix("global"), "global").await?;
            let mut entries = Global::new(None);
            entries.add_entries(read.entries)?;
            steps(bucket, &log_keys, &mut entries).await?;
            let db = built(&mut entries)?;
            let wrong: Vec<u32> = candidates.iter().filter(|(n, k)| db.epoch_hash(&Scope::Global, **n) != Some(one::keys::epoch_hash(k))).map(|(n, _)| *n).collect();
            if wrong.is_empty() {
                return Ok(View { user: user.into(), seal: *seal, global: log_keys, at: read.at, floor: read.floor, entries, db, epochs: HashMap::from([(Scope::Global, candidates)]), apps: HashMap::new() });
            }
            for n in wrong {
                candidates.remove(&n);
            }
        }
    }

    /// The entries written since it last read: false if one is under an epoch it hasn't opened, so it must read again.
    async fn refresh(&mut self, bucket: &crate::Js) -> Result<bool> {
        let read = one::log::read(bucket, &self.global, &one::log::prefix("global"), "global", self.at, self.floor).await?;
        if read.ahead.is_some() {
            return Ok(false);
        }
        (self.at, self.floor) = (read.at, read.floor);
        let (had, new) = (self.entries.at(), !read.entries.is_empty());
        if new {
            self.entries.add_entries(read.entries)?;
        }
        steps(bucket, &self.global, &mut self.entries).await?;
        if new || had != self.entries.at() {
            self.db = built(&mut self.entries)?;
        }
        Ok(true)
    }

    /// A scope's epochs this device opens, by number: each one its entry made, from its copy in the bucket sealed to
    /// this device whose hash is the one the entry names (`Node::epoch_keys`).
    async fn epoch_keys(&mut self, bucket: &crate::Js, me: &Key, scope: &Scope) -> Result<one::db::Keys> {
        let m = &self.db;
        let hashes: BTreeMap<u32, String> = (1..=m.epoch(scope)).filter_map(|n| Some((n, m.epoch_hash(scope, n)?))).collect();
        let kept = self.epochs.get(scope).cloned().unwrap_or_default();
        let mut keys: one::db::Keys = kept.into_iter().filter(|(n, k)| hashes.get(n) == Some(&one::keys::epoch_hash(k))).collect();
        let mine = data_encoding::HEXLOWER.encode(me);
        for (n, hash) in &hashes {
            if keys.contains_key(n) {
                continue;
            }
            let Some(o) = one::bucket::Bucket::get(bucket, &one::keys::epoch_name(&scope.to_string(), *n, &mine)).await? else { continue };
            if let Some(k) = one::kernel::open_sealed(&self.seal, &o.bytes).ok().and_then(|k| Key::try_from(k).ok()).filter(|k| one::keys::epoch_hash(k) == *hash) {
                keys.insert(*n, k);
            }
        }
        self.epochs.insert(scope.clone(), keys.clone());
        Ok(keys)
    }

    /// The compute endpoints the install lists as running some of its person's calls: `{ key, seal, scopes }` each,
    /// hex, `scopes` the ones of theirs it runs.
    fn nodes(&self) -> Vec<Value> {
        let m = &self.db;
        let theirs: Vec<Scope> = m.scopes().into_iter().filter(|s| m.may(&self.user, "use", &object(s))).collect();
        m.computes()
            .into_iter()
            .filter(|c| m.serves(&c.key, &self.user))
            .map(|c| {
                let scopes: Vec<String> = theirs.iter().filter(|s| m.runs_for(&c.key, s)).map(|s| s.to_string()).collect();
                json!({ "key": one::entry::hex(&c.key), "seal": one::entry::hex(&c.seal), "scopes": scopes })
            })
            .collect()
    }
}

/// Takes a device's view out while `f` works on it, and puts it back: a device's object asks one at a time.
async fn with_view<R>(me: Key, f: impl AsyncFnOnce(&mut Option<View>) -> Result<R>) -> Result<R> {
    let mut view = VIEWS.with(|v| v.borrow_mut().remove(&me));
    let out = f(&mut view).await;
    if let Some(view) = view
        && running(&me).is_some()
    {
        VIEWS.with(|v| v.borrow_mut().insert(me, view));
    }
    out
}

/// What the running device `me` (hex), of `user`, with its seal key's secret `seal`, knows of who's who, read from
/// `bucket` (as `worker.js`'s, through the URL signer): the first time, everything, after that what's new. → `{
/// listed, nodes }`: whether the entries list it as a client device of `user`, and `View::nodes`.
#[wasm_bindgen]
pub async fn device_view(me: String, user: String, seal: Vec<u8>, bucket: JsValue) -> Result<String, JsError> {
    let me = key(&me)?;
    let seal: Key = seal.try_into().map_err(|_| thrown("a seal key's secret is 32 bytes"))?;
    let bucket = crate::Js(bucket);
    let out = with_view(me, async |view: &mut Option<View>| {
        let fresh = match view.take().filter(|v| v.user == user && v.seal == seal) {
            Some(mut v) => match v.refresh(&bucket).await? {
                true => v,
                false => View::read(&bucket, &me, &user, &seal).await?,
            },
            None => View::read(&bucket, &me, &user, &seal).await?,
        };
        let listed = fresh.db.client(&me).is_some_and(|c| c.user == user);
        let out = json!({ "listed": listed, "nodes": fresh.nodes() });
        *view = Some(fresh);
        Ok(out)
    })
    .await
    .map_err(|e| thrown(format!("{e:#}")))?;
    Ok(out.to_string())
}

/// The keys of the databases `need` (JSON, `[{ scope, app, database }]`) names, for the running device `me` (hex), as
/// `device_view` last read who's who: each epoch of the scope this device opens from the one the database's newest copy
/// is under (`keys::oldest_needed`), the app's key in it from the bucket (made in the newest, if no one has yet). → `[{ scope, epoch, app, database, key }]`, `key` base64url: what it
/// hands a node, sealed to it.
#[wasm_bindgen]
pub async fn device_keys(me: String, need: String, bucket: JsValue) -> Result<String, JsError> {
    let me = key(&me)?;
    let need: Vec<Value> = serde_json::from_str(&need)?;
    let bucket = crate::Js(bucket);
    let out = with_view(me, async |view: &mut Option<View>| {
        let view = view.as_mut().context("read who's who first")?;
        let mut out = Vec::new();
        for n in &need {
            let field = |k: &str| n[k].as_str().ok_or_else(|| anyhow!("a need names its {k}"));
            let (scope, app, database) = (field("scope")?, field("app")?, field("database")?);
            let prefix = crate::prefix_of(scope, app, database)?;
            let scope = Scope::parse(scope)?;
            ensure!(view.db.may(&view.user, "use", &object(&scope)), "you don't use {scope}");
            let epochs = view.epoch_keys(&bucket, &me, &scope).await?;
            let newest = epochs.keys().next_back().copied().unwrap_or(0);
            // From the epoch its newest copy is under: older ones open only its history.
            let hint = one::bucket::Bucket::get(&bucket, &one::db::newest_name(&prefix)).await?;
            let oldest = one::keys::oldest_needed(hint.as_ref().map(|o| &o.bytes[..]), view.db.epoch(&scope));
            for (epoch, epoch_key) in epochs.range(oldest..) {
                let id = (scope.clone(), app.to_string(), *epoch);
                let app_key = match view.apps.get(&id) {
                    Some(k) => Some(*k),
                    None => one::db::app_key(&bucket, epoch_key, &scope.to_string(), app, *epoch, *epoch == newest).await?,
                };
                let Some(app_key) = app_key else { continue };
                view.apps.insert(id, app_key);
                let key = data_encoding::BASE64URL_NOPAD.encode(&one::kernel::database_key(&app_key, database));
                out.push(json!({ "scope": scope.to_string(), "epoch": epoch, "app": app, "database": database, "key": key }));
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| thrown(format!("{e:#}")))?;
    Ok(Value::from(out).to_string())
}
