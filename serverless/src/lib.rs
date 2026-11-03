//! The serverless node's Rust: the machine node's own code, as WebAssembly on Workers. `worker.js` answers requests
//! and runs apps; this checks access tokens, opens the keys callers hand it, signs what it asks the URL signer, and
//! reads and writes apps' databases in the bucket (`src/db.rs`), through the presigned URLs `worker.js` gets. It holds
//! no keys but those handed it for a sign-in's calls.

mod clients;
mod net;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, anyhow};
use js_sys::{Function, Promise, Reflect, Uint8Array};
use one::bucket::{Bucket, Failed, Object, Trouble};
use one::call::Append;
use one::db::Db;
use one::entry::{Entry, Hash, Scope, causal};
use one::members::{self, Members, ScopeState};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// The bucket as `worker.js` reaches it: an object whose `get(name)`, `create(name, bytes)` and `replace(name, bytes,
/// tag)` give promises, of `{ bytes, tag }` or null, and of whether it was written.
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
    if let Some(m) = err.downcast_ref::<one::db::Missing>() {
        let _ = Reflect::set(&out, &"missing".into(), &m.epoch.into());
    }
    out.into()
}

/// A database's keys as `worker.js` keeps them: JSON, `{ <epoch>: <key, base64url> }`.
fn keys_of(keys: &str) -> Result<one::db::Keys> {
    let keys: BTreeMap<u32, String> = serde_json::from_str(keys)?;
    keys.into_iter().map(|(n, k)| Ok((n, one::kernel::decode_key(&k)?))).collect()
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

/// The keys a caller handed this node with `token`'s calls (`x-one-keys`), kept no longer than the token lasts
/// (`exp`, milliseconds): → `{ until, keys: [{ scope, epoch, app, database, key }] }`.
#[wasm_bindgen]
pub fn handoff(seed: &str, token: &str, sealed: &str, now: f64, exp: f64) -> Result<String, JsError> {
    let handoff = one::keys::open_handoff(&one::keys::seal_secret(&signing_key(seed).map_err(thrown)?), sealed).map_err(thrown)?;
    if handoff.for_ != one::keys::token_id(token) {
        return Err(JsError::new("the keys handed are for another sign-in"));
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
/// on a machine says it (src/node/sessions.rs): every epoch from the one the database's newest copy is under to the
/// scope's current one, as the install's signed entries say (`epochs`, JSON `{ <scope>: n }`), never what the bucket
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
        let hint = match bucket.get(&one::db::newest_name(&prefix_of(scope, app, database).map_err(thrown)?)).await.map_err(thrown)? {
            Some(o) => Some(one::db::newest(&o.bytes)),
            None => None,
        };
        let oldest = match hint {
            Some((_, 0, _)) => 1,
            Some((_, _, epoch)) => epoch.clamp(1, newest),
            None => newest,
        };
        need.extend((oldest..=newest).filter(|n| !have.contains(n)).map(|epoch| json!({ "scope": scope, "epoch": epoch, "app": app, "database": database })));
    }
    Ok(Value::from(need).to_string())
}

/// The keys of a database a call uses, of those handed (`keys_of`), for a scope whose current epoch is `current`
/// (`one::db::handed`): a `missing` error names the epoch it lacks.
fn usable(keys: &str, current: u32) -> Result<one::db::Keys, JsValue> {
    one::db::handed(keys_of(keys).map_err(failed)?, current.max(1)).map_err(|m| failed(m.into()))
}

/// How many people's databases and entries one isolate keeps, the least lately used going first: each person's are
/// their Durable Object's, and several may share an isolate, which outlives them.
const OWNERS_MOST: usize = 16;

thread_local! {
    /// The databases calls running in this isolate have open, by handle, each with whose object opened it.
    static OPEN: RefCell<(u32, HashMap<u32, (String, Db)>)> = RefCell::new((0, HashMap::new()));
    /// When each person's object last used what this isolate keeps for it, to forget the least lately used.
    static OWNERS: RefCell<(u64, HashMap<String, u64>)> = RefCell::new((0, HashMap::new()));
}

/// Notes that `owner`'s object is using what this isolate keeps for it, forgetting the least lately used owner's
/// databases and entries once there are too many.
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

/// An access token's claims, as JSON, if a key in `jwks` (JSON) signed it, `iss` issued it and it's unexpired at
/// `now` (milliseconds).
#[wasm_bindgen]
pub fn verify(token: &str, jwks: &str, iss: &str, now: f64) -> Result<String, JsError> {
    let jwks: Value = serde_json::from_str(jwks)?;
    let claims = one::kernel::verify(token, &jwks, iss, now as u64).map_err(thrown)?;
    Ok(json!({ "sub": claims.sub, "azp": claims.azp, "aud": claims.aud, "exp": claims.exp, "scope": claims.scope }).to_string())
}

/// Opens a database at `prefix` for `owner`'s object with the keys handed (`usable`, under the scope's `current`
/// epoch) and brings it to `migrations` (JSON, its app's schema for the scope's kind): → a handle for `query`, `write`
/// and `close`.
#[wasm_bindgen]
pub async fn open(owner: String, bucket: JsValue, prefix: String, keys: String, current: u32, migrations: String) -> Result<u32, JsValue> {
    touch(&owner);
    let bucket = Js(bucket);
    let keys = usable(&keys, current)?;
    let migrations: Vec<String> = serde_json::from_str(&migrations).map_err(|e| failed(e.into()))?;
    let mut db = Db::open(&bucket, &prefix, keys).await.map_err(failed)?;
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

/// Brings an open database up to what the bucket has, and to `migrations`, with the keys it was opened with.
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

/// Reads with a statement that only reads: its rows, as JSON.
#[wasm_bindgen]
pub fn query(handle: u32, sql: &str, params: &str) -> Result<String, JsError> {
    let params: Vec<Value> = serde_json::from_str(params)?;
    OPEN.with(|o| {
        let o = o.borrow();
        let (_, db) = o.1.get(&handle).context("that database isn't open").map_err(thrown)?;
        Ok(Value::from(db.query(sql, &params).map_err(thrown)?).to_string())
    })
}

/// Writes a call's statements (JSON, `[[sql, params, check]]`) as one step: → its position, or 0 if they changed
/// nothing.
#[wasm_bindgen]
pub async fn write(handle: u32, bucket: JsValue, statements: String) -> Result<f64, JsError> {
    let list: Vec<(String, Vec<Value>, Option<String>)> = serde_json::from_str(&statements)?;
    let (owner, mut db) = take(handle).map_err(thrown)?;
    let at = db.at();
    let wrote = db.write(&Js(bucket), |c| one::db::statements(c, &list)).await;
    let step = if db.at() > at { db.at() as f64 } else { 0.0 };
    put(handle, (owner, db));
    wrote.map_err(thrown)?;
    Ok(step)
}

#[wasm_bindgen]
pub fn close(handle: u32) {
    OPEN.with(|o| o.borrow_mut().1.remove(&handle));
}

/// The migrations of a scope's Vault database (`node`'s `vault`), for `open`.
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
/// as a node on a machine fills an app's requests (`src/outbound.rs`).
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
                    return Err(JsError::new("a serverless node doesn't keep blobs yet: use a node on one of your machines"));
                }
                out.entry(scope).or_insert_with(|| json!([])).as_array_mut().unwrap().push(json!([sql, params, check]));
            }
            // What it spent on AI shows in insights, which this node doesn't keep yet: nothing is lost by not counting it.
            Append::Cost { .. } => {}
            _ => return Err(JsError::new("a serverless node writes only an app's own data yet: use a node on one of your machines")),
        }
    }
    Ok(Value::Object(out).to_string())
}

