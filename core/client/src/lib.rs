//! One Core in a client: what every client and every app's client-side code stands on. It holds the device's key,
//! reaches a server by its key and asks it, keeps what servers answered (`kept`), and answers what an app's page asks
//! through its bridge (`Subscription`). The clients are programs on it (`clients/`): the CLI and MCP, and the app, a window
//! on it. A program is built with the native code of the apps it ships with, which fills in what a device does of
//! a tool's inputs (`resolve`).
//!
//! A client asks a server in MCP's JSON-RPC: one message on a stream of its own, and the answer back on that stream.
//! A server tells a client something the same way, on a stream the server opens. No connection is kept open for
//! that: a client that wants to hear, as the app does while it runs, says it's watching, and a server connects to it
//! when there's something to say. A server acting for its person asks a watching client to call a tool of the device's
//! own the same way too, as MCP has a server send requests to its client (`DeviceTools`).

mod resolve;
mod cache;
mod telemetry;
pub mod serve;

use std::io::{ErrorKind, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use anyhow::{Context, Result, anyhow, bail};
use data_encoding::BASE64URL_NOPAD;
use futures_util::StreamExt;
use futures_util::future::{BoxFuture, join_all};
use futures_util::stream::FuturesUnordered;
use iroh::endpoint::{QuicTransportConfig, ReadError, RecvStream, RelayMode, VarInt, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, NetReportConfig, RelayConfig, RelayUrl, SecretKey, TransportAddr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, watch};
use tokio::time::{sleep, timeout};

pub use resolve::{InputResolver, Input};
pub use cache::Subscription;
use cache::write;

/// The build this program is: the commit it was built from, as `build.rs` says.
pub const BUILD: &str = env!("ONE_BUILD");

/// The kind of machine this build is for, as builds are named, the same as a server names it (`OneCore.Builds`):
/// `linux-x86_64`, `macos-aarch64`. It's the target it was compiled for, so the x64 build an ARM PC runs emulated says
/// `windows-x86_64`.
pub fn target() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

/// The protocols this build speaks with the network's other parts, from `oldest` to `newest` (`core/protocol.json`),
/// which it says in each request: a server refuses one only where they speak none in common (`OneCore.Protocol`).
pub static PROTOCOL: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    let protocol: Value =
        serde_json::from_str(include_str!("../../protocol.json")).expect("core/protocol.json is JSON");
    json!({"oldest": protocol["oldest"], "newest": protocol["newest"]})
});

/// What a device says of itself as it asks for its token, which the network keeps so a list of its devices says
/// which build each is: its build, the protocols it speaks, its kind of machine and its name.
fn device_info() -> Value {
    json!({"build": BUILD, "protocol": *PROTOCOL, "target": target(), "name": name()})
}

/// What servers and clients speak.
pub const ALPN: &[u8] = b"one/mcp/1";
/// The most one message may carry: an app's files fit.
pub const LIMIT: usize = 64 << 20;
/// How long reaching a server may take: long enough for a phone to find the relay and be let in, and for a first
/// packet that's lost to be sent again.
pub const CONNECT: Duration = Duration::from_secs(3);
/// How long a server that's there takes, at most, to say so. Nothing is sent over a connection that's asked nothing,
/// so a server that has gone is found out by asking it, and this soon. A server taken for gone that wasn't is only
/// asked again over a new connection, or has its lease taken by a standby, and neither loses anything. A server
/// running what it was asked says so this often too, as MCP's progress notifications, on the request's stream.
pub const THERE: Duration = Duration::from_secs(1);
/// What a server ends a stream with when it doesn't answer what was asked on it: `REFUSED` where it took nothing of
/// it, as one that's stopping takes nothing new, and `STOPPED` where it went away after it took it, so it may have
/// done it.
pub const REFUSED: VarInt = VarInt::from_u32(1);
pub const STOPPED: VarInt = VarInt::from_u32(2);
/// How long a connection that's asked nothing stays open. To keep one open, something has to be said over it every
/// few seconds, and through a relay each of those wakes the relay. So a connection is let go once it's idle, and
/// made again when there's something to ask.
const IDLE: Duration = Duration::from_secs(2);
/// How long a connection lasts once nothing arrives over it, which is how a server that went away while it ran a
/// call, or before it answered one just sent, is found out: both ends send a QUIC keep-alive every `THERE`, which
/// the other's QUIC acknowledges at once, and one that's running a call says so every `THERE` too. So a few of
/// those missed in a row is a machine that's gone, and whatever was asked of it is asked elsewhere this soon after
/// it went. QUIC takes the shorter of the two sides' `SILENT`. Whoever opens a connection closes it once it's
/// `IDLE`, and one opened by a server that stopped lasts this long on the other side. Meanwhile iroh sends what it
/// has for that server only where that connection was, so a server that starts again listens on the port it did
/// (`one_core_iroh`'s `start`).
const SILENT: Duration = Duration::from_secs(4);

/// What an app's native code does on the device itself, when a server acting for the device's person asks: a call of
/// one of the app's tools that act on the device, as Computer Use's on a phone's own screen. A server asks it as MCP's
/// `tools/call`, the tool named as the network names its tools (`computer-use_screenshot`), and it's answered as a
/// tool's call is. Only a client that's watching is asked, as the app is while it runs.
pub trait DeviceTools: Send + Sync {
    /// The app whose tools these are.
    fn app(&self) -> &'static str;

    /// Calls the tool named `tool`, without its app's name, with `arguments`, and gives its result, as MCP's
    /// `CallToolResult`. An error is its result too, with `isError`.
    fn call<'a>(&'a self, tool: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value>>;
}

/// A network's address, and the token this machine shows there: what says it's one of the network's own.
#[derive(Clone, Deserialize)]
pub struct Network {
    pub address: String,
    pub token: String,
}

impl Network {
    /// The headers of an ask of `path` at the address, with `body`: `key`'s signature of it, and the token. The pieces
    /// there check both (`core/edge/ask.ts`), and a server's Elixir signs the same bytes (`OneCore.Edge`).
    pub fn signed(&self, key: &SecretKey, path: &str, body: &str) -> Result<[(&'static str, String); 4]> {
        let at = (millis(SystemTime::now())? + SKEW.load(Ordering::Relaxed)).to_string();
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

    /// The servers acting for `of`, a user's or a group's prefix, as the presigner at the address says: the ones on their
    /// list and then the ones acting for the network, which act for everyone. Each is its key, reached through the
    /// relay there until a straighter path is found.
    async fn servers(&self, key: &SecretKey, of: &str) -> Result<Vec<EndpointAddr>> {
        let url = format!("{}/presigner/servers", self.address.trim_end_matches('/'));
        let (status, response) =
            ask(self, key, "/presigner/servers", "", || Ok(web()?.get(&url).timeout(CONNECT).query(&[("of", of)]))).await?;
        if !status.is_success() {
            bail!("{}", response["error"].as_str().unwrap_or("the presigner refused"))
        }
        let relay: RelayUrl = self.address.parse().with_context(|| format!("{} isn't an address", self.address))?;
        let keys = response["servers"].as_array().context("the presigner listed no servers")?;
        keys.iter()
            .map(|key| {
                let key: EndpointId = key.as_str().unwrap_or_default().parse().context("that isn't a server's key")?;
                Ok(EndpointAddr::new(key).with_relay_url(relay.clone()))
            })
            .collect()
    }
}

/// What a client keeps of its network, in the file `network` of its home: the network's address, which whatever
/// set the client up put there, where a device asks there to be let in, once it has looked, and once the device is
/// someone's, its token to the relay there, whose it is and the build it was when it got that, which it said as it
/// asked.
#[derive(Clone, Serialize, Deserialize)]
struct NetworkConfig {
    address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authorization_server: Option<AuthorizationServer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    who: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<String>,
}

/// Where a device asks its network to be let in, as the network's metadata says: the authorization server that its
/// protected resource names (RFC 9728), and that server's device authorization endpoint and token endpoint (RFC
/// 8414, RFC 8628).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct AuthorizationServer {
    device_authorization_endpoint: String,
    token_endpoint: String,
}

impl AuthorizationServer {
    /// Looks for it at the network's address, `address`.
    async fn discovered(address: &str) -> Result<Self> {
        let web = web()?;
        let read = |url: String| {
            let web = web.clone();
            async move {
                anyhow::Ok(web.get(url).timeout(CONNECT * 3).send().await?.error_for_status()?.json::<Value>().await?)
            }
        };
        let resource = read(format!("{}/.well-known/oauth-protected-resource", address.trim_end_matches('/'))).await?;
        let issuer =
            resource["authorization_servers"][0].as_str().context("the network names no authorization server")?;
        let issuer: reqwest::Url = issuer.parse().with_context(|| format!("{issuer} isn't an address"))?;
        let at = format!(
            "{}/.well-known/oauth-authorization-server{}",
            issuer.origin().ascii_serialization(),
            issuer.path()
        );
        serde_json::from_value(read(at.trim_end_matches('/').to_owned()).await?)
            .context("the network's authorization server lets no device in by a code")
    }
}

/// Where a device is with its network, from when its client opens: what every client shows, the app's window a
/// page for each and the CLI and MCP a line. Whether its network answers is beside it (`Connection`). It moves only
/// as `moves_to` says, and only by its client's own steps (`sessions`, `Client::admit`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Session {
    /// Asking its network's address for its token, or for a code to be let in by, which it asks again while its
    /// network doesn't answer (`reconnecting`).
    Opening,
    /// In its network: it asks its servers, as whoever it signed in as.
    SignedIn,
    /// Not in its network: it never was, or its network's address says it was taken out since, which a server's
    /// word alone never decides. It shows `code`, and is in once its person approves it where they sign in. What it
    /// kept stays until someone else signs in on it (`NetworkConfig::store_token`). `why` says why it's out, if
    /// it says: its last code was denied or expired, or it was removed.
    SignIn { code: Box<DeviceAuthorization>, why: Option<NotApproved> },
}

