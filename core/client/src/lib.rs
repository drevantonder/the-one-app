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

mod auth;
mod cache;
mod resolve;
mod rpc;
pub mod serve;
mod state;
pub mod system;
mod telemetry;
mod transport;

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime};
use std::{env, fs};

use anyhow::{Context, Result, anyhow};
use futures_util::future::BoxFuture;
use iroh::SecretKey;
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast, watch};
use tokio::time::timeout;

pub use resolve::{InputResolver, Input};
pub use cache::Subscription;
pub use state::{State, Connection};
pub use auth::{Network, DeviceAuthorization, NotApproved};
pub use transport::{ALPN, LIMIT, CONNECT, THERE, REFUSED, STOPPED, endpoint, address, relay, RequestError, Reached, Unreached, notify};
pub use rpc::{progress_token, request_id, progress, progress_message};
use auth::{NetworkConfig, sessions};
use transport::{Reach, Link, accept};
use state::reconnecting;
use telemetry::resource;

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

/// What asks over the web: the presigner at a network's address, the bucket by a link, and the core's release, whose
/// builds every update downloads. Its TLS is the one iroh reaches the relay with.
pub fn web() -> Result<reqwest::Client> {
    let tls = iroh::tls::CaTlsConfig::default().client_config(iroh::tls::default_provider())?;
    Ok(reqwest::Client::builder().tls_backend_preconfigured(tls).connect_timeout(WEB_CONNECT).build()?)
}

/// How long connecting over the web may take: TCP sends a connection's first packet again after one second and then
/// three more, so two lost on a lossy link take seven. At three seconds, an update's download from GitHub timed out
/// now and then from machines whose every other connection worked (20 Nov). What must fail fast says so itself
/// (`.timeout(CONNECT)`).
const WEB_CONNECT: Duration = Duration::from_secs(15);

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
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

/// The `network` file an app made for one network has among its resources, if it has one: `{"address": "https://..."}`,
/// which the network's finishing puts in the plain build, as white-label apps are made (`tooling/testflight`). An app
/// with none starts on no network until one is given. Its resources are beside its program, or a Mac bundle's
/// `Contents/Resources`, or on Android its APK's `assets`.
fn bundled() -> Result<Option<NetworkConfig>> {
    let Some(network) = bundled_file() else { return Ok(None) };
    Ok(Some(serde_json::from_slice(&network).context("the app's own `network` isn't a network's address")?))
}

#[cfg(not(target_os = "android"))]
fn bundled_file() -> Option<Vec<u8>> {
    let program = env::current_exe().ok()?.canonicalize().ok()?;
    let folder = program.parent()?;
    #[cfg(target_os = "macos")]
    let folder = &folder.parent()?.join("Resources");
    fs::read(folder.join("network")).ok()
}

