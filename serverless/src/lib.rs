//! The serverless node's Rust: the machine node's own code, as WebAssembly on Workers. `worker.js` answers requests,
//! asks the vault and runs apps; this checks access tokens, and reads and writes apps' databases in the bucket
//! (`src/db.rs`), through the presigned URLs `worker.js` gets for each call. Nothing outlives the call.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, anyhow};
use js_sys::{Function, Promise, Reflect, Uint8Array};
use one::bucket::{Bucket, Object};
use one::call::Append;
use one::db::Db;
use one::entry::{Entry, Scope};
use one::members::{self, Members, ScopeState};
use p2panda_core::Hash;
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
        JsFuture::from(promise).await.map_err(said)
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
}

fn said(err: JsValue) -> anyhow::Error {
    anyhow!("{}", err.as_string().or_else(|| Reflect::get(&err, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{err:?}")))
}

fn thrown(err: anyhow::Error) -> JsError {
    JsError::new(&format!("{err:#}"))
}

thread_local! {
    /// The databases calls running in this isolate have open, by handle.
    static OPEN: RefCell<(u32, HashMap<u32, Db>)> = RefCell::new((0, HashMap::new()));
}

fn take(handle: u32) -> Result<Db> {
    OPEN.with(|o| o.borrow_mut().1.remove(&handle)).context("that database isn't open")
}

fn put(handle: u32, db: Db) {
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

/// Opens a database at `prefix` with its key (base64url, from the vault) and brings it to `migrations` (JSON, its
/// app's schema for the scope's kind): → a handle for `query`, `write` and `close`.
#[wasm_bindgen]
pub async fn open(bucket: JsValue, prefix: String, key: String, migrations: String) -> Result<u32, JsError> {
    let bucket = Js(bucket);
    let key = one::kernel::decode_key(&key).map_err(thrown)?;
    let migrations: Vec<String> = serde_json::from_str(&migrations)?;
    let mut db = Db::open(&bucket, &prefix, key).await.map_err(thrown)?;
    db.migrate(&bucket, &migrations).await.map_err(thrown)?;
    let handle = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        o.0 += 1;
        o.0
    });
    put(handle, db);
    Ok(handle)
}

/// Reads with a statement that only reads: its rows, as JSON.
#[wasm_bindgen]
pub fn query(handle: u32, sql: &str, params: &str) -> Result<String, JsError> {
    let params: Vec<Value> = serde_json::from_str(params)?;
    OPEN.with(|o| {
        let o = o.borrow();
        let db = o.1.get(&handle).context("that database isn't open").map_err(thrown)?;
        Ok(Value::from(db.query(sql, &params).map_err(thrown)?).to_string())
    })
}

/// Writes a call's statements (JSON, `[[sql, params, check]]`) as one step: → its position, or 0 if they changed
/// nothing.
#[wasm_bindgen]
pub async fn write(handle: u32, bucket: JsValue, statements: String) -> Result<f64, JsError> {
    let list: Vec<(String, Vec<Value>, Option<String>)> = serde_json::from_str(&statements)?;
    let mut db = take(handle).map_err(thrown)?;
    let at = db.at();
    let wrote = db.write(&Js(bucket), |c| one::db::statements(c, &list)).await;
    let step = if db.at() > at { db.at() as f64 } else { 0.0 };
    put(handle, db);
    wrote.map_err(thrown)?;
    Ok(step)
}

#[wasm_bindgen]
pub fn close(handle: u32) {
    OPEN.with(|o| o.borrow_mut().1.remove(&handle));
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
    Ok(one::screen::page(&serde_json::from_str(context)?, body))
}

/// A client's own screen's page (`src/screen.rs`), from what the screen is told (JSON) and its name.
#[wasm_bindgen]
pub fn client_screen(context: &str, name: &str) -> Result<String, JsError> {
    Ok(one::screen::page(&serde_json::from_str(context)?, one::screen::client_screen(name)))
}

/// The white label this node is: `{ name, cli, accent }`.
#[wasm_bindgen]
pub fn config() -> String {
    json!({ "name": one::config::NAME, "cli": one::config::CLI, "accent": one::config::ACCENT }).to_string()
}

/// Where an app's database of a scope is in the bucket (`src/db.rs`).
#[wasm_bindgen]
pub fn prefix(scope: &str, app: &str, database: &str) -> String {
    format!("{}/{app}/{database}", one::bucket::folder(scope))
}

/// The names of the `n` steps after `name`, if it's a database's step, as JSON: what's read next.
#[wasm_bindgen]
pub fn following(name: &str, n: u32) -> String {
    let Some((prefix, at)) = name.rsplit_once('/') else { return "[]".into() };
    let Some(at) = at.parse::<u64>().ok().filter(|n| one::db::step_name(prefix, *n) == name) else { return "[]".into() };
    Value::from((1..=n as u64).map(|i| one::db::step_name(prefix, at + i)).collect::<Vec<_>>()).to_string()
}

// Who can do what: each scope's entries (`src/log.rs`), and what they add up to (`src/members.rs`).

/// A scope's entries this isolate read: the key they were read with, the last step read, and the entries in replay
/// order.
struct Log {
    key: [u8; 32],
    at: u64,
    entries: Vec<Entry>,
}

/// What a scope's entries add up to, as of the steps they were read to: `global`'s members, and the scope's state.
struct Replayed {
    at: (u64, u64),
    members: Members,
    state: ScopeState,
}

thread_local! {
    /// Each scope's entries this isolate read, by scope. A request uses them only once it brings the same key, so
    /// they're only ever shown to someone who may open the scope.
    static LOGS: RefCell<HashMap<String, Log>> = RefCell::new(HashMap::new());
    /// Each scope's replay, by scope, while its steps and `global`'s are the same.
    static REPLAYED: RefCell<HashMap<String, Replayed>> = RefCell::new(HashMap::new());
}

/// Reads a scope's entries (with the key of its log, from the vault) up to its newest step: from the newest copy the
/// first time, and after that only the steps since. Requests reading at once each add what they read.
#[wasm_bindgen]
pub async fn read_log(bucket: JsValue, scope: String, key: String) -> Result<(), JsError> {
    let bucket = Js(bucket);
    let key = one::kernel::decode_key(&key).map_err(thrown)?;
    let prefix = one::log::prefix(&scope);
    let from = LOGS.with(|l| l.borrow().get(&scope).filter(|l| l.key == key).map(|l| l.at));
    let Some(from) = from else {
        let (at, mut entries) = one::log::open(&bucket, &key, &prefix, &scope).await.map_err(thrown)?;
        entries.sort_by_key(Entry::order);
        LOGS.with(|l| l.borrow_mut().insert(scope, Log { key, at, entries }));
        return Ok(());
    };
    let (at, new) = one::log::read(&bucket, &key, &prefix, &scope, from).await.map_err(thrown)?;
    LOGS.with(|l| {
        if let Some(log) = l.borrow_mut().get_mut(&scope).filter(|l| l.key == key) {
            let fresh: Vec<Entry> = new.into_iter().filter(|e| !log.entries.iter().any(|k| k.author() == e.author() && k.seq() == e.seq())).collect();
            if !fresh.is_empty() {
                log.entries.extend(fresh);
                log.entries.sort_by_key(Entry::order);
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

/// `global`'s members and each of `scopes`' state, from the entries this isolate read (`read_log`, this request).
fn replayed<R>(scopes: &[String], f: impl FnOnce(&Members, &BTreeMap<String, ScopeState>) -> R) -> Result<R> {
    LOGS.with(|logs| {
        let logs = logs.borrow();
        let global = logs.get("global").context("read global's entries first")?;
        let member_entries: Vec<Entry> = global.entries.iter().filter(|e| members::is_member_entry(e)).cloned().collect();
        let root = root(&global.entries);
        REPLAYED.with(|r| {
            let mut r = r.borrow_mut();
            let mut states = BTreeMap::new();
            let mut m = None;
            for scope in scopes {
                let Some(log) = logs.get(scope) else { continue };
                let at = (global.at, log.at);
                if r.get(scope).is_none_or(|x| x.at != at) {
                    let (members, state) = members::replay(root, &member_entries, &log.entries, |_, _, _| {});
                    r.insert(scope.clone(), Replayed { at, members, state });
                }
                let x = &r[scope];
                m.get_or_insert_with(|| x.members.clone());
                states.insert(scope.clone(), x.state.clone());
            }
            let m = m.unwrap_or_else(|| Members::replay(root, &member_entries));
            Ok(f(&m, &states))
        })
    })
}

/// The groups `user` is in, as scopes: whose entries to read next.
#[wasm_bindgen]
pub fn groups(user: &str) -> Result<String, JsError> {
    let groups = replayed(&["global".into()], |m, _| m.groups_of(user).into_keys().map(|g| format!("group:{g}")).collect::<Vec<_>>()).map_err(thrown)?;
    Ok(json!(groups).to_string())
}

/// The install as `user` sees it, from the entries of `scopes` (JSON) this isolate read: `{ state, apps, me }`.
/// `state` is `GET /v1/state`'s, for a node that knows of no server being on; `apps` is `GET /v1/apps`'s from the
/// members alone, each with `reach`, the scopes it reaches for `user` (what its widgets and calls are given), for
/// `worker.js` to add what each app says it does; `me` is what an app is told of who it runs for. `client` names the
/// sign-in's client.
#[wasm_bindgen]
pub fn install(user: &str, client: &str, scopes: &str, now: f64) -> Result<String, JsError> {
    let scopes: Vec<String> = serde_json::from_str(scopes)?;
    let out = replayed(&scopes, |m, states| view(m, states, user, client, now as u64)).map_err(thrown)?;
    Ok(out.to_string())
}

fn view(m: &Members, states: &BTreeMap<String, ScopeState>, me: &str, client: &str, now: u64) -> Value {
    let mine = states.get(&Scope::User(me.into()).to_string());
    // Requests for an item waiting for `me` to answer, as src/node.rs's `item_requests` counts them.
    let manages = |s: &String| Scope::parse(s).is_ok_and(|s| members::manages_scope(m, me, &s));
    let waiting = states.iter().filter(|(s, _)| manages(s)).flat_map(|(_, s)| s.requests.values()).filter(|r| r.answer.is_none() && now < r.time + 86_400_000).count();
    // It knows of no server being on, nor of who else is looking.
    let state = one::state::state(
        Some(m),
        me,
        one::state::Here {
            me: json!({ "key": "", "name": client, "user": me }),
            client: Some(json!({ "key": "", "name": client })),
            server: json!({ "key": "", "name": "", "ready": true, "guide": null }),
            member: false,
            entries: 0,
            waiting,
            arrangements: json!(mine.map(|s| s.arrangements.clone()).unwrap_or_default()),
            backups: None,
            guide: None,
            server_now: &|_| Default::default(),
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
    json!({ "state": state, "apps": apps, "me": me_ })
}

/// A deploy's bundle from the bucket (`bucket::blob_name`), opened with the key of the node's `blobs` database in
/// `global` and checked against its hash: → the app's module.
#[wasm_bindgen]
pub async fn bundle(bucket: JsValue, key: String, hash: String) -> Result<String, JsError> {
    let key = one::kernel::decode_key(&key).map_err(thrown)?;
    let name = one::bucket::blob_name("global", &hash);
    let sealed = Js(bucket).get(&name).await.map_err(thrown)?.ok_or_else(|| JsError::new(&format!("{name} isn't in the bucket yet: a node on one of your machines puts it there")))?;
    let bytes = one::db::open(&key, &name, &sealed.bytes).map_err(thrown)?;
    if blake3::hash(&bytes).to_hex().as_str() != hash {
        return Err(JsError::new(&format!("the bundle kept as {hash} isn't it")));
    }
    one::bundle::module(&bytes).map_err(thrown)
}