impl Session {
    /// Whether a device in this state moves to `next`: the only moves there are.
    fn moves_to(&self, next: &Session) -> bool {
        use Session::*;
        matches!(
            (self, next),
            // It's given its token, or a code to be let in by, or carries on with the token it has.
            (Opening, SignedIn | SignIn { .. })
            // Its network's address says it's nobody's now, or its person signed out on it.
            | (SignedIn, SignIn { .. } | Opening)
            // Its person approved it, or its code was denied or expired, which it asks about again.
            | (SignIn { .. }, Opening)
        )
    }
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opening => f.write_str("Opening…"),
            Self::SignedIn => f.write_str("Signed in."),
            Self::SignIn { code, why: Some(NotApproved::Removed) } => {
                write!(f, "This device was removed from your network. Open this where you sign in, and check it shows {}:\n{}", code.code, code.link)
            }
            Self::SignIn { code, why } => {
                let why = why.map(|why| format!("{why} ")).unwrap_or_default();
                write!(f, "{why}This device isn't in your network yet. Open this where you sign in, and check it shows {}:\n{}", code.code, code.link)
            }
        }
    }
}

/// Whether a client reaches its network, beside its session (`Session`): what every client shows, the app's window
/// as it opens and as a banner over its apps, and the CLI and MCP as a line. Its network answers where what the
/// client asks of it is answered: its address, while the device asks to be let in, and its servers from then on. It
/// moves only as `moves_to` says, and only by its client's own steps (`Client::answered`, `reconnecting`). Whether it
/// lost its network is which state it's in: asking again after it did is `Reconnecting`, never `Connecting`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Connection {
    /// Asking, and not answered yet, as the client opens.
    Connecting,
    /// Its network answered the last thing asked.
    Online,
    /// Its network didn't answer. It tries again by itself, sooner at first and then less often, and at once when
    /// its person tries again (`retry`).
    Offline,
    /// Asking again after its network didn't answer, and not answered yet.
    Reconnecting,
}

impl Connection {
    /// Whether a client in this state moves to `next`: the only moves there are.
    fn moves_to(&self, next: &Connection) -> bool {
        use Connection::*;
        matches!(
            (self, next),
            // It was answered, or wasn't.
            (Connecting | Reconnecting, Online | Offline)
            // It tries again.
            | (Offline, Reconnecting)
            // It lost its network.
            | (Online, Offline)
        )
    }
}

impl std::fmt::Display for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Connecting => "Connecting to your network…",
            Self::Reconnecting => "Reconnecting to your network…",
            Self::Online => "Online.",
            Self::Offline => "Can't reach your network. Check this device is online, then try again.",
        })
    }
}

/// A code a device that isn't in its network is let in by, as the network's authorization server gave it (RFC 8628),
/// which it asked for naming its key as its `client_id` and signing what it asked with it, so the code is only ever
/// for this key. A device's person sees the `link` and `code`, and nothing else of it is said anywhere.
#[derive(Clone, Serialize)]
pub struct DeviceAuthorization {
    /// Where its person lets it in: a page at the network's address, carrying the code.
    pub link: String,
    /// The code, as it's shown: WDJB-MJHT.
    pub code: String,
    /// Where it asks for its answer.
    #[serde(skip)]
    token_endpoint: String,
    #[serde(skip)]
    key: SecretKey,
    /// What the device asks for its answer with, which only it was given.
    #[serde(skip)]
    device_code: String,
    /// How long it waits between asks.
    #[serde(skip)]
    interval: Duration,
    /// When the code ends, as the authorization server said.
    #[serde(skip)]
    expires: Instant,
}

impl PartialEq for DeviceAuthorization {
    fn eq(&self, other: &Self) -> bool {
        self.device_code == other.device_code
    }
}

impl std::fmt::Debug for DeviceAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceAuthorization").field("link", &self.link).field("code", &self.code).finish_non_exhaustive()
    }
}

impl DeviceAuthorization {
    /// Asks the network's authorization server for a code to let the device with `key` in by, under this
    /// machine's name.
    async fn request(server: &AuthorizationServer, key: &SecretKey) -> Result<Self> {
        let ask = serde_urlencoded::to_string([("client_id", key.public().to_string()), ("name", name())])?;
        let (given, response) = ask_edge(&server.device_authorization_endpoint, key, FORM, ask).await?;
        let (true, Some(link), Some(code), Some(device_code)) = (
            given,
            response["verification_uri_complete"].as_str(),
            response["user_code"].as_str(),
            response["device_code"].as_str(),
        ) else {
            bail!("{}", response["error_description"].as_str().unwrap_or("the network's edge gave this device no code"))
        };
        let halves: Vec<String> = code.as_bytes().chunks(4).map(|half| String::from_utf8_lossy(half).into()).collect();
        Ok(Self {
            link: link.to_owned(),
            code: halves.join("-"),
            token_endpoint: server.token_endpoint.clone(),
            key: key.clone(),
            device_code: device_code.to_owned(),
            interval: Duration::from_secs(response["interval"].as_u64().unwrap_or(5)),
            expires: Instant::now() + Duration::from_secs(response["expires_in"].as_u64().unwrap_or(30 * 60)),
        })
    }

    /// Waits for this device's person to answer, asking the network's authorization server as often as it allows,
    /// and again after an ask that wasn't answered, as a phone that's offline for a moment asks: `Ok` once they
    /// approved it, or why it isn't, if they stopped it or its code ended first.
    async fn wait_for_approval(&self) -> Result<(), NotApproved> {
        let mut interval = self.interval;
        loop {
            sleep(interval).await;
            if Instant::now() > self.expires {
                return Err(NotApproved::Expired);
            }
            let ask = serde_urlencoded::to_string([
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &self.device_code),
                ("client_id", &self.key.public().to_string()),
            ])
            .expect("a form of strings is one");
            let Ok((given, response)) = ask_edge(&self.token_endpoint, &self.key, FORM, ask).await else { continue };
            match response["error"].as_str() {
                _ if given => return Ok(()),
                Some("slow_down") => interval += Duration::from_secs(5),
                Some("access_denied") => return Err(NotApproved::Denied),
                Some("expired_token") => return Err(NotApproved::Expired),
                // Still waiting, or an answer that isn't one, which is asked again.
                _ => {}
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
        let (vm, context) = android_context();
        let vm = unsafe { jni::JavaVM::from_raw(vm.cast()) }?;
        let mut env = vm.attach_current_thread()?;
        let context = unsafe { JObject::from_raw(context.cast()) };
        let text = |env: &mut jni::JNIEnv, string: JObject| -> jni::errors::Result<String> {
            if string.is_null() {
                return Err(jni::errors::Error::NullPtr("nothing was said"));
            }
            Ok(env.get_string(&JString::from(string))?.into())
        };
        let set = (|| {
            let resolver =
                env.call_method(&context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let setting = env.new_string("device_name")?;
            let get = "(Landroid/content/ContentResolver;Ljava/lang/String;)Ljava/lang/String;";
            let string = env
                .call_static_method(
                    "android/provider/Settings$Global",
                    "getString",
                    get,
                    &[(&resolver).into(), (&setting).into()],
                )?
                .l()?;
            text(&mut env, string)
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

/// The app's Java VM and its application's context on Android, for native code that asks Android something through
/// JNI: what the app's window gives `ndk_context` as it starts, or what the app gave `started_without_window` where it
/// started for something else first, as a push that wakes it does. Each is valid until the process ends.
#[cfg(target_os = "android")]
pub fn android_context() -> (*mut std::ffi::c_void, *mut std::ffi::c_void) {
    match WITHOUT_WINDOW.get() {
        Some(&(vm, context)) => (vm as _, context as _),
        None => {
            let android = ndk_context::android_context();
            (android.vm(), android.context())
        }
    }
}

/// Keeps the app's Java VM and its application's context, a global reference, where the app starts without its
/// window: the window's runtime gives `ndk_context` its own as it starts, once, and refuses to if it was given one
/// already, so this is kept apart from it.
///
/// # Safety
///
/// Both must stay valid until the process ends.
#[cfg(target_os = "android")]
pub unsafe fn started_without_window(vm: *mut std::ffi::c_void, context: *mut std::ffi::c_void) {
    let _ = WITHOUT_WINDOW.set((vm as usize, context as usize));
}

#[cfg(target_os = "android")]
static WITHOUT_WINDOW: std::sync::OnceLock<(usize, usize)> = std::sync::OnceLock::new();

/// Why a device isn't in its network now, where it says: its person stopped it where they signed in, its code ended
/// first, or it was in and was removed since. The app says it on the sign-in page it shows the next code on, told it
/// as `why`, in these same words (`apps/sign-in/edge/page`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NotApproved {
    Denied,
    Expired,
    Removed,
}

impl NotApproved {
    /// How the sign-in page is told it.
    pub fn why(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Removed => "removed",
        }
    }
}

impl std::fmt::Display for NotApproved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Denied => "It was denied where you signed in.",
            Self::Expired => "Its code expired before it was approved. Try again.",
            Self::Removed => "This device was removed from your network.",
        })
    }
}

impl std::error::Error for NotApproved {}

/// What a device sends where it asks to be let in, as RFC 8628 has it.
const FORM: &str = "application/x-www-form-urlencoded";

/// Asks the network's edge at `url`, with `body` signed by `key`. Gives whether it agreed, and what it said.
async fn ask_edge(url: &str, key: &SecretKey, content_type: &str, body: String) -> Result<(bool, Value)> {
    let parsed: reqwest::Url = url.parse().with_context(|| format!("{url} isn't an address"))?;
    let network = Network { address: parsed.origin().ascii_serialization(), token: String::new() };
    let request = || Ok(web()?.post(url).timeout(CONNECT * 3).header("content-type", content_type).body(body.clone()));
    let (status, response) = ask(&network, key, parsed.path(), &body, request).await?;
    Ok((status.is_success(), response))
}

/// How far this device's clock is from the edge's, in milliseconds, as the edge last said (`ask`).
static SKEW: AtomicI64 = AtomicI64::new(0);

fn millis(time: SystemTime) -> Result<i64> {
    Ok(time.duration_since(UNIX_EPOCH)?.as_millis().try_into()?)
}

