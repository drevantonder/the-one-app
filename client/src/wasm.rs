//! The client in a browser, for the web client's one SharedWorker, which every tab and screen asks through: `start`
//! once, as an endpoint of the session's own key its device vouches for, then `fetch` as often and as at once as they
//! like, `certify` before the session's certificate ends, `reconnect` on the `online` event or a long freeze, and
//! `close`. Keys are hex, headers a flat list of names and values, and a body's parts come as `chunk` is awaited.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::js_sys;

fn js(err: impl std::fmt::Display) -> JsError {
    JsError::new(&err.to_string())
}

fn key(hex: &str) -> Result<[u8; 32], JsError> {
    data_encoding::HEXLOWER_PERMISSIVE.decode(hex.as_bytes()).ok().and_then(|k| k.try_into().ok()).ok_or_else(|| js(format!("{hex} isn't a key")))
}

#[wasm_bindgen]
pub struct Client(crate::Client);

#[wasm_bindgen]
impl Client {
    /// `secret`: the 32 bytes of this session's key, kept in WebAssembly's memory while it runs (the device's own key
    /// stays in WebCrypto). `certificate`: the device's word for it (`certify` in kernel/shared/crypto.js).
    /// `address`: the install's, where its relay is. `nodes`: the nodes to ask, hex, in the order things run on them.
    pub async fn start(secret: Vec<u8>, certificate: String, address: String, nodes: Vec<String>) -> Result<Client, JsError> {
        let secret: [u8; 32] = secret.try_into().map_err(|_| js("a session's key is 32 bytes"))?;
        let nodes = nodes.iter().map(|n| key(n)).collect::<Result<Vec<_>, _>>()?;
        Ok(Client(crate::Client::start(secret, Some(certificate), &address, &nodes).await.map_err(js)?))
    }

    /// A new certificate for the session, before the last ends.
    pub async fn certify(&self, certificate: String) {
        self.0.certify(certificate).await
    }

    /// This session's endpoint's key, hex.
    pub fn id(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.0.id())
    }

    /// The nodes to ask, as the install's state lists them.
    pub fn nodes(&self, nodes: Vec<String>) -> Result<(), JsError> {
        let nodes = nodes.iter().map(|n| key(n)).collect::<Result<Vec<_>, _>>()?;
        self.0.set_nodes(&nodes).map_err(js)
    }

    /// Each node's run priority, `keys` (hex) beside `ranks`: those on the same one are raced at once.
    pub fn ranks(&self, keys: Vec<String>, ranks: Vec<u32>) -> Result<(), JsError> {
        let keys = keys.iter().map(|n| key(n)).collect::<Result<Vec<_>, _>>()?;
        self.0.set_ranks(&keys.into_iter().zip(ranks).collect::<Vec<_>>()).map_err(js)
    }

    /// The nodes the relay says are on its network now, hex: raced first.
    pub fn online(&self, nodes: Vec<String>) -> Result<(), JsError> {
        let nodes = nodes.iter().map(|n| key(n)).collect::<Result<Vec<_>, _>>()?;
        self.0.set_online(&nodes).map_err(js)
    }

    /// `headers`: names and values, one after the other. `keys`, if given, is asked for the `x-one-keys` of the one node
    /// the request is sent to, once it's picked (its key, hex → a promise of the header's value, or none), so keys are
    /// sealed to no other. A request with `x-one-trace` has its trace in its response, or in what it failed with.
    pub async fn fetch(&self, method: String, path: String, headers: Vec<String>, body: Vec<u8>, keys: Option<js_sys::Function>) -> Result<Response, Failed> {
        let headers = headers.chunks(2).map(|h| (h[0].clone(), h.get(1).cloned().unwrap_or_default())).collect();
        let sealed = |node: &[u8; 32]| {
            let (keys, node) = (keys.clone(), data_encoding::HEXLOWER.encode(node));
            async move {
                let Some(asked) = keys.and_then(|k| k.call1(&JsValue::NULL, &JsValue::from_str(&node)).ok()) else { return Vec::new() };
                match wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&asked)).await.ok().and_then(|v| v.as_string()) {
                    Some(k) if !k.is_empty() => vec![("x-one-keys".to_string(), k)],
                    _ => Vec::new(),
                }
            }
        };
        let response = self.0.fetch_with(crate::Request { method, path, headers, body: body.into() }, sealed).await.map_err(|e| Failed { message: e.to_string(), trace: traced(e.trace) })?;
        let headers = response.headers.into_iter().flat_map(|(n, v)| [n, v]).collect();
        let trace = traced(response.trace);
        Ok(Response { node: data_encoding::HEXLOWER.encode(&response.node), status: response.status, headers, trace, body: std::rc::Rc::new(std::cell::RefCell::new(Some(response.body))) })
    }

    pub async fn reconnect(&self) {
        self.0.reconnect().await
    }

    pub async fn close(&self) {
        self.0.close().await
    }
}

