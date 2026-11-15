//! One Core in a client: what every client and every plugin's client part stands on. It holds the device's key,
//! reaches a server by its key and asks it, keeps what servers answered (`kept`), and answers what an app's page asks
//! through its bridge (`Subscription`). The clients are programs on it (`clients/`): the CLI and MCP, and the app, a window
//! on it. A program is built with the native code of the plugins it ships with, which fills in what a device does of
//! a tool's inputs (`resolve`).
//!
//! A client asks a server in MCP's JSON-RPC: one message on a stream of its own, and the answer back on that stream.
//! A server tells a client something the same way, on a stream the server opens. No connection is kept open for
//! that: a client that wants to hear, as the app does while it runs, says it's watching, and a server connects to it
//! when there's something to say.

mod resolve;
mod cache;

use std::io::{ErrorKind, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{Context, Result, anyhow, bail};
use data_encoding::BASE64URL_NOPAD;
use futures_util::StreamExt;
use futures_util::future::join_all;
use futures_util::stream::FuturesUnordered;
use iroh::endpoint::{Connection, QuicTransportConfig, RecvStream, RelayMode, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, NetReportConfig, RelayConfig, RelayUrl, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, watch};
use tokio::time::{sleep, timeout};

pub use resolve::{InputResolver, Input};
pub use cache::Subscription;
use cache::write;

/// The build this program is: the commit it was built from, as `build.rs` says.
pub const BUILD: &str = env!("ONE_BUILD");

/// The kind of machine this is, as builds are named, the same as a server names it (`OneCore.Builds`):
/// `linux-x86_64`, `macos-aarch64`.
pub fn target() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

/// What a device says of itself as it asks for its token, which the network keeps so a list of its devices says
/// which build each is: its build, its kind of machine and its name.
fn said() -> Value {
    json!({"build": BUILD, "target": target(), "name": name()})
}

/// What servers and clients speak.
pub const ALPN: &[u8] = b"one/mcp/1";
/// The most one message may carry: a plugin's files fit.
pub const LIMIT: usize = 64 << 20;
/// How long reaching a server may take: long enough for a phone to find the relay and be let in, and for a first
/// packet that's lost to be sent again.
pub const CONNECT: Duration = Duration::from_secs(3);
/// How long a server that's there takes, at most, to say so. Nothing is sent over a connection that's asked nothing,
/// so a server that has gone is found out by asking it, and this soon. A server taken for gone that wasn't is only
/// asked again over a new connection, or has its lease taken by a spare, and neither loses anything.
pub const THERE: Duration = Duration::from_secs(1);
/// How long a connection that's asked nothing stays open. To keep one open, something has to be said over it every
/// few seconds, and through a relay each of those wakes the relay. So a connection is let go once it's idle, and
/// made again when there's something to ask.
const IDLE: Duration = Duration::from_secs(2);
/// How long a connection lasts once nothing arrives over it: longer than `IDLE`, so that one that's let go says so,
/// and sooner than iroh would say anything to keep it. iroh sends what it has for a machine only where a
/// connection it still has with it last was, so the one a server that stopped left behind has to end before that
/// server is found where it starts again.
const SILENT: Duration = Duration::from_secs(3);

/// A network's address, and the token this machine shows there: what says it's one of the network's own.
#[derive(Clone, Deserialize)]
pub struct Network {
    pub address: String,
    pub token: String,
}

impl Network {
    /// The headers of an ask of `path` at the address, with `body`: `key`'s signature of it, and the token. The pieces
    /// there check both (`core/edge/ask.js`), and a server's Elixir signs the same bytes (`OneCore.Edge`).
    pub fn signed(&self, key: &SecretKey, path: &str, body: &str) -> Result<[(&'static str, String); 4]> {
        let at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().to_string();
        let signature = key.sign(format!("one/ask/1\n{path}\n{at}\n{body}").as_bytes());
        Ok([
            ("one-key", key.public().to_string()),
            ("one-at", at),
            ("one-signature", hex(&signature.to_bytes())),
            ("one-token", self.token.clone()),
        ])
    }

    /// What a run of the client on the device with the key `device` shows the relay here, which `attestation` is the
    /// device's word for: the device's token, the device's key and the attestation, joined by dots.
    fn shown(&self, device: &SecretKey, attestation: &str) -> Self {
        Self { address: self.address.clone(), token: format!("{}.{}.{attestation}", self.token, device.public()) }
    }

    /// The servers acting for `of`, a user's or a group's prefix, as the signer at the address says: the ones on their
    /// list and then the ones acting for the network, which act for everyone. Each is its key, reached through the
    /// relay there until a straighter path is found.
    async fn servers(&self, key: &SecretKey, of: &str) -> Result<Vec<EndpointAddr>> {
        let mut ask = web()?
            .get(format!("{}/signer/servers", self.address.trim_end_matches('/')))
            .timeout(CONNECT)
            .query(&[("of", of)]);
        for (name, value) in self.signed(key, "/signer/servers", "")? {
            ask = ask.header(name, value);
        }
        let answer = ask.send().await?;
        let (status, said): (_, Value) = (answer.status(), answer.json().await?);
        if !status.is_success() {
            bail!("{}", said["error"].as_str().unwrap_or("the signer refused"))
        }
        let relay: RelayUrl = self.address.parse().with_context(|| format!("{} isn't an address", self.address))?;
        let keys = said["servers"].as_array().context("the signer listed no servers")?;
        keys.iter()
            .map(|key| {
                let key: EndpointId = key.as_str().unwrap_or_default().parse().context("that isn't a server's key")?;
                Ok(EndpointAddr::new(key).with_relay_url(relay.clone()))
            })
            .collect()
    }
}

/// What a client keeps of its network, in the file `network` of its home: the network's address, which whatever
/// set the client up put there, and once the device is someone's, its token to the relay there, whose it is and
/// the build it was when it got that, which it said as it asked.
#[derive(Clone, Serialize, Deserialize)]
struct Told {
    address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    who: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<String>,
}

/// Why a client that knows its network isn't in it: its device is nobody's there yet. It has asked the network's
/// edge for a code to be let in by, as Better Auth's device authorization has it (`plugins/sign-in/edge`),
/// naming its key and signing what it asks with it, so the code is only ever for this key.
#[derive(Debug)]
pub struct DeviceAuthorization {
    /// Where its person lets it in: a page at the network's address, carrying the code.
    pub link: String,
    /// The code, as it's shown: WDJB-MJHT.
    pub code: String,
    address: String,
    key: SecretKey,
    /// What the device asks for its answer with, which only it was given.
    device_code: String,
    /// How long it waits between asks.
    interval: Duration,
}

impl std::fmt::Display for DeviceAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let how = "This device isn't in your network yet. Open this where you sign in, and check it shows";
        write!(f, "{how} {}:\n{}", self.code, self.link)
    }
}