/// An app screen's page (`src/screen.rs`), from what the screen is told (JSON) and the HTML the app wrote.
#[wasm_bindgen]
pub fn screen(context: &str, body: &str) -> Result<String, JsError> {
    Ok(one::screen::page(&serde_json::from_str(context)?, body, false))
}

/// A client's own screen's page (`src/screen.rs`), from what the screen is told (JSON) and its name.
#[wasm_bindgen]
pub fn client_screen(context: &str, name: &str) -> Result<String, JsError> {
    Ok(one::screen::page(&serde_json::from_str(context)?, one::screen::client_screen(name), true))
}

/// The white label this node is: `{ name, cli, accent, port, made_here }`.
#[wasm_bindgen]
pub fn config() -> String {
    json!({ "name": one::config::NAME, "cli": one::config::CLI, "accent": one::config::ACCENT, "port": one::config::PORT, "made_here": one::config::MADE_HERE }).to_string()
}

/// Where an app's database of a scope is in the bucket (`src/db.rs`): only of a scope there can be, and an app and
/// database whose names are only letters, digits and dashes.
fn prefix_of(scope: &str, app: &str, database: &str) -> Result<String> {
    let scope = Scope::parse(scope)?.to_string();
    let plain = |s: &str| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    anyhow::ensure!(plain(app) && plain(database), "{app}/{database} isn't an app's database");
    Ok(format!("{}/{app}/{database}", one::bucket::folder(&scope)))
}

