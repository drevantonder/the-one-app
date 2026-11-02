//! The serverless node's Rust: the machine node's own code, as WebAssembly on Workers. `worker.js` answers requests,
//! asks the vault and runs apps; this checks access tokens, and reads and writes apps' databases in the bucket
//! (`src/db.rs`), through the presigned URLs `worker.js` gets for each call. Nothing outlives the call.

use std::cell::RefCell;
use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use js_sys::{Function, Promise, Reflect, Uint8Array};
use one::bucket::{Bucket, Object};
use one::call::Append;
use one::db::Db;
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

/// Writes a call's statements (JSON, `[[sql, params, check]]`) as one step.
#[wasm_bindgen]
pub async fn write(handle: u32, bucket: JsValue, statements: String) -> Result<(), JsError> {
    let list: Vec<(String, Vec<Value>, Option<String>)> = serde_json::from_str(&statements)?;
    let mut db = take(handle).map_err(thrown)?;
    let wrote = db.write(&Js(bucket), |c| one::db::statements(c, &list)).await;
    put(handle, db);
    wrote.map_err(thrown)
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