impl std::error::Error for DeviceAuthorization {}

impl DeviceAuthorization {
    /// Asks the network's edge at `address` for a code to let the device with `key` in by, under this
    /// machine's name.
    async fn asked(address: &str, key: &SecretKey) -> Result<Self> {
        let ask = json!({"client_id": key.public().to_string(), "name": name()});
        let (given, said) = ask_edge(address, key, "/sign-in/auth/device/code", ask).await?;
        let (true, Some(link), Some(code), Some(device_code)) = (
            given,
            said["verification_uri_complete"].as_str(),
            said["user_code"].as_str(),
            said["device_code"].as_str(),
        ) else {
            bail!("{}", said["error_description"].as_str().unwrap_or("the network's edge gave this device no code"))
        };
        let halves: Vec<String> = code.as_bytes().chunks(4).map(|half| String::from_utf8_lossy(half).into()).collect();
        Ok(Self {
            link: link.to_owned(),
            code: halves.join("-"),
            address: address.to_owned(),
            key: key.clone(),
            device_code: device_code.to_owned(),
            interval: Duration::from_secs(said["interval"].as_u64().unwrap_or(5)),
        })
    }

    /// Waits for this device's person to answer, asking the network's edge as often as it allows. It's an
    /// error if they stopped it, or its code ended first.
    pub async fn answered(mut self) -> Result<()> {
        loop {
            sleep(self.interval).await;
            let ask = json!({
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                "device_code": self.device_code,
                "client_id": self.key.public().to_string(),
            });
            let (given, said) = ask_edge(&self.address, &self.key, "/sign-in/auth/device/token", ask).await?;
            match said["error"].as_str() {
                _ if given => return Ok(()),
                Some("authorization_pending") => {}
                Some("slow_down") => self.interval += Duration::from_secs(5),
                Some("access_denied") => bail!(NotApproved::Denied),
                Some("expired_token") => bail!(NotApproved::Expired),
                _ => bail!("{}", said["error_description"].as_str().unwrap_or("the network's edge didn't answer")),
            }
        }
    }
}

/// What this device is called, as its person named it and its system says to an app that asks without a
/// permission: a Mac's name in its settings ("Andre's MacBook Air"), a Linux machine's pretty hostname if it has one,
/// and another computer's name as a host; an Android phone's name in its settings or else its model, and an iPhone's
/// name from UIKit, which without Apple's entitlement for it is "iPhone" or "iPad". It names a device and a server
/// alike. A phone has no name of its own as a host: that's `localhost`.
#[cfg(target_os = "macos")]
pub fn name() -> String {
    let named = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output();
    match named {
        Ok(named) if named.status.success() => String::from_utf8_lossy(&named.stdout).trim().to_owned(),
        _ => gethostname::gethostname().to_string_lossy().into_owned(),
    }
}

#[cfg(target_os = "linux")]
pub fn name() -> String {
    // Where systemd keeps it, as `hostnamectl set-hostname --pretty` sets it.
    let info = fs::read_to_string("/etc/machine-info").unwrap_or_default();
    let pretty = info.lines().find_map(|line| line.strip_prefix("PRETTY_HOSTNAME="));
    match pretty.map(|pretty| pretty.trim_matches('"').trim()).filter(|pretty| !pretty.is_empty()) {
        Some(pretty) => pretty.to_owned(),
        None => gethostname::gethostname().to_string_lossy().into_owned(),
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android", target_os = "macos", target_os = "linux")))]
pub fn name() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

#[cfg(target_os = "ios")]
pub fn name() -> String {
    // UIKit answers on the main thread.
    dispatch2::run_on_main(|main| objc2_ui_kit::UIDevice::currentDevice(main).name().to_string())
}

#[cfg(target_os = "android")]
pub fn name() -> String {
    use jni::objects::{JObject, JString};
    let named = || -> jni::errors::Result<String> {
        let android = ndk_context::android_context();
        // Tauri's window on Android gives what it runs in to `ndk_context` as it starts.
        let vm = unsafe { jni::JavaVM::from_raw(android.vm().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(android.context().cast()) };
        let text = |env: &mut jni::JNIEnv, said: JObject| -> jni::errors::Result<String> {
            if said.is_null() {
                return Err(jni::errors::Error::NullPtr("nothing was said"));
            }
            Ok(env.get_string(&JString::from(said))?.into())
        };
        let set = (|| {
            let resolver =
                env.call_method(&activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let setting = env.new_string("device_name")?;
            let get = "(Landroid/content/ContentResolver;Ljava/lang/String;)Ljava/lang/String;";
            let said = env
                .call_static_method(
                    "android/provider/Settings$Global",
                    "getString",
                    get,
                    &[(&resolver).into(), (&setting).into()],
                )?
                .l()?;
            text(&mut env, said)
        })();
        if set.is_err() {
            env.exception_clear()?;
        }
        set.or_else(|_| {
            let model = env.get_static_field("android/os/Build", "MODEL", "Ljava/lang/String;")?.l()?;
            text(&mut env, model)
        })
    };
    named().unwrap_or_else(|_| "An Android phone".to_owned())
}

/// Why a device's person didn't let it in: they stopped it where they signed in, or its code ended first. The app
/// says it on the sign-in page it shows the next code on, told it as `why`, in these same words.
#[derive(Debug, Clone, Copy)]
pub enum NotApproved {
    Denied,
    Expired,
}

impl NotApproved {
    /// How the sign-in page is told it.
    pub fn why(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Expired => "expired",
        }
    }
}

impl std::fmt::Display for NotApproved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Denied => "It was denied where you signed in.",
            Self::Expired => "Its code expired before it was approved. Try again.",
        })
    }
}

