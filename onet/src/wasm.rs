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
pub struct Client(std::rc::Rc<crate::Client>);

#[wasm_bindgen]
impl Client {
    /// `secret`: the 32 bytes of this session's key, kept in WebAssembly's memory while it runs (the device's own key
    /// stays in WebCrypto). `certificate`: the device's word for it (`certify` in key/crypto.js).
    /// `address`: the network's, where its relay is. `nodes`: the nodes to ask, hex, in the order things run on them.
    pub async fn start(secret: Vec<u8>, certificate: String, address: String, nodes: Vec<String>) -> Result<Client, JsError> {
        let secret: [u8; 32] = secret.try_into().map_err(|_| js("a session's key is 32 bytes"))?;
        let nodes = nodes.iter().map(|n| key(n)).collect::<Result<Vec<_>, _>>()?;
        Ok(Client(std::rc::Rc::new(crate::Client::start(secret, Some(certificate), &address, &nodes).await.map_err(js)?)))
    }

    pub async fn certify(&self, certificate: String) {
        self.0.certify(certificate).await
    }

    /// This session's endpoint's key, hex.
    pub fn id(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.0.id())
    }

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
                    Some(k) if !k.is_empty() => vec![(custom_build::protocol::header::KEYS.to_string(), k)],
                    _ => Vec::new(),
                }
            }
        };
        let response = self.0.fetch_with(crate::Request { method, path, headers, body: body.into() }, sealed).await.map_err(Failed::of)?;
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

/// This browser as a device of its user (onet/src/keyring.rs): who's who as it reads it itself, the keys it opens,
/// and its requests, which hand them, through its session's `Client`. `key` and `seal` are its keys, hex; `on` is what it
/// reaches through JavaScript (`crate::js::On`), and `shared(share)`, the X25519 secret its seal key, which WebCrypto
/// never lets out, agrees with `share`.
#[wasm_bindgen]
pub struct Keyring {
    keyring: crate::keyring::Keyring<crate::js::Seal>,
    me: key::Endpoint,
    on: crate::js::On,
    client: std::cell::RefCell<Option<std::rc::Rc<crate::Client>>>,
    answers: std::cell::RefCell<Option<std::rc::Rc<crate::answers::Answers<crate::js::Shelf>>>>,
}

/// The last answer of each call screens make that only reads, kept in this browser for its user (`answers`), through
/// `shelf` (`js::Shelf`): what's kept is read at once, before the session starts; what's kept comes from its device's
/// requests (`Keyring::keeping`).
#[wasm_bindgen]
pub struct Answers(std::rc::Rc<crate::answers::Answers<crate::js::Shelf>>);

#[wasm_bindgen]
impl Answers {
    #[wasm_bindgen(constructor)]
    pub fn new(shelf: JsValue) -> Answers {
        Answers(std::rc::Rc::new(crate::answers::Answers::new(crate::js::Shelf(crate::js::Js(shelf)))))
    }

    /// What `app`'s `tool` answered last for `input` (JSON), if it only reads: `{ answer, at }` as JSON, or "".
    pub async fn kept(&self, app: String, tool: String, input: String) -> String {
        match self.0.kept(&app, &tool, input.as_bytes()).await {
            Some((answer, at)) => format!(r#"{{"answer":{},"at":{at}}}"#, String::from_utf8_lossy(&answer)),
            None => String::new(),
        }
    }

    pub async fn forget(&self) {
        self.0.forget().await
    }
}

#[wasm_bindgen]
impl Keyring {
    #[wasm_bindgen(constructor)]
    pub fn new(key: String, seal: String, on: JsValue) -> Result<Keyring, JsError> {
        let opens = crate::js::Seal { public: self::key(&seal)?, js: crate::js::Js(on.clone()) };
        let me = key::Endpoint { key: self::key(&key)?, seal: self::key(&seal)? };
        Ok(Keyring { keyring: crate::keyring::Keyring::new(self::key(&key)?, opens), me, on: crate::js::On::of(on, self::key(&key)?), client: Default::default(), answers: Default::default() })
    }