/// Sends what `request` makes, signed by `key` as an ask of `path` with `body` at `network`'s edge, and gives its
/// status and what it said. One the edge refuses for this device's clock (`RequestTimeTooSkewed`, as S3 says) is
/// signed again by the edge's clock, its `Date`, and sent once more, as AWS's SDKs do: the device keeps how far its
/// clock is from the edge's for every ask after.
async fn ask(
    network: &Network,
    key: &SecretKey,
    path: &str,
    body: &str,
    request: impl Fn() -> Result<reqwest::RequestBuilder>,
) -> Result<(reqwest::StatusCode, Value)> {
    let mut again = true;
    loop {
        let mut ask = request()?;
        for (name, value) in network.signed(key, path, body)? {
            ask = ask.header(name, value);
        }
        let sent = ask.send().await?;
        let status = sent.status();
        let date = sent.headers().get(reqwest::header::DATE).and_then(|date| httpdate::parse_http_date(date.to_str().ok()?).ok());
        let response: Value = sent.json().await?;
        // The edge's `Date` is to the second, so its time is taken as the middle of that second.
        if let Some(edge) = date.filter(|_| again && response["code"] == "RequestTimeTooSkewed") {
            SKEW.store(millis(edge)? + 500 - millis(SystemTime::now())?, Ordering::Relaxed);
            again = false;
            continue;
        }
        return Ok((status, response));
    }
}

impl NetworkConfig {
    /// What a client of the network at `address` keeps before it has joined it.
    fn at(address: &str) -> Self {
        Self { address: address.to_owned(), authorization_server: None, token: None, who: None, build: None }
    }

    /// What's kept in `home`, if it's there whole.
    fn read(home: &Path) -> Option<Self> {
        serde_json::from_slice(&fs::read(home.join("network")).ok()?).ok()
    }

    /// How long the token this client has lasts before it's renewed, by when it says it ends: nothing if it has none.
    fn left(&self) -> Duration {
        self.ends().duration_since(SystemTime::now() + RENEW).unwrap_or_default()
    }

    /// When the token this client has ends, as it says: at once if it has none.
    fn ends(&self) -> SystemTime {
        let claims = self.token.as_ref().and_then(|token| BASE64URL_NOPAD.decode(token.as_bytes()).ok());
        let until =
            claims.and_then(|claims| serde_json::from_slice::<Value>(&claims).ok()).and_then(|claims| claims["until"].as_u64());
        UNIX_EPOCH + Duration::from_millis(until.unwrap_or_default())
    }

    /// Asks the network's edge for this device's token, signed with its key, and keeps it in `home` with whose
    /// device it is (`store_token`). Gives whether the device is someone's. One that's nobody's drops its token,
    /// which nothing takes now, and keeps whose it was and what it kept for them: the edge's word alone takes
    /// nothing else from it, and what it kept goes only once someone else signs in on it.
    async fn admit(&mut self, key: &SecretKey, home: &Path) -> Result<bool> {
        let ask = json!({"jsonrpc": "2.0", "id": 1, "method": "one/token", "params": {"build": device_info()}});
        let mcp = format!("{}/mcp", self.address.trim_end_matches('/'));
        let (_, response) = ask_edge(&mcp, key, "application/json", ask.to_string()).await?;
        if response["error"]["code"] == NOBODY {
            if self.token.take().is_some() {
                write(&home.join("network"), &serde_json::to_vec(self)?)?;
            }
            return Ok(false);
        }
        if response["result"].is_null() {
            bail!("{}", response["error"]["message"].as_str().unwrap_or("the network's edge gave this device no token"))
        }
        self.store_token(&response["result"], home)?;
        Ok(true)
    }

    /// Asks the network's authorization server for a code this device is let in by, signed with its key.
    async fn device_code(&mut self, key: &SecretKey, home: &Path) -> Result<DeviceAuthorization> {
        let server = match self.authorization_server.clone() {
            Some(server) => server,
            None => AuthorizationServer::discovered(&self.address).await?,
        };
        let requested = DeviceAuthorization::request(&server, key).await;
        // What it found is kept for the next time, and what didn't answer is looked for again then.
        self.authorization_server = requested.is_ok().then_some(server);
        write(&home.join("network"), &serde_json::to_vec(self)?)?;
        requested
    }

    /// Keeps a token this device was given, `{token, who}`, in `home`. Once the device is someone else's, what it kept
    /// of whoever's it was goes.
    fn store_token(&mut self, given: &Value, home: &Path) -> Result<()> {
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

/// What asks over the web: the presigner at a network's address, and the bucket by a link. Its TLS is the one iroh
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
/// machine has no address, and so no relay. It listens on `port`, or on any that's free with `0`. A server's NIF
/// binds this same endpoint.
pub async fn endpoint(key: SecretKey, network: Option<&Network>, port: u16) -> Result<Endpoint> {
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
        .transport_config(
            QuicTransportConfig::builder()
                .max_idle_timeout(Some(SILENT.try_into()?))
                .keep_alive_interval(THERE)
                // The Android emulator's network drops a batch of UDP datagrams sent as one (GSO) without an error, so
                // a direct path there passes its handshake and pings but no answer of any size: a phone's datagrams
                // go one at a time, which its radio's speed doesn't notice.
                .enable_segmentation_offload(!cfg!(target_os = "android"))
                .build(),
        )
        .relay_mode(relay)
        .net_report_config(probes)
        .bind_addr(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))?
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
    let write_key = || -> std::io::Result<()> {
        let mut making = tempfile::NamedTempFile::new_in(folder)?;
        making.write_all(&key.to_bytes())?;
        making.persist_noclobber(file).map(drop).map_err(|error| error.error)
    };
    match write_key() {
        Ok(()) => Ok(key),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => read(),
        Err(error) => Err(error).with_context(|| format!("couldn't make {}", file.display())),
    }
}

/// The folder a client on a computer is kept in: `ONE_HOME` if that's set, as a network made to build or test with
/// sets it, and otherwise where the system keeps an app's data on this machine, under a name of this client's own:
/// on Windows `%LOCALAPPDATA%`, as a device's key and a server's builds are this PC's and never roam with its person.
/// A phone gives its app a folder instead.
pub fn home() -> Result<PathBuf> {
    match env::var_os("ONE_HOME") {
        Some(home) => Ok(home.into()),
        None => Ok(dirs::data_local_dir().context("this system has no folder for an app's data: set ONE_HOME")?.join(NAME)),
    }
}

/// What the app is to the system it's on: the name of its folder, and of the app among a phone's. It's the build's
/// Tauri identifier (`build.rs`), so a network's own build keeps its own folder beside the One App's. The app
/// before this one is `one` and `the One App` there, and this is neither. A debug build's has `.dev` after it
/// (`build.rs`): a client of its own, never the installed one's.
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
/// server asks another the same way. `gone` is whether the server was found not to be there, or to be going: it
/// stopped answering, or refused what it was asked, so nothing more is asked over it.
pub struct Reached {
    connection: iroh::endpoint::Connection,
    received: Arc<std::sync::Mutex<Instant>>,
    asking: Arc<AtomicUsize>,
    gone: AtomicBool,
}

/// One message being asked over a connection, for as long as it's held: a connection that's asking isn't idle.
struct Asking(Arc<AtomicUsize>);

impl Asking {
    fn new(asking: &Arc<AtomicUsize>) -> Self {
        asking.fetch_add(1, Ordering::Relaxed);
        Self(asking.clone())
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Reached {
    /// Connects to the server at `address`, or gives nothing if it doesn't answer `within` that long.
    pub async fn connect(endpoint: &Endpoint, address: EndpointAddr, within: Duration) -> Option<Self> {
        let connection = timeout(within, endpoint.connect(address, ALPN)).await.ok()?.ok()?;
        let received = Arc::new(std::sync::Mutex::new(Instant::now()));
        let asking = Arc::new(AtomicUsize::new(0));
        let (open, last, busy) = (connection.clone(), received.clone(), asking.clone());
        tokio::spawn(async move {
            while open.close_reason().is_none() {
                let idle = last.lock().expect("no ask panics").elapsed();
                match IDLE.checked_sub(idle) {
                    Some(left) => sleep(left).await,
                    None if busy.load(Ordering::Relaxed) > 0 => sleep(IDLE).await,
                    None => open.close(0u32.into(), b"idle"),
                }
            }
        });
        Some(Self { connection, received, asking, gone: AtomicBool::new(false) })
    }

    pub fn connection(&self) -> &iroh::endpoint::Connection {
        &self.connection
    }

    /// How the connection reaches its server now: `direct`, or through a `relay`.
    pub fn path(&self) -> &'static str {
        match self.connection.paths().iter().find(|path| path.is_selected()) {
            Some(path) if path.is_relay() => "relay",
            Some(_) => "direct",
            None => "none",
        }
    }

    /// Whether the connection is still one to ask over: it hasn't closed, and its server hasn't been found gone.
    pub fn open(&self) -> bool {
        self.connection.close_reason().is_none() && !self.gone()
    }

    /// Whether the server didn't say it was there when it was asked over this connection.
    pub fn gone(&self) -> bool {
        self.gone.load(Ordering::Relaxed)
    }

    /// Asks the server one message, and gives its answer, handing `notified` each notification the server sends
    /// about it first, as MCP has a server send them before its response: one a line, with the response last. A
    /// server that hasn't answered anything lately is first asked whether it's there. One running a request that
    /// carries a `progressToken` sends a progress notification every `THERE` while it runs, which keeps the
    /// connection from being `IDLE` or `SILENT` however long it takes. One that goes silent, or ends the stream
    /// without an answer, is gone: what's asked over this connection from now on is asked elsewhere, while what was
    /// asked already is still answered.
    pub async fn ask(&self, message: &[u8], mut notified: impl FnMut(&[u8])) -> Result<Vec<u8>, RequestError> {
        if !self.open() || (self.received.lock().expect("no ask panics").elapsed() > THERE && !self.there().await) {
            return Err(RequestError::Unreachable);
        }
        let _asking = Asking::new(&self.asking);
        let mut receive = self.send(message).await.ok_or(RequestError::Unreachable)?;
        let (mut read, mut scanned) = (Vec::new(), 0);
        loop {
            let chunk = receive.read_chunk(LIMIT).await.map_err(|unanswered| {
                self.gone.store(true, Ordering::Relaxed);
                match unanswered {
                    ReadError::Reset(REFUSED) => RequestError::Unreachable,
                    _ => RequestError::Stopped,
                }
            })?;
            *self.received.lock().expect("no ask panics") = Instant::now();
            let Some(chunk) = chunk else { return Ok(read) };
            if read.len() + chunk.len() > LIMIT {
                self.gone.store(true, Ordering::Relaxed);
                return Err(RequestError::Stopped);
            }
            read.extend_from_slice(&chunk);
            // A response is JSON on one line, so whatever ends in a newline before it is a notification.
            while let Some(end) = read[scanned..].iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = read.drain(..=scanned + end).collect();
                notified(&line[..line.len() - 1]);
                scanned = 0;
            }
            scanned = read.len();
        }
    }

