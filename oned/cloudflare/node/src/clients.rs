//! The remote MCP server's devices (onet/worker/), each a client of the network as any app is (onet, over `<cli>/http/1`),
//! with no socket: one per remote MCP client, in its own Durable Object, which may share this isolate with others'.
//! What one sends goes to its object's `send(dst, segment size, contents)`, and the relay hands it what's sent to its
//! key, by RPC, which comes in at `device_deliver`. It lives while its object does, and holds its key only while it
//! runs.
//!
//! What it knows of who's who, the keys it opened and how it asks with them are onet's `Keyring`, as every client
//! device's: its object hands it the bucket, through the URL signer, its storage and the relay (`On`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use js_sys::{Function, Reflect, Uint8Array};
use onet::js::On;
use onet::keyring::Keyring;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

type Key = [u8; 32];

const ANSWER_MOST: usize = 8 << 20;
/// A device not asked for this long is taken to have ended with its object, which didn't close it: its key leaves.
const FORGOTTEN_AFTER: f64 = 10.0 * 60_000.0;

thread_local! {
    /// Each device running here, by its key: its client, where the relay's datagrams for it go, and when it was last
    /// used (ms).
    static CLIENTS: RefCell<HashMap<Key, (onet::Client, onet::relayed::Inbox, f64)>> = RefCell::new(HashMap::new());
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
    let gone: Vec<(Key, onet::Client)> = CLIENTS.with(|c| {
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

fn running(me: &Key) -> Option<onet::Client> {
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
        let to_relay: onet::relayed::Send = Arc::new(move |dst: &Key, segment: u16, contents: &[u8]| {
            SENDS.with(|s| {
                if let Some(send) = s.borrow().get(&me) {
                    let args = [Uint8Array::from(&dst[..]).into(), JsValue::from(segment), Uint8Array::from(contents).into()];
                    let _ = send.apply(&JsValue::NULL, &args.iter().collect());
                }
            })
        });
        let (client, inbox) = onet::Client::relayed(seed, to_relay, &nodes).await.map_err(thrown)?;
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

/// Asks the first of the device `me`'s nodes that answers, as its user, with the keys that kind of request needs
/// (`Keyring::request`), handed until `until` (ms), through `on` (`On`): `headers` names and values, one after the other.
/// → `{ node, status, headers, body }`, `node` the key (hex) of the node that answered, the body whole, at most
/// `ANSWER_MOST`. Throws with `sent: false` if it reached none, and `notLetIn` if who's who doesn't list it.
#[wasm_bindgen]
pub async fn device_fetch(me: String, method: String, path: String, headers: Vec<String>, body: Vec<u8>, until: f64, on: JsValue) -> Result<JsValue, JsValue> {
    let me = key(&me).map_err(JsValue::from)?;
    let client = running(&me).ok_or_else(|| failed(&onet::keyring::refused()))?;
    let keyring = KEYRINGS.with(|k| k.borrow().get(&me).cloned()).ok_or_else(|| failed(&onet::keyring::refused()))?;
    let on = On::of(on, me);
    if method == "POST" && path == "/mcp"
        && let Some(answer) = tool_here(&keyring, &client, &on, &body, until as u64).await
    {
        let answer = answer.map_err(|e| failed(&e))?;
        return Ok(answered([0; 32], 200, vec![("content-type".into(), "application/json".into())], &answer));
    }
    let headers = headers.chunks(2).map(|h| (h[0].clone(), h.get(1).cloned().unwrap_or_default())).collect();
    let request = onet::Request { method, path, headers, body: body.into() };
    let response = keyring.request(&client, &on, request, until as u64).await.map_err(|e| failed(&e))?;
    let body = response.body.bytes_within(ANSWER_MOST).await.map_err(|e| failed(&e))?;
    Ok(answered(response.node, response.status, response.headers, &body))
}

fn answered(node: Key, status: u16, headers: Vec<(String, String)>, body: &[u8]) -> JsValue {
    let out = js_sys::Object::new();
    let _ = Reflect::set(&out, &"node".into(), &data_encoding::HEXLOWER.encode(&node).into());
    let _ = Reflect::set(&out, &"status".into(), &status.into());
    let said: js_sys::Array = headers.into_iter().flat_map(|(n, v)| [n, v]).map(JsValue::from).collect();
    let _ = Reflect::set(&out, &"headers".into(), &said);
    let _ = Reflect::set(&out, &"body".into(), &Uint8Array::from(body));
    out.into()
}

/// The answer to an MCP client's call of a core tool that's one request the device answers itself (`api_features::core_tools`,
/// `scope::settings::answered`): one that changes who can do what, which it signs as its user's, as any device does.
/// None for any other MCP message, which goes to a node, as its user.
async fn tool_here(keyring: &Keyring, client: &onet::Client, on: &On, body: &[u8], until: u64) -> Option<Result<Vec<u8>, onet::Error>> {
    use api::Reader;
    let msg: Value = serde_json::from_slice(body).ok()?;
    if msg["method"] != "tools/call" {
        return None;
    }
    let name = msg["params"]["name"].as_str()?;
    let (feature, at) = api_features::core_tools::find(name)?;
    let mut input = msg["params"]["arguments"].as_object().cloned().unwrap_or_default();
    let confirmed = input.remove("confirm") == Some(json!(true));
    // Any other a node runs.
    let asks = Some(feature.request(at, &input, confirmed)).filter(|r| scope::settings::answered(&r.path))?;
    if feature.tools[at]["confirm"] == true && !confirmed {
        return Some(Ok(result(&msg, Err(Reader::Mcp.unconfirmed(name)))));
    }
    let request = onet::Request {
        method: "POST".into(),
        path: asks.path,
        headers: vec![("content-type".into(), "application/json".into())],
        body: asks.body.to_string().into_bytes().into(),
    };
    let response = match keyring.request(client, on, request, until).await {
        Ok(r) => r,
        Err(err) => return Some(Err(err)),
    };
    let status = response.status;
    let answer: Value = response.body.bytes().await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let said = match status {
        200 => Ok(api::feature::said(answer)),
        _ => Err(answer["error"].as_str().map_or_else(|| format!("That wasn't done ({status})."), str::to_string)),
    };
    Some(Ok(result(&msg, said)))
}

/// An MCP answer to `msg` of what a tool said, as a node's MCP answers it: text as it is, anything else as JSON.
fn result(msg: &Value, said: Result<Value, String>) -> Vec<u8> {
    let result = match said {
        Ok(Value::String(text)) => json!({ "content": [{ "type": "text", "text": text }] }),
        Ok(other) => json!({ "content": [{ "type": "text", "text": other.to_string() }] }),
        Err(text) => json!({ "isError": true, "content": [{ "type": "text", "text": text }] }),
    };
    json!({ "jsonrpc": "2.0", "id": msg["id"], "result": result }).to_string().into_bytes()
}

fn failed(err: &onet::Error) -> JsValue {
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
/// time, everything, after that what's new. → `{ user, nodes: [{ key, seal }] }`: its user as who's who lists it, and
/// the nodes it may ask, hex. Throws with `notLetIn` until who's who lists it as a client device, with its keys.
#[wasm_bindgen]
pub async fn device_view(me: String, seal: Vec<u8>, on: JsValue) -> Result<String, JsValue> {
    let me = key(&me).map_err(JsValue::from)?;
    let seal: Key = seal.try_into().map_err(|_| JsValue::from(thrown("a seal key's secret is 32 bytes")))?;
    let keyring = KEYRINGS.with(|k| k.borrow_mut().entry(me).or_insert_with(|| Rc::new(Keyring::new(me, seal))).clone());
    keyring.read(&On::of(on, me)).await.map_err(|e| failed(&e))?;
    let nodes: Vec<_> = keyring.nodes().iter().map(|(key, seal)| json!({ "key": key::hex(key), "seal": key::hex(seal) })).collect();
    Ok(json!({ "user": keyring.user(), "nodes": nodes }).to_string())
}
