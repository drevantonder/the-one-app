//! The Cloudflare node's Rust: the machine node's own code, as WebAssembly on Workers. `worker.js` answers requests
//! and runs apps; this checks who asks, opens the keys callers hand it, signs what it asks the URL signer, and
//! reads and writes apps' databases in the bucket (`src/database/stored.rs`), through the presigned URLs `worker.js` gets. It holds
//! no keys but those handed it for a device's requests.

mod clients;
mod net;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, anyhow};
use js_sys::Reflect;
use one::bucket::Bucket;
use one::call::Append;
use one::database::stored::Db;
use one::database::scopes::{Scope, ScopeDb, ScopeState};
use one::keys::Hash;
use one_client::js::Js;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

fn thrown(err: anyhow::Error) -> JsError {
    JsError::new(&format!("{err:#}"))
}

/// An error as `worker.js` reads it: one that stopped at an epoch whose key it wasn't handed says which (`missing`).
fn failed(err: anyhow::Error) -> JsValue {
    let out = js_sys::Error::new(&format!("{err:#}"));
    if let Some(m) = err.downcast_ref::<one::database::stored::Missing>() {
        let _ = Reflect::set(&out, &"missing".into(), &m.epoch.into());
    }
    out.into()
}

/// A database's keys as `worker.js` keeps them: JSON, `{ <epoch>: <key, base64url> }`.
fn keys_of(keys: &str) -> Result<one::database::stored::Keys> {
    let keys: BTreeMap<u32, String> = serde_json::from_str(keys)?;
    keys.into_iter().map(|(n, k)| Ok((n, one::keys::decode_key(&k)?))).collect()
}

fn signing_key(seed: &str) -> Result<ed25519_dalek::SigningKey> {
    Ok(ed25519_dalek::SigningKey::from_bytes(&one::keys::unhex(seed)?))
}

/// This node as an endpoint, from its key (hex): `{ key, seal }`, hex, what a caller seals the keys it hands to.
#[wasm_bindgen]
pub fn endpoint_of(seed: &str) -> Result<String, JsError> {
    let e = one::keys::Endpoint::of(&signing_key(seed).map_err(thrown)?);
    Ok(json!({ "key": data_encoding::HEXLOWER.encode(&e.key), "seal": data_encoding::HEXLOWER.encode(&e.seal) }).to_string())
}

/// `fields` (JSON) signed by this node's key for `domain`, as devices sign what they ask the URL signer.
#[wasm_bindgen]
pub fn signed(seed: &str, domain: &str, fields: &str, at: f64) -> Result<String, JsError> {
    Ok(one::keys::signed_request(&signing_key(seed).map_err(thrown)?, domain, serde_json::from_str(fields)?, at as u64).to_string())
}

/// The most keys one handoff may carry: a call's databases in each epoch since their newest copies.
const HANDED_MOST: usize = 256;

/// The keys the device `device` (hex) handed this node with its requests (`x-one-keys`), kept no longer than its word
/// lasts (`exp`, milliseconds): → `{ until, keys: [{ scope, epoch, app, database, key }] }`.
#[wasm_bindgen]
pub fn handoff(seed: &str, device: &str, sealed: &str, now: f64, exp: f64) -> Result<String, JsError> {
    let handoff = one::keys::open_handoff(&one::keys::seal_secret(&signing_key(seed).map_err(thrown)?), sealed).map_err(thrown)?;
    if handoff.for_ != device {
        return Err(JsError::new("the keys handed are for another device"));
    }
    if handoff.until <= now as u64 {
        return Err(JsError::new("the keys handed have expired"));
    }
    if handoff.keys.len() > HANDED_MOST {
        return Err(JsError::new("too many keys handed at once"));
    }
    Ok(json!({ "until": handoff.until.min(exp as u64), "keys": handoff.keys }).to_string())
}

