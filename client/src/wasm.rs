//! The client in a browser, for the web client's one SharedWorker, which every tab and screen asks through: `start`
//! once, as an endpoint of the session's own key its device vouches for, then `fetch` as often and as at once as they
//! like, `certify` before the session's certificate ends, `reconnect` on the `online` event or a long freeze, and
//! `close`. Keys are hex, headers a flat list of names and values, and a body's parts come as `chunk` is awaited.

use wasm_bindgen::prelude::*;

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

    /// `headers`: names and values, one after the other.
    pub async fn fetch(&self, method: String, path: String, headers: Vec<String>, body: Vec<u8>) -> Result<Response, JsError> {
        let headers = headers.chunks(2).map(|h| (h[0].clone(), h.get(1).cloned().unwrap_or_default())).collect();
        let response = self.0.fetch(crate::Request { method, path, headers, body }).await.map_err(js)?;
        let headers = response.headers.into_iter().flat_map(|(n, v)| [n, v]).collect();
        Ok(Response { status: response.status, headers, body: std::rc::Rc::new(std::cell::RefCell::new(Some(response.body))) })
    }

    pub async fn reconnect(&self) {
        self.0.reconnect().await
    }

    pub async fn close(&self) {
        self.0.close().await
    }
}

#[wasm_bindgen]
pub struct Response {
    status: u16,
    headers: Vec<String>,
    /// Taken out while a part is awaited, as a method awaiting can't hold `&mut self` (wasm-bindgen would refuse a
    /// second call meanwhile).
    body: std::rc::Rc<std::cell::RefCell<Option<crate::Body>>>,
}

#[wasm_bindgen]
impl Response {
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> u16 {
        self.status
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
