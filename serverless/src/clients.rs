//! The MCP Worker's devices (kernel/mcp/), each a client of the install as any app is (one-client, over `<cli>/http/1`),
//! with no socket: one per remote MCP client, in its own Durable Object, which may share this isolate with others'.
//! What one sends goes to its object's `send(dst, segment size, contents)`, and the relay hands it what's sent to its
//! key, by RPC, which comes in at `device_deliver`. It lives while its object does, and holds its key only while it
//! runs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use js_sys::{Function, Uint8Array};
use wasm_bindgen::prelude::*;

type Key = [u8; 32];

thread_local! {
    /// Each device running here, by its key: its client and where the relay's datagrams for it go.
    static CLIENTS: RefCell<HashMap<Key, (one_client::Client, one_client::relayed::Inbox)>> = RefCell::new(HashMap::new());
    /// Each device's `send`, which can't cross into the transport (a JavaScript function is neither Send nor Sync).
    static SENDS: RefCell<HashMap<Key, Function>> = RefCell::new(HashMap::new());
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
    SENDS.with(|s| s.borrow_mut().insert(me, send));
    let running = CLIENTS.with(|c| c.borrow().get(&me).map(|(client, _)| client.clone()));
    if let Some(client) = running {
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
        let started = one_client::Client::relayed(seed, to_relay, &nodes).await.map_err(thrown)?;
        CLIENTS.with(|c| c.borrow_mut().insert(me, started));
    }
    Ok(data_encoding::HEXLOWER.encode(&me))
}

/// Datagrams the relay was given for the device `to` (bytes), from `src`: one, or `segment_size` long each.
#[wasm_bindgen]
pub fn device_deliver(to: &[u8], src: &[u8], segment_size: u32, contents: &[u8]) {
    let Ok(to) = Key::try_from(to) else { return };
    if let Some(inbox) = CLIENTS.with(|c| c.borrow().get(&to).map(|(_, inbox)| inbox.clone())) {
        inbox.deliver(src, segment_size, contents);
    }
}

/// The nodes the device `me` (hex) asks, in order.
#[wasm_bindgen]
pub fn device_nodes(me: &str, nodes: Vec<String>) -> Result<(), JsError> {
    let me = key(me)?;
    let client = CLIENTS.with(|c| c.borrow().get(&me).map(|(client, _)| client.clone())).ok_or_else(|| thrown("that device isn't running"))?;
    client.set_nodes(&keys(&nodes)?).map_err(thrown)
}

/// Asks the first of the device `me`'s nodes that answers: `headers` names and values, one after the other. → `{
/// status, headers, body }`, the body whole. Throws with `sent: false` if it reached none.
#[wasm_bindgen]
pub async fn device_fetch(me: String, method: String, path: String, headers: Vec<String>, body: Vec<u8>) -> Result<JsValue, JsValue> {
    let failed = |message: String, sent: bool| -> JsValue {
        let err = js_sys::Error::new(&message);
        let _ = js_sys::Reflect::set(&err, &"sent".into(), &sent.into());
        err.into()
    };
    let me = key(&me).map_err(JsValue::from)?;
    let client = CLIENTS.with(|c| c.borrow().get(&me).map(|(client, _)| client.clone())).ok_or_else(|| failed("that device isn't running".into(), false))?;
    let headers = headers.chunks(2).map(|h| (h[0].clone(), h.get(1).cloned().unwrap_or_default())).collect();
    let response = client.fetch(one_client::Request { method, path, headers, body }).await.map_err(|e| failed(e.to_string(), e.sent))?;
    let body = response.body.bytes().await.map_err(|e| failed(e.to_string(), true))?;
    let out = js_sys::Object::new();
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
    let Some((client, _)) = CLIENTS.with(|c| c.borrow_mut().remove(&me)) else { return };
    let _ = n0_future::time::timeout(std::time::Duration::from_secs(1), client.close()).await;
    SENDS.with(|s| s.borrow_mut().remove(&me));
}