/// What a sign-in needs for `wants` (JSON, `[{ scope, app, database, have: [epoch] }]`) that it wasn't handed, as a node
/// on a machine says it (src/node/sessions.rs): every epoch from the oldest it needs (`one::keys::oldest_needed`) to the
/// scope's current one, as the network's signed steps say (`epochs`, JSON `{ <scope>: n }`), never what the bucket
/// holds, where any endpoint can put a name. → `[{ scope, epoch, app, database }]`.
#[wasm_bindgen]
pub async fn needs(bucket: JsValue, wants: String, epochs: String) -> Result<String, JsError> {
    let bucket = Js(bucket);
    let wants: Vec<Value> = serde_json::from_str(&wants)?;
    let epochs: BTreeMap<String, u32> = serde_json::from_str(&epochs)?;
    let mut need = Vec::new();
    for w in wants {
        let (scope, app, database) = (w["scope"].as_str().unwrap_or(""), w["app"].as_str().unwrap_or(""), w["database"].as_str().unwrap_or(""));
        let have: Vec<u32> = serde_json::from_value(w["have"].clone()).unwrap_or_default();
        let newest = epochs.get(scope).copied().unwrap_or(1).max(1);
        let hint = bucket.get(&one::database::stored::newest_name(&prefix_of(scope, app, database).map_err(thrown)?)).await.map_err(thrown)?;
        let oldest = one::keys::oldest_needed(database, hint.as_ref().map(|o| &o.bytes[..]), newest);
        need.extend((oldest..=newest).filter(|n| !have.contains(n)).map(|epoch| json!({ "scope": scope, "epoch": epoch, "app": app, "database": database })));
    }
    Ok(Value::from(need).to_string())
}

/// The keys of a database a call uses, of those handed (`keys_of`), for a scope whose current epoch is `current`
/// (`one::database::stored::handed`): a `missing` error names the epoch it lacks.
fn usable(keys: &str, current: u32) -> Result<one::database::stored::Keys, JsValue> {
    one::database::stored::handed(keys_of(keys).map_err(failed)?, current.max(1)).map_err(|m| failed(m.into()))
}

/// How many people's databases and steps one isolate keeps, the least lately used going first: each person's are
/// their Durable Object's, and several may share an isolate, which outlives them.
const OWNERS_MOST: usize = 16;

thread_local! {
    /// The databases calls running in this isolate have open, by handle, each with whose object opened it.
    static OPEN: RefCell<(u32, HashMap<u32, (String, Db)>)> = RefCell::new((0, HashMap::new()));
    /// When each person's object last used what this isolate keeps for it, to forget the least lately used.
    static OWNERS: RefCell<(u64, HashMap<String, u64>)> = RefCell::new((0, HashMap::new()));
}

/// Notes that `owner`'s object is using what this isolate keeps for it, forgetting the least lately used owner's
/// databases and steps once there are too many.
fn touch(owner: &str) {
    let gone = OWNERS.with(|o| {
        let mut o = o.borrow_mut();
        o.0 += 1;
        let now = o.0;
        o.1.insert(owner.to_string(), now);
        if o.1.len() <= OWNERS_MOST {
            return None;
        }
        let oldest = o.1.iter().min_by_key(|(_, at)| **at).map(|(k, _)| k.clone())?;
        o.1.remove(&oldest);
        Some(oldest)
    });
    if let Some(gone) = gone {
        OPEN.with(|o| o.borrow_mut().1.retain(|_, (by, _)| *by != gone));
        LOGS.with(|l| l.borrow_mut().retain(|(by, _), _| *by != gone));
    }
}

fn take(handle: u32) -> Result<(String, Db)> {
    OPEN.with(|o| o.borrow_mut().1.remove(&handle)).context("that database isn't open")
}

fn put(handle: u32, db: (String, Db)) {
    OPEN.with(|o| o.borrow_mut().1.insert(handle, db));
}

#[wasm_bindgen]
pub fn epoch_name(scope: &str, n: u32, to: &str) -> String {
    one::keys::epoch_name(scope, n, to)
}

#[wasm_bindgen]
pub fn app_key_name(scope: &str, app: &str, n: u32) -> String {
    one::keys::app_key_name(scope, app, n)
}

/// A session's certificate (`x-one-endpoint`, `keys::certify`'s), checked at `now` (milliseconds): `{ by, endpoint,
/// until }`, `by` the device that vouched for `endpoint` (hex). Whether `by` is a device of the network is the caller's
/// to check.
#[wasm_bindgen]
pub fn check_certificate(certificate: &str, now: f64) -> Result<String, JsError> {
    let s = one::keys::check_certificate(certificate, now as u64).map_err(thrown)?;
    Ok(json!({ "by": one::keys::hex(&s.by), "endpoint": one::keys::hex(&s.endpoint), "until": s.until }).to_string())
}

