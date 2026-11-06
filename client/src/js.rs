//! What a client device reaches through JavaScript, where it runs as WebAssembly (a browser, a Worker): the bucket,
//! where it keeps what outlives one run of it, and the relay, each a JS object's promises.

use anyhow::{Context, Result, anyhow};
use one::bucket::{Bucket, Failed, Object, Trouble};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::js_sys::{self, Function, Promise, Reflect, Uint8Array};

use crate::keyring::{Address, Keep};

/// The bucket as JavaScript reaches it: an object whose `get(name)`, `create(name, bytes)`, `replace(name, bytes,
/// tag)`, `objects(folder)` and `wait(ms)` give promises, of `{ bytes, tag }` or null, of whether it was written, and of
/// `[name, written]` for each object in the folder. One that throws with `unanswered` set wasn't reached.
pub struct Js(pub JsValue);

impl Js {
    pub async fn ask(&self, method: &str, args: &[JsValue]) -> Result<JsValue> {
        let f: Function = Reflect::get(&self.0, &method.into()).map_err(said)?.dyn_into().map_err(|_| anyhow!("{method} isn't there"))?;
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

    async fn objects(&self, folder: &str) -> Result<Vec<(String, u64)>> {
        let listed: js_sys::Array = self.ask("objects", &[folder.into()]).await?.dyn_into().map_err(|_| anyhow!("a listing is a list"))?;
        listed
            .iter()
            .map(|o| {
                let o: js_sys::Array = o.dyn_into().map_err(|_| anyhow!("a listed object is [name, written]"))?;
                Ok((o.get(0).as_string().context("a listed object has its name")?, o.get(1).as_f64().unwrap_or(0.0) as u64))
            })
            .collect()
    }
}

pub fn said(err: JsValue) -> anyhow::Error {
    anyhow!("{}", err.as_string().or_else(|| Reflect::get(&err, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{err:?}")))
}

/// What a device's object hands its keyring: `{ bucket, address, get(name), put(name, bytes), delete(name), letIn(),
/// ask(method, url, header, value, body), sign(bytes) }`: the bucket (`Js`); the network's address; what it keeps, by
/// name; whether the relay still lets it in; a JSON request to the address (→ `{ status, body }`, its JSON, or null);
/// and its key's signature of `bytes`. Each method a promise.
pub struct On {
    bucket: Js,
    address: String,
    key: [u8; 32],
    js: Js,
}

impl On {
    /// `key`: the device's own, which `sign` signs with.
    pub fn of(on: JsValue, key: [u8; 32]) -> On {
        let address = Reflect::get(&on, &"address".into()).ok().and_then(|a| a.as_string()).unwrap_or_default();
        On { bucket: Js(Reflect::get(&on, &"bucket".into()).unwrap_or(JsValue::NULL)), address, key, js: Js(on) }
    }
}

impl one::keys::Signs for On {
    fn public(&self) -> [u8; 32] {
        self.key
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> Result<[u8; 64]> {
        let got = self.js.ask("sign", &[Uint8Array::from(bytes).into()]).await?;
        Uint8Array::new(&got).to_vec().try_into().map_err(|_| anyhow!("a signature is 64 bytes"))
    }
}

impl Keep for On {
    async fn get(&self, name: &str) -> Option<Vec<u8>> {
        let got = self.js.ask("get", &[name.into()]).await.ok()?;
        (!got.is_null() && !got.is_undefined()).then(|| Uint8Array::new(&got).to_vec())
    }

    async fn put(&self, name: &str, bytes: &[u8]) -> Result<()> {
        self.js.ask("put", &[name.into(), Uint8Array::from(bytes).into()]).await.map(|_| ())
    }

    async fn delete(&self, name: &str) {
        let _ = self.js.ask("delete", &[name.into()]).await;
    }
}

impl Address for On {
    type Bucket = Js;

    fn bucket(&self) -> &Js {
        &self.bucket
    }

    /// Unless the relay says it doesn't, as a relay that didn't answer may still.
    async fn let_in(&self) -> bool {
        self.js.ask("letIn", &[]).await.ok().is_none_or(|v| v.is_truthy())
    }

    fn address(&self) -> &str {
        &self.address
    }

    async fn ask(&self, method: &str, url: &str, header: Option<(&str, String)>, body: Option<serde_json::Value>) -> Result<(u16, serde_json::Value), crate::Error> {
        let (name, value) = header.map(|(n, v)| (JsValue::from(n), JsValue::from(v))).unwrap_or((JsValue::NULL, JsValue::NULL));
        let body = body.map(|b| JsValue::from(b.to_string())).unwrap_or(JsValue::NULL);
        let got = self.js.ask("ask", &[method.into(), url.into(), name, value, body]).await.map_err(|e| crate::unsent(format!("Couldn't reach {}: {e}", one::config::YOURS)))?;
        let status = Reflect::get(&got, &"status".into()).ok().and_then(|s| s.as_f64()).unwrap_or(0.0) as u16;
        let said = Reflect::get(&got, &"body".into()).ok().and_then(|b| b.as_string()).and_then(|b| serde_json::from_str(&b).ok()).unwrap_or_default();
        Ok((status, said))
    }
}

/// A seal key whose secret JavaScript holds where it can't be read (a browser's WebCrypto key): `shared(share)` gives a
/// promise of the X25519 secret it agrees with `share`.
pub struct Seal {
    pub public: [u8; 32],
    pub js: Js,
}

impl one::keys::Opens for Seal {
    fn public(&self) -> [u8; 32] {
        self.public
    }

    async fn shared(&self, share: &[u8; 32]) -> Result<[u8; 32]> {
        let got = self.js.ask("shared", &[Uint8Array::from(&share[..]).into()]).await?;
        Uint8Array::new(&got).to_vec().try_into().map_err(|_| anyhow!("a shared secret is 32 bytes"))
    }
}