impl std::error::Error for NotApproved {}

/// Asks the network's edge at `path`, with `body` signed by `key`. Gives whether it agreed, and what it said.
async fn ask_edge(address: &str, key: &SecretKey, path: &str, body: Value) -> Result<(bool, Value)> {
    let body = body.to_string();
    let network = Network { address: address.to_owned(), token: String::new() };
    let mut asked = web()?
        .post(format!("{}{path}", address.trim_end_matches('/')))
        .timeout(CONNECT * 3)
        .header("content-type", "application/json");
    for (name, value) in network.signed(key, path, &body)? {
        asked = asked.header(name, value);
    }
    let answer = asked.body(body).send().await?;
    Ok((answer.status().is_success(), answer.json().await?))
}

impl Told {
    /// How long the token this client has lasts before it's renewed, by when it says it ends: nothing if it has none.
    fn left(&self) -> Duration {
        self.ends().duration_since(SystemTime::now() + RENEW).unwrap_or_default()
    }

    /// When the token this client has ends, as it says: at once if it has none.
    fn ends(&self) -> SystemTime {
        let said = self.token.as_ref().and_then(|token| BASE64URL_NOPAD.decode(token.as_bytes()).ok());
        let until =
            said.and_then(|said| serde_json::from_slice::<Value>(&said).ok()).and_then(|said| said["until"].as_u64());
        UNIX_EPOCH + Duration::from_millis(until.unwrap_or_default())
    }

    /// Asks the network's edge for this device's token, signed with its key, and keeps it in `home` with whose
    /// device it is (`took`). A device that's nobody's there asks for a code to be let in by, and keeps nothing of
    /// whoever's it was: the next person on it sees none of it.
    async fn admit(&mut self, key: &SecretKey, home: &Path) -> Result<()> {
        let ask = json!({"jsonrpc": "2.0", "id": 1, "method": "one/token", "params": {"build": said()}});
        let (_, answer) = ask_edge(&self.address, key, "/mcp", ask).await?;
        if answer["error"]["code"] == NOBODY {
            if self.who.is_some() {
                (self.token, self.who) = (None, None);
                self.keep(home)?;
            }
            bail!(DeviceAuthorization::asked(&self.address, key).await?)
        }
        if answer["result"].is_null() {
            bail!("{}", answer["error"]["message"].as_str().unwrap_or("the network's edge gave this device no token"))
        }
        self.took(&answer["result"], home)
    }

    /// Keeps a token this device was given, `{token, who}`, in `home`. Once the device is someone else's, what it kept
    /// of whoever's it was goes.
    fn took(&mut self, given: &Value, home: &Path) -> Result<()> {
        let (Some(token), Some(who)) = (given["token"].as_str(), given["who"].as_str()) else {
            bail!("this device was given no token")
        };
        let someone_else = self.who.as_deref() != Some(who);
        (self.token, self.who, self.build) = (Some(token.to_owned()), Some(who.to_owned()), Some(BUILD.to_owned()));
        if someone_else { self.keep(home) } else { write(&home.join("network"), &serde_json::to_vec(self)?) }
    }

    /// Keeps this in `home`, and nothing it kept for whoever's the device was before.
    fn keep(&self, home: &Path) -> Result<()> {
        for cached in ["cache", "files"] {
            match fs::remove_dir_all(home.join(cached)) {
                Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
        }
        write(&home.join("network"), &serde_json::to_vec(self)?)
    }
}

/// What asks over the web: the signer at a network's address, and the bucket by a link. Its TLS is the one iroh
/// reaches the relay with.
pub fn web() -> Result<reqwest::Client> {
    let tls = iroh::tls::CaTlsConfig::default().client_config(iroh::tls::default_provider())?;
    Ok(reqwest::Client::builder().tls_backend_preconfigured(tls).connect_timeout(CONNECT).build()?)
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// An endpoint holding `key`. It reaches others, and is reached, by key: straight to the addresses it's told and the
/// ones it finds, and otherwise through the relay at its network's address, which is also the only thing it times.
/// With no other relay and no address lookup, nothing about a network goes to anyone outside it. A network on one
/// machine has no address, and so no relay. A server's NIF binds this same endpoint.
pub async fn endpoint(key: SecretKey, network: Option<&Network>) -> Result<Endpoint> {
    let mut probes = NetReportConfig::minimal();
    let relay = match network {
        Some(Network { address, token }) => {
            let address = address.parse().with_context(|| format!("{address} isn't a network's address"))?;
            // iroh takes the relay it timed as its own, so with none timed it would have none.
            probes.https_probes = true;
            RelayMode::Custom(RelayConfig::new(address, None).with_auth_token(token).into())
        }
        None => RelayMode::Disabled,
    };
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .transport_config(QuicTransportConfig::builder().max_idle_timeout(Some(SILENT.try_into()?)).build())
        .relay_mode(relay)
        .net_report_config(probes)
        .bind()
        .await
        .context("couldn't open a socket")
}

/// Where `endpoint` is reached, as another is told it: its key, the machine's addresses on its networks, and the
/// relay at its network's address, which it's told now, before the endpoint has connected there. Another on this
/// machine reaches it with no network too.
pub fn address(endpoint: &Endpoint, network: Option<&Network>) -> Result<EndpointAddr> {
    let here = endpoint.bound_sockets().into_iter().filter(SocketAddr::is_ipv4);
    let address = here.fold(endpoint.addr(), |address, socket| {
        address.with_ip_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, socket.port())))
    });
    match network {
        Some(Network { address: at, .. }) => {
            Ok(address.with_relay_url(at.parse().with_context(|| format!("{at} isn't an address"))?))
        }
        None => Ok(address),
    }
}