/// Opens a database at `prefix` for `owner`'s object with the keys handed (`usable`, under the scope's `current`
/// epoch) and brings it to `migrations` (JSON, its app's schema for the scope's kind), signing what it writes with the
/// node's endpoint key (`seed`): → a handle for `query`, `write` and `close`.
#[wasm_bindgen]
pub async fn open(owner: String, bucket: JsValue, prefix: String, keys: String, current: u32, migrations: String, seed: String) -> Result<u32, JsValue> {
    touch(&owner);
    let bucket = Js(bucket);
    let keys = usable(&keys, current)?;
    let migrations: Vec<String> = serde_json::from_str(&migrations).map_err(|e| failed(e.into()))?;
    let mut db = Db::open(&bucket, &prefix, keys).await.map_err(failed)?;
    db.sign_as(signing_key(&seed).map_err(failed)?);
    db.migrate(&bucket, &migrations).await.map_err(failed)?;
    let handle = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        o.0 += 1;
        o.0
    });
    put(handle, (owner, db));
    Ok(handle)
}

/// Whether an open database was opened with exactly the keys a call was handed (`usable`): only then may the call use
/// it, since a key it holds that the call wasn't handed may open what the call mustn't read, and one the call was
/// handed that it lacks may be a wrong key, which mustn't change what others read. False once it's been closed.
#[wasm_bindgen]
pub fn opens(handle: u32, keys: &str, current: u32) -> bool {
    let Ok(keys) = usable(keys, current) else { return false };
    OPEN.with(|o| o.borrow().1.get(&handle).is_some_and(|(_, db)| *db.keys() == keys))
}

#[wasm_bindgen]
pub async fn refresh(handle: u32, bucket: JsValue, migrations: String) -> Result<(), JsValue> {
    let bucket = Js(bucket);
    let migrations: Vec<String> = serde_json::from_str(&migrations).map_err(|e| failed(e.into()))?;
    let (owner, mut db) = take(handle).map_err(failed)?;
    touch(&owner);
    let done = async {
        db.catch_up(&bucket).await?;
        db.migrate(&bucket, &migrations).await
    }
    .await;
    put(handle, (owner, db));
    done.map_err(failed)
}

#[wasm_bindgen]
pub fn query(handle: u32, sql: &str, params: &str) -> Result<String, JsError> {
    let params: Vec<Value> = serde_json::from_str(params)?;
    OPEN.with(|o| {
        let o = o.borrow();
        let (_, db) = o.1.get(&handle).context("that database isn't open").map_err(thrown)?;
        Ok(Value::from(db.query(sql, &params).map_err(thrown)?).to_string())
    })
}

/// Whether `user` may do each permission to each object of `asks` (JSON, `[[permission, object]]`), in order (JSON,
/// `[bool]`), by `owner`'s scope database and an app's own `types` (JSON, as it declares them) and relation rows in its
/// open database `handle`, if it declares any (`ScopeDb::app_may`), as a node on a machine answers.
#[wasm_bindgen]
pub fn may(owner: &str, handle: Option<u32>, user: &str, asks: &str, types: &str, now: f64) -> Result<String, JsError> {
    let asks: Vec<(String, String)> = serde_json::from_str(asks)?;
    let types: Value = serde_json::from_str(types)?;
    let rows = match handle {
        Some(h) => OPEN.with(|o| o.borrow().1.get(&h).map(|(_, db)| one::database::scopes::app_relations(db.connection()))).context("that database isn't open").map_err(thrown)?,
        None => Vec::new(),
    };
    replayed(owner, &[], |r| Ok(Value::from(r.global().app_may(user, &asks, &types, &rows, now as u64)).to_string())).map_err(thrown)
}

/// Writes a call's statements (JSON, `[[sql, params, check]]`) as one step: → its position, or 0 if they changed
/// nothing.
#[wasm_bindgen]
pub async fn write(handle: u32, bucket: JsValue, statements: String) -> Result<f64, JsError> {
    let list: Vec<(String, Vec<Value>, Option<String>)> = serde_json::from_str(&statements)?;
    let (owner, mut db) = take(handle).map_err(thrown)?;
    let (at, bucket) = (db.at(), Js(bucket));
    let wrote = db.write(&bucket, |c| one::database::stored::statements(c, &list)).await;
    let step = if db.at() > at { db.at() as f64 } else { 0.0 };
    put(handle, (owner, db));
    wrote.map_err(thrown)?;
    Ok(step)
}

#[wasm_bindgen]
pub fn close(handle: u32) {
    OPEN.with(|o| o.borrow_mut().1.remove(&handle));
}

#[wasm_bindgen]
pub fn vault_schema() -> String {
    json!([one::vault::SCHEMA]).to_string()
}

/// The items `app` is switched on for in a call's scopes, as `ctx.vault.items()` gives them (never a secret field's
/// value): `opened` is `[[scope, handle]]`, each scope's Vault database (`node`'s `vault`), opened with its keys.
#[wasm_bindgen]
pub fn vault_items(opened: &str, app: &str, user: &str) -> Result<String, JsError> {
    Ok(Value::from(lookup(opened, app, user)?.for_app()).to_string())
}