#[wasm_bindgen]
pub fn prefix(scope: &str, app: &str, database: &str) -> Result<String, JsError> {
    prefix_of(scope, app, database).map_err(thrown)
}

/// The names of the `n` steps after `name`, if it's a database's step, as JSON: what's read next.
#[wasm_bindgen]
pub fn following(name: &str, n: u32) -> String {
    let Some((prefix, at)) = name.rsplit_once('/') else { return "[]".into() };
    let Some(at) = at.parse::<u64>().ok().filter(|n| one::db::step_name(prefix, *n) == name) else { return "[]".into() };
    Value::from((1..=n as u64).map(|i| one::db::step_name(prefix, at + i)).collect::<Vec<_>>()).to_string()
}

// Who can do what: each scope's entries (`src/log.rs`), and what they add up to (`src/members.rs`).

/// A scope's entries this isolate read for one person's object: the keys they were read with, the last step read, the
/// entries (in replay order for `global`; another scope's are put in order with `global`'s when replayed, as they
/// follow them), and which reading of them this is.
struct Log {
    keys: one::db::Keys,
    at: u64,
    entries: Vec<Entry>,
    read: u64,
}

/// What a scope's entries add up to, as of the readings of them and of `global`'s: `global`'s members, and the
/// scope's state.
struct Replayed {
    at: (u64, u64),
    members: Members,
    state: ScopeState,
}

thread_local! {
    /// Each scope's entries this isolate read, by whose object read them and the scope. A request uses them only if it
    /// was handed exactly the keys they were read with: a key that opened nothing passes over what's under its epoch,
    /// and what was read that way is only ever shown to whoever handed it.
    static LOGS: RefCell<HashMap<(String, String), Log>> = RefCell::new(HashMap::new());
    /// Each scope's replay, by owner and scope, while its readings and `global`'s are the same.
    static REPLAYED: RefCell<HashMap<(String, String), Replayed>> = RefCell::new(HashMap::new());
    /// Readings of entries, counted.
    static READINGS: RefCell<u64> = const { RefCell::new(0) };
}

fn reading() -> u64 {
    READINGS.with(|r| {
        *r.borrow_mut() += 1;
        *r.borrow()
    })
}