#[cfg(target_os = "android")]
fn bundled_file() -> Option<Vec<u8>> {
    use jni::objects::JObject;
    let (vm, context) = android_context();
    let vm = unsafe { jni::JavaVM::from_raw(vm.cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let context = unsafe { JObject::from_raw(context.cast()) };
    let mut read = || -> jni::errors::Result<Vec<u8>> {
        let assets = env.call_method(&context, "getAssets", "()Landroid/content/res/AssetManager;", &[])?.l()?;
        let name = env.new_string("network")?;
        let stream = env.call_method(&assets, "open", "(Ljava/lang/String;)Ljava/io/InputStream;", &[(&name).into()])?.l()?;
        let buffer = env.new_byte_array(4096)?;
        let mut bytes = Vec::new();
        loop {
            let count = env.call_method(&stream, "read", "([B)I", &[(&buffer).into()])?.i()?;
            if count < 0 {
                break;
            }
            let mut chunk = vec![0i8; count as usize];
            env.get_byte_array_region(&buffer, 0, &mut chunk)?;
            bytes.extend(chunk.into_iter().map(|byte| byte as u8));
        }
        env.call_method(&stream, "close", "()V", &[])?;
        Ok(bytes)
    };
    let bytes = read();
    // An app with no `network` among its assets: AssetManager says so by throwing.
    if bytes.is_err() {
        let _ = env.exception_clear();
    }
    bytes.ok()
}

/// A client, kept in its home folder: its `key`, where its `network` is or its `servers` are, and what servers
/// answered (`kept`).
pub struct Client {
    home: PathBuf,
    key: SecretKey,
    /// Its network's address, if it has one: a network on one machine has none.
    address: Option<String>,
    /// Where it is with its network, which only `moved` changes.
    state: watch::Sender<State>,
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
    /// What the native code of the apps the program ships with fills in of a tool's inputs (`resolve`).
    resolvers: Vec<Arc<dyn InputResolver>>,
    /// The tools of the device's own that the native code of the apps the program ships with answers a server.
    tools: Vec<Arc<dyn DeviceTools>>,
    /// What this client knows of each tool, as a server last listed them (`resolve`).
    listed: std::sync::Mutex<Option<Arc<resolve::Listed>>>,
    /// Asking a server for `tools/list`, one at a time, so concurrent startup calls don't stampede.
    listing: Mutex<()>,
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

impl Client {
    /// Which server this client asks now, by its key's first 12 hex digits, and how it reaches it (`direct` or
    /// `relay`), for what it reports of a request: `None` where it reaches none, or it's reaching one now.
    pub fn asking(&self) -> Option<String> {
        let reach = self.reach.try_lock().ok()?;
        let open = reach.open.as_ref().filter(|open| open.open())?;
        Some(format!("{} {}", &open.connection().remote_id().to_string()[..12], open.path()))
    }

    /// Opens the client kept in `home`, which fills in of a tool's inputs what One Core does and what `resolvers` do,
    /// and answers a server's call of `tools`, the device's own. It opens whether or not its device is in its network:
    /// where that is, is its `state`, which it moves on by itself from here (`sessions`). Only a home it can't read
    /// is an error. `service` is which program it's of, as all it reports says (OpenTelemetry's `service.name`:
    /// `cli`, `app`, `server`, `mcp`), from the first thing it reports, which may be as it opens.
    pub async fn open(
        home: impl Into<PathBuf>,
        service: &str,
        resolvers: Vec<Arc<dyn InputResolver>>,
        tools: Vec<Arc<dyn DeviceTools>>,
    ) -> Result<Arc<Self>> {
        let home = home.into();
        let config = match fs::read(home.join("network")) {
            Ok(network) => {
                Some(serde_json::from_slice::<NetworkConfig>(&network).context("`network` isn't a network's address")?)
            }
            // An app made for one network knows its address.
            Err(error) if error.kind() == ErrorKind::NotFound => bundled()?,
            Err(error) => return Err(error).context("couldn't read `network`"),
        };
        let key = key(&home.join("key"))?;
        // What a program of this device was on before is kept with its token: one that's now on another was updated.
        let updated = config.as_ref().and_then(|config| config.build.clone()).filter(|was| was != BUILD);
        // A device with a token is in, and renews it as it goes, but one whose token has ended renews it before
        // anything is asked with it, as no server takes it, and carries on with it if its network doesn't answer.
        // One with none asks for one, and while it's nobody's, for a code to be let in by. A network on one machine
        // has no address, and whatever started it says who this acts as.
        let connection = Connection::Connecting;
        let (network, who, state) = match &config {
            None => (None, None, State::SignedIn { connection }),
            Some(config) => match &config.token {
                Some(token) => {
                    let network = Network { address: config.address.clone(), token: token.clone() };
                    let state = match config.ends() > SystemTime::now() {
                        true => State::SignedIn { connection },
                        false => State::Opening { connection },
                    };
                    (Some(network), config.who.clone(), state)
                }
                None => (None, None, State::Opening { connection }),
            },
        };
        let link = Link::run(&key, network, who).await?;
        let says = [("as", "ONE_AS"), ("link", "ONE_LINK"), ("subject", "ONE_SUBJECT")];
        let says = says.into_iter().filter_map(|(says, variable)| Some((says, env::var(variable).ok()?))).collect();
        let notifications = broadcast::channel(16).0;
        let endpoint = link.endpoint.clone();
        let link = std::sync::RwLock::new(link);
        let watching = AtomicBool::new(false);
        let listed = std::sync::Mutex::default();
        let listing = Mutex::default();
        let reach = Mutex::default();
        let resource = Arc::new(std::sync::RwLock::new(resource(service)));
        let address = config.map(|config| config.address);
        let client = Arc::new_cyclic(|me| Self {
            home, key, address, state: watch::Sender::new(state), admitting: Mutex::default(),
            again: Arc::default(), link, reach, notifications, says, watching, resolvers, tools,
            listed, listing, resource, telemetry: Arc::default(), sending: Mutex::default(), unanswered: std::sync::Mutex::default(),
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

    /// What servers tell this client: each a JSON-RPC notification.
    pub fn notifications(&self) -> broadcast::Receiver<Value> {
        self.notifications.subscribe()
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