/// An app's request out (JSON `{ url, headers, body }`, its body text or null) with the fields of the items it's
/// switched on for filled in from `opened` (as `vault_items`), a secret one only if `host` is one of its item's hosts,
/// as a node on a machine fills an app's requests (`src/app/outbound.rs`).
#[wasm_bindgen]
pub fn vault_fill(opened: &str, app: &str, user: &str, host: &str, request: &str) -> Result<String, JsError> {
    let found = lookup(opened, app, user)?;
    let fill = |text: &str| one::vault::fill(text, host, |name| found.get(name)).map_err(thrown);
    let mut req: Value = serde_json::from_str(request)?;
    req["url"] = json!(fill(req["url"].as_str().unwrap_or(""))?);
    if let Some(headers) = req["headers"].as_object_mut() {
        for value in headers.values_mut() {
            *value = json!(fill(value.as_str().unwrap_or(""))?);
        }
    }
    if let Some(body) = req["body"].as_str() {
        req["body"] = json!(fill(body)?);
    }
    Ok(req.to_string())
}

fn lookup(opened: &str, app: &str, user: &str) -> Result<one::vault::Lookup, JsError> {
    let opened: Vec<(String, u32)> = serde_json::from_str(opened)?;
    let mut scopes = Vec::new();
    for (scope, handle) in opened {
        let items = OPEN.with(|o| o.borrow().1.get(&handle).context("that database isn't open").and_then(|(_, db)| one::vault::load(db.connection()))).map_err(thrown)?;
        scopes.push((Scope::parse(&scope).map_err(thrown)?, items));
    }
    Ok(one::vault::Lookup { app: app.into(), user: user.into(), scopes })
}

/// A call's appends (JSON, as the shim gives them) as each scope's statements, `{ <scope>: [[sql, params, check]] }`,
/// for the scopes the call named; what this node can't write yet refuses the call, so it writes nothing.
#[wasm_bindgen]
pub fn statements(appends: &str, scopes: &str) -> Result<String, JsError> {
    let scopes: Vec<String> = serde_json::from_str(scopes)?;
    let mut out = serde_json::Map::new();
    for append in one::call::appends(&serde_json::from_str(appends)?).map_err(thrown)? {
        match append {
            Append::Exec { scope, sql, params, blobs, check } => {
                if !scopes.contains(&scope) {
                    return Err(JsError::new(&format!("the call didn't name {scope}")));
                }
                if !blobs.is_empty() {
                    return Err(JsError::new("a Cloudflare node doesn't keep blobs yet: use a node on one of your machines"));
                }
                out.entry(scope).or_insert_with(|| json!([])).as_array_mut().unwrap().push(json!([sql, params, check]));
            }
            // What it spent on AI shows in insights, which this node doesn't keep yet: nothing is lost by not counting it.
            Append::Cost { .. } => {}
            _ => return Err(JsError::new("a Cloudflare node writes only an app's own data yet: use a node on one of your machines")),
        }
    }
    Ok(Value::Object(out).to_string())
}

/// An app screen's page (`src/screen.rs`), from what the screen is told (JSON) and the HTML the app wrote, its kit
/// `linked` or not.
#[wasm_bindgen]
pub fn screen(context: &str, body: &str, linked: bool) -> Result<String, JsError> {
    Ok(one::screen::page(&serde_json::from_str(context)?, body, false, linked))
}

/// The custom build this node is: `{ name, cli, port, made_here }`.
#[wasm_bindgen]
pub fn config() -> String {
    json!({ "name": one::config::NAME, "cli": one::config::CLI, "port": one::config::PORT, "made_here": one::config::MADE_HERE }).to_string()
}

/// Where an app's database of a scope is in the bucket (`src/database/stored.rs`): only of a scope there can be, and an app and
/// database whose names are only letters, digits and dashes.
fn prefix_of(scope: &str, app: &str, database: &str) -> Result<String> {
    let scope = Scope::parse(scope)?.to_string();
    let plain = |s: &str| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    anyhow::ensure!(plain(app) && plain(database), "{app}/{database} isn't an app's database");
    Ok(one::bucket::database_prefix(&scope, app, database))
}

#[wasm_bindgen]
pub fn prefix(scope: &str, app: &str, database: &str) -> Result<String, JsError> {
    prefix_of(scope, app, database).map_err(thrown)
}

// Who can do what: each scope's database's steps (`src/database/steps.rs`), which every question of who's who and who
// may is asked of.