/// Reads a scope's entries for `owner`'s object, with the keys of its log a caller handed, up to its newest step: from
/// the newest copy the first time, or when the keys differ from those last read with, and after that only the steps
/// since.
#[wasm_bindgen]
pub async fn read_log(owner: String, bucket: JsValue, scope: String, keys: String) -> Result<(), JsValue> {
    touch(&owner);
    let bucket = Js(bucket);
    let keys = keys_of(&keys).map_err(failed)?;
    Scope::parse(&scope).map_err(failed)?;
    let prefix = one::log::prefix(&scope);
    let id = (owner, scope.clone());
    let from = LOGS.with(|l| l.borrow().get(&id).filter(|l| l.keys == keys).map(|l| l.at));
    let Some(from) = from else {
        let one::log::Read { at, entries, .. } = one::log::open(&bucket, &keys, &prefix, &scope).await.map_err(failed)?;
        let entries = if scope == "global" { causal(entries, &HashMap::new()) } else { entries };
        LOGS.with(|l| l.borrow_mut().insert(id, Log { keys, at, entries, read: reading() }));
        return Ok(());
    };
    let one::log::Read { at, entries: new, .. } = one::log::read(&bucket, &keys, &prefix, &scope, from, 0).await.map_err(failed)?;
    LOGS.with(|l| {
        if let Some(log) = l.borrow_mut().get_mut(&id).filter(|l| l.keys == keys) {
            let fresh: Vec<Entry> = new.into_iter().filter(|e| !log.entries.iter().any(|k| k.author() == e.author() && k.seq() == e.seq())).collect();
            if !fresh.is_empty() {
                log.entries.extend(fresh);
                if scope == "global" {
                    log.entries = causal(std::mem::take(&mut log.entries), &HashMap::new());
                }
                log.read = reading();
            }
            log.at = log.at.max(at);
        }
    });
    Ok(())
}

/// The install's genesis, which its members are replayed from.
fn root(global: &[Entry]) -> Option<Hash> {
    global.iter().find(|e| e.content.app == members::APP && e.content.kind == "genesis").map(Entry::hash)
}

/// `global`'s members and each of `scopes`' state, from the entries `owner`'s object read (`read_log`).
fn replayed<R>(owner: &str, scopes: &[String], f: impl FnOnce(&Members, &BTreeMap<String, ScopeState>) -> R) -> Result<R> {
    touch(owner);
    LOGS.with(|logs| {
        let logs = logs.borrow();
        let log_of = |scope: &str| logs.get(&(owner.to_string(), scope.to_string()));
        let global = log_of("global").context("read global's entries first")?;
        let member_entries: Vec<Entry> = global.entries.iter().filter(|e| members::is_member_entry(e)).cloned().collect();
        let root = root(&global.entries);
        // The serverless node keeps no fork's evidence: its members are replayed from the entries alone.
        let known = Members::replay(root, &member_entries, &Default::default());
        REPLAYED.with(|r| {
            let mut r = r.borrow_mut();
            let mut states = BTreeMap::new();
            let mut m = None;
            for scope in scopes {
                let Some(log) = log_of(scope) else { continue };
                let at = (global.read, log.read);
                let id = (owner.to_string(), scope.clone());
                if r.get(&id).is_none_or(|x| x.at != at) {
                    let entries = match scope.as_str() {
                        "global" => log.entries.clone(),
                        _ => causal(log.entries.clone(), &global.entries.iter().map(|e| (e.hash(), e.depth)).collect()),
                    };
                    let (members, state) = members::replay(&known, &member_entries, &entries, |_, _, _| {});
                    r.insert(id.clone(), Replayed { at, members, state });
                }
                let x = &r[&id];
                m.get_or_insert_with(|| x.members.clone());
                states.insert(scope.clone(), x.state.clone());
            }
            let m = m.unwrap_or(known);
            Ok(f(&m, &states))
        })
    })
}

/// The groups `user` is in, as scopes: whose entries to read next.
#[wasm_bindgen]
pub fn groups(owner: &str, user: &str) -> Result<String, JsError> {
    let groups = replayed(owner, &["global".into()], |m, _| m.groups_of(user).into_keys().map(|g| format!("group:{g}")).collect::<Vec<_>>()).map_err(thrown)?;
    Ok(json!(groups).to_string())
}

/// The scopes `user` may use, as the install's log says, whether they're a person of it, and whether they were removed
/// from it: `{ scopes, joined, removed }`.
#[wasm_bindgen]
pub fn scopes_of(owner: &str, user: &str) -> Result<String, JsError> {
    let out = replayed(owner, &["global".into()], |m, _| {
        let scopes: Vec<String> = m.scopes().into_iter().filter(|s| m.may_use(user, s)).map(|s| s.to_string()).collect();
        json!({ "scopes": scopes, "joined": m.user(user).is_some(), "removed": m.removed_user(user) })
    })
    .map_err(thrown)?;
    Ok(out.to_string())
}

