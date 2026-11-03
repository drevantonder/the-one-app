//! The client as the native apps call it (Swift, Kotlin, C#): the app keeps its device's key in the Keychain, the
//! keystore or the Credential Locker, and hands it here at start (`ThisDevice::new`); while it isn't let in yet, it shows
//! `code()` and asks `wait` every few seconds; then every request to its install's nodes is `fetch`, over iroh. Each
//! call waits for its answer, so apps call from a thread of their own, never their main one.

use std::sync::{Arc, LazyLock, Mutex};

uniffi::setup_scaffolding!();

static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("one-client").enable_all().build().expect("a runtime"));

fn run<T>(work: impl Future<Output = T>) -> T {
    RUNTIME.block_on(work)
}

/// Why something failed (`reason`, in words a person reads): `sent` if the request may have reached a node, so that sending it again might run it twice.
#[derive(Debug, uniffi::Error)]
pub enum OneError {
    Failed { reason: String, sent: bool },
    /// The install doesn't list this device, yet or any more: the app shows its own screen to approve it, or its sign-in.
    NotLetIn,
}

impl std::fmt::Display for OneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OneError::Failed { reason, .. } => f.write_str(reason),
            OneError::NotLetIn => f.write_str("This device hasn't been let in yet."),
        }
    }
}

impl std::error::Error for OneError {}

impl From<one_client::Error> for OneError {
    fn from(e: one_client::Error) -> Self {
        if e.not_let_in {
            return OneError::NotLetIn;
        }
        OneError::Failed { reason: e.to_string(), sent: e.sent }
    }
}

fn failed(message: &str) -> OneError {
    OneError::Failed { reason: message.into(), sent: false }
}

/// A new device's key, 32 random bytes: the app keeps it, and hands it to `ThisDevice::new` each start.
#[uniffi::export]
pub fn new_device_secret() -> Vec<u8> {
    one_client::new_secret().to_vec()
}

#[derive(Clone, uniffi::Record)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// This app as a device of its person: `ThisDevice`, as apps list the install's other devices as `Device`.
#[derive(uniffi::Object)]
pub struct ThisDevice(one_client::Device);

#[uniffi::export]
impl ThisDevice {
    /// The device whose key is `secret`, of the install at `address`, keeping what it learns in the folder `dir`.
    #[uniffi::constructor]
    pub fn new(secret: Vec<u8>, address: String, dir: String) -> Result<Arc<ThisDevice>, OneError> {
        let secret: [u8; 32] = secret.try_into().map_err(|_| failed("a device's key is 32 bytes"))?;
        Ok(Arc::new(ThisDevice(one_client::Device::new(secret, &address, &dir))))
    }

    /// Its key, hex.
    pub fn key(&self) -> String {
        self.0.key()
    }

    /// The code it shows to be approved: "ABCD EFGH".
    pub fn code(&self) -> String {
        self.0.code()
    }

    /// Asks to be let in, as the person signed in with `token` (an access token), calling it `name`; someone new
    /// brings their invite's secret.
    pub fn ask(&self, token: String, name: String, invite: Option<String>) -> Result<(), OneError> {
        Ok(run(self.0.ask(&token, &name, invite.as_deref()))?)
    }

    /// Whether it's been let in yet; asking again, if its request waited too long.
    pub fn wait(&self, token: String) -> Result<bool, OneError> {
        Ok(run(self.0.wait(&token))?)
    }

    /// Whether the install took this device out: it was in once, and isn't listed now.
    pub fn removed(&self) -> bool {
        self.0.removed()
    }

    /// Reaches its nodes, if it's been let in: false if it hasn't.
    pub fn connect(&self) -> Result<bool, OneError> {
        Ok(run(self.0.connect())?)
    }

    /// A request to its install's nodes' local API (`/v1/…`), as its person, with the keys it needs.
    pub fn fetch(&self, method: String, path: String, headers: Vec<Header>, body: Vec<u8>) -> Result<Arc<Response>, OneError> {
        let headers = headers.into_iter().map(|h| (h.name, h.value)).collect();
        let response = run(self.0.fetch(&method, &path, headers, body))?;
        let headers = response.headers.into_iter().map(|(name, value)| Header { name, value }).collect();
        Ok(Arc::new(Response { status: response.status, headers, body: Mutex::new(Some(response.body)) }))
    }

    /// Starts again with a fresh connection: when the network comes back.
    pub fn reconnect(&self) {
        run(self.0.reconnect())
    }

    /// Closes its connections, until it's asked again: when its person signs out. (`close` is each binding's own.)
    pub fn disconnect(&self) {
        run(self.0.close())
    }
}

/// A node's answer, its body read as it comes.
#[derive(uniffi::Object)]
pub struct Response {
    status: u16,
    headers: Vec<Header>,
    body: Mutex<Option<one_client::Body>>,
}

#[uniffi::export]
impl Response {
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Its headers, names lowercase.
    pub fn headers(&self) -> Vec<Header> {
        self.headers.clone()
    }

    /// The body's next part as it arrives, or none at its end.
    pub fn chunk(&self) -> Result<Option<Vec<u8>>, OneError> {
        let mut body = self.body.lock().unwrap();
        let Some(b) = body.as_mut() else { return Ok(None) };
        Ok(run(b.chunk())?)
    }

    /// The rest of the body, whole.
    pub fn bytes(&self) -> Result<Vec<u8>, OneError> {
        let Some(body) = self.body.lock().unwrap().take() else { return Ok(Vec::new()) };
        Ok(run(body.bytes())?)
    }

    /// Stops reading the body: the node stops sending it.
    pub fn cancel(&self) {
        self.body.lock().unwrap().take();
    }
}