/// A scope's steps this isolate read for one person's object, with the keys they were read with: `global`'s by who's
/// who's reader, as every device reads it; another scope's by its own, once `global`'s log is frozen and its start read.
struct Held {
    keys: one::database::stored::Keys,
    steps: Steps,
}

enum Steps {
    Global(one::database::steps::Reader),
    Scope(Option<one::database::steps::ScopeReader>),
}

impl Held {
    fn truth(&self) -> Option<&one::database::steps::Truth> {
        match &self.steps {
            Steps::Global(r) => r.truth(),
            Steps::Scope(_) => None,
        }
    }

    /// The database its steps leave, if it's read its first.
    fn db(&self) -> Option<&ScopeDb> {
        match &self.steps {
            Steps::Global(r) => r.truth().map(|t| &t.db),
            Steps::Scope(r) => r.as_ref().map(|r| &r.db),
        }
    }
}

/// Each scope's database read, `global`'s among them, which says who's who in the whole network.
struct Replays<'a>(BTreeMap<String, &'a ScopeDb>);

impl Replays<'_> {
    fn global(&self) -> &ScopeDb {
        self.0["global"]
    }
}

thread_local! {
    /// Each scope's steps this isolate read, by whose object read them and the scope. A request uses them only if it
    /// was handed exactly the keys they were read with: a key that opened nothing passes over what's under its epoch,
    /// and what was read that way is only ever shown to whoever handed it.
    static LOGS: RefCell<HashMap<(String, String), Held>> = RefCell::new(HashMap::new());
}

/// Reads a scope's steps for `owner`'s object, with the keys a caller handed, up to its newest: from the first the
/// first time, or when the keys differ from those last read with, and after that only those since.
#[wasm_bindgen]
pub async fn read_scope(owner: String, bucket: JsValue, scope: String, keys: String) -> Result<(), JsValue> {
    touch(&owner);
    let keys = keys_of(&keys).map_err(failed)?;
    let scope = Scope::parse(&scope).map_err(failed)?;
    let id = (owner, scope.to_string());
    if LOGS.with(|l| l.borrow().get(&id).is_none_or(|l| l.keys != keys)) {
        let steps = match scope {
            Scope::Global => Steps::Global(one::database::steps::Reader::new(ROOT.with(|r| *r.borrow()))),
            _ => Steps::Scope(None),
        };
        LOGS.with(|l| l.borrow_mut().insert(id.clone(), Held { keys: keys.clone(), steps }));
    }
    read_steps(&id, &scope, &Js(bucket), &keys).await.map_err(failed)
}

/// Runs `f` on what `id`'s object read of a scope, if it's still read with `keys`, and on `global`'s as it is.
fn held<R>(id: &(String, String), keys: &one::database::stored::Keys, f: impl FnOnce(&mut Held, Option<&Held>) -> Result<R>) -> Result<Option<R>> {
    LOGS.with(|l| {
        let mut logs = l.borrow_mut();
        let Some(mut log) = logs.remove(id) else { return Ok(None) };
        let out = if log.keys == *keys { f(&mut log, logs.get(&(id.0.clone(), "global".to_string()))).map(Some) } else { Ok(None) };
        logs.insert(id.clone(), log);
        out
    })
}

/// A scope's database's steps after those read, with `keys`: another scope's once `global`'s log is frozen, from its
/// start the first time. One under an epoch made whose key it wasn't handed is asked for (428, `Missing`).
async fn read_steps(id: &(String, String), scope: &Scope, bucket: &Js, keys: &one::database::stored::Keys) -> Result<()> {
    use one::database::steps::{self, ScopeReader};
    let waits = if *scope == Scope::Global {
        let at = held(id, keys, |h, _| Ok(match &h.steps {
            Steps::Global(r) => r.at(),
            Steps::Scope(_) => 0,
        }))?;
        let Some(at) = at else { return Ok(()) };
        let sealed = steps::fetch(bucket, at, None).await?;
        held(id, keys, |h, _| match &mut h.steps {
            Steps::Global(r) => r.add(&sealed, keys, None, now(), |_, _, _| Ok(())),
            Steps::Scope(_) => Ok(None),
        })?
        .flatten()
    } else {
        let frozen = held(id, keys, |h, global| Ok((matches!(h.steps, Steps::Scope(Some(_))), global.and_then(Held::truth).is_some())))?;
        let Some((started, true)) = frozen else { return Ok(()) };
        if !started {
            let start = ScopeReader::fetch_start(bucket, keys, scope).await?;
            held(id, keys, |h, global| {
                let truth = global.and_then(Held::truth).context("global's log isn't frozen")?;
                h.steps = Steps::Scope(Some(ScopeReader::start(truth, scope, start.as_ref(), now())?));
                Ok(())
            })?;
        }
        let at = held(id, keys, |h, global| {
            let made = global.and_then(Held::truth).context("global's log isn't frozen")?.db.epoch(scope)?;
            Ok(match &h.steps {
                Steps::Scope(Some(r)) => Some((r.at, made)),
                _ => None,
            })
        })?;
        let Some(Some((from, made))) = at else { return Ok(()) };
        let read = steps::read(bucket, keys, scope, from, made).await?;
        held(id, keys, |h, global| {
            let global = global.and_then(Held::db).context("global's log isn't frozen")?;
            if let Steps::Scope(Some(r)) = &mut h.steps {
                r.take(from, &read, global, scope, now())?;
            }
            Ok(())
        })?;
        read.waits
    };
    match waits {
        Some(epoch) => Err(one::database::stored::Missing { epoch }.into()),
        None => Ok(()),
    }
}