    /// Whether the server is there: an empty message, which a server answers at once with nothing. One that doesn't
    /// in time is gone, and so is this connection to it.
    async fn there(&self) -> bool {
        let probe = async { self.send(b"").await?.read_to_end(LIMIT).await.ok() };
        let there = timeout(THERE, probe).await.is_ok_and(|response| response.is_some());
        if there {
            *self.received.lock().expect("no ask panics") = Instant::now();
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

/// The `progressToken` a request carries, with which MCP asks for progress notifications while it runs.
pub fn progress_token(message: &[u8]) -> Option<Value> {
    #[derive(Deserialize)]
    struct Request {
        params: Option<Params>,
    }
    #[derive(Deserialize)]
    struct Params {
        #[serde(rename = "_meta")]
        meta: Option<Meta>,
    }
    #[derive(Deserialize)]
    struct Meta {
        #[serde(rename = "progressToken")]
        progress_token: Option<Value>,
    }
    let token = serde_json::from_slice::<Request>(message).ok()?.params?.meta?.progress_token?;
    (token.is_string() || token.is_number()).then_some(token)
}

/// An id for a request of this run's own, which no other request of it has, so one is cancelled by it alone.
pub fn request_id() -> u64 {
    static IDS: AtomicU64 = AtomicU64::new(1);
    IDS.fetch_add(1, Ordering::Relaxed)
}

/// MCP's progress notification for the request whose `token` it is, `progress` being how far it is and `message`
/// what it's doing, if it says, on a line of its own as it's sent ahead of the response (`Reached::ask`).
pub fn progress(token: &Value, progress: u64, message: Option<&str>) -> Vec<u8> {
    let mut params = json!({"progressToken": token, "progress": progress});
    if let Some(message) = message {
        params["message"] = json!(message);
    }
    let mut line =
        json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": params}).to_string().into_bytes();
    line.push(b'\n');
    line
}

/// What a progress notification says its request is doing, if it says.
pub fn progress_message(line: &[u8]) -> Option<String> {
    let notification: Value = serde_json::from_slice(line).ok()?;
    (notification["method"] == "notifications/progress")
        .then(|| notification["params"]["message"].as_str().map(str::to_owned))?
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
    /// Its network's address, if it has one: a network on one machine has none.
    address: Option<String>,
    /// Where its device is with its network, which only `moved` changes.
    session: watch::Sender<Session>,
    /// One ask of the network's address for this device's token at a time (`admit`), however many find at once that
    /// it may be nobody's.
    admitting: Mutex<()>,
    /// Said by its person trying again, so a network that didn't answer is asked now rather than when it next would
    /// (`reconnecting`).
    again: Arc<tokio::sync::Notify>,
    /// Its endpoint, its network and whose device it is, which are made again with each new token (`relink`).
    link: std::sync::RwLock<Link>,
    reach: Mutex<Reach>,
    notifications: broadcast::Sender<Value>,
    /// What every request says of who it's from and for, where the environment says: `ONE_LINK` is a sharing link,
    /// whose holder asks, `ONE_AS` names a user to act as, which only a server of a network being built allows, and
    /// `ONE_SUBJECT` names the group it's for (`groups/family`), or `network`.
    says: Vec<(&'static str, String)>,
    /// Whether this client is to hear what changes, so its servers are to tell it.
    watching: AtomicBool,
    /// Whether its network answers, which only `connection_moved` changes.
    connection: watch::Sender<Connection>,
    /// Why a server refused it for being on a version before the server's that doesn't talk with it (`OneCore.Mcp`),
    /// once one has: from then on it asks only what brings it up to date (`updates`), as an app that requires an
    /// update does nothing but offer it, rather than being refused again for everything it asks. It's behind until a
    /// server talks with it again, which it asks afresh as it reconnects and as its person tries again (`retry`):
    /// a reason kept from before is never taken for a server's answer now.
    behind: watch::Sender<Option<String>>,
    /// What the native code of the apps the program ships with fills in of a tool's inputs (`resolve`).
    resolvers: Vec<Arc<dyn InputResolver>>,
    /// The tools of the device's own that the native code of the apps the program ships with answers a server.
    tools: Vec<Arc<dyn DeviceTools>>,
    /// What this client knows of each tool, as a server last listed them (`resolve`).
    listed: std::sync::Mutex<Option<Arc<resolve::Listed>>>,
    /// What this client says of itself in what it records (`report`), as OpenTelemetry's resource attributes say a
    /// service: which client it is (`service.name`), its build, its device's name and its operating system, until its
    /// SDK is made, as it first records something (`telemetry`).
    resource: Arc<std::sync::RwLock<serde_json::Map<String, Value>>>,
    telemetry: Arc<std::sync::OnceLock<telemetry::Telemetry>>,
    /// Telling a server what was recorded, one at a time.
    sending: Mutex<()>,
    /// Since when no server has answered, if none has, and whether this device was offline then (`offline`).
    unanswered: std::sync::Mutex<Option<(Instant, bool)>>,
    me: std::sync::Weak<Client>,
    /// The runtime the client was opened on, which telling a server runs on, since a record may come from a thread
    /// with none, as a window's commands do.
    runtime: tokio::runtime::Handle,
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
/// The presigner at the address and a server check it (`core/edge/presigner`, `OneCore.User`).
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
        let endpoint = endpoint(key, shown.as_ref(), 0).await?;
        Ok(Self { endpoint, network, who, attestation })
    }
}

impl Client {
    /// Opens the client kept in `home`, which fills in of a tool's inputs what One Core does and what `resolvers` do,
    /// and answers a server's call of `tools`, the device's own. It opens whether or not its device is in its network:
    /// where that is, is its `session`, which it moves on by itself from here (`sessions`). Only a home it can't read
    /// is an error.
    pub async fn open(
        home: impl Into<PathBuf>,
        resolvers: Vec<Arc<dyn InputResolver>>,
        tools: Vec<Arc<dyn DeviceTools>>,
    ) -> Result<Arc<Self>> {
        let home = home.into();
        let config = match fs::read(home.join("network")) {
            Ok(network) => {
                Some(serde_json::from_slice::<NetworkConfig>(&network).context("`network` isn't a network's address")?)
            }
            // An app built for one network knows its address.
            Err(error) if error.kind() == ErrorKind::NotFound => BUILT_FOR.map(NetworkConfig::at),
            Err(error) => return Err(error).context("couldn't read `network`"),
        };
        let key = key(&home.join("key"))?;
        // What a program of this device was on before is kept with its token: one that's now on another was updated.
        let updated = config.as_ref().and_then(|config| config.build.clone()).filter(|was| was != BUILD);
        // A device with a token is in, and renews it as it goes, but one whose token has ended renews it before
        // anything is asked with it, as no server takes it, and carries on with it if its network doesn't answer.
        // One with none asks for one, and while it's nobody's, for a code to be let in by. A network on one machine
        // has no address, and whatever started it says who this acts as.
        let (network, who, session) = match &config {
            None => (None, None, Session::SignedIn),
            Some(config) => match &config.token {
                Some(token) => {
                    let network = Network { address: config.address.clone(), token: token.clone() };
                    let session = if config.ends() > SystemTime::now() { Session::SignedIn } else { Session::Opening };
                    (Some(network), config.who.clone(), session)
                }
                None => (None, None, Session::Opening),
            },
        };
        let link = Link::run(&key, network, who).await?;
        let says = [("as", "ONE_AS"), ("link", "ONE_LINK"), ("subject", "ONE_SUBJECT")];
        let says = says.into_iter().filter_map(|(says, variable)| Some((says, env::var(variable).ok()?))).collect();
        let notifications = broadcast::channel(16).0;
        let endpoint = link.endpoint.clone();
        let link = std::sync::RwLock::new(link);
        let (watching, connection) = (AtomicBool::new(false), watch::Sender::new(Connection::Connecting));
        let listed = std::sync::Mutex::default();
        let reach = Mutex::default();
        let resource = Arc::new(std::sync::RwLock::new(resource()));
        let address = config.map(|config| config.address);
        let client = Arc::new_cyclic(|me| Self {
            home, key, address, session: watch::Sender::new(session), admitting: Mutex::default(),
            again: Arc::default(), link, reach, notifications, says, watching, connection, behind: watch::Sender::new(None), resolvers, tools,
            listed, resource, telemetry: Arc::default(), sending: Mutex::default(), unanswered: std::sync::Mutex::default(),
            me: me.clone(), runtime: tokio::runtime::Handle::current(),
        });
        accept(endpoint, client.me.clone());
        client.panics();
        if let Some(was) = updated {
            client.report(json!({"name": "one.client.updated", "attributes": {"one.build.previous": was}}));
        }
        tokio::spawn(sessions(Arc::downgrade(&client)));
        tokio::spawn(reconnecting(Arc::downgrade(&client)));
        Ok(client)
    }

    /// Where this device is with its network, as that changes.
    pub fn session(&self) -> watch::Receiver<Session> {
        self.session.subscribe()
    }

    /// Moves this device's session to `next`, if that's a move there is from where it is (`Session::moves_to`).
    /// Gives whether it moved.
    fn moved(&self, next: Session) -> bool {
        self.session.send_if_modified(|now| {
            let moves = now.moves_to(&next);
            if moves {
                *now = next;
            }
            moves
        })
    }

    /// Has a client whose network didn't answer ask it again now, as the device says it's back online or its app is
    /// in front again, rather than when it next would (`reconnecting`). One that's answered asks nothing.
    pub fn reconnect(&self) {
        if *self.connection.borrow() == Connection::Offline {
            self.again.notify_one();
        }
    }

    /// Asks its network again now, as its person does by trying again: where it didn't answer, and where a server
    /// refused it for being behind (`behind`), which it's no longer taken to be until a server refuses it again, as
    /// one updated or rolled back since may talk with it. What it asks is the list of its files, as itself, which a
    /// server refuses while it's behind: it answers once it has, failing with why where it wasn't answered.
    pub async fn retry(&self) -> Result<()> {
        self.reconnect();
        self.behind.send_replace(None);
        self.fresh("resources/list", &json!({})).await.map(drop)
    }

    /// Signs this device out, as its person does once they've removed it from their devices: what it kept of
    /// theirs goes now, as it would for someone else signing in on it, and it opens again as a device that isn't in.
    pub async fn sign_out(&self) -> Result<()> {
        let _one = self.admitting.lock().await;
        let mut config = self.config().context("A device signs out of a network with an address")?;
        (config.token, config.who) = (None, None);
        config.keep(&self.home)?;
        self.moved(Session::Opening);
        Ok(())
    }

    /// What this client keeps of its network, as it is now in its home, where every client of the device keeps
    /// it: nothing for a network on one machine.
    fn config(&self) -> Option<NetworkConfig> {
        let address = self.address.as_deref()?;
        Some(NetworkConfig::read(&self.home).unwrap_or_else(|| NetworkConfig::at(address)))
    }

    /// Asks its network's address for this device's token, one ask at a time, while its session is still `from`,
    /// and moves the session on by what that says (`Session::moves_to`): in, with the token, or out, with a code to be
    /// let in by. Only the address says a device is nobody's, never a server alone, and an address that doesn't
    /// answer takes nothing from one that's in: it carries on with the token it has, and one with none stays
    /// opening, while its network is offline (`Connection`). `why` is why it asks for a new code, if it does: its
    /// last one was denied or expired. Gives how long until it asks again, while it's in.
    async fn admit(&self, from: &Session, why: Option<NotApproved>) -> Duration {
        let _one = self.admitting.lock().await;
        let Some(mut config) = self.config().filter(|_| *self.session.borrow() == *from) else { return AGAIN };
        let failed = match config.admit(&self.key, &self.home).await {
            Ok(true) => match self.relink(&config).await {
                Ok(()) => {
                    self.moved(Session::SignedIn);
                    self.answered(true);
                    self.rewatch().await;
                    return config.left();
                }
                Err(error) => error,
            },
            Ok(false) => {
                let removed = config.who.is_some().then_some(NotApproved::Removed);
                match config.device_code(&self.key, &self.home).await {
                    Ok(code) => {
                        self.moved(Session::SignIn { code: Box::new(code), why: why.or(removed) });
                        self.answered(true);
                        return AGAIN;
                    }
                    Err(error) => error,
                }
            }
            Err(error) => error,
        };
        let failed = failed.context(format!("Couldn't join the network at {}", config.address));
        // A device that's in carries on with the token it has, and asks again soon.
        if *from == Session::SignedIn || config.token.is_some() {
            self.moved(Session::SignedIn);
        } else {
            self.report(json!({"name": "one.client.join", "error": format!("{failed:#}")}));
        }
        self.answered(false);
        AGAIN
    }

    /// Asks for this device's next token while it's in. A server gives one to a device that's someone's. One that
    /// can't be reached, as where the token this has ended, is asked at the network's address, which also says
    /// whether the device is nobody's now (`admit`). Gives how long until it asks again.
    async fn renew(&self) -> Duration {
        let Some(mut config) = self.config() else { return AGAIN };
        // No server takes a token that has ended, so that's asked at the address straight away.
        let given = match config.ends() > SystemTime::now() {
            true => self.request("one/token", json!({"build": device_info()})).await.ok(),
            false => None,
        };
        let Some(given) = given else { return self.admit(&Session::SignedIn, None).await };
        let _one = self.admitting.lock().await;
        let renewed = async {
            config.store_token(&given, &self.home)?;
            self.relink(&config).await
        };
        match renewed.await {
            Ok(()) => {
                self.rewatch().await;
                config.left()
            }
            Err(_) => AGAIN,
        }
    }

    /// Makes the run's endpoint again to show the token `config` now has, since iroh shows a relay the token an
    /// endpoint was made with. The new one has a key of its own too, as the old one is still there. The old endpoint
    /// goes once its token ends, so what was asked over it finishes, and what's asked from now on is asked from the
    /// new one, whose run the device now attests to.
    async fn relink(&self, config: &NetworkConfig) -> Result<()> {
        let token = config.token.clone().context("this device was given no token")?;
        let network = Network { address: config.address.clone(), token };
        let new = Link::run(&self.key, Some(network), config.who.clone()).await?;
        accept(new.endpoint.clone(), self.me.clone());
        let old = std::mem::replace(&mut *self.link.write().expect("no renewal panics"), new);
        self.reach.lock().await.open = None;
        tokio::spawn(async move {
            sleep(RENEW).await;
            old.endpoint.close().await
        });
        Ok(())
    }

    /// Says again that this client is watching, if it is, as servers reach it where its run now is (`relink`).
    async fn rewatch(&self) {
        if self.watching.load(Ordering::Relaxed) {
            self.watch(true).await;
        }
    }

    /// Whose device this is, while it's in its network: the user it signed in as.
    pub fn who(&self) -> Option<String> {
        if *self.session.borrow() != Session::SignedIn {
            return None;
        }
        self.link().who
    }

    /// This client's key, as others know it.
    pub fn key(&self) -> String {
        self.key.public().to_string()
    }

    /// This device's own key, for the native code of the apps the program ships with: what's encrypted to the
    /// device opens with it. No app's page and no server is handed it.
    pub fn device_key(&self) -> &SecretKey {
        &self.key
    }

    /// The network's address, if it has one.
    pub fn address(&self) -> Option<String> {
        self.address.clone()
    }

    fn link(&self) -> Link {
        self.link.read().expect("no renewal panics").clone()
    }

    /// Whether its network answers, as that changes (`Connection`). It's found out by asking: nothing is said over
    /// a connection to learn it.
    pub fn connection(&self) -> watch::Receiver<Connection> {
        self.connection.subscribe()
    }

    /// Why its servers refused it for being behind them, once one has, until one talks with it again (`behind`).
    pub fn behind(&self) -> watch::Receiver<Option<String>> {
        self.behind.subscribe()
    }

    /// Moves whether its network answers to `next`, if that's a move there is from where it is
    /// (`Connection::moves_to`), and once it answers again after it didn't for longer than a person is shown nothing
    /// of it, reports how long it didn't. Gives whether it moved.
    fn connection_moved(&self, next: Connection) -> bool {
        let moved = self.connection.send_if_modified(|now| {
            let moves = now.moves_to(&next);
            if moves {
                *now = next;
            }
            moves
        });
        let mut since = self.unanswered.lock().expect("no ask panics");
        match next {
            Connection::Offline if moved => drop(since.get_or_insert_with(|| (Instant::now(), self.offline()))),
            Connection::Online if moved => {
                // Back, it asks its servers afresh whether they talk with it, as one may have been updated meanwhile.
                self.behind.send_replace(None);
                if let Some((since, offline)) = since.take().filter(|(since, _)| since.elapsed() > UNNOTICED) {
                    let took = since.elapsed().as_secs_f64() * 1000.0;
                    self.report(match offline {
                        true => json!({"name": "one.client.unanswered", "took": took, "attributes": {"one.outcome": "offline"}}),
                        false => json!({"name": "one.client.unanswered", "took": took, "error": "No server is answering"}),
                    })
                }
            }
            _ => {}
        }
        moved
    }

    /// Says whether its network answered what was just asked. What's asked while it's offline leaves it offline
    /// until it's tried again (`reconnecting`), and an answer to it is a try that was answered.
    fn answered(&self, answered: bool) {
        if answered {
            self.connection_moved(Connection::Reconnecting);
        }
        self.connection_moved(if answered { Connection::Online } else { Connection::Offline });
    }

    /// Says whether this client is to hear what changes, as the app does while it runs, to every server acting for
    /// its user. While it is, each tells it when something changed, and keeps telling it once it starts again. A
    /// server that isn't up now is told when it's next asked anything, and one that isn't told the app has gone
    /// finds out when it next has something to say. It's said again where a server may not know it: from this run's
    /// next endpoint (`relink`), and once a server answers again after none did, as one that's new may not have heard.
    pub async fn watch(&self, watching: bool) {
        self.watching.store(watching, Ordering::Relaxed);
        if *self.session.borrow() != Session::SignedIn {
            return;
        }
        let Ok(Some(servers)) = self.servers(false).await else { return };
        let watches = servers.into_iter().map(|server| async move {
            if let Some(open) = self.connect(server).await
                && !watching
            {
                let mut unwatch = json!({"jsonrpc": "2.0", "method": WATCH, "params": {}});
                self.say(&mut unwatch);
                let _ = open.ask(unwatch.to_string().as_bytes(), |_| {}).await;
            }
        });
        join_all(watches).await;
    }

    /// Connects to `server`, hears what it tells and asks this client over the connection, and if this client is
    /// watching, says so first, with where a server reaches it and the device it runs for.
    async fn connect(&self, server: EndpointAddr) -> Option<Reached> {
        let link = self.link();
        let open = Reached::connect(&link.endpoint, server, CONNECT).await?;
        tokio::spawn(receive_notifications(open.connection().clone(), self.notifications.clone()));
        tokio::spawn(answer(self.me.clone(), open.connection().clone()));
        tokio::spawn(straight(self.me.clone(), open.connection().clone()));
        if self.watching.load(Ordering::Relaxed) {
            // Where it says a server reaches it is its network's relay, so it says so once it's there, as a client
            // that has only just opened, as one a push woke, isn't yet.
            if link.network.is_some() {
                let _ = tokio::time::timeout(CONNECT, link.endpoint.online()).await;
            }
            let here = address(&link.endpoint, link.network.as_ref()).ok()?;
            let mut watch = json!({"jsonrpc": "2.0", "method": WATCH, "params": {"address": here}});
            self.say(&mut watch);
            open.ask(watch.to_string().as_bytes(), |_| {}).await.ok()?;
        }
        Some(open)
    }

    /// What this device answers a server that asked it `message`, `from` being the server's key: a call of a tool of
    /// the device's own (`DeviceTools`), as MCP's `tools/call`, from a server acting for this device's person. Anyone
    /// else of the network is refused, as is anything else.
    async fn answer(&self, from: EndpointId, message: &[u8]) -> Value {
        let request: Value = serde_json::from_slice(message).unwrap_or_default();
        let error = |code: i64, message: String| json!({"jsonrpc": "2.0", "id": request["id"], "error": {"code": code, "message": message}});
        let ours = matches!(self.servers(false).await, Ok(Some(servers)) if servers.iter().any(|server| server.id == from));
        if !ours {
            return error(NOBODY, "Only a server acting for this device's person asks it".into());
        }
        let name = request["params"]["name"].as_str().unwrap_or_default();
        let found = self.tools.iter().find_map(|tools| Some((tools, name.strip_prefix(tools.app())?.strip_prefix('_')?)));
        let Some((tools, tool)) = found.filter(|_| request["method"] == "tools/call") else {
            return error(-32601, format!("This device has no {} {name}", request["method"].as_str().unwrap_or("method")));
        };
        let result = match tools.call(tool, request["params"]["arguments"].clone()).await {
            Ok(result) => result,
            Err(why) => json!({"content": [{"type": "text", "text": format!("{why:#}")}], "isError": true}),
        };
        json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
    }

    /// What servers tell this client: each a JSON-RPC notification.
    pub fn notifications(&self) -> broadcast::Receiver<Value> {
        self.notifications.subscribe()
    }

    /// Asks a server one JSON-RPC message and gives its answer, which for a notification is nothing. A server that
    /// has gone was asked nothing, so whichever answers now is asked instead.
    pub async fn ask(&self, message: &[u8]) -> Result<Vec<u8>> {
        self.ask_notified(message, |_| {}).await
    }

    /// Asks as `ask` does, handing `notified` each notification the server sends about the request while it runs,
    /// as MCP's progress notifications. A request is asked for them with a `progressToken` of its own where it
    /// carries none, since they're what keeps a long one from being taken for a server that went away.
    pub async fn ask_notified(&self, message: &[u8], notified: impl Fn(&[u8])) -> Result<Vec<u8>> {
        // Only a device that's in asks its servers. One that's opening is asked once it's in or isn't, unless its
        // network doesn't answer, which leaves it unable to say which.
        let (mut session, mut connection) = (self.session.subscribe(), self.connection.subscribe());
        loop {
            let offline = *connection.borrow_and_update() == Connection::Offline;
            match &*session.borrow_and_update() {
                Session::SignedIn => break,
                Session::Opening if offline => bail!("{}", Connection::Offline),
                Session::Opening => {}
                out => bail!("{out}"),
            }
            tokio::select! {
                changed = session.changed() => changed?,
                changed = connection.changed() => changed?,
            }
        }
        // Behind its servers, it asks them nothing they'd refuse it for that.
        if let Some(why) = self.behind.borrow().clone()
            && !updates(message)
        {
            bail!("{why}")
        }
        // A request is read once on its way out: for what this device fills in of it, what it says of itself, and
        // the span it is of a trace (`span`).
        let (message, lists, reads, span): (std::borrow::Cow<[u8]>, bool, bool, Option<opentelemetry_sdk::trace::Span>) =
            match serde_json::from_slice::<Value>(message) {
                Ok(mut request) if request["method"].is_string() => {
                    if request.get("id").is_some() && request["params"]["_meta"]["progressToken"].is_null() {
                        static TOKENS: AtomicU64 = AtomicU64::new(0);
                        request["params"]["_meta"]["progressToken"] = json!(TOKENS.fetch_add(1, Ordering::Relaxed));
                    }
                    self.resolve(&mut request).await?;
                    let reads = self.reads(&request).await?;
                    let span = request.get("id").is_some().then(|| self.span(&mut request));
                    self.say(&mut request);
                    (request.to_string().into_bytes().into(), request["method"] == "tools/list", reads, span)
                }
                _ => (message.into(), false, false, None),
            };
        let reached = std::sync::Mutex::new("none");
        // A server that went away after it was asked is asked again elsewhere only for what only reads: anything
        // else it may have done.
        let request = async {
            let mut stopped = false;
            for _ in 0..2 {
                let open = self.reached().await?;
                *reached.lock().expect("no ask panics") = open.path();
                match open.ask(&message, &notified).await {
                    Ok(response) => return Ok(response),
                    Err(RequestError::Unreachable) => {}
                    Err(RequestError::Stopped) if reads => stopped = true,
                    Err(RequestError::Stopped) => bail!(
                        "The server went away before it answered, as one does while it restarts, so this may or may \
                         not have been done. Look before trying again."
                    ),
                }
            }
            match stopped {
                true => bail!("The server went away before it answered, as one does while it restarts. Try again in a moment."),
                false => bail!(UNANSWERED),
            }
        };
        let response = request.await;
        self.answered(response.is_ok());
        // Refused for being behind, it's behind; answered what only a server it talks with answers, it isn't.
        if let Ok(response) = &response
            && !updates(&message)
        {
            let why = left_behind(response);
            self.behind.send_if_modified(|now| now.is_some() != why.is_some() && { *now = why; true });
        }
        if let Some(mut span) = span {
            use opentelemetry::trace::{Span, Status};
            let refused = match &response {
                Err(why) => Some(why.to_string()),
                Ok(response) => refused(response),
            };
            span.set_attribute(opentelemetry::KeyValue::new("one.reached", *reached.lock().expect("no ask panics")));
            match refused {
                Some(_) if response.is_err() && self.offline() => {
                    span.set_attribute(opentelemetry::KeyValue::new("one.outcome", "offline"))
                }
                Some(refused) => span.set_status(Status::error(refused)),
                None => {}
            }
            span.end();
        }
        if response.is_ok() {
            self.send_soon();
        }
        match response {
            Ok(response) => {
                // What a device fills in of each tool's inputs is what the latest list of them says.
                if lists
                    && let Ok(listed) = serde_json::from_slice::<Value>(&response)
                    && listed["result"]["tools"].is_array()
                {
                    let known = resolve::listed(&listed["result"], &self.formats());
                    *self.listed.lock().expect("no ask panics") = Some(Arc::new(known));
                }
                Ok(self.taken_out(response).await)
            }
            refused => refused,
        }
    }

    /// What a server answered, unless it says this device is nobody's: one that was taken out of its network since
    /// it got its token. A server alone may be wrong, so its network's address is asked (`admit`), and only if it
    /// says so too is the device out, and the answer says where its person lets it in again.
    async fn taken_out(&self, bytes: Vec<u8>) -> Vec<u8> {
        let Ok(mut response) = serde_json::from_slice::<Value>(&bytes) else { return bytes };
        if response["error"]["code"] != NOBODY || self.address.is_none() {
            return bytes;
        }
        self.admit(&Session::SignedIn, None).await;
        let session = self.session.borrow().clone();
        if !matches!(session, Session::SignIn { .. }) {
            return bytes;
        }
        response["error"]["message"] = json!(session.to_string());
        response.to_string().into_bytes()
    }

    /// Has `request` say who it's from, in its `_meta`: the device this run is of, with its attestation for the run's key,
    /// and what the environment says, unless the request says otherwise, as one of a device's own does. It says the
    /// protocols it speaks too, which a server refuses, saying why, only where it speaks none of them (`OneCore.Mcp`),
    /// and this build, and which client this is, as MCP's 2026-07-28 has each request say (`one-cli`, `one-app`), unless it's an MCP
    /// client's own request that says which that is.
    fn say(&self, request: &mut Value) {
        for (says, value) in [("device", self.key.public().to_string()), ("attestation", self.link().attestation)] {
            request["params"]["_meta"][says] = json!(value);
        }
        request["params"]["_meta"]["protocol"] = PROTOCOL.clone();
        request["params"]["_meta"]["version"] = json!(BUILD);
        let info = &mut request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"];
        if info.is_null() {
            let service = self.resource.read().expect("no report panics")["service.name"].clone();
            *info = json!({"name": format!("one-{}", service.as_str().unwrap_or("cli")), "version": BUILD});
        }
        for (says, value) in &self.says {
            if request["params"]["_meta"][says].is_null() {
                request["params"]["_meta"][says] = json!(value);
            }
        }
    }

    /// Asks the server for `method`, and gives its result.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_notified(request_id(), method, params, |_| {}).await
    }

    /// Asks as `request` does, as the request `id`, which cancels it (`notifications/cancelled`), handing `notified`
    /// each notification the server sends about it while it runs, as MCP's progress notifications.
    pub async fn request_notified(
        &self,
        id: u64,
        method: &str,
        params: Value,
        notified: impl Fn(&[u8]),
    ) -> Result<Value> {
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let response = self.ask_notified(message.to_string().as_bytes(), notified).await?;
        let mut response: Value = serde_json::from_slice(&response).context("the server's answer isn't JSON")?;
        if let Some(error) = response.get("error") {
            bail!("{}", error["message"].as_str().unwrap_or("the server refused"))
        }
        let mut result = response["result"].take();
        self.keep_files(&mut result).await?;
        Ok(result)
    }

    /// Has `request` carry the span it is, as W3C's `traceparent` in its `_meta`, which a server's span of it is under,
    /// so a trace runs from this client on, by OpenTelemetry's propagator. Gives the span, which is kept should it fail
    /// or be slow. An MCP client says which it is, for what it records: in `initialize`, or in each request's `_meta`
    /// from MCP's 2026-07-28 on.
    fn span(&self, request: &mut Value) -> opentelemetry_sdk::trace::Span {
        use opentelemetry::propagation::TextMapPropagator;
        use opentelemetry::trace::{Span, TraceContextExt};
        let method = request["method"].as_str().unwrap_or_default().to_owned();
        let client = match method.as_str() {
            "initialize" => &request["params"]["clientInfo"]["name"],
            _ => &request["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
        };
        if client.is_string() {
            let mut resource = self.resource.write().expect("no report panics");
            resource.insert("service.name".into(), json!("mcp"));
            resource.insert("mcp.client.name".into(), client.clone());
        }
        let client = client.as_str().map(str::to_owned);
        let tool = request["params"]["name"].as_str().filter(|_| method == "tools/call").map(str::to_owned);
        let mut span = telemetry::request_span(self.telemetry(), &method, tool.as_deref());
        if let Some(client) = client {
            span.set_attribute(opentelemetry::KeyValue::new("mcp.client.name", client));
        }
        let meta = &mut request["params"]["_meta"];
        if meta["traceparent"].is_null() {
            let mut carrier = std::collections::HashMap::new();
            let cx = opentelemetry::Context::new().with_remote_span_context(span.span_context().clone());
            opentelemetry_sdk::propagation::TraceContextPropagator::new().inject_context(&cx, &mut carrier);
            for (name, value) in carrier {
                meta[name] = json!(value);
            }
        }
        span
    }

    /// This client's OpenTelemetry SDK, made with what it says of itself the first time it records something.
    fn telemetry(&self) -> &telemetry::Telemetry {
        self.telemetry.get_or_init(|| {
            let mut resource = self.resource.write().expect("no report panics");
            resource.entry("host.name").or_insert_with(|| json!(name()));
            telemetry::Telemetry::new(self.home.clone(), &resource)
        })
    }

    /// Keeps `record` for Telemetry, as from this client, and tells a server once one answers, with whatever else
    /// this device's clients kept: so what went wrong while nothing could be reached is told once something can. A
    /// record is Telemetry's (its tool `record`): a `name` and `attributes`, and a `value`, or how long something
    /// `took` with its `error` if it failed, or else a log line, with an `error` if it is one. Names, sizes and
    /// outcomes only: never what anyone wrote, was given or answered, and never a secret.
    pub fn report(&self, record: Value) {
        self.telemetry().record(&record);
        if *self.connection.borrow() == Connection::Online {
            self.send_soon();
        }
    }

    /// Which client this is, as what it reports says: OpenTelemetry's `service.name`, such as `app`. It's `cli`
    /// unless said, and an MCP client's says which it is as it starts.
    pub fn service(&self, name: &str) {
        self.resource.write().expect("no report panics").insert("service.name".into(), json!(name));
    }

    /// Reports a panic of this program's when it happens, so it's told next time any of its device's clients
    /// reaches a server, if not before.
    fn panics(&self) {
        static HOOKED: std::sync::Once = std::sync::Once::new();
        let Some(me) = self.me.upgrade() else { return };
        let me = Arc::downgrade(&me);
        HOOKED.call_once(move || {
            let before = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic| {
                let message = panic.payload_as_str().unwrap_or("panicked");
                let mut record = json!({"name": "exception", "error": message.lines().next().unwrap_or(message),
                    "attributes": {"exception.type": "panic"}});
                if let Some(at) = panic.location() {
                    record["attributes"]["code.file.path"] = json!(at.file());
                    record["attributes"]["code.line.number"] = json!(at.line());
                }
                if let Some(me) = me.upgrade() {
                    me.telemetry().record(&record);
                }
                before(panic)
            }));
        });
    }

    /// Tells a server what was reported, soon after it's asked something else, and never in the way of that.
    fn send_soon(&self) {
        if let Some(me) = self.me.upgrade()
            && self.home.join(telemetry::REPORTS).exists()
        {
            self.runtime.spawn(async move { me.send_reports().await });
        }
    }

    /// Tells a server, in one batch, what this device's clients reported and kept (`report`), for the apps that
    /// hear `measured`. What it can't tell is kept again for next time. Telling reports nothing itself.
    async fn send_reports(&self) {
        let _one = self.sending.lock().await;
        // The kept exports are this run's to tell once it has moved them: other clients keep theirs anew.
        let sending = self.home.join(format!("{}.{}", telemetry::REPORTS, self.link().endpoint.id()));
        if fs::rename(self.home.join(telemetry::REPORTS), &sending).is_err() {
            return;
        }
        let stored = fs::read_to_string(&sending).unwrap_or_default();
        let _ = fs::remove_file(&sending);
        // A line a client before OpenTelemetry's SDK kept, a record, is let go.
        let exports: Vec<Value> = stored
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|line| line["signal"].is_string())
            .collect();
        let notification = json!({"jsonrpc": "2.0", "method": MEASURED, "params": {"otlp": exports}});
        if !exports.is_empty() && self.ask(notification.to_string().as_bytes()).await.is_err() {
            for export in &exports {
                telemetry::keep(&self.home, export);
            }
        }
    }

