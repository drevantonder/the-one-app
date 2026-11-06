//! The Cloudflare node's Rust: the machine node's own code, as WebAssembly on Workers. `worker.js` answers requests
//! and runs apps; this checks who asks, opens the keys callers hand it, signs what it asks the URL signer, and
//! reads and writes apps' databases in the bucket (`src/database/stored.rs`), through the presigned URLs `worker.js` gets. It holds
//! no keys but those handed it for a device's requests.

mod clients;
mod net;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, anyhow};
use js_sys::{Function, Promise, Reflect, Uint8Array};
use one::bucket::{Bucket, Failed, Object, Trouble};
use one::call::Append;
use one::database::stored::Db;
use one::database::scopes::{Scope, ScopeDb, app, object};
use one::keys::Hash;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// The bucket as `worker.js` reaches it: an object whose `get(name)`, `create(name, bytes)`, `replace(name, bytes,
/// tag)` and `objects(folder)` give promises, of `{ bytes, tag }` or null, of whether it was written, and of
/// `[name, written]` for each object in the folder.
struct Js(JsValue);

impl Js {
    async fn ask(&self, method: &str, args: &[JsValue]) -> Result<JsValue> {
        let f: Function = Reflect::get(&self.0, &method.into()).map_err(said)?.dyn_into().map_err(|_| anyhow!("the bucket has no {method}"))?;
        let promise: Promise = f.apply(&self.0, &args.iter().collect()).map_err(said)?.dyn_into().map_err(|_| anyhow!("{method} gave no promise"))?;
        JsFuture::from(promise).await.map_err(|err| match Reflect::get(&err, &"unanswered".into()).is_ok_and(|u| u.is_truthy()) {
            true => Failed(Trouble::Unreachable, format!("{}", said(err))).into(),
            false => said(err),
        })
    }
}

impl Bucket for Js {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        let got = self.ask("get", &[name.into()]).await?;
        if got.is_null() || got.is_undefined() {
            return Ok(None);
        }
        let bytes = Uint8Array::new(&Reflect::get(&got, &"bytes".into()).map_err(said)?).to_vec();
        let tag = Reflect::get(&got, &"tag".into()).map_err(said)?.as_string().context("an object has its tag")?;
        Ok(Some(Object { bytes, tag }))
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        Ok(self.ask("create", &[name.into(), Uint8Array::from(bytes).into()]).await?.is_truthy())
    }

    async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
        Ok(self.ask("replace", &[name.into(), Uint8Array::from(bytes).into(), tag.map(JsValue::from).unwrap_or(JsValue::NULL)]).await?.is_truthy())
    }

    async fn wait(&self, ms: u64) {
        let _ = self.ask("wait", &[(ms as f64).into()]).await;
    }

    async fn objects(&self, folder: &str) -> Result<Vec<(String, u64)>> {
        let listed: js_sys::Array = self.ask("objects", &[folder.into()]).await?.dyn_into().map_err(|_| anyhow!("a listing is a list"))?;
        listed
            .iter()
            .map(|o| {
                let o: js_sys::Array = o.dyn_into().map_err(|_| anyhow!("a listed object is [name, written]"))?;
                Ok((o.get(0).as_string().context("a listed object has its name")?, o.get(1).as_f64().unwrap_or(0.0) as u64))
            })
            .collect()
    }
}