thread_local! {
    /// The network's root, as the device that deployed this node holds it (`pin_root`).
    static ROOT: RefCell<Option<Hash>> = const { RefCell::new(None) };
}

/// Pins the network's root to `root` (hex, the Worker's), as auth pins it: a freeze naming any other is never
/// taken for its first.
#[wasm_bindgen]
pub fn pin_root(root: &str) -> Result<(), JsError> {
    let hash = Hash::from_hex(root).map_err(|_| JsError::new("the network's root isn't a hash"))?;
    ROOT.with(|r| *r.borrow_mut() = Some(hash));
    Ok(())
}

/// `global`'s scope database's steps after `from` as the bucket holds them, sealed, for `user`, as a node on a machine
/// gives them (`Node::global_steps`): `{ at, epoch, steps: [{ at, sealed }] }` (base64url), and for a device that starts
/// from its `welcome` (`steps::Welcome`), its freeze's step and the copy before its step, `{ welcome, freeze, start }`.
/// Fails before the log is frozen. The caller opens and checks them all itself: this trusts the node with nothing.
#[wasm_bindgen]
pub async fn sealed_global_steps(owner: String, user: String, bucket: JsValue, from: f64, welcome: Option<String>) -> Result<String, JsValue> {
    use one::database::steps;
    let bucket = Js(bucket);
    let (people, epoch) = replayed(&owner, &[], |r| Ok((r.global().may(&user, "use", one::database::scopes::NETWORK), r.global().epoch(&Scope::Global)?))).map_err(failed)?;
    if !people {
        return Err(failed(anyhow!("you aren't one of this network's people")));
    }
    let read = LOGS.with(|l| l.borrow().get(&(owner.clone(), "global".to_string())).and_then(|g| g.truth().map(|t| t.at)));
    let read = read.ok_or_else(|| failed(anyhow!("global's log isn't frozen")))?;
    let from = (from as u64).min(read);
    let welcome = welcome.as_deref().and_then(steps::Welcome::parse).filter(|w| w.at <= read);
    let mut out = steps::fetch(&bucket, from, welcome.as_ref()).await.map_err(failed)?.to_json(from);
    out["epoch"] = json!(epoch);
    Ok(out.to_string())
}

fn now() -> u64 {
    js_sys::Date::now() as u64
}

/// `global`'s database and each of `scopes`', from the steps `owner`'s object read (`read_scope`): nobody's until
/// they're read.
fn replayed<R>(owner: &str, scopes: &[String], f: impl FnOnce(&Replays) -> Result<R>) -> Result<R> {
    touch(owner);
    LOGS.with(|logs| {
        let logs = logs.borrow();
        let log_of = |scope: &str| logs.get(&(owner.to_string(), scope.to_string()));
        log_of("global").context("read global's steps first")?;
        let mut read = vec!["global".to_string()];
        read.extend(scopes.iter().filter(|s| *s != "global" && log_of(s).is_some()).cloned());
        let nobody: BTreeMap<String, ScopeDb> = read.iter().filter(|s| log_of(s).and_then(Held::db).is_none()).map(|s| Ok((s.clone(), ScopeDb::empty(&Scope::parse(s)?, now())?))).collect::<Result<_>>()?;
        f(&Replays(read.iter().map(|s| (s.clone(), log_of(s).and_then(Held::db).unwrap_or_else(|| &nobody[s]))).collect()))
    })
}

