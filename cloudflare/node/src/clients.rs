//! The remote MCP server's devices (cloudflare/mcp/), each a client of the network as any app is (one-client, over `<cli>/http/1`),
//! with no socket: one per remote MCP client, in its own Durable Object, which may share this isolate with others'.
//! What one sends goes to its object's `send(dst, segment size, contents)`, and the relay hands it what's sent to its
//! key, by RPC, which comes in at `device_deliver`. It lives while its object does, and holds its key only while it
//! runs.
//!
//! What it knows of who's who, the keys it opened and how it asks with them are one-client's `Keyring`, as every client
//! device's: its object hands it the bucket, through the URL signer, its storage and the relay (`On`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use js_sys::{Function, Reflect, Uint8Array};
use one_client::keyring::{Address, Keep, Keyring};
use serde_json::json;
use wasm_bindgen::prelude::*;

type Key = [u8; 32];

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
    static KEYRINGS: RefCell<HashMap<Key, Rc<Keyring>>> = RefCell::new(HashMap::new());
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
    KEYRINGS.with(|k| k.borrow_mut().remove(key));
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

/// Asks the first of the device `me`'s nodes that answers, as its person, with the keys that kind of request needs
/// (`Keyring::request`), handed until `until` (ms), through `on` (`On`): `headers` names and values, one after the other.
/// → `{ node, status, headers, body }`, `node` the key (hex) of the node that answered, the body whole, at most
/// `ANSWER_MOST`. Throws with `sent: false` if it reached none, and `notLetIn` if who's who doesn't list it.
#[wasm_bindgen]
pub async fn device_fetch(me: String, method: String, path: String, headers: Vec<String>, body: Vec<u8>, until: f64, on: JsValue) -> Result<JsValue, JsValue> {
    let me = key(&me).map_err(JsValue::from)?;
    let client = running(&me).ok_or_else(|| failed(&one_client::keyring::refused()))?;
    let keyring = KEYRINGS.with(|k| k.borrow().get(&me).cloned()).ok_or_else(|| failed(&one_client::keyring::refused()))?;
    let headers = headers.chunks(2).map(|h| (h[0].clone(), h.get(1).cloned().unwrap_or_default())).collect();
    let request = one_client::Request { method, path, headers, body: body.into() };
    let response = keyring.request(&client, &On::of(on), request, until as u64, None).await.map_err(|e| failed(&e))?;
    let node = data_encoding::HEXLOWER.encode(&response.node);
    let body = response.body.bytes_within(ANSWER_MOST).await.map_err(|e| failed(&e))?;
    let out = js_sys::Object::new();
    let _ = Reflect::set(&out, &"node".into(), &node.into());
    let _ = Reflect::set(&out, &"status".into(), &response.status.into());
    let said: js_sys::Array = response.headers.into_iter().flat_map(|(n, v)| [n, v]).map(JsValue::from).collect();
    let _ = Reflect::set(&out, &"headers".into(), &said);
    let _ = Reflect::set(&out, &"body".into(), &Uint8Array::from(&body[..]));
    Ok(out.into())
}

fn failed(err: &one_client::Error) -> JsValue {
    let out = js_sys::Error::new(&err.to_string());
    let _ = Reflect::set(&out, &"sent".into(), &err.sent.into());
    let _ = Reflect::set(&out, &"notLetIn".into(), &err.not_let_in.into());
    out.into()
}

/// Ends the device `me` (hex): its connections close, telling the nodes, and its key leaves this isolate.
#[wasm_bindgen]
pub async fn device_close(me: String) {
    let Ok(me) = key(&me) else { return };
    KEYRINGS.with(|k| k.borrow_mut().remove(&me));
    let Some((client, _, _)) = CLIENTS.with(|c| c.borrow_mut().remove(&me)) else { return };
    let _ = n0_future::time::timeout(std::time::Duration::from_secs(1), client.close()).await;
    SENDS.with(|s| s.borrow_mut().remove(&me));
}

/// What the device `me` (hex), whose seal key's secret is `seal`, knows of who's who, read through `on` (`On`): the first
/// time, everything, after that what's new. → `{ user, nodes: [{ key, seal }] }`: its person as who's who lists it, and
/// the nodes it may ask, hex. Throws with `notLetIn` until who's who lists it as a client device, with its keys.
#[wasm_bindgen]
pub async fn device_view(me: String, seal: Vec<u8>, on: JsValue) -> Result<String, JsValue> {
    let me = key(&me).map_err(JsValue::from)?;
    let seal: Key = seal.try_into().map_err(|_| JsValue::from(thrown("a seal key's secret is 32 bytes")))?;
    let keyring = KEYRINGS.with(|k| k.borrow_mut().entry(me).or_insert_with(|| Rc::new(Keyring::new(me, seal))).clone());
    keyring.read(&On::of(on)).await.map_err(|e| failed(&e))?;
    let nodes: Vec<_> = keyring.nodes().iter().map(|(key, seal)| json!({ "key": one::keys::hex(key), "seal": one::keys::hex(seal) })).collect();
    Ok(json!({ "user": keyring.user(), "nodes": nodes }).to_string())
}

/// What a device's object hands its keyring: `{ bucket, get(name), put(name, bytes), delete(name), letIn() }`, the
/// bucket as `worker.js` reads it through the URL signer (`crate::Js`), what it keeps in its storage, and whether the
/// relay still lets it in, each a promise.
struct On {
    bucket: crate::Js,
    js: crate::Js,
}

impl On {
    fn of(on: JsValue) -> On {
        On { bucket: crate::Js(Reflect::get(&on, &"bucket".into()).unwrap_or(JsValue::NULL)), js: crate::Js(on) }
    }
}

impl Keep for On {
    async fn get(&self, name: &str) -> Option<Vec<u8>> {
        let got = self.js.ask("get", &[name.into()]).await.ok()?;
        (!got.is_null() && !got.is_undefined()).then(|| Uint8Array::new(&got).to_vec())
    }

    async fn put(&self, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
        self.js.ask("put", &[name.into(), Uint8Array::from(bytes).into()]).await.map(|_| ())
    }

    async fn delete(&self, name: &str) {
        let _ = self.js.ask("delete", &[name.into()]).await;
    }
}

impl Address for On {
    type Bucket = crate::Js;

    fn bucket(&self) -> &crate::Js {
        &self.bucket
    }

    /// Unless the relay says it doesn't, as a relay that didn't answer may still.
    async fn let_in(&self) -> bool {
        self.js.ask("letIn", &[]).await.ok().is_none_or(|v| v.is_truthy())
    }
}
