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
    /// stays in WebCrypto). `certificate`: the device's word for it (`certify` in cloudflare/shared/crypto.js).
    /// `address`: the network's, where its relay is. `nodes`: the nodes to ask, hex, in the order things run on them.
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

    /// The nodes to ask, as the network's state lists them.
    pub fn nodes(&self, nodes: Vec<String>) -> Result<(), JsError> {
        let nodes = nodes.iter().map(|n| key(n)).collect::<Result<Vec<_>, _>>()?;
        self.0.set_nodes(&nodes).map_err(js)
    }

    /// Each node's run priority, `keys` (hex) beside `ranks`: those on the same one are raced at once.
    pub fn ranks(&self, keys: Vec<String>, ranks: Vec<u32>) -> Result<(), JsError> {
        let keys = keys.iter().map(|n| key(n)).collect::<Result<Vec<_>, _>>()?;
        self.0.set_ranks(&keys.into_iter().zip(ranks).collect::<Vec<_>>()).map_err(js)
    }

    /// The node that wakes when asked (the Cloudflare node), hex, if the network has one.
    pub fn wakes(&self, node: Option<String>) -> Result<(), JsError> {
        self.0.set_wakes(node.map(|n| key(&n)).transpose()?).map_err(js)
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

/// Who's who as this browser reads it (src/database/steps.rs, client/src/members.rs): from where it read `global`'s
/// scope database's steps to, as it kept that, of the network whose genesis is `root` (hex, or none the first time).
/// Throws if a step doesn't check, or they're another network's.
#[wasm_bindgen]
pub struct Global {
    root: String,
    steps: String,
    at: u64,
    members: String,
    wants_log: u64,
}

#[wasm_bindgen]
impl Global {
    #[wasm_bindgen(constructor)]
    /// `steps`, as JSON: `{ kept, given, epochs, apps }`, where its scope database's steps were read to as it kept that
    /// (`members::Global::kept`), and those this browser read since from the bucket itself (`{ steps, log, welcome,
    /// freeze, start }`, through the URL signer's `get`) with the keys to open them: `global`'s epochs sealed to this
    /// device, opened (hex, by number), and the node's key sealed in each (base64url, by number)
    /// (`members::Global::add_sealed`).
    pub fn new(root: String, steps: Option<String>) -> Result<Global, JsError> {
        let root = (!root.is_empty()).then(|| one::entry::Hash::from_hex(&root)).transpose().map_err(js)?;
        let mut g = crate::members::Global::new(root);
        let mut wants_log = 0;
        if let Some(steps) = steps.filter(|s| !s.is_empty()) {
            let steps: serde_json::Value = serde_json::from_str(&steps).map_err(js)?;
            if steps["kept"].is_object() {
                g.restart(&steps["kept"]).map_err(js)?;
            }
            let epochs = steps["epochs"].as_object().into_iter().flatten().filter_map(|(n, k)| Some((n.parse().ok()?, key(k.as_str()?).ok()?))).collect();
            let apps = steps["apps"].as_object().into_iter().flatten().filter_map(|(n, k)| Some((n.parse().ok()?, data_encoding::BASE64URL_NOPAD.decode(k.as_str()?.as_bytes()).ok()?))).collect();
            wants_log = g.add_sealed(&steps["given"], &epochs, &apps).map_err(js)?.unwrap_or(0);
        }
        let members = crate::members::view_of(&g.who().map_err(js)?).map_err(js)?.to_string();
        Ok(Global { root: g.root().map(|r| r.to_hex()).unwrap_or_default(), steps: g.kept().map_err(js)?.to_string(), at: g.at(), members, wants_log })
    }

    /// The step of `global`'s log this browser reads to and gives again (`given.log`) before it reads on: its genesis's,
    /// 1, which names the recovery key, while it doesn't know it; 0 for none.
    #[wasm_bindgen(getter)]
    pub fn wants_log(&self) -> f64 {
        self.wants_log as f64
    }

    /// Where its scope database's steps were read to, as JSON (`members::Global::kept`): to keep, and give again.
    #[wasm_bindgen(getter)]
    pub fn steps(&self) -> String {
        self.steps.clone()
    }

    /// The last of them: what to ask a node for those after (`GET /v1/steps/global?from=`).
    #[wasm_bindgen(getter)]
    pub fn at(&self) -> f64 {
        self.at as f64
    }

    /// The network's genesis, hex: to keep, and give again.
    #[wasm_bindgen(getter)]
    pub fn root(&self) -> String {
        self.root.clone()
    }

    /// Who's who, as JSON (`members::view_of`).
    #[wasm_bindgen(getter)]
    pub fn members(&self) -> String {
        self.members.clone()
    }
}

/// Where the issuer of a new device's sign-in (`proof`, as sign-in passed it on, JSON) publishes its keys, for the issuers
/// devices of the network at `address` take: "" for a passkey's, or any other issuer.
#[wasm_bindgen(js_name = issuerKeysAt)]
pub fn issuer_keys_at(proof: String, address: String) -> String {
    let proof: Option<one::sign_in_proof::Proof> = serde_json::from_str(&proof).ok();
    proof.and_then(|p| p.issuer()).and_then(|i| one::sign_in_proof::keys_at(&i, &address)).unwrap_or_default()
}

/// What a new device (`key`, `seal`, hex) signed in with, as its sign-in (`proof`, JSON) shows, checked here
/// (`one::sign_in_proof`): with its issuer's `keys` (JSON, or "" for a passkey's), for the network at `address` whose
/// first entry's hash is `genesis` (hex) and whose people sign in with `clients` (its `sign_in_clients` setting, as
/// its entries say, or ""); against `accounts` (its person's, as their devices record them, JSON), for one let in by
/// it, or with none, for one approved by its code. → the account, as JSON, to record with it; throws why it doesn't
/// check out.
#[wasm_bindgen(js_name = signedInWith)]
#[allow(clippy::too_many_arguments)]
pub fn signed_in_with(proof: String, key: String, seal: String, address: String, genesis: String, clients: String, keys: String, accounts: Option<String>, now: f64) -> Result<String, JsError> {
    let proof: one::sign_in_proof::Proof = serde_json::from_str(&proof).map_err(js)?;
    let device = one::keys::Endpoint { key: self::key(&key)?, seal: self::key(&seal)? };
    let network = one::sign_in_proof::Network::new(&address, self::key(&genesis)?, Some(&clients));
    let keys: Option<serde_json::Value> = (!keys.is_empty()).then(|| serde_json::from_str(&keys)).transpose().map_err(js)?;
    let now = now as u64;
    let account = match accounts {
        Some(theirs) => {
            let theirs: Vec<one::sign_in_proof::Account> = serde_json::from_str(&theirs).map_err(js)?;
            one::sign_in_proof::vouches(&proof, &device, &network, keys.as_ref(), &theirs, now)
        }
        None => one::sign_in_proof::check(&proof, &device, &network, keys.as_ref(), now),
    };
    Ok(serde_json::to_string(&account.map_err(js)?).map_err(js)?)
}