/// The groups `user` is in, as scopes: whose steps to read next.
#[wasm_bindgen]
pub fn groups(owner: &str, user: &str) -> Result<String, JsError> {
    let groups = replayed(owner, &[], |r| Ok(r.global().groups_of(user)?.into_keys().map(|g| format!("group:{g}")).collect::<Vec<_>>())).map_err(thrown)?;
    Ok(json!(groups).to_string())
}

/// The scopes `user` may use, as the network's log says, whether they're a person of it, whether they were removed
/// from it, the name of `device` (hex) if it's a client device of theirs, else null, and the scopes of theirs this node
/// runs (a person's own only as their own device chose): `{ scopes, joined, removed, device, runs }`.
#[wasm_bindgen]
pub fn scopes_of(owner: &str, user: &str, device: &str) -> Result<String, JsError> {
    let key = one::keys::unhex(device).ok();
    let out = replayed(owner, &[], |r| {
        let m = r.global();
        let scopes: Vec<Scope> = m.scopes()?.into_iter().filter(|s| m.may(user, "use", &one::database::scopes::object(s))).collect();
        let device = match key {
            Some(k) => m.client(&k)?,
            None => None,
        };
        let device = device.filter(|d| d.user == user).map(|d| d.name);
        let cloudflare = m.cloudflare_node()?;
        let runs: Vec<String> = scopes.iter().filter(|s| cloudflare.is_some_and(|k| m.runs_for(&k, s))).map(Scope::to_string).collect();
        let scopes: Vec<String> = scopes.iter().map(Scope::to_string).collect();
        let removed = m.user_ever(user)?.is_some_and(|u| u.removed);
        Ok(json!({ "scopes": scopes, "joined": m.user(user)?.is_some(), "removed": removed, "device": device, "runs": runs }))
    })
    .map_err(thrown)?;
    Ok(out.to_string())
}

/// Each of `scopes`' (JSON) current epoch, as the network's signed `epoch` steps say, 1 for one with none yet:
/// `{ <scope>: n }`, all a node writes under.
#[wasm_bindgen]
pub fn epochs(owner: &str, scopes: &str) -> Result<String, JsError> {
    let scopes: Vec<String> = serde_json::from_str(scopes)?;
    let out = replayed(owner, &[], |r| {
        let mut epochs = serde_json::Map::new();
        for s in &scopes {
            if let Ok(scope) = Scope::parse(s) {
                epochs.insert(s.clone(), Value::from(r.global().epoch(&scope)?.max(1)));
            }
        }
        Ok(epochs)
    })
    .map_err(thrown)?;
    Ok(Value::Object(out).to_string())
}

/// The nodes on machines a call of `user`'s reaching `scopes` (JSON) may be passed on to (`ScopeDb::runners`), each
/// `{ key, seal }` in hex.
#[wasm_bindgen]
pub fn runners(owner: &str, user: &str, scopes: &str) -> Result<String, JsError> {
    let scopes = serde_json::from_str::<Vec<String>>(scopes)?.iter().map(|s| Scope::parse(s)).collect::<Result<Vec<_>>>().map_err(thrown)?;
    let out = replayed(owner, &[], |r| {
        Ok(r.global().runners(user, &scopes)?.iter().map(|r| json!({ "key": one::keys::hex(&r.key), "seal": one::keys::hex(&r.seal) })).collect::<Vec<_>>())
    })
    .map_err(thrown)?;
    Ok(Value::from(out).to_string())
}

/// What this node (its key, `seed`) asks a node on a machine to run `user`'s call (JSON) with, handing it `keys` (JSON,
/// `[{ scope, epoch, app, database, key }]`) of the call's `scopes` (JSON) sealed to its `seal` key until `until`
/// (`keys::forwarded`): the message, as JSON.
#[wasm_bindgen]
pub fn forward(seed: &str, user: &str, client: &str, call: &str, seal: &str, until: f64, keys: &str, scopes: &str) -> Result<String, JsError> {
    let from = signing_key(seed).map_err(thrown)?.verifying_key().to_bytes();
    let seal = one::keys::unhex(seal).map_err(thrown)?;
    let asked = one::keys::forwarded(&serde_json::from_str(call)?, user, client, &from, &seal, until as u64, serde_json::from_str(keys)?, &serde_json::from_str::<Vec<String>>(scopes)?).map_err(thrown)?;
    Ok(asked.to_string())
}