    /// Asks auth to let this browser or app window in, as the user signed in with `token`, calling it `name`, saying
    /// it's built for `platform` (and with someone new's `invite` secret, its proof): `join::ask`.
    pub async fn ask(&self, token: String, name: String, platform: Option<String>, invite: Option<String>) -> Result<bool, Failed> {
        crate::join::ask(&self.on, &token, &crate::join::asking(&self.me, &name, None, platform.as_deref(), invite.as_deref())).await.map_err(Failed::of)
    }

    /// What auth says of its request now, acted on (`join::waited`): "signedOut", "approved", "gone" or "waiting".
    pub async fn waited(&self, token: String) -> Result<String, Failed> {
        use ::api::handed::Waited;
        let said = match crate::join::waited(&self.on, &token, &self.me.key).await.map_err(Failed::of)? {
            Waited::SignedOut => "signedOut",
            Waited::Approved { .. } => "approved",
            Waited::Gone => "gone",
            Waited::Waiting => "waiting",
        };
        Ok(said.into())
    }

    /// Keeps, in `answers`, what its requests' calls that only read answered.
    pub fn keeping(&self, answers: &Answers) {
        *self.answers.borrow_mut() = Some(answers.0.clone());
    }

    /// The session's client, which its requests go through.
    pub fn connect(&self, client: &Client) {
        *self.client.borrow_mut() = Some(client.0.clone());
    }

    /// Who's who as it read it when it last ran, if it kept it, with nothing asked of the network (the URL signer
    /// remembers a while that a device not let in yet isn't): → whether it knows who's who now.
    pub async fn restore(&self) -> bool {
        self.keyring.restore(&self.on).await
    }

    /// Who's who as it read it when it last ran, or from the bucket if it kept none: throws, with `notLetIn`, until it
    /// lists this device.
    pub async fn load(&self) -> Result<(), Failed> {
        self.keyring.load(&self.on).await.map_err(Failed::of)
    }

    /// Who's who, with what's new in the bucket.
    pub async fn read(&self) -> Result<(), Failed> {
        self.keyring.read(&self.on).await.map_err(Failed::of)
    }

    /// The nodes it may ask, hex (`nodes`), with each one's run priority (`ranks`, by key), and the Cloudflare node's
    /// key (`wakes`), if the network has one, as JSON.
    pub fn nodes(&self) -> String {
        let hex = |k: &[u8; 32]| data_encoding::HEXLOWER.encode(k);
        let ranks: serde_json::Map<String, serde_json::Value> = self.keyring.ranks().into_iter().map(|(k, r)| (hex(&k), r.into())).collect();
        let nodes: Vec<String> = self.keyring.nodes().keys().map(hex).collect();
        serde_json::json!({ "nodes": nodes, "ranks": ranks, "wakes": self.keyring.cloudflare_node().as_ref().map(hex) }).to_string()
    }

    /// A request to its user's nodes (`Keyring::request`), with the keys its kind needed, handed until `until` (ms),
    /// learning what a node asks for: as `Client::fetch`, with its trace. Before who's who lists
    /// this device, it's read again a few times while the relay lets it in, as its approval may not be there yet.
    pub async fn fetch(&self, method: String, path: String, headers: Vec<String>, body: Vec<u8>, until: f64) -> Result<Response, Failed> {
        use crate::keyring::Address;
        let mut behind = 0;
        loop {
            match self.keyring.load(&self.on).await {
                Err(e) if e.not_let_in && behind < ::api::handed::BEHIND.len() && self.on.let_in().await => {
                    n0_future::time::sleep(std::time::Duration::from_millis(::api::handed::BEHIND[behind])).await;
                    behind += 1;
                }
                loaded => break loaded.map_err(Failed::of)?,
            }
        }
        let headers = headers.chunks(2).map(|h| (h[0].clone(), h.get(1).cloned().unwrap_or_default())).collect();
        let client = self.client.borrow().clone().ok_or_else(|| Failed::of(crate::keyring::refused()))?;
        let request = crate::Request { method: method.clone(), path: path.clone(), headers, body: body.clone().into() };
        let sent = self.keyring.request(&client, &self.on, request, until as u64).await;
        let answers = self.answers.borrow().clone();
        let sent = match (answers, sent) {
            (Some(answers), Ok(response)) => answers.passing(&method, path.split('?').next().unwrap_or(&path), &body, response).await,
            (_, sent) => sent,
        };
        let response = sent.map_err(Failed::of)?;
        let headers = response.headers.into_iter().flat_map(|(n, v)| [n, v]).collect();
        let trace = traced(response.trace);
        Ok(Response { node: data_encoding::HEXLOWER.encode(&response.node), status: response.status, headers, trace, body: std::rc::Rc::new(std::cell::RefCell::new(Some(response.body))) })
    }