fn said(err: JsValue) -> anyhow::Error {
    anyhow!("{}", err.as_string().or_else(|| Reflect::get(&err, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{err:?}")))
}

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
        REPLAYED.with(|r| r.borrow_mut().retain(|(by, _), _| *by != gone));
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
    replayed(owner, &[], |r| Value::from(r.global().app_may(user, &asks, &types, &rows, now as u64)).to_string()).map_err(thrown)
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

/// If `name` is a database's step, or the signature beside one: the signature beside it, and the `n` steps after it,
/// each with the signature beside it, as JSON: what's read next. A scope's log's steps have none.
#[wasm_bindgen]
pub fn following(name: &str, n: u32) -> String {
    let Some((prefix, at)) = one::database::stored::position(name) else { return "[]".into() };
    let signed = one::database::stored::signed_beside(prefix);
    let names = (at..=at + n as u64).flat_map(|i| [Some(one::database::stored::step_name(prefix, i)), signed.then(|| one::database::stored::sig_name(prefix, i))]).flatten();
    Value::from(names.filter(|other| other.as_str() > name).collect::<Vec<_>>()).to_string()
}

// Who can do what: each scope's database's steps (`src/database/steps.rs`), which every question of who's who and who
// may is asked of.

/// A scope's steps this isolate read for one person's object: the keys they were read with, which reading of them this
/// is, and the steps, by position from the first: of `global`, its freeze among them.
struct Held {
    keys: one::database::stored::Keys,
    read: u64,
    steps: Vec<Option<one::database::steps::Given>>,
    /// Of another scope: its start (`steps::START`, step 0), once looked for; none inside if there's none.
    start: Option<Option<one::database::steps::Given>>,
}

impl Held {
    /// Of `global`'s: who's who from the network's first freeze (`steps::Truth::first`), of the root pinned
    /// (`pin_root`), and the steps after; none before. Fails at a newer release's freeze or step, which stops this node
    /// there as it stops every reader.
    fn truth(&self) -> Result<Option<one::database::steps::Truth>> {
        one::database::steps::Truth::read(&self.steps, ROOT.with(|r| *r.borrow()), now())
    }

    /// The newest epoch of `scope` made, as `global`'s steps say: any, before it's read the freeze.
    fn made(global: &Held, scope: &Scope) -> Result<u32> {
        Ok(global.truth()?.map_or(u32::MAX, |t| t.db.epoch(scope)))
    }
}

/// A scope's database, as of the readings of its steps and of `global`'s.
struct Replayed {
    at: (u64, u64),
    db: ScopeDb,
}

/// Each scope's database `replayed` read, `global`'s among them, which says who's who in the whole network.
struct Replays<'a>(BTreeMap<String, &'a ScopeDb>);

impl Replays<'_> {
    fn global(&self) -> &ScopeDb {
        self.0["global"]
    }

    fn get(&self, scope: &Scope) -> Option<&ScopeDb> {
        self.0.get(&scope.to_string()).copied()
    }
}

thread_local! {
    /// Each scope's steps this isolate read, by whose object read them and the scope. A request uses them only if it
    /// was handed exactly the keys they were read with: a key that opened nothing passes over what's under its epoch,
    /// and what was read that way is only ever shown to whoever handed it.
    static LOGS: RefCell<HashMap<(String, String), Held>> = RefCell::new(HashMap::new());
    /// Each scope's database, by owner and scope, while its readings and `global`'s are the same.
    static REPLAYED: RefCell<HashMap<(String, String), Replayed>> = RefCell::new(HashMap::new());
    /// Readings of steps, counted.
    static READINGS: RefCell<u64> = const { RefCell::new(0) };
}

fn reading() -> u64 {
    READINGS.with(|r| {
        *r.borrow_mut() += 1;
        *r.borrow()
    })
}

/// Reads a scope's steps for `owner`'s object, with the keys a caller handed, up to its newest: from the first the
/// first time, or when the keys differ from those last read with, and after that only those since.
#[wasm_bindgen]
pub async fn read_scope(owner: String, bucket: JsValue, scope: String, keys: String) -> Result<(), JsValue> {
    touch(&owner);
    let keys = keys_of(&keys).map_err(failed)?;
    Scope::parse(&scope).map_err(failed)?;
    read_scope_with((owner, scope), &Js(bucket), keys).await.map_err(failed)
}

/// `read_scope`, of `id`'s scope for its owner, with its `keys`.
async fn read_scope_with(id: (String, String), bucket: &Js, keys: one::database::stored::Keys) -> Result<()> {
    if LOGS.with(|l| l.borrow().get(&id).is_none_or(|l| l.keys != keys)) {
        LOGS.with(|l| l.borrow_mut().insert(id.clone(), Held { keys, read: reading(), steps: Vec::new(), start: None }));
    }
    read_steps(&id, bucket).await
}

/// A scope's database's steps after those read, with its keys: another scope's once `global`'s log is frozen, with its
/// start the first time.
async fn read_steps(id: &(String, String), bucket: &Js) -> Result<()> {
    let global = (id.0.clone(), "global".to_string());
    let Some((keys, from, started)) = LOGS.with(|l| l.borrow().get(id).map(|g| (g.keys.clone(), g.steps.len() as u64, g.start.is_some()))) else { return Ok(()) };
    let scope = Scope::parse(&id.1)?;
    if id.1 != "global" {
        let frozen = LOGS.with(|l| l.borrow().get(&global).map(|g| g.truth().map(|t| t.is_some())).transpose())?.unwrap_or(false);
        if !frozen {
            return Ok(());
        }
        if !started {
            let start = match bucket.get(&one::database::steps::start_name(&scope)).await? {
                Some(o) => match one::database::steps::open(&keys, u32::MAX, &scope, 0, &o.bytes) {
                    one::database::steps::Opened::Step(g) => Some(g),
                    _ => None,
                },
                None => None,
            };
            LOGS.with(|l| l.borrow_mut().get_mut(id).map(|log| log.start = Some(start)));
        }
    }
    let mut from = from;
    loop {
        let read = one::database::steps::read(bucket, &keys, &scope, from).await?;
        let n = read.steps.len() as u64;
        if n > 0 {
            LOGS.with(|l| {
                if let Some(log) = l.borrow_mut().get_mut(id).filter(|g| g.steps.len() as u64 == from) {
                    log.steps.extend(read.steps.iter().cloned());
                    log.read = reading();
                }
            });
        }
        from += n;
        // One under an epoch made whose key it wasn't handed: asked for (428), as the log's are. One under an epoch
        // above every one made, as the steps before it leave it, is junk: passed over.
        let made = LOGS.with(|l| l.borrow().get(&global).map(|g| Held::made(g, &scope)).transpose());
        match (read.waits, made) {
            (Some(epoch), Ok(Some(made))) if epoch <= made => return Err(one::database::stored::Missing { epoch }.into()),
            (Some(_), Ok(Some(made))) if read.junk_at_end(made) => {}
            _ => return Ok(()),
        }
        let passed = LOGS.with(|l| {
            let mut logs = l.borrow_mut();
            let log = logs.get_mut(id).filter(|g| g.steps.len() as u64 == from)?;
            log.steps.push(None);
            log.read = reading();
            Some(())
        });
        if passed.is_none() {
            return Ok(());
        }
        from += 1;
    }
}

/// A scope's own database from its steps (`one::database::steps`): from its start (`steps::started`), each that counts
/// with `global`'s scope database `truth`. Fails at a newer release's step, which stops this node there as it stops
/// every reader.
fn scope_truth(truth: &one::database::steps::Truth, start: Option<&one::database::steps::Given>, scope: &Scope, steps: &[Option<one::database::steps::Given>]) -> Result<ScopeDb> {
    use one::database::steps;
    let db = steps::started(&truth.freeze, &truth.db, scope, start, now())?;
    let took = steps::take_scope(0, db, &truth.db, scope, steps, now())?;
    anyhow::ensure!(!took.newer, steps::Newer);
    Ok(took.db)
}

thread_local! {
    /// The network's root, as the device that deployed this node holds it (`pin_root`).
    static ROOT: RefCell<Option<Hash>> = const { RefCell::new(None) };
}

/// Pins the network's root to `root` (hex, the Worker's), as sign-in pins it: a freeze naming any other is never
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
    let (people, epoch) = replayed(&owner, &[], |r| (r.global().may(&user, "use", one::database::scopes::NETWORK), r.global().epoch(&Scope::Global))).map_err(failed)?;
    if !people {
        return Err(failed(anyhow!("you aren't one of this network's people")));
    }
    let read = LOGS.with(|l| l.borrow().get(&(owner.clone(), "global".to_string())).map(|g| g.truth().map(|t| t.map(|t| t.at)))).transpose().map_err(failed)?.flatten();
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

/// `global`'s database and each of `scopes`', from the steps `owner`'s object read (`read_scope`), built again only
/// once what was read of them changed.
fn replayed<R>(owner: &str, scopes: &[String], f: impl FnOnce(&Replays) -> R) -> Result<R> {
    touch(owner);
    LOGS.with(|logs| {
        let logs = logs.borrow();
        let log_of = |scope: &str| logs.get(&(owner.to_string(), scope.to_string()));
        let global = log_of("global").context("read global's steps first")?;
        REPLAYED.with(|r| {
            let mut r = r.borrow_mut();
            let mut read = vec!["global".to_string()];
            read.extend(scopes.iter().filter(|s| *s != "global" && log_of(s).is_some()).cloned());
            for scope in &read {
                let log = log_of(scope).context("read global's steps first")?;
                let at = (global.read, log.read);
                let id = (owner.to_string(), scope.clone());
                if r.get(&id).is_none_or(|x| x.at != at) {
                    // Who's who is `global`'s scope database: its freeze's, and the steps after; and a scope's own state
                    // its own steps', from its start on. Nobody's until they're read.
                    let truth = global.truth()?;
                    let s = Scope::parse(scope)?;
                    let db = match (truth, scope.as_str(), &log.start) {
                        (Some(t), "global", _) => t.db,
                        (Some(t), _, Some(start)) => scope_truth(&t, start.as_ref(), &s, &log.steps)?,
                        _ => ScopeDb::empty(&s, now())?,
                    };
                    r.insert(id.clone(), Replayed { at, db });
                }
            }
            Ok(f(&Replays(read.iter().map(|s| (s.clone(), &r[&(owner.to_string(), s.clone())].db)).collect())))
        })
    })
}

/// The groups `user` is in, as scopes: whose steps to read next.
#[wasm_bindgen]
pub fn groups(owner: &str, user: &str) -> Result<String, JsError> {
    let groups = replayed(owner, &[], |r| r.global().groups_of(user).into_keys().map(|g| format!("group:{g}")).collect::<Vec<_>>()).map_err(thrown)?;
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
        let scopes: Vec<Scope> = m.scopes().into_iter().filter(|s| m.may(user, "use", &object(s))).collect();
        let device = key.and_then(|k| m.client(&k)).filter(|d| d.user == user).map(|d| d.name);
        let runs: Vec<String> = scopes.iter().filter(|s| m.cloudflare_node().is_some_and(|k| m.runs_for(&k, s))).map(Scope::to_string).collect();
        let scopes: Vec<String> = scopes.iter().map(Scope::to_string).collect();
        let removed = m.user_ever(user).is_some_and(|u| u.removed);
        json!({ "scopes": scopes, "joined": m.user(user).is_some(), "removed": removed, "device": device, "runs": runs })
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
        scopes.iter().filter_map(|s| Some((s.clone(), Value::from(r.global().epoch(&Scope::parse(s).ok()?).max(1))))).collect::<serde_json::Map<_, _>>()
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
        r.global().runners(user, &scopes).iter().map(|r| json!({ "key": one::keys::hex(&r.key), "seal": one::keys::hex(&r.seal) })).collect::<Vec<_>>()
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
/// erased, whose copies it forgets. `client` names the sign-in's client.
#[wasm_bindgen]
pub fn network(owner: &str, user: &str, client: &str, scopes: &str, now: f64) -> Result<String, JsError> {
    let scopes: Vec<String> = serde_json::from_str(scopes)?;
    let out = replayed(owner, &scopes, |r| view(r, user, client, now as u64)).map_err(thrown)?;
    Ok(out.to_string())
}

fn view(r: &Replays, me: &str, client: &str, now: u64) -> Value {
    let m = r.global();
    // Requests for an item waiting for `me` to answer, as src/node.rs's `item_requests` counts them.
    let since = i64::try_from(now).unwrap_or(i64::MAX) - 86_400_000;
    let waiting: i64 = r
        .0
        .iter()
        .filter(|(s, _)| Scope::parse(s).is_ok_and(|s| m.may(me, "add-apps", &object(&s))))
        .map(|(_, d)| d.conn().query_row("SELECT count(*) FROM vault_requests WHERE answer IS NULL AND asked_at > ?", [since], |r| r.get(0)).unwrap_or(0))
        .sum();
    let mut arrangements = serde_json::Map::new();
    if let Some(d) = r.get(&Scope::User(me.into())) {
        let pages = d.conn().prepare("SELECT page, arrangement FROM pages ORDER BY page").and_then(|mut q| q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>());
        for (page, arrangement) in pages.unwrap_or_default() {
            arrangements.insert(page, serde_json::from_str(&arrangement).unwrap_or(Value::Null));
        }
    }
    // It knows of no device being on, nor of who else is looking.
    let state = one::state::state(
        Some(m),
        me,
        one::state::Here {
            me: json!({ "key": "", "name": client, "user": me }),
            client: Some(json!({ "key": "", "name": client })),
            device: json!({ "key": m.cloudflare_node().map(|k| one::keys::hex(&k)).unwrap_or_default(), "name": "Cloudflare", "ready": true, "guide": null }),
            member: false,
            steps: 0,
            waiting: waiting as usize,
            arrangements: Value::Object(arrangements),
            backups: None,
            guide: None,
            // No throwaway network has a Cloudflare node.
            throwaway: false,
            device_now: &|_| Default::default(),
            client_now: &|_| Default::default(),
        },
    );
    let me_user = m.user(me);
    let installed_in = |app: &str, scope: &Scope| r.get(scope).is_some_and(|d| d.installed().contains(app));
    let handle = |s: &Scope| {
        let name = match s {
            Scope::Global => m.name(),
            Scope::Group(g) => m.group(g).map(|g| g.name).unwrap_or_default(),
            Scope::User(u) => m.user_ever(u).map(|u| u.name).unwrap_or_default(),
        };
        json!({ "id": s.to_string(), "name": name, "role": m.role_in(me, s) })
    };
    let every = m.apps();
    let apps: Vec<Value> = every
        .iter()
        .filter(|a| !a.erased)
        .map(|a| {
            let mut installed: Vec<String> = r.0.keys().filter(|s| Scope::parse(s).is_ok_and(|s| installed_in(&a.name, &s) && m.may(me, "use", &object(&s)))).cloned().collect();
            installed.sort_by_key(|s| (!s.starts_with("user:"), s == "global", s.clone()));
            let history: Vec<Value> = a.history.iter().map(|(bundle, time, user)| json!({ "bundle": bundle, "time": time, "user": user })).collect();
            let forks: Vec<&String> = every.iter().filter(|f| f.from.as_ref() == Some(&a.name)).map(|f| &f.name).collect();
            let mut info = json!({ "name": a.name, "owner": a.owner.to_string(), "from": a.from, "forks": forks, "deploys": a.deploys, "bundle": a.bundle, "history": history, "installed": installed });
            if let Some(link) = a.source.as_ref().filter(|l| !l.is_empty()) {
                info["source"] = json!(link);
            }
            if let Some((time, by)) = &a.deleted {
                info["installed"] = json!([]);
                info["deleted"] = json!({ "time": time, "by": m.user_ever(by).map(|u| u.name), "yours": m.may(me, "delete", &app(&a.name)) });
                info["reach"] = json!([]);
                return info;
            }
            // As a node on a machine reaches them (src/node.rs: `reachable`), of the scopes read here.
            let global = installed_in(&a.name, &Scope::Global);
            let mut reach: Vec<Scope> = m.groups_of(me).into_keys().map(Scope::Group).filter(|s| r.get(s).is_some() && (global || installed_in(&a.name, s))).collect();
            let own = Scope::User(me.into());
            if global || installed_in(&a.name, &own) || !reach.is_empty() {
                reach.insert(0, own);
            }
            if global {
                reach.push(Scope::Global);
            }
            // Only those this node runs, if it runs some, as a node on a machine leaves out a person's own scope that
            // no node running the rest runs (src/node/calls.rs: `reaching`): their own, until they choose this one.
            let runs = |s: &Scope| m.cloudflare_node().is_some_and(|k| m.runs_for(&k, s));
            if reach.iter().any(runs) {
                reach.retain(runs);
            }
            info["reach"] = Value::from(reach.iter().map(handle).collect::<Vec<_>>());
            info
        })
        .collect();
    let me_ = json!({ "key": "", "user": me, "name": client, "role": me_user.as_ref().map(|u| u.role), "user_name": me_user.as_ref().map(|u| u.name.clone()).unwrap_or_default(), "handle": me_user.as_ref().map(|u| u.handle.clone()), "agent": null });
    let erased: Vec<&String> = every.iter().filter(|a| a.erased).map(|a| &a.name).collect();
    json!({ "state": state, "apps": apps, "me": me_, "erased": erased, "handles": m.handles() })
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