/// The network as `user` sees it, from the steps of `scopes` (JSON) `owner`'s object read: `{ state, apps, me, erased }`.
/// `state` is `GET /v1/state`'s, for a node that knows of no device being on; `apps` is `GET /v1/apps`'s from the
/// members alone, each with `reach`, the scopes it reaches for `user` (what its widgets and calls are given), for
/// `worker.js` to add what each app says it does; `me` is what an app is told of who it runs for; `erased`, the apps
/// erased, whose copies it forgets. `client` names auth's client.
#[wasm_bindgen]
pub fn network(owner: &str, user: &str, client: &str, scopes: &str, now: f64) -> Result<String, JsError> {
    let scopes: Vec<String> = serde_json::from_str(scopes)?;
    let out = replayed(owner, &scopes, |r| view(r, user, client, now as u64)).map_err(thrown)?;
    Ok(out.to_string())
}

fn view(r: &Replays, me: &str, client: &str, now: u64) -> Result<Value> {
    let m = r.global();
    let states: Vec<(Scope, ScopeState)> = r.0.iter().map(|(s, d)| Ok((Scope::parse(s)?, d.state()?))).collect::<Result<_>>()?;
    let state_of = |s: &Scope| states.iter().find(|(x, _)| x == s).map(|(_, st)| st);
    let read = || states.iter().map(|(s, st)| (s, st));
    let arrangements = state_of(&Scope::User(me.into())).map(|st| st.arrangements.clone()).unwrap_or_default();
    let cloudflare = m.cloudflare_node()?;
    // It knows of no device being on, nor of who else is looking.
    let state = one::state::state(
        Some(m),
        me,
        one::state::Here {
            me: json!({ "key": "", "name": client, "user": me }),
            client: Some(json!({ "key": "", "name": client })),
            device: json!({ "key": cloudflare.map(|k| one::keys::hex(&k)).unwrap_or_default(), "name": "Cloudflare", "ready": true, "guide": null }),
            member: false,
            steps: 0,
            waiting: one::state::waiting(m, me, read(), now),
            arrangements: json!(arrangements),
            backups: None,
            guide: None,
            // No throwaway network has a Cloudflare node.
            throwaway: false,
            device_now: &|_| Ok(Default::default()),
            client_now: &|_| Default::default(),
        },
    )?;
    let runs = |s: &Scope| cloudflare.is_some_and(|k| m.runs_for(&k, s));
    let every = m.apps()?;
    let apps: Vec<Value> = every
        .iter()
        .filter(|a| !a.erased)
        .map(|a| -> Result<Value> {
            let mut info = one::state::app_info(m, a, &every, me, one::state::added(m, &a.name, me, read()))?;
            if a.deleted.is_some() {
                info["reach"] = json!([]);
                return Ok(info);
            }
            let mut reach = one::state::reachable(m, me, |s| runs(s).then(|| state_of(s).is_some_and(|st| st.added.contains(&a.name))))?;
            // Only those this node runs, if it runs some, as a node on a machine leaves out a person's own scope that
            // no node running the rest runs (src/node/calls.rs: `reaching`): their own, until they choose this one.
            if reach.iter().any(runs) {
                reach.retain(runs);
            }
            info["reach"] = json!(one::state::handles(m, &reach, me)?);
            Ok(info)
        })
        .collect::<Result<_>>()?;
    let me = one::state::me(m, me, String::new(), client.to_string(), None)?;
    let erased: Vec<&String> = every.iter().filter(|a| a.erased).map(|a| &a.name).collect();
    Ok(json!({ "state": state, "apps": apps, "me": me, "erased": erased, "handles": m.handles()? }))
}

/// A deploy's bundle from the bucket (`bucket::blob_name`), opened with the keys of the node's `blobs` database in
/// `global` and checked against its hash: → the app's module.
#[wasm_bindgen]
pub async fn bundle(bucket: JsValue, keys: String, hash: String) -> Result<String, JsValue> {
    let keys = keys_of(&keys).map_err(failed)?;
    let name = one::bucket::blob_name("global", &hash);
    let sealed = Js(bucket).get(&name).await.map_err(failed)?.ok_or_else(|| failed(anyhow!("{name} isn't in the bucket yet: a node on one of your machines puts it there")))?;
    let epoch = one::keys::epoch_of(&sealed.bytes).map_err(failed)?;
    let key = keys.get(&epoch).ok_or_else(|| failed(one::database::stored::Missing { epoch }.into()))?;
    let bytes = one::keys::open(key, &name, &sealed.bytes).map_err(failed)?;
    if blake3::hash(&bytes).to_hex().as_str() != hash {
        return Err(failed(anyhow!("the bundle kept as {hash} isn't it")));
    }
    one::app::bundle::module(&bytes).map_err(failed)
}