/// The key kept in `file`, which is made on first use. It's made whole under a name of its own and then given its
/// name only if nothing has it yet, so of two programs that start at once in a new home, as the app and a command
/// can, one makes it and the other reads it whole. `tempfile` gives it that name without a hard link where the
/// system has a rename that won't replace, which a phone needs: Android lets an app make no hard links.
pub fn key(file: &Path) -> Result<SecretKey> {
    let read = || -> Result<SecretKey> {
        let bytes = fs::read(file)?.try_into().map_err(|_| anyhow!("{} doesn't hold a key", file.display()))?;
        Ok(SecretKey::from_bytes(&bytes))
    };
    match read() {
        Err(error)
            if error.downcast_ref::<std::io::Error>().is_some_and(|error| error.kind() == ErrorKind::NotFound) => {}
        read => return read,
    }
    let folder = file.parent().context("a key is kept in a folder")?;
    fs::create_dir_all(folder)?;
    let key = SecretKey::generate();
    let made = || -> std::io::Result<()> {
        let mut making = tempfile::NamedTempFile::new_in(folder)?;
        making.write_all(&key.to_bytes())?;
        making.persist_noclobber(file).map(drop).map_err(|error| error.error)
    };
    match made() {
        Ok(()) => Ok(key),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => read(),
        Err(error) => Err(error).with_context(|| format!("couldn't make {}", file.display())),
    }
}

/// The folder a client on a computer is kept in: `ONE_HOME` if that's set, as a network made to build or test with
/// sets it, and otherwise where the system keeps an app's data, under a name of this client's own. A phone gives
/// its app a folder instead.
pub fn home() -> Result<PathBuf> {
    match env::var_os("ONE_HOME") {
        Some(home) => Ok(home.into()),
        None => Ok(dirs::data_dir().context("this system has no folder for an app's data: set ONE_HOME")?.join(NAME)),
    }
}

/// What the app is to the system it's on: the name of its folder, and of the app among a phone's. It's the build's
/// Tauri identifier (`build.rs`), so a workplace's own build keeps its own folder beside The One App's. The app
/// before this one is `one` and `The One App` there, and this is neither.
pub const NAME: &str = env!("ONE_IDENTIFIER");

/// The address of the network an app was built for, if it was: what `TAURI_ONE_ADDRESS` was as it was built. Tauri's
/// builds for phones give the Rust build only the variables whose names start with TAURI.
const BUILT_FOR: Option<&str> = option_env!("TAURI_ONE_ADDRESS");

/// Why a server didn't answer what it was asked.
#[derive(Debug, PartialEq)]
pub enum RequestError {
    /// It couldn't be reached, so it was asked nothing.
    Unreachable,
    /// It stopped answering after it was asked, so it may have done what was asked.
    Stopped,
}

/// An open connection to a server, and when the server last answered on it. A client asks a server over one, and a
/// server asks another the same way. `gone` is whether the server was found not to be there.
pub struct Reached {
    connection: Connection,
    heard: Arc<std::sync::Mutex<Instant>>,
    gone: AtomicBool,
}