    /// As each start of a client does: its user's own things it settles if they aren't (`Keyring::settle_own`). A
    /// browser says `platform`; an app's window leaves it to the app around it, which says its own.
    pub async fn settle(&self, platform: Option<String>) -> Result<(), Failed> {
        self.keyring.load(&self.on).await.map_err(Failed::of)?;
        let client = self.client.borrow().clone().ok_or_else(|| Failed::of(crate::keyring::refused()))?;
        self.keyring.settle_own(&client, &self.on, None, platform.as_deref()).await.map_err(Failed::of)
    }

    /// Renews its user's app schedules of `app` as they use it (`Keyring::renew_app_schedules`).
    #[wasm_bindgen(js_name = renewAppSchedules)]
    pub async fn renew_app_schedules(&self, app: String) {
        let Some(client) = self.client.borrow().clone() else { return };
        self.keyring.renew_app_schedules(&client, &self.on, &app).await
    }

    /// Registers this browser's push subscription (`{ endpoint, keys }`, JSON) with the network's notifications Worker.
    #[wasm_bindgen(js_name = registerNotifications)]
    pub async fn register_notifications(&self, subscription: String) -> Result<(), Failed> {
        let sub: serde_json::Value = serde_json::from_str(&subscription).map_err(|e| Failed::of(crate::unsent(e)))?;
        let address = notifications::Address { via: "webpush".into(), address: sub["endpoint"].as_str().unwrap_or_default().into(), env: String::new(), topic: String::new(), keys: Some(serde_json::json!({ "p256dh": sub["keys"]["p256dh"], "auth": sub["keys"]["auth"] })) };
        self.keyring.load(&self.on).await.map_err(Failed::of)?;
        self.keyring.register_notifications(&self.on, &address).await.map_err(Failed::of)
    }

    /// Forgets all it knew and kept but the network's root, and its calls' answers, as a device that's out does.
    pub async fn forget(&self) {
        self.keyring.forget(&self.on).await;
        let answers = self.answers.borrow().clone();
        if let Some(answers) = answers {
            answers.forget().await;
        }
    }
}

fn traced(trace: Option<serde_json::Value>) -> String {
    trace.map(|t| t.to_string()).unwrap_or_default()
}

/// Why a request failed, and the nodes it tried, as JSON.
#[wasm_bindgen]
pub struct Failed {
    message: String,
    trace: String,
    not_let_in: bool,
    code: Option<String>,
}

impl Failed {
    fn of(e: crate::Error) -> Failed {
        Failed { message: e.to_string(), trace: traced(e.trace.clone()), not_let_in: e.not_let_in, code: e.code }
    }
}

#[wasm_bindgen]
impl Failed {
    /// The network doesn't list this device (yet, or any more).
    #[wasm_bindgen(getter, js_name = notLetIn)]
    pub fn not_let_in(&self) -> bool {
        self.not_let_in
    }

    /// What kind of refusal the node said it was (shared/protocol.js `CODES`), if it said.
    #[wasm_bindgen(getter)]
    pub fn code(&self) -> Option<String> {
        self.code.clone()
    }

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