    /// Ends the connection, if one was made, and tells the server. The goodbye leaves this machine at once. What
    /// else closing an endpoint waits for is a server that was tried and never answered, which QUIC gives three
    /// seconds, so that isn't waited for.
    pub async fn close(&self) {
        // What's left to tell is told first, to a server this run reached.
        if let Some(telemetry) = self.telemetry.get() {
            telemetry.flush();
        }
        if self.reach.lock().await.open.is_some() {
            self.send_reports().await;
        }
        if let Some(open) = self.reach.lock().await.open.take() {
            open.connection().close(0u32.into(), b"done");
            let _ = timeout(Duration::from_millis(100), self.link().endpoint.close()).await;
        }
    }

    /// Whether this device is offline, as far as it can tell: its network has a relay at its address, and it isn't
    /// connected to it, as a phone with no signal or an app iOS held in the background isn't. Nothing could have
    /// answered it then, so what wasn't answered meanwhile is the device's being offline, not a failure of its
    /// network's. A network being built has no relay, so there it can't tell.
    fn offline(&self) -> bool {
        use iroh::Watcher;
        let link = self.link();
        link.network.is_some() && !link.endpoint.home_relay_status().get().iter().any(|relay| relay.is_connected())
    }

    /// The connection to a server, made when there's none open: to whichever of the servers acting for this client's
    /// user answers first. Which they are is what the client kept from when it last asked, and what it's told when
    /// it asks now if none of those answers.
    async fn reached(&self) -> Result<Arc<Reached>> {
        let started = Instant::now();
        let mut reach = self.reach.lock().await;
        if let Some(open) = reach.open.as_ref().filter(|open| open.open()) {
            return Ok(open.clone());
        }
        reach.open = None;
        // Many asks at once share one try: no server answered while this one waited its turn.
        if reach.failed.is_some_and(|failed| failed > started) {
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
        let of = self.of()?;
        let cached = self.cached_servers(&of);
        if !now {
            return Ok(cached);
        }
        let servers = network
            .servers(&self.key, &of)
            .await
            .with_context(|| format!("Couldn't ask {} which servers to reach", network.address))?;
        // One that was reached straight before is tried there again, beside the relay.
        let servers: Vec<EndpointAddr> = servers
            .into_iter()
            .map(|server| match cached.iter().flatten().find(|was| was.id == server.id) {
                Some(was) => server.with_addrs(was.addrs.iter().filter(|addr| addr.is_ip()).cloned()),
                None => server,
            })
            .collect();
        self.keep(SERVERS, &json!({"of": of}), &serde_json::to_value(&servers)?)?;
        Ok(Some(servers))
    }

    /// Whose servers this client asks: whoever its device signed in as, or in a network being built, who it says it
    /// acts as. A sharing link's holder asks the servers of whoever the link leads to, which it starts with.
    fn of(&self) -> Result<String> {
        let header = |what| self.says.iter().find(|(says, _)| *says == what).map(|(_, header)| header.as_str());
        Ok(match (header("link").map(|link| link.split('/').collect::<Vec<_>>()), header("as"), &self.link().who) {
            (Some(leads), ..) if leads[0] == "network" => "network/".to_owned(),
            (Some(leads), ..) if leads.len() > 2 => format!("{}/{}/", leads[0], leads[1]),
            (_, Some(acts_as), _) => format!("users/{acts_as}/"),
            (_, _, Some(who)) => format!("{who}/"),
            _ => bail!("This device isn't signed in here. In a network being built, ONE_AS says who it acts as."),
        })
    }

    /// The servers acting for `of` as this client last kept them, each with where it was last reached straight.
    fn cached_servers(&self, of: &str) -> Option<Vec<EndpointAddr>> {
        self.cached(SERVERS, &json!({"of": of})).and_then(|cached| serde_json::from_value(cached).ok())
    }

    /// Keeps `addr` as where `server` is reached straight, as a connection to it found it, so the next run of this
    /// device's clients tries it from the start, beside the relay, rather than waiting for the relay to find it again.
    fn reached_straight(&self, server: EndpointId, addr: SocketAddr) {
        let (Some(_), Ok(of)) = (self.link().network, self.of()) else { return };
        let Some(mut servers) = self.cached_servers(&of) else { return };
        let Some(cached) = servers.iter_mut().find(|cached| cached.id == server) else { return };
        let relay = cached.addrs.iter().filter(|addr| !addr.is_ip()).cloned();
        let now: std::collections::BTreeSet<_> = relay.chain([TransportAddr::Ip(addr)]).collect();
        if now != cached.addrs {
            cached.addrs = now;
            let cached = serde_json::to_value(&servers).map_err(anyhow::Error::from);
            if let Err(error) = cached.and_then(|servers| self.keep(SERVERS, &json!({"of": of}), &servers)) {
                self.report(json!({"name": "one.client.cache", "error": format!("{error:#}")}));
            }
        }
    }
}

/// Keeps where the server at the other end of `connection` is reached straight, each time that's the path taken.
async fn straight(client: std::sync::Weak<Client>, connection: iroh::endpoint::Connection) {
    let server = connection.remote_id();
    let mut paths = connection.paths_stream();
    while let Some(paths) = paths.next().await {
        let selected = paths.iter().find(|path| path.is_selected()).map(|path| path.remote_addr().clone());
        match (client.upgrade(), selected) {
            (Some(client), Some(TransportAddr::Ip(addr))) => client.reached_straight(server, addr),
            (Some(_), _) => {}
            (None, _) => return,
        }
    }
}

/// How long before a device's token ends it asks for the next one: half of what a server gives one
/// (`OneCore.Edge`), so a device that's taken out of its network uses its relay for an hour at most.
const RENEW: Duration = Duration::from_secs(30 * 60);

/// How soon a token that couldn't be asked for is asked for again: the device may be offline.
const AGAIN: Duration = Duration::from_secs(60);

/// What a client tells a server of what it reported, for the apps that hear `measured`.
const MEASURED: &str = "notifications/one/measured";

/// How long its network not answering goes unshown: the window's page shows nothing for a moment's gap.
const UNNOTICED: Duration = Duration::from_secs(3);

/// What a client says of itself in what it reports, until it says more (`Client::report`).
fn resource() -> serde_json::Map<String, Value> {
    serde_json::Map::from_iter([
        ("service.name".to_owned(), json!("cli")),
        ("service.version".to_owned(), json!(BUILD)),
        ("os.type".to_owned(), json!(env::consts::OS)),
    ])
}

/// Why a server refused what it was asked, as the person asking is told, if it did: a JSON-RPC error, or a tool's.
/// A refusal is short, and a long answer isn't read twice to find it isn't one.
/// Whether `message` asks what a server answers whatever version asks it, as it's how a client catches up: the list
/// of tools, which builds there are, downloading one, updating, and following an update's task (`talks` in
/// `OneCore.Mcp`).
fn updates(message: &[u8]) -> bool {
    let Ok(request) = serde_json::from_slice::<Value>(message) else { return false };
    let tool = request["params"]["name"].as_str().unwrap_or_default();
    ["tools/list", "tasks/get"].contains(&request["method"].as_str().unwrap_or_default())
        || (request["method"] == "tools/call" && ["builds", "download", "update"].contains(&tool))
}

/// Why a server refused this client, where it's for this client speaking only protocols older than the server's: a
/// server answers that -32022, with the protocols it speaks, as MCP refuses a protocol version it doesn't speak. One
/// behind this client, which is asked again once it's updated, isn't this client's to fix.
fn left_behind(response: &[u8]) -> Option<String> {
    if response.len() > 16 << 10 {
        return None;
    }
    let response: Value = serde_json::from_slice(response).ok()?;
    let error = &response["error"];
    let oldest = error["data"]["supported"]["oldest"].as_u64()?;
    (error["code"] == -32022 && PROTOCOL["newest"].as_u64()? < oldest)
        .then(|| error["message"].as_str().unwrap_or("This app needs updating").to_owned())
}

fn refused(response: &[u8]) -> Option<String> {
    if response.len() > 16 << 10 {
        return None;
    }
    let response: Value = serde_json::from_slice(response).ok()?;
    if let Some(why) = response["error"]["message"].as_str() {
        return Some(why.to_owned());
    }
    let result = &response["result"];
    (result["isError"] == true).then(|| result["content"][0]["text"].as_str().unwrap_or("The tool failed").to_owned())
}

/// What a person is told when no server answered: what went wrong, and what to do about it.
const UNANSWERED: &str = "No server is answering. Check this device is online and a server is on, then try again.";

/// The JSON-RPC error a server answers a device that's nobody's (`OneCore.User`).
const NOBODY: i64 = -32003;

/// What a client tells a server to be told what changes, at the `address` it gives, or with none, no longer.
const WATCH: &str = "notifications/one/watch";

/// What the servers the presigner listed are kept as.
const SERVERS: &str = "presigner/servers";

/// A server with something to say or ask connects to this client to say it, at `endpoint`, until it closes. Only one
/// of the network's own reaches a client, and all it can make one do is ask again, or call a tool of the device's own,
/// which only one acting for the device's person may (`Client::answer`).
fn accept(endpoint: Endpoint, client: std::sync::Weak<Client>) {
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let client = client.clone();
            tokio::spawn(async move {
                let Ok(connection) = incoming.await else { return };
                let Some(notifications) = client.upgrade().map(|client| client.notifications.clone()) else { return };
                tokio::spawn(answer(client, connection.clone()));
                receive_notifications(connection, notifications).await
            });
        }
    });
}