impl Reached {
    /// Connects to the server at `address`, or gives nothing if it doesn't answer `within` that long.
    pub async fn connect(endpoint: &Endpoint, address: EndpointAddr, within: Duration) -> Option<Self> {
        let connection = timeout(within, endpoint.connect(address, ALPN)).await.ok()?.ok()?;
        let heard = Arc::new(std::sync::Mutex::new(Instant::now()));
        let (open, last) = (connection.clone(), heard.clone());
        tokio::spawn(async move {
            while open.close_reason().is_none() {
                let idle = last.lock().expect("no ask panics").elapsed();
                match IDLE.checked_sub(idle) {
                    Some(left) => sleep(left).await,
                    None => open.close(0u32.into(), b"idle"),
                }
            }
        });
        Some(Self { connection, heard, gone: AtomicBool::new(false) })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Whether the connection is still one to ask over: it hasn't closed, and its server hasn't been found gone.
    pub fn open(&self) -> bool {
        self.connection.close_reason().is_none()
    }

    /// Whether the server didn't say it was there when it was asked over this connection.
    pub fn gone(&self) -> bool {
        self.gone.load(Ordering::Relaxed)
    }

    /// Asks the server one message, and gives its answer. A server that hasn't answered anything lately is first
    /// asked whether it's there, and so is one that's taking its time over this.
    pub async fn ask(&self, message: &[u8]) -> Result<Vec<u8>, RequestError> {
        if !self.open() || (self.heard.lock().expect("no ask panics").elapsed() > THERE && !self.there().await) {
            return Err(RequestError::Unreachable);
        }
        let mut receive = self.send(message).await.ok_or(RequestError::Unreachable)?;
        let answer = receive.read_to_end(LIMIT);
        tokio::pin!(answer);
        loop {
            tokio::select! {
                answer = &mut answer => {
                    *self.heard.lock().expect("no ask panics") = Instant::now();
                    return answer.map_err(|_| RequestError::Stopped);
                }
                _ = sleep(THERE) => if !self.there().await {
                    return Err(RequestError::Stopped);
                }
            }
        }
    }

    /// Whether the server is there: an empty message, which a server answers at once with nothing. One that doesn't
    /// in time is gone, and so is this connection to it.
    async fn there(&self) -> bool {
        let asked = async { self.send(b"").await?.read_to_end(LIMIT).await.ok() };
        let there = timeout(THERE, asked).await.is_ok_and(|answer| answer.is_some());
        if there {
            *self.heard.lock().expect("no ask panics") = Instant::now();
        } else {
            self.gone.store(true, Ordering::Relaxed);
            self.connection.close(0u32.into(), b"gone");
        }
        there
    }

    async fn send(&self, message: &[u8]) -> Option<RecvStream> {
        let (mut send, receive) = self.connection.open_bi().await.ok()?;
        send.write_all(message).await.ok()?;
        send.finish().ok()?;
        Some(receive)
    }
}

/// The connection to a server, once one is open, and when trying to open one last failed.
#[derive(Default)]
struct Reach {
    open: Option<Arc<Reached>>,
    failed: Option<Instant>,
}

/// A client, kept in its home folder: its `key`, where its `network` is or its `servers` are, and what servers
/// answered (`kept`).
pub struct Client {
    home: PathBuf,
    key: SecretKey,
    /// Its endpoint, its network and whose device it is, which are made again with each new token (`renew`).
    link: std::sync::RwLock<Link>,
    reach: Mutex<Reach>,
    notifications: broadcast::Sender<Value>,
    /// What every request says of who it's from and for, where the environment says: `ONE_LINK` is a sharing link,
    /// whose holder asks, `ONE_AS` names a user to act as, which only a server of a network being built allows, and
    /// `ONE_FOR` names the group it's for (`groups/family`), or `network`.
    says: Vec<(&'static str, String)>,
    /// Whether this client is to hear what changes, so its servers are to tell it.
    watching: AtomicBool,
    /// Whether a server answered the last thing asked.
    connected: watch::Sender<bool>,
    /// What the native code of the plugins the program ships with fills in of a tool's inputs (`resolve`).
    resolvers: Vec<Arc<dyn InputResolver>>,
    /// The inputs this device fills in of each tool, as a server last listed them (`resolve`).
    resolvable: std::sync::Mutex<Option<Arc<resolve::Resolvable>>>,
    /// The calls that failed before a server answered them, each by its tool and why, until a server is told
    /// (`record`).
    failed: std::sync::Mutex<Vec<Value>>,
}

/// A client's endpoint, and its network: the network's address, if it has one, where the client asks which servers
/// act for its user, with the token it shows there. A network on one machine has none, and whatever starts it tells
/// the client where its servers are, in `servers`. `who` is whose device this is, once it's someone's: the user it
/// signed in as.
///
/// The endpoint is this run's, with a key of its own that's made with it and kept nowhere, and `attestation` is the
/// device's word for that key: its signature of `one/run/1`, a line, and the key. A device runs many clients at
/// once, the app, each agent's MCP and every command, and iroh reaches one endpoint by a key, as the relay does: two
/// with one key take each other's place there, and neither is answered. So each run is its own endpoint, and what it
/// shows the relay and says to a server names the device it runs for, with the attestation (`Network::shown`, `say`).
/// The signer at the address and a server check it (`core/edge/signer`, `OneCore.User`).
#[derive(Clone)]
struct Link {
    endpoint: Endpoint,
    network: Option<Network>,
    who: Option<String>,
    attestation: String,
}

impl Link {
    /// A new run's link for the device with the key `device`, on its `network`.
    async fn run(device: &SecretKey, network: Option<Network>, who: Option<String>) -> Result<Self> {
        let key = SecretKey::generate();
        let attestation = hex(&device.sign(format!("one/run/1\n{}", key.public()).as_bytes()).to_bytes());
        let shown = network.as_ref().map(|network| network.shown(device, &attestation));
        let endpoint = endpoint(key, shown.as_ref()).await?;
        Ok(Self { endpoint, network, who, attestation })
    }
}

impl Client {
    /// Opens the client kept in `home`, which fills in of a tool's inputs what One Core does and what `resolvers` do.
    pub async fn open(home: impl Into<PathBuf>, resolvers: Vec<Arc<dyn InputResolver>>) -> Result<Arc<Self>> {
        let home = home.into();
        let told = match fs::read(home.join("network")) {
            Ok(network) => {
                Some(serde_json::from_slice::<Told>(&network).context("`network` isn't a network's address")?)
            }
            // An app built for one network knows its address.
            Err(error) if error.kind() == ErrorKind::NotFound => {
                BUILT_FOR.map(|address| Told { address: address.to_owned(), token: None, who: None, build: None })
            }
            Err(error) => return Err(error).context("couldn't read `network`"),
        };
        // A device with no token yet asks for one at the network's address, and while it's nobody's, for a code to
        // be let in by.
        let key = key(&home.join("key"))?;
        // A device that's a new build since it last got its token gets its next one now, which says so.
        let said = told.as_ref().is_some_and(|told| told.build.as_deref() == Some(BUILD));
        let (network, who) = match told {
            Some(mut told) => {
                if told.token.is_none() {
                    match told.admit(&key, &home).await {
                        Err(error) if !error.is::<DeviceAuthorization>() => {
                            return Err(error)
                                .with_context(|| format!("Couldn't join the network at {}", told.address));
                        }
                        admitted => admitted?,
                    }
                }
                let token = told.token.context("this device has no token")?;
                (Some(Network { address: told.address, token }), told.who)
            }
            None => (None, None),
        };
        let link = Link::run(&key, network, who).await?;
        let says = [("as", "ONE_AS"), ("link", "ONE_LINK"), ("for", "ONE_FOR")];
        let says = says.into_iter().filter_map(|(says, variable)| Some((says, env::var(variable).ok()?))).collect();
        let notifications = broadcast::channel(16).0;
        accept(link.endpoint.clone(), notifications.clone());
        let renews = link.network.is_some();
        let link = std::sync::RwLock::new(link);
        let (watching, connected) = (AtomicBool::new(false), watch::Sender::new(true));
        let (resolvable, failed) = (std::sync::Mutex::default(), std::sync::Mutex::default());
        let reach = Mutex::default();
        let client = Arc::new(Self { home, key, link, reach, notifications, says, watching, connected, resolvers, resolvable, failed });
        // A token that's about to end is renewed before anything is asked with it, and so is one a new build hasn't
        // renewed yet, and the next while this runs.
        if renews {
            let left = match client.network().left() {
                left if !left.is_zero() && said => left,
                _ => match client.renew().await {
                    Err(error) if error.is::<DeviceAuthorization>() => return Err(error),
                    renewed => renewed.unwrap_or(AGAIN),
                },
            };
            tokio::spawn(renewing(Arc::downgrade(&client), left));
        }
        Ok(client)
    }