/// Each of `scopes`' (JSON) current epoch, as the install's signed `epoch` entries say, 1 for one with none yet:
/// `{ <scope>: n }`, all a node writes under.
#[wasm_bindgen]
pub fn epochs(owner: &str, scopes: &str) -> Result<String, JsError> {
    let scopes: Vec<String> = serde_json::from_str(scopes)?;
    let out = replayed(owner, &["global".into()], |m, _| {
        scopes.iter().filter_map(|s| Some((s.clone(), Value::from(m.epoch(&Scope::parse(s).ok()?).max(1))))).collect::<serde_json::Map<_, _>>()
    })
    .map_err(thrown)?;
    Ok(Value::Object(out).to_string())
}

/// The nodes on machines a call of `user`'s reaching `scopes` (JSON) may be passed on to (`Members::runners`), each
/// `{ key, seal }` in hex.
#[wasm_bindgen]
pub fn runners(owner: &str, user: &str, scopes: &str) -> Result<String, JsError> {
    let scopes = serde_json::from_str::<Vec<String>>(scopes)?.iter().map(|s| Scope::parse(s)).collect::<Result<Vec<_>>>().map_err(thrown)?;
    let out = replayed(owner, &["global".into()], |m, _| {
        m.runners(user, &scopes).iter().map(|r| json!({ "key": one::entry::hex(&r.key), "seal": one::entry::hex(&r.seal) })).collect::<Vec<_>>()
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

/// The install as `user` sees it, from the entries of `scopes` (JSON) `owner`'s object read: `{ state, apps, me, erased }`.
/// `state` is `GET /v1/state`'s, for a node that knows of no device being on; `apps` is `GET /v1/apps`'s from the
/// members alone, each with `reach`, the scopes it reaches for `user` (what its widgets and calls are given), for
/// `worker.js` to add what each app says it does; `me` is what an app is told of who it runs for; `erased`, the apps
/// erased, whose copies it forgets. `client` names the sign-in's client.
#[wasm_bindgen]
pub fn install(owner: &str, user: &str, client: &str, scopes: &str, now: f64) -> Result<String, JsError> {
    let scopes: Vec<String> = serde_json::from_str(scopes)?;
    let out = replayed(owner, &scopes, |m, states| view(m, states, user, client, now as u64)).map_err(thrown)?;
    Ok(out.to_string())
}

fn view(m: &Members, states: &BTreeMap<String, ScopeState>, me: &str, client: &str, now: u64) -> Value {
    let mine = states.get(&Scope::User(me.into()).to_string());
    // Requests for an item waiting for `me` to answer, as src/node.rs's `item_requests` counts them.
    let manages = |s: &String| Scope::parse(s).is_ok_and(|s| members::manages_scope(m, me, &s));
    let waiting = states.iter().filter(|(s, _)| manages(s)).flat_map(|(_, s)| s.requests.values()).filter(|r| r.answer.is_none() && now < r.time + 86_400_000).count();
    // It knows of no device being on, nor of who else is looking.
    let state = one::state::state(
        Some(m),
        me,
        one::state::Here {
            me: json!({ "key": "", "name": client, "user": me }),
            client: Some(json!({ "key": "", "name": client })),
            device: json!({ "key": "", "name": "", "ready": true, "guide": null }),
            member: false,
            entries: 0,
            waiting,
            arrangements: json!(mine.map(|s| s.arrangements.clone()).unwrap_or_default()),
            backups: None,
            guide: None,
            // No try install has a serverless node.
            passkeys: one::config::SIGN_IN != "google",
            device_now: &|_| Default::default(),
            client_now: &|_| Default::default(),
        },
    );
    let user_name = m.user(me).map(|u| u.name.clone()).unwrap_or_default();
    let installed_in = |app: &str, scope: &Scope| states.get(&scope.to_string()).is_some_and(|s| s.installed.contains(app));
    let handle = |s: &Scope| {
        let name = match s {
            Scope::Global => m.name.clone(),
            Scope::Group(g) => m.groups.get(g).map(|g| g.name.clone()).unwrap_or_default(),
            Scope::User(u) => m.users.get(u).map(|u| u.name.clone()).unwrap_or_default(),
        };
        json!({ "id": s.to_string(), "name": name, "role": m.role_in(me, s) })
    };
    let apps: Vec<Value> = m
        .apps
        .values()
        .filter(|a| !a.erased)
        .map(|a| {
            let mut installed: Vec<String> = states.keys().filter(|s| Scope::parse(s).is_ok_and(|s| installed_in(&a.name, &s) && m.may_use(me, &s))).cloned().collect();
            installed.sort_by_key(|s| (!s.starts_with("user:"), s == "global", s.clone()));
            let history: Vec<Value> = a.history.iter().map(|(bundle, time, user)| json!({ "bundle": bundle, "time": time, "user": user })).collect();
            let forks: Vec<&String> = m.apps.values().filter(|f| f.from.as_ref() == Some(&a.name)).map(|f| &f.name).collect();
            let mut info = json!({ "name": a.name, "owner": a.owner.to_string(), "from": a.from, "forks": forks, "deploys": a.deploys, "bundle": a.bundle, "history": history, "installed": installed });
            if let Some(link) = a.source.as_ref().filter(|l| !l.is_empty()) {
                info["source"] = json!(link);
            }
            if let Some((time, by)) = &a.deleted {
                info["installed"] = json!([]);
                info["deleted"] = json!({ "time": time, "by": m.users.get(by).map(|u| &u.name), "yours": m.acts_for(me, &a.owner) });
                info["reach"] = json!([]);
                return info;
            }
            // As a node on a machine reaches them (src/node.rs: `reachable`), of the scopes read here.
            let global = installed_in(&a.name, &Scope::Global);
            let mut reach: Vec<Scope> = m.groups_of(me).into_keys().map(Scope::Group).filter(|s| states.contains_key(&s.to_string()) && (global || installed_in(&a.name, s))).collect();
            let own = Scope::User(me.into());
            if global || installed_in(&a.name, &own) || !reach.is_empty() {
                reach.insert(0, own);
            }
            if global {
                reach.push(Scope::Global);
            }
            info["reach"] = Value::from(reach.iter().map(handle).collect::<Vec<_>>());
            info
        })
        .collect();
    let me_ = json!({ "key": "", "user": me, "name": client, "role": m.user(me).map(|u| u.role), "user_name": user_name, "agent": null });
    let erased: Vec<&String> = m.apps.values().filter(|a| a.erased).map(|a| &a.name).collect();
    json!({ "state": state, "apps": apps, "me": me_, "erased": erased })
}

/// A deploy's bundle from the bucket (`bucket::blob_name`), opened with the keys of the node's `blobs` database in
/// `global` and checked against its hash: → the app's module.
#[wasm_bindgen]
pub async fn bundle(bucket: JsValue, keys: String, hash: String) -> Result<String, JsValue> {
    let keys = keys_of(&keys).map_err(failed)?;
    let name = one::bucket::blob_name("global", &hash);
    let sealed = Js(bucket).get(&name).await.map_err(failed)?.ok_or_else(|| failed(anyhow!("{name} isn't in the bucket yet: a node on one of your machines puts it there")))?;
    let epoch = one::keys::epoch_of(&sealed.bytes).map_err(failed)?;
    let key = keys.get(&epoch).ok_or_else(|| failed(one::db::Missing { epoch }.into()))?;
    let bytes = one::keys::open(key, &name, &sealed.bytes).map_err(failed)?;
    if blake3::hash(&bytes).to_hex().as_str() != hash {
        return Err(failed(anyhow!("the bundle kept as {hash} isn't it")));
    }
    one::bundle::module(&bytes).map_err(failed)
}