/// Answers what a server asks this client over `connection`, each message on a stream of its own, as a server answers
/// a client (`Client::answer`), until the connection ends. An empty message only asks whether it's there.
async fn answer(client: std::sync::Weak<Client>, connection: iroh::endpoint::Connection) {
    let from = connection.remote_id();
    while let Ok((mut send, mut receive)) = connection.accept_bi().await {
        let client = client.clone();
        tokio::spawn(async move {
            let Ok(message) = receive.read_to_end(LIMIT).await else { return };
            let Some(client) = client.upgrade().filter(|_| !message.is_empty()) else { return drop(send.finish()) };
            let response = client.answer(from, &message).await.to_string();
            if send.write_all(response.as_bytes()).await.is_ok() {
                let _ = send.finish();
            }
        });
    }
}

/// Moves a device's session on (`Session`), for as long as its client is open: asks for its token while it's
/// opening, renews it while it's in, and waits for its person to approve it while it shows a code. One whose network
/// didn't answer as it opened is asked about again as its network is (`reconnecting`). A server saying the device is
/// nobody's asks the address too (`Client::taken_out`), through the same `admit`.
async fn sessions(client: std::sync::Weak<Client>) {
    // Why it asks for a new code, when it does: its last one was denied or expired.
    let mut why = None;
    // How long until it renews its token, while it's in.
    let mut renews = None;
    loop {
        let Some(open) = client.upgrade() else { return };
        let mut session = open.session.subscribe();
        let now = session.borrow_and_update().clone();
        match &now {
            Session::Opening => {
                renews = Some(open.admit(&now, why.take()).await);
                drop(open);
                if session.changed().await.is_err() {
                    return;
                }
            }
            Session::SignedIn => {
                // A network on one machine has no token to renew. One a new build hasn't renewed yet is renewed now,
                // which says so.
                let Some(config) = open.config() else {
                    drop(open);
                    if session.changed().await.is_err() {
                        return;
                    }
                    continue;
                };
                let built_same = config.build.as_deref() == Some(BUILD);
                let left = renews.take().unwrap_or_else(|| if built_same { config.left() } else { Duration::ZERO });
                drop(open);
                tokio::select! {
                    _ = sleep(left) => {}
                    changed = session.changed() => if changed.is_err() { return } else { continue },
                }
                let Some(open) = client.upgrade() else { return };
                renews = Some(open.renew().await);
            }
            Session::SignIn { code, .. } => {
                drop(open);
                let answered = tokio::select! {
                    answered = code.wait_for_approval() => answered,
                    changed = session.changed() => if changed.is_err() { return } else { continue },
                };
                let Some(open) = client.upgrade() else { return };
                why = answered.err();
                open.moved(Session::Opening);
            }
        }
    }
}