    /// What this client keeps of its network, as it is now.
    fn network(&self) -> Told {
        let Link { network, who, .. } = self.link();
        let network = network.expect("only a client of a network with an address has a token");
        Told { address: network.address, token: Some(network.token), who, build: Some(BUILD.to_owned()) }
    }

    /// Asks for this device's next token, and makes the run's endpoint again to show it, since iroh shows a relay the
    /// token an endpoint was made with. The new one has a key of its own too, as the old one is still there. A server
    /// gives a token to a device that's someone's. One that can't be reached, as where the token this has ended, is
    /// asked at the network's address, which says where its person signs in for a device that's nobody's now. The
    /// old endpoint goes once its token ends, so what was asked over it finishes, and what's asked from now on is
    /// asked from the new one, whose run the device now attests to. Gives how long the new token lasts before it's
    /// renewed.
    async fn renew(&self) -> Result<Duration> {
        let mut told = self.network();
        // No server takes a token that has ended, so that's asked at the address straight away.
        let given = match told.ends() > SystemTime::now() {
            true => self.request("one/token", json!({"build": said()})).await.ok(),
            false => None,
        };
        match given {
            Some(given) => told.took(&given, &self.home)?,
            None => told.admit(&self.key, &self.home).await?,
        }
        let token = told.token.clone().context("this device was given no token")?;
        let network = Network { address: told.address.clone(), token };
        let new = Link::run(&self.key, Some(network), told.who.clone()).await?;
        accept(new.endpoint.clone(), self.notifications.clone());
        let old = std::mem::replace(&mut *self.link.write().expect("no renewal panics"), new);
        self.reach.lock().await.open = None;
        tokio::spawn(async move {
            sleep(RENEW).await;
            old.endpoint.close().await
        });
        Ok(told.left())
    }

    /// Whose device this is, once it's someone's: the user it signed in as.
    pub fn who(&self) -> Option<String> {
        self.link().who
    }

    /// This client's key, as others know it.
    pub fn key(&self) -> String {
        self.key.public().to_string()
    }

    /// This device's own key, for the native code of the plugins the program ships with: what's encrypted to the
    /// device opens with it. No app's page and no server is handed it.
    pub fn device_key(&self) -> &SecretKey {
        &self.key
    }

    /// The network's address, if it has one.
    pub fn address(&self) -> Option<String> {
        self.link().network.map(|network| network.address)
    }

    fn link(&self) -> Link {
        self.link.read().expect("no renewal panics").clone()
    }

    /// Whether a server answered the last thing this client asked, as that changes. It's found out by asking:
    /// nothing is said over a connection to learn it.
    pub fn connected(&self) -> watch::Receiver<bool> {
        self.connected.subscribe()
    }

    /// Says whether this client is to hear what changes, as the app does while it runs. While it is, every server
    /// acting for its user tells it when something changed. A server that isn't up now is told when it's next asked
    /// anything, and one that isn't told the app has gone finds out when it next has something to say.
    pub async fn watch(&self, watching: bool) {
        if self.watching.swap(watching, Ordering::Relaxed) == watching {
            return;
        }
        let Ok(Some(servers)) = self.servers(false).await else { return };
        let told = servers.into_iter().map(|server| async move {
            if let Some(open) = self.connect(server).await
                && !watching
            {
                let _ = open.ask(json!({"jsonrpc": "2.0", "method": WATCH, "params": {}}).to_string().as_bytes()).await;
            }
        });
        join_all(told).await;
    }

    /// Connects to `server`, hears what it tells this client over the connection, and if this client is watching,
    /// says so first, with where a server reaches it.
    async fn connect(&self, server: EndpointAddr) -> Option<Reached> {
        let link = self.link();
        let open = Reached::connect(&link.endpoint, server, CONNECT).await?;
        tokio::spawn(hear(open.connection().clone(), self.notifications.clone()));
        if self.watching.load(Ordering::Relaxed) {
            let here = address(&link.endpoint, link.network.as_ref()).ok()?;
            let watch = json!({"jsonrpc": "2.0", "method": WATCH, "params": {"address": here}});
            open.ask(watch.to_string().as_bytes()).await.ok()?;
        }
        Some(open)
    }

    /// What servers tell this client: each a JSON-RPC notification.
    pub fn notifications(&self) -> broadcast::Receiver<Value> {
        self.notifications.subscribe()
    }

    /// Asks a server one JSON-RPC message and gives its answer, which for a notification is nothing. A server that
    /// has gone was asked nothing, so whichever answers now is asked instead.
    pub async fn ask(&self, message: &[u8]) -> Result<Vec<u8>> {
        // A request is read once on its way out: for what this device fills in of it, and what it says of itself.
        let (message, lists, tool): (std::borrow::Cow<[u8]>, bool, Option<String>) =
            match serde_json::from_slice::<Value>(message) {
                Ok(mut request) if request["method"].is_string() => {
                    self.resolve(&mut request).await?;
                    self.say(&mut request);
                    let calls = request["method"] == "tools/call";
                    let tool = calls.then(|| request["params"]["name"].as_str().map(str::to_owned)).flatten();
                    (request.to_string().into_bytes().into(), request["method"] == "tools/list", tool)
                }
                _ => (message.into(), false, None),
            };
        let asked = async {
            for _ in 0..2 {
                match self.reached().await?.ask(&message).await {
                    Ok(answer) => return Ok(answer),
                    Err(RequestError::Unreachable) => continue,
                    Err(RequestError::Stopped) => bail!(
                        "The server stopped answering partway, so this may or may not have been done. Look before \
                         trying again."
                    ),
                }
            }
            bail!(UNANSWERED)
        };
        let answer = asked.await;
        if let (Err(why), Some(tool)) = (&answer, tool) {
            let mut failed = self.failed.lock().expect("no ask panics");
            if failed.len() < FAILED_KEPT {
                failed.push(json!({"tool": tool, "why": why.to_string()}));
            }
        }
        self.connected.send_if_modified(|answering| std::mem::replace(answering, answer.is_ok()) != answer.is_ok());
        match answer {
            Ok(answer) => {
                // What a device fills in of each tool's inputs is what the latest list of them says.
                if lists
                    && let Ok(listed) = serde_json::from_slice::<Value>(&answer)
                    && listed["result"]["tools"].is_array()
                {
                    let fills = resolve::resolvable(&listed["result"], &self.formats());
                    *self.resolvable.lock().expect("no ask panics") = Some(Arc::new(fills));
                }
                Ok(self.taken_out(answer).await)
            }
            refused => refused,
        }
    }