    /// The nodes it tried and what the one that answered did, as JSON.
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

/// Signing in at the network's address as a browser does (`sign_in::Auth`), each request sent by `http.send(method,
/// url, type, body)` (a promise of `{ status, body }`, the body as text): the page keeps its registration and tokens,
/// and goes to auth and back itself. A refusal throws an Error with `signedOut` if auth refused its tokens.
#[wasm_bindgen]
pub struct SignIn {
    auth: crate::sign_in::Auth,
    http: crate::js::Js,
}

async fn sent(http: &crate::js::Js, r: crate::sign_in::Request) -> Result<(u16, serde_json::Value), crate::sign_in::Failed> {
    let (kind, body) = r.body.map_or((JsValue::NULL, JsValue::NULL), |(kind, body)| (kind.into(), body.into()));
    let got = http.ask("send", &[r.method.into(), r.url.into(), kind, body]).await.map_err(|e| crate::sign_in::failed(e.to_string()))?;
    let status = js_sys::Reflect::get(&got, &"status".into()).ok().and_then(|s| s.as_f64()).unwrap_or(0.0) as u16;
    let said = js_sys::Reflect::get(&got, &"body".into()).ok().and_then(|b| b.as_string()).and_then(|b| serde_json::from_str(&b).ok()).unwrap_or_default();
    Ok((status, said))
}

fn thrown(f: crate::sign_in::Failed) -> JsValue {
    let err = js_sys::Error::new(&f.message);
    let _ = js_sys::Reflect::set(&err, &"signedOut".into(), &f.signed_out.into());
    err.into()
}

fn tokens(t: crate::sign_in::Tokens) -> String {
    serde_json::json!({ "access": t.access, "refresh": t.refresh, "expiresIn": t.expires_in }).to_string()
}

#[wasm_bindgen]
impl SignIn {
    /// The auth of the network at `address`, as its metadata says.
    pub async fn at(address: String, http: JsValue) -> Result<SignIn, JsValue> {
        let http = crate::js::Js(http);
        let (status, meta) = sent(&http, crate::sign_in::metadata(&address)).await.unwrap_or((0, serde_json::Value::Null));
        Ok(SignIn { auth: crate::sign_in::Auth::of(&address, status, meta).map_err(thrown)?, http })
    }

    /// Registers this browser, called `name`, coming back to `back` → its client id.
    pub async fn register(&self, name: String, back: String) -> Result<String, JsValue> {
        let (status, out) = sent(&self.http, self.auth.registration(&name, &back, false).map_err(thrown)?).await.map_err(thrown)?;
        crate::sign_in::Auth::registered(status, &out).map_err(thrown)
    }

    /// An authorization for `client`, coming back to `back`, for the device `join` names → `{ url, asked }` as JSON,
    /// `asked` for `finish` once it's back.
    pub fn ask(&self, client: String, back: String, join: String) -> Result<String, JsValue> {
        let asked = self.auth.ask(&client, &back, &join);
        Ok(serde_json::json!({ "url": self.auth.url(&asked).map_err(thrown)?, "asked": asked.to_json() }).to_string())
    }

    /// What came back (`got`, its query's fields as JSON) to `asked` (`ask`'s), exchanged for tokens → `{ access,
    /// refresh, expiresIn }` as JSON.
    pub async fn finish(&self, asked: String, got: String) -> Result<String, JsValue> {
        let asked = serde_json::from_str(&asked).ok().and_then(|a| crate::sign_in::Asked::from_json(&a)).ok_or_else(|| thrown(crate::sign_in::failed("That sign-in didn’t start here. Sign in again.")))?;
        let got: std::collections::HashMap<String, String> = serde_json::from_str(&got).unwrap_or_default();
        let (status, out) = sent(&self.http, self.auth.finishing(&asked, &got).map_err(thrown)?).await.map_err(thrown)?;
        crate::sign_in::Auth::granted(status, &out).map(tokens).map_err(thrown)
    }

    /// `client`'s refresh token exchanged for new tokens, as `finish` gives them.
    pub async fn refresh(&self, client: String, refresh: String) -> Result<String, JsValue> {
        let (status, out) = sent(&self.http, self.auth.refresh_grant(&client, &refresh).map_err(thrown)?).await.map_err(thrown)?;
        crate::sign_in::Auth::granted(status, &out).map(tokens).map_err(thrown)
    }

    /// `client`'s refresh token revoked, so it opens nothing again.
    pub async fn revoke(&self, client: String, refresh: String) {
        if let Some(r) = self.auth.revocation(&client, &refresh) {
            let _ = sent(&self.http, r).await;
        }
    }
}

/// What's read next after `name` (`stored::following`), as JSON.
#[wasm_bindgen]
pub fn following(name: &str, n: u32) -> String {
    serde_json::Value::from(bucket::names::following(name, n)).to_string()
}