/// While a client's network doesn't answer, it tries again by itself, after 1 s, then 2 s, then every 3 s at most,
/// each wait a random time from half of it to all of it, and at once when its person tries again (`retry`) or the
/// device is back online or in front (`reconnect`), as the device or its network may be back any time: what's outside a
/// device comes and goes, and no person should have to ask for what was going to come back anyway. A device that's
/// opening asks its network's address to let it in, and one that's in and watching asks a server. Once its network
/// answers again, it says again that it's watching, to every server, since one that was starting may not have heard.
async fn reconnecting(client: std::sync::Weak<Client>) {
    const FIRST: Duration = Duration::from_secs(1);
    const MOST: Duration = Duration::from_secs(3);
    let Some((mut connection, tried)) = client.upgrade().map(|client| (client.connection(), client.again.clone())) else {
        return;
    };
    // Where it was when it last looked: answered after `Offline` or `Reconnecting`, it's back.
    let (mut again, mut was) = (FIRST, *connection.borrow());
    loop {
        let now = *connection.borrow_and_update();
        let before = std::mem::replace(&mut was, now);
        if now == Connection::Offline {
            tokio::select! {
                _ = sleep(again.mul_f64(0.5 + fastrand::f64() / 2.0)) => {}
                _ = tried.notified() => {}
                changed = connection.changed() => if changed.is_err() { return } else { continue },
            }
            again = (again * 2).min(MOST);
            let Some(client) = client.upgrade() else { return };
            let session = client.session.borrow().clone();
            match session {
                Session::Opening => {
                    client.connection_moved(Connection::Reconnecting);
                    client.admit(&session, None).await;
                }
                Session::SignedIn if client.watching.load(Ordering::Relaxed) => {
                    client.connection_moved(Connection::Reconnecting);
                    let _ = client.request("ping", json!({})).await;
                }
                _ => {}
            }
            continue;
        }
        if now == Connection::Online && matches!(before, Connection::Offline | Connection::Reconnecting) {
            again = FIRST;
            let Some(client) = client.upgrade() else { return };
            if client.watching.load(Ordering::Relaxed) {
                client.watch(true).await;
            }
        }
        if connection.changed().await.is_err() {
            return;
        }
    }
}