    /// What a server answered, unless it says this device is nobody's: one that was taken out of its network since
    /// it got its token. Then the answer says where its person lets it in again, as the network's edge says it.
    async fn taken_out(&self, answer: Vec<u8>) -> Vec<u8> {
        let (Some(network), Ok(mut said)) = (self.link().network, serde_json::from_slice::<Value>(&answer)) else {
            return answer;
        };
        if said["error"]["code"] != NOBODY {
            return answer;
        }
        let mut told = Told { address: network.address, token: None, who: self.link().who, build: None };
        let asked = told.admit(&self.key, &self.home).await;
        match asked.as_ref().map_err(|error| error.downcast_ref::<DeviceAuthorization>()) {
            Err(Some(not_in)) => said["error"]["message"] = json!(not_in.to_string()),
            _ => return answer,
        }
        said.to_string().into_bytes()
    }

    /// Has `request` say who it's from, in its `_meta`: the device this run is of, with its attestation for the run's key,
    /// and what the environment says, unless the request says otherwise, as one of a device's own does.
    fn say(&self, request: &mut Value) {
        for (says, value) in [("device", self.key.public().to_string()), ("attestation", self.link().attestation)] {
            request["params"]["_meta"][says] = json!(value);
        }
        for (says, value) in &self.says {
            if request["params"]["_meta"][says].is_null() {
                request["params"]["_meta"][says] = json!(value);
            }
        }
    }

    /// Tells a server how long a view took to open, from its tap to its first drawing, and the calls that failed
    /// before a server answered them since one was last told, for the plugins that hear `measured`. Says nothing of
    /// what was asked.
    pub async fn record(&self, view: &str, ms: u32) {
        let failed = std::mem::take(&mut *self.failed.lock().expect("no ask panics"));
        let said = json!({"jsonrpc": "2.0", "method": "notifications/one/measured", "params": {
            "client": std::env::consts::OS, "opened": [{"view": view, "ms": ms}], "failed": failed.clone()}});
        if self.ask(said.to_string().as_bytes()).await.is_err() {
            self.failed.lock().expect("no ask panics").extend(failed);
        }
    }

    /// Asks the server for `method`, and gives its result.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let answer = self.ask(message.to_string().as_bytes()).await?;
        let mut answer: Value = serde_json::from_slice(&answer).context("the server's answer isn't JSON")?;
        if let Some(error) = answer.get("error") {
            bail!("{}", error["message"].as_str().unwrap_or("the server refused"))
        }
        let mut result = answer["result"].take();
        self.keep_files(&mut result).await?;
        Ok(result)
    }

    /// Ends the connection, if one was made, and tells the server. The goodbye leaves this machine at once. What
    /// else closing an endpoint waits for is a server that was tried and never answered, which QUIC gives three
    /// seconds, so that isn't waited for.
    pub async fn close(&self) {
        if let Some(open) = self.reach.lock().await.open.take() {
            open.connection().close(0u32.into(), b"done");
            let _ = timeout(Duration::from_millis(100), self.link().endpoint.close()).await;
        }
    }

    /// The connection to a server, made when there's none open: to whichever of the servers acting for this client's
    /// user answers first. Which they are is what the client kept from when it last asked, and what it's told when
    /// it asks now if none of those answers.
    async fn reached(&self) -> Result<Arc<Reached>> {
        let asked = Instant::now();
        let mut reach = self.reach.lock().await;
        if let Some(open) = reach.open.as_ref().filter(|open| open.open()) {
            return Ok(open.clone());
        }
        reach.open = None;
        // Many asks at once share one try: no server answered while this one waited its turn.
        if reach.failed.is_some_and(|failed| failed > asked) {
            bail!(UNANSWERED)
        }
        let mut tried = None;
        for now in [false, true] {
            let Some(servers) = self.servers(now).await? else { continue };
            if tried.as_ref() == Some(&servers) {
                continue;
            }
            let mut connecting =
                FuturesUnordered::from_iter(servers.iter().cloned().map(|server| self.connect(server)));
            while let Some(connected) = connecting.next().await {
                if let Some(open) = connected {
                    return Ok(reach.open.insert(Arc::new(open)).clone());
                }
            }
            tried = Some(servers);
        }
        reach.failed = Some(Instant::now());
        bail!(UNANSWERED)
    }