fn traced(trace: Option<serde_json::Value>) -> String {
    trace.map(|t| t.to_string()).unwrap_or_default()
}

/// Why a request failed, and the nodes it tried, as JSON (empty if it asked for no trace).
#[wasm_bindgen]
pub struct Failed {
    message: String,
    trace: String,
}

#[wasm_bindgen]
impl Failed {
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn trace(&self) -> String {
        self.trace.clone()
    }
}

#[wasm_bindgen]
pub struct Response {
    node: String,
    status: u16,
    headers: Vec<String>,
    trace: String,
    /// Taken out while a part is awaited, as a method awaiting can't hold `&mut self` (wasm-bindgen would refuse a
    /// second call meanwhile).
    body: std::rc::Rc<std::cell::RefCell<Option<crate::Body>>>,
}

#[wasm_bindgen]
impl Response {
    /// The node that answered, hex, as its connection proved.
    #[wasm_bindgen(getter)]
    pub fn node(&self) -> String {
        self.node.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The nodes it tried and what the one that answered did, as JSON: empty if it asked for no trace.
    #[wasm_bindgen(getter)]
    pub fn trace(&self) -> String {
        self.trace.clone()
    }

    /// Names and values, one after the other, names lowercase.
    #[wasm_bindgen(getter)]
    pub fn headers(&self) -> Vec<String> {
        self.headers.clone()
    }

    /// The body's next part as it arrives, or undefined at its end.
    pub async fn chunk(&self) -> Result<Option<Vec<u8>>, JsError> {
        let mut body = self.body.borrow_mut().take().ok_or_else(|| js("a part is already being read"))?;
        let part = body.chunk().await;
        *self.body.borrow_mut() = Some(body);
        part.map_err(js)
    }

    /// Stops reading the body: the node stops sending it.
    pub fn cancel(&self) {
        self.body.borrow_mut().take();
    }
}

/// `global`'s entries this browser holds, checked, and the members they make (src/members.rs): from what it kept
/// (`kept`, a pack, of the install whose genesis is `root`, hex, or none the first time) and what a node just gave
/// (`given`, a pack). Throws if any entry doesn't check, or they're another install's.
#[wasm_bindgen]
pub struct Global {
    root: String,
    pack: Vec<u8>,
    have: String,
    members: String,
}

#[wasm_bindgen]
impl Global {
    #[wasm_bindgen(constructor)]
    pub fn new(root: String, kept: Vec<u8>, given: Vec<u8>) -> Result<Global, JsError> {
        let root = (!root.is_empty()).then(|| one::entry::Hash::from_hex(&root)).transpose().map_err(js)?;
        let mut g = crate::members::Global::new(root);
        if !kept.is_empty() {
            g.add(&kept).map_err(js)?;
        }
        if !given.is_empty() {
            g.add(&given).map_err(js)?;
        }
        let members = crate::members::view(&g.members().map_err(js)?).to_string();
        Ok(Global { root: g.root().map(|r| r.to_hex()).unwrap_or_default(), pack: g.pack(), have: g.have(), members })
    }

    /// The install's genesis, hex: to keep, and give again.
    #[wasm_bindgen(getter)]
    pub fn root(&self) -> String {
        self.root.clone()
    }

    /// Every entry held, packed: to keep.
    #[wasm_bindgen(getter)]
    pub fn pack(&self) -> Vec<u8> {
        self.pack.clone()
    }

    /// What to ask a node for next: `GET /v1/entries/global?have=`.
    #[wasm_bindgen(getter)]
    pub fn have(&self) -> String {
        self.have.clone()
    }

    /// The members, as JSON (`members::view`).
    #[wasm_bindgen(getter)]
    pub fn members(&self) -> String {
        self.members.clone()
    }
}