/// Tells whoever `connection` is to one message, on a stream of its own, which isn't answered. Gives whether it
/// was sent.
pub async fn notify(connection: &iroh::endpoint::Connection, message: &[u8]) -> bool {
    let sent = async {
        let mut send = connection.open_uni().await.ok()?;
        send.write_all(message).await.ok()?;
        send.finish().ok()
    };
    sent.await.is_some()
}

/// Passes on what a server tells this client over `connection`, until the connection ends.
async fn receive_notifications(connection: iroh::endpoint::Connection, notifications: broadcast::Sender<Value>) {
    while let Ok(mut receive) = connection.accept_uni().await {
        let notifications = notifications.clone();
        tokio::spawn(async move {
            let Ok(message) = receive.read_to_end(LIMIT).await else { return };
            if let Ok(message) = serde_json::from_slice(&message) {
                // With nobody listening, as in the CLI, it's dropped.
                let _ = notifications.send(message);
            }
        });
    }
}

/// The files of `folder`, by their paths in it, as an app's folder is deployed: names starting with a dot are left
/// out, and so is whatever the `.gitignore` files in it and above it leave out, as git would, in a git repo or not.
/// So is an app's native code but what a server loads: the clients are built with its `client/native/`, and the
/// source of a NIF or a program, a folder of its `server/nif/` or `server/bin/`, stays here, but for what's built from
/// it for each kind of machine, `<kind>.so` or `<kind>` there, which is deployed though git ignores it as built
/// (`OneCore.App`). A server reads an app's folder this way too (`core/server/nif/folder`).
pub fn files(folder: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let root = folder.to_owned();
    let never = move |entry: &ignore::DirEntry| {
        let path = entry.path().strip_prefix(&root).unwrap_or(entry.path());
        path == Path::new("client/native")
            || [Path::new("server/nif"), Path::new("server/bin")].contains(&path.parent().unwrap_or(path))
                && entry.file_type().is_some_and(|kind| kind.is_dir())
    };
    let walk = ignore::WalkBuilder::new(folder).require_git(false).filter_entry(move |entry| !never(entry)).build();
    for entry in walk {
        let entry = entry?;
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            let path = entry.path().strip_prefix(folder)?;
            files.push(path.iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
    for native in ["nif", "bin"] {
        for source in fs::read_dir(folder.join("server").join(native)).into_iter().flatten() {
            let source = source?;
            for built in fs::read_dir(source.path()).into_iter().flatten() {
                let built = built?.file_name().to_string_lossy().into_owned();
                let kind = ["linux-", "macos-", "windows-"].iter().any(|os| built.starts_with(os));
                if kind && (native == "bin" || built.ends_with(".so")) {
                    files.push(format!("server/{native}/{}/{built}", source.file_name().to_string_lossy()));
                }
            }
        }
    }
    files.sort();
    Ok(files)
}