    /// The servers acting for this client's user: the ones it kept, or with `now` the ones it's told when it asks.
    /// Nothing if it kept none, or has nobody to ask.
    async fn servers(&self, now: bool) -> Result<Option<Vec<EndpointAddr>>> {
        let Some(network) = self.link().network else {
            if now {
                return Ok(None);
            }
            let file = self.home.join("servers");
            let servers = fs::read(&file)
                .with_context(|| format!("{} doesn't say which servers to ask", file.display()))
                .context("This device hasn't joined a network")?;
            let servers = serde_json::from_slice(&servers)
                .with_context(|| format!("{} isn't servers' addresses", file.display()));
            return servers.map(Some);
        };
        // This client's user is whoever its device signed in as, or in a network being built, who it says it acts
        // as. A sharing link's holder asks the servers of whoever the link leads to, which it starts with.
        let said = |what| self.says.iter().find(|(says, _)| *says == what).map(|(_, said)| said.as_str());
        let of = match (said("link").map(|link| link.split('/').collect::<Vec<_>>()), said("as"), &self.link().who) {
            (Some(leads), ..) if leads[0] == "network" => "network/".to_owned(),
            (Some(leads), ..) if leads.len() > 2 => format!("{}/{}/", leads[0], leads[1]),
            (_, Some(acts_as), _) => format!("users/{acts_as}/"),
            (_, _, Some(who)) => format!("{who}/"),
            _ => bail!("This device is nobody's here. In a network being built, ONE_AS says who it acts as."),
        };
        let asked = (SERVERS, json!({"of": of}));
        if !now {
            return Ok(self.cached(asked.0, &asked.1).and_then(|kept| serde_json::from_value(kept).ok()));
        }
        let servers = network
            .servers(&self.key, &of)
            .await
            .with_context(|| format!("Couldn't ask {} which servers to reach", network.address))?;
        self.keep(asked.0, &asked.1, &serde_json::to_value(&servers)?)?;
        Ok(Some(servers))
    }
}

/// How long before a device's token ends it asks for the next one: half of what a server gives one
/// (`OneCore.Edge`), so a device that's taken out of its network uses its relay for an hour at most.
const RENEW: Duration = Duration::from_secs(30 * 60);

/// How soon a token that couldn't be asked for is asked for again: the device may be offline.
const AGAIN: Duration = Duration::from_secs(60);

/// The most failed calls kept until a server is told of them.
const FAILED_KEPT: usize = 50;

/// What a person is told when no server answered: what went wrong, and what to do about it.
const UNANSWERED: &str = "No server is answering. Check this device is online and a server is on, then try again.";

/// The JSON-RPC error a server answers a device that's nobody's (`OneCore.User`).
const NOBODY: i64 = -32003;

/// What a client tells a server to be told what changes, at the `address` it gives, or with none, no longer.
const WATCH: &str = "notifications/one/watch";

/// What the servers the signer listed are kept as.
const SERVERS: &str = "signer/servers";

/// A server with something to say connects to this client to say it, at `endpoint`, until it closes. Only one of the
/// network's own reaches a client, and all it can make one do is ask again.
fn accept(endpoint: Endpoint, tell: broadcast::Sender<Value>) {
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let tell = tell.clone();
            tokio::spawn(async move {
                if let Ok(connection) = incoming.await {
                    hear(connection, tell).await
                }
            });
        }
    });
}

/// Keeps a device's token fresh while its client runs: once `left` has passed, the next is asked for (`renew`). One
/// that can't be asked for, as on a device that's offline, is asked for again a minute later, and a device that's
/// nobody's now has none to renew.
async fn renewing(client: std::sync::Weak<Client>, mut left: Duration) {
    loop {
        sleep(left).await;
        let Some(client) = client.upgrade() else { return };
        left = match client.renew().await {
            Ok(left) => left,
            Err(error) if error.is::<DeviceAuthorization>() => return,
            Err(_) => AGAIN,
        };
    }
}

/// Tells whoever `connection` is to one message, on a stream of its own, which isn't answered. Gives whether it
/// was sent.
pub async fn tell(connection: &Connection, message: &[u8]) -> bool {
    let told = async {
        let mut send = connection.open_uni().await.ok()?;
        send.write_all(message).await.ok()?;
        send.finish().ok()
    };
    told.await.is_some()
}

/// Passes on what a server tells this client over `connection`, until the connection ends.
async fn hear(connection: Connection, told: broadcast::Sender<Value>) {
    while let Ok(mut receive) = connection.accept_uni().await {
        let told = told.clone();
        tokio::spawn(async move {
            let Ok(message) = receive.read_to_end(LIMIT).await else { return };
            if let Ok(message) = serde_json::from_slice(&message) {
                // With nobody listening, as in the CLI, it's dropped.
                let _ = told.send(message);
            }
        });
    }
}

/// The files of `folder`, by their paths in it, as a plugin's folder is deployed: names starting with a dot are left
/// out, and so is whatever the `.gitignore` files in it and above it leave out, as git would, in a git repo or not.
/// So is a plugin's `test/`, which tests it where it's made, and its native code but what a server loads: the clients
/// are built with its `client/native/`, and a NIF's source, a folder of its `server/nif/`, stays here, but for the
/// library built from it for each kind of machine, `<kind>.so` there, which is deployed though git ignores it as
/// built (`OneCore.Plugin`). A server reads the plugins it ships with this way too (`core/server/nif/folder`).
pub fn files(folder: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let root = folder.to_owned();
    let never = move |entry: &ignore::DirEntry| {
        let path = entry.path().strip_prefix(&root).unwrap_or(entry.path());
        ["client/native", "test"].iter().any(|never| path == Path::new(never))
            || path.parent() == Some(Path::new("server/nif")) && entry.file_type().is_some_and(|kind| kind.is_dir())
    };
    let walk = ignore::WalkBuilder::new(folder).require_git(false).filter_entry(move |entry| !never(entry)).build();
    for entry in walk {
        let entry = entry?;
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            let path = entry.path().strip_prefix(folder)?;
            files.push(path.iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
    for source in fs::read_dir(folder.join("server/nif")).into_iter().flatten() {
        let source = source?;
        for built in fs::read_dir(source.path()).into_iter().flatten() {
            let built = built?.file_name().to_string_lossy().into_owned();
            if built.ends_with(".so") && !built.starts_with('.') {
                files.push(format!("server/nif/{}/{built}", source.file_name().to_string_lossy()));
            }
        }
    }
    files.sort();
    Ok(files)
}
