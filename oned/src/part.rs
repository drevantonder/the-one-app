//! A service's part: a device's abilities (a service) offered to the network, as a part of its device's oned, a process
//! of its own (`oned <name>`). It has a key of its own, kept in its folder, which its device's oned lists as its user's,
//! a part of that device (`services_runtime::list_part`); it binds no endpoint of its own, and is reached only through its device's
//! app runtime, which carries what it's asked sealed and signed (`carried`): it registers there over the loopback and
//! long-polls for asks (`keep_socket`). It holds no keys and reads no bucket: it reads `global`'s scope database's steps
//! from its runtime, from the first freeze naming the network's root (which oned gives it, so nothing on the way can
//! make it another network's), from which it works out who's who and checks each asker by itself: a compute of the
//! network that runs the calls of a user it serves. The program offering it, its app's own (`oned::program`), which this
//! starts, registers over a loopback with the services' API and long-polls for calls, in its sandbox (`sandbox`),
//! reaching out only through this part's proxy. A service writes nothing itself: what it asks for is a call, which its
//! runtime runs (`/api/call` on the loopback).

use ::custom_build::allowed;
use ::custom_build::config::YOURS;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::SeqCst;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};


use ::key::{Hash, Key, hex, short, unhex};
use ::scope::scopes::{self, ScopeDb};
use ::scope::steps::{Reader, Truth};
use ::services::carried;
use ::services::{Network, Registry, Services, folder, refused, write_private};


#[derive(Clone)]
pub struct Service(Arc<Inner>);

struct Inner {
    name: String,
    dir: PathBuf,
    key: SigningKey,
    network: Network,
    /// Its device's app runtime, on the loopback oned gave both (`--runtime=`).
    runtime: String,
    /// Who's who, once it read `global`'s steps from its runtime.
    known: Mutex<ScopeDb>,
    /// Reads who's who from the steps of `global`'s scope database its runtime gives, kept in its folder (`KEPT`) to
    /// start again from: reading every step again costs a restart seconds of CPU for each of its device's services.
    reader: Mutex<Reader>,
    /// Where the program offering it registers and waits for calls.
    registry: Arc<Registry>,
    /// The asks it took, so none is taken twice.
    once: carried::Once,
    /// What insights was told before it knew who's who, and so whether it's listed yet.
    untold: Mutex<Vec<Value>>,
    /// What its app declares its program may reach, as oned gave it (`declared.json`), if it did.
    declared: Option<::services::sandbox::Declared>,
}

/// Where it keeps what its reader of who's who read to, in its folder.
const KEPT: &str = "whos-who.json";

/// How many things it tells insights it holds on to before it knows who's who.
const MOST_UNTOLD: usize = 32;

/// Where oned puts what a service's app declares it may reach, in its folder.
pub const DECLARED: &str = "declared.json";

/// How long its runtime may take to answer anything but a long poll for asks or a call.
const RUNTIME_WAIT: Duration = Duration::from_secs(30);


impl Service {
    /// The service `name` kept in `dir`: its key (made the first time), the network oned gave it, and its device's app
    /// runtime at `runtime`.
    pub fn open(dir: &Path, name: &str, runtime: &str) -> Result<Service> {
        ensure!(scopes::valid_app_name(name), "a service's name is lowercase words joined by -");
        std::fs::create_dir_all(dir)?;
        platform::os::set_mode(dir, 0o700)?;
        let key_file = dir.join("key");
        let key = match std::fs::read_to_string(&key_file) {
            Ok(secret) => SigningKey::from_bytes(&unhex(secret.trim()).context("its key file isn't a key")?),
            Err(_) => {
                let key = SigningKey::from_bytes(&rand::random());
                write_private(&key_file, hex(&key.to_bytes()).as_bytes())?;
                key
            }
        };
        let network: Network = serde_json::from_slice(&std::fs::read(Network::path(dir)).with_context(|| format!("{name} is started by its device's oned, which says which network it's in"))?)?;
        let token = match std::fs::read_to_string(dir.join(::api::local::LOCAL_TOKEN)) {
            Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => {
                let t = hex(&rand::random::<[u8; 32]>());
                write_private(&dir.join(::api::local::LOCAL_TOKEN), t.as_bytes())?;
                t
            }
        };
        let root = Hash::from_hex(&network.root)?;
        let (reader, members) = reader_kept(root, &dir.join(KEPT));
        Ok(Service(Arc::new(Inner {
            name: name.to_string(),
            dir: dir.to_path_buf(),
            key,
            network,
            runtime: runtime.to_string(),
            known: Mutex::new(members),
            reader: Mutex::new(reader),
            registry: Registry::new(name, &token),
            once: carried::Once::default(),
            untold: Mutex::default(),
            declared: std::fs::read(dir.join(DECLARED)).ok().and_then(|b| serde_json::from_slice(&b).ok()),
        })))
    }

    pub fn key(&self) -> Key {
        *self.0.key.verifying_key().as_bytes()
    }

    pub fn endpoint(&self) -> ::key::Endpoint {
        ::key::Endpoint::of(&self.0.key)
    }

    /// Where the program offering it registers.
    pub fn services(&self) -> &Services {
        self.0.registry.services()
    }

    pub fn token(&self) -> &str {
        self.0.registry.token()
    }

    /// Whose service it is, once a user's device approved it.
    pub fn owner(&self) -> Result<Option<String>> {
        self.owner_in(&self.0.known.lock().unwrap())
    }

    fn owner_in(&self, m: &ScopeDb) -> Result<Option<String>> {
        // Only a listing as the user whose link it was given counts: nobody else can make it theirs.
        Ok(m.service(&self.key())?.filter(|s| s.service.as_deref() == Some(self.0.name.as_str()) && s.user == self.0.network.user).map(|s| s.user))
    }

    /// The device it's part of, once who's who lists it.
    fn device(&self) -> Option<Key> {
        self.0.known.lock().unwrap().service(&self.key()).ok().flatten().and_then(|s| s.device)
    }

    /// Starts it: the loopback the program offering it registers at, its socket to its device's app runtime, and
    /// asking to be listed until it is. → that loopback's address.
    pub async fn start(&self) -> Result<std::net::SocketAddr> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let at = listener.local_addr()?;
        let router = self.loopback();
        // A program in a sandbox reaches it through a socket in its sandbox's folder.
        #[cfg(target_os = "linux")]
        if matches!(::services::sandbox::how(self.0.declared.as_ref(), ""), ::services::sandbox::How::Unit | ::services::sandbox::How::User) {
            use std::os::unix::fs::PermissionsExt;
            let dir = ::services::sandbox::dir(&self.0.name);
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(::services::sandbox::SOCKET);
            let _ = std::fs::remove_file(&path);
            let socket = tokio::net::UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
            let router = router.clone();
            tokio::spawn(async move {
                if let Err(err) = axum::serve(socket, router).await {
                    tracing::warn!("the service's socket stopped: {err:#}");
                }
            });
            // What a unit's program says, which systemd would keep where its user can't read it.
            let path = dir.join(::services::sandbox::LOG);
            let _ = std::fs::remove_file(&path);
            let log = tokio::net::UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
            tokio::spawn(async move {
                while let Ok((mut said, _)) = log.accept().await {
                    tokio::spawn(async move {
                        // As it says it, to stderr and the log.
                        let mut line = String::new();
                        let mut said = tokio::io::BufReader::new(&mut said);
                        while tokio::io::AsyncBufReadExt::read_line(&mut said, &mut line).await.is_ok_and(|n| n > 0) {
                            eprint!("{line}");
                            ::device_logs::record(&line);
                            line.clear();
                        }
                    });
                }
            });
        }
        tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, router).await {
                tracing::warn!("the service's loopback stopped: {err:#}");
            }
        });
        tokio::spawn(self.clone().keep_socket());
        tokio::spawn(self.clone().ask_to_be_listed());
        Ok(at)
    }

    /// Asks its device's app runtime `method` of `path` with the token in the runtime's folder, waiting `wait`: → what
    /// it answered, and its status.
    async fn runtime(&self, method: &'static str, path: &str, body: Option<Value>, wait: Duration) -> Result<(u16, Value)> {
        let url = format!("http://{}{path}", self.0.runtime);
        tokio::task::spawn_blocking(move || {
            let token = ::api::local::local_token(&::api::home::runtime_folder(&platform::home())).context("its device's app runtime has no token yet")?;
            let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(wait)).build().into();
            let mut resp = match body {
                Some(body) => agent.post(&url).header(::custom_build::protocol::header::LOCAL_TOKEN, token.trim()).send_json(body)?,
                None if method == "GET" => agent.get(&url).header(::custom_build::protocol::header::LOCAL_TOKEN, token.trim()).call()?,
                None => agent.post(&url).header(::custom_build::protocol::header::LOCAL_TOKEN, token.trim()).send_empty()?,
            };
            let status = resp.status().as_u16();
            let said: Value = resp.body_mut().with_config().limit(64 << 20).read_json().unwrap_or(Value::Null);
            anyhow::Ok((status, said))
        })
        .await?
    }

    /// Reads `global`'s scope database's steps its runtime has after the ones it read (`{ at, step }` each), each
    /// checked as from anyone from the first freeze naming the network's root (`Truth::first`), then who's who from them.
    async fn read_whos_who(&self) -> Result<()> {
        let from = self.0.reader.lock().unwrap().at();
        let (status, said) = self.runtime("GET", &format!("/api/services/steps?from={from}"), None, RUNTIME_WAIT).await?;
        ensure!(status == 200, "its runtime didn't give who's who ({status}): {}", said["error"].as_str().unwrap_or(""));
        self.take_steps(&said["steps"])
    }

    fn take_steps(&self, given: &Value) -> Result<()> {
        let mut known = self.0.known.lock().unwrap();
        let mut reader = self.0.reader.lock().unwrap();
        let before = reader.at();
        let now = now_ms();
        for s in given.as_array().into_iter().flatten() {
            let at = s["at"].as_u64().unwrap_or(0);
            if at == reader.at() + 1 {
                let bytes = s["step"].as_str().and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok());
                reader.take(::scope::steps::given(&scopes::Scope::Global, at, bytes.as_deref()).as_ref(), now)?;
            }
        }
        if reader.at() == before {
            return Ok(());
        }
        let kept = reader.kept()?;
        write_private(&self.0.dir.join(KEPT), &serde_json::to_vec(&kept)?)?;
        *known = whos_who(&kept)?;
        drop((known, reader));
        self.tell_untold();
        Ok(())
    }

    /// Its socket to its device's app runtime, for good: registered there as what the program offering it says it
    /// is, once it has, then taking each ask the runtime carries to it. While none comes it reads who's who again.
    async fn keep_socket(self) {
        let name = self.0.name.clone();
        loop {
            let registered = self.services().describe().into_iter().next();
            let Some(mut describe) = registered else {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };
            describe["runs"] = json!("service");
            if let Err(err) = self.read_whos_who().await {
                tracing::debug!("{name} reading who's who: {err:#}");
            }
            match self.runtime("POST", &format!("/api/services/carry/{name}"), Some(json!({ "describe": describe })), RUNTIME_WAIT).await {
                Ok((200, _)) => {}
                Ok((status, said)) => {
                    tracing::debug!("{name} registering with its runtime ({status}): {}", said["error"].as_str().unwrap_or(""));
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
                Err(err) => {
                    tracing::debug!("{name} registering with its runtime: {err:#}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            }
            match self.runtime("GET", &format!("/api/services/carry/{name}"), None, Duration::from_secs(40)).await {
                Ok((200, said)) => match serde_json::from_value::<carried::Carried>(said) {
                    Ok(ask) => {
                        let service = self.clone();
                        tokio::spawn(async move { service.take(ask).await });
                    }
                    Err(err) => tracing::debug!("{name}: what its runtime carried: {err:#}"),
                },
                Ok((204, _)) => {}
                Ok((status, said)) => {
                    tracing::debug!("{name} asking its runtime for asks ({status}): {}", said["error"].as_str().unwrap_or(""));
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(err) => {
                    tracing::debug!("{name} asking its runtime for asks: {err:#}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    /// Answers an ask its runtime carried: opened and checked (`carried::open_ask`), run if its signer may, and the
    /// answer signed and sealed to it. One that can't be opened is answered with why, which says nothing of a call.
    async fn take(&self, ask: carried::Carried) {
        let answered = async {
            let sealed = carried::unb64(&ask.ask)?;
            let (signer, body, at) = carried::open_ask(&self.0.key, &sealed, &self.0.once)?;
            // A node this service hasn't read of yet: who's who is read again first.
            if !self.is_node(&signer) {
                self.read_whos_who().await?;
            }
            let answer = self.run(signer, &body["call"], body["traceparent"].as_str()).await.unwrap_or_else(|err| json!({ "error": format!("{err:#}") }));
            let to = self.0.known.lock().unwrap().machine(&signer)?.map(|m| ::key::Endpoint { key: m.key, seal: m.seal }).context("who asked isn't one of the network's devices")?;
            anyhow::Ok(json!({ "answer": carried::b64(&carried::answer(&self.0.key, &to, at, answer)?) }))
        }
        .await
        .unwrap_or_else(|err| json!({ "error": format!("{err:#}") }));
        let name = &self.0.name;
        if let Err(err) = self.runtime("POST", &format!("/api/services/carry/{name}/{}", ask.id), Some(answered), RUNTIME_WAIT).await {
            tracing::debug!("{name} answering an ask: {err:#}");
        }
    }

    /// Asks its device's oned to list it, once the program offering it has said what it is (so whether it serves
    /// everyone): `ask` in its folder, its key and seal key, which oned reads and lists, until it's listed as its
    /// user's and as part of a device (one listed before parts named their device is listed again). Who's who is read
    /// again each time until it is.
    async fn ask_to_be_listed(self) {
        let ask = self.0.dir.join("ask");
        loop {
            let registered = !self.services().describe().is_empty();
            let listed = match (self.owner(), self.0.known.lock().unwrap().service(&self.key())) {
                (Ok(owner), Ok(service)) => owner.is_some() && service.is_some_and(|s| s.device.is_some()),
                (owner, service) => {
                    tracing::warn!("reading whether {} is listed: {:#}", self.0.name, owner.err().or(service.err()).expect("one failed"));
                    false
                }
            };
            if listed || !registered {
                let _ = std::fs::remove_file(&ask);
            } else {
                if !ask.exists() {
                    let me = self.endpoint();
                    let said = json!({ "key": hex(&me.key), "seal": hex(&me.seal) });
                    if let Err(err) = write_private(&ask, said.to_string().as_bytes()) {
                        tracing::warn!("asking oned to list {}: {err:#}", self.0.name);
                    }
                }
                if let Err(err) = self.read_whos_who().await {
                    tracing::debug!("{} reading who's who: {err:#}", self.0.name);
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Whether `key` is a node of the network, as this service knows it.
    fn is_node(&self, key: &Key) -> bool {
        // Who's who that can't answer lists no one, so no one is let in.
        self.0.known.lock().unwrap().compute(key).map(|d| d.is_some()).unwrap_or_else(|err| {
            tracing::warn!("reading whether {} is a node: {err:#}", short(key));
            false
        })
    }

    /// Runs a call a node asked for, if it may: → `{ ok }`, `{ error }`, or `{ elsewhere }` for a call this service
    /// can't run here.
    async fn run(&self, remote: Key, call: &Value, traceparent: Option<&str>) -> Result<Value> {
        let me = &call["me"];
        let (key, user) = match (me["key"].as_str().map(unhex), me["user"].as_str()) {
            (Some(Ok(key)), Some(user)) => (key, user.to_string()),
            _ => return Ok(json!({ "error": "who's asking?" })),
        };
        let caller = {
            let known = self.0.known.lock().unwrap();
            let m = &*known;
            if let Err(err) = may_call(m, &self.key(), &remote, &key, &user) {
                return Ok(json!({ "error": format!("{err:#}") }));
            }
            let member = m.user(&user)?;
            ::scope::state::Me {
                key: hex(&key),
                user: user.clone(),
                name: m.machine(&key)?.map(|d| d.name).unwrap_or_default(),
                role: member.as_ref().map(|u| u.role),
                user_name: member.as_ref().map(|u| u.name.clone()).unwrap_or_default(),
                handle: member.map(|u| u.handle).unwrap_or_default(),
                agent: me["agent"].as_str().map(str::to_string),
            }
        };
        let input = if call["input"].is_null() { json!({}) } else { call["input"].clone() };
        let from = call["from"].as_str().map(str::to_string);
        Ok(match self.services().call(&self.0.name, call["function"].as_str().unwrap_or(""), input, caller, from, traceparent.map(str::to_string)).await {
            Ok(answer) => json!({ "ok": answer.ok, "blobs": answer.blobs }),
            Err(err) if err.is::<::services::Elsewhere>() => json!({ "elsewhere": err.to_string() }),
            Err(err) => match ::custom_build::protocol::Coded::of(&err) {
                Some(code) => json!({ "error": format!("{err:#}"), "code": code }),
                None => json!({ "error": format!("{err:#}") }),
            },
        })
    }

    /// Asks its device's app runtime `body` (`{ call }` or `{ insights }`), signed by this service: → its answer.
    async fn ask_runtime(&self, body: Value, wait: Duration) -> Result<Value> {
        let device = self.device().with_context(|| format!("{} isn't listed as a part of its device yet", self.0.name))?;
        let signed = carried::own_ask(&self.0.key, &device, body);
        let (status, said) = self.runtime("POST", "/api/services/ask", Some(signed), wait).await?;
        ensure!(status == 200, "its device's app runtime refused it ({status}): {}", said["error"].as_str().unwrap_or(""));
        Ok(said)
    }

    /// Tells this service's own device what insights keeps, in the background, or once it knows who's who and so
    /// whether it's listed. Telemetry may be lost, and the program that said it doesn't wait.
    pub fn tell_node(&self, body: Value, runtime: &tokio::runtime::Handle) {
        if self.device().is_none() {
            let mut untold = self.0.untold.lock().unwrap();
            if untold.len() < MOST_UNTOLD {
                untold.push(body);
            }
            return;
        }
        let service = self.clone();
        runtime.spawn(async move {
            if let Err(err) = service.ask_runtime(json!({ "insights": body }), RUNTIME_WAIT).await {
                tracing::debug!("telling insights: {err:#}");
            }
        });
    }

    /// Who's who has come: whatever was waiting for it goes.
    fn tell_untold(&self) {
        if self.device().is_none() {
            return;
        }
        for body in std::mem::take(&mut *self.0.untold.lock().unwrap()) {
            self.tell_node(body, &tokio::runtime::Handle::current());
        }
    }

    /// Asks its device's app runtime to run a call, as its user. A service asks only for its own app, in its user's own
    /// scope (`limited`).
    pub async fn ask_node(&self, call: Value) -> Result<Value> {
        let owner = self.owner()?.context("this service isn't approved yet")?;
        let mut call = ::services::limited(&self.0.name, &call)?;
        call["me"] = json!({ "key": hex(&self.key()), "user": owner });
        self.ask_runtime(json!({ "call": call }), Duration::from_secs(600)).await
    }

    /// The loopback the program offering it registers at, with the local token in its folder: the services' API, its
    /// machine as `/api/state` says, and `/api/call`, which a node of its user's runs.
    fn loopback(&self) -> Router {
        Router::new()
            .route("/api/state", get(state))
            .route("/api/call", post(call))
            // A file is as big as it is (a browser profile is hundreds of MB): the node gives each user their room.
            .route("/api/blobs", post(put_blob).layer(axum::extract::DefaultBodyLimit::disable()))
            .route("/api/blobs/{hash}", get(get_blob))
            // Standard OTLP/JSON from its program, as `OTEL_EXPORTER_OTLP_ENDPOINT` says (`program_env`).
            .route("/v1/traces", post(otlp))
            .route("/v1/logs", post(otlp))
            .route("/v1/metrics", post(otlp))
            // Its program's way out, from its sandbox: `CONNECT host:port`, to a host its app declares.
            .fallback(proxy)
            .with_state(self.clone())
            .merge(self.0.registry.routes())
    }

    /// What `/api/state` says here: this service as the device, and the others offering it, as far as it knows.
    fn state(&self) -> Result<Value> {
        let known = self.0.known.lock().unwrap();
        let m = &*known;
        // The devices it runs on, each the device it's part of there. A service's listing outlives its being switched off, so that's
        // read from the switches, not the listing; whether one that's on is online, it can't know: asked, one that isn't says so.
        let listed = m.service(&self.key())?;
        let device = listed.as_ref().and_then(|s| s.device);
        // As a node reads its own parts: a process of oned's is on by its entry.
        let switched_on = |d: &Key| match m.switch(d, &self.0.name) {
            Ok(Some(s)) => s.on,
            Ok(None) => false,
            Err(_) => true,
        };
        let devices: Vec<Value> = m
            .offering(&self.0.name)?
            .into_iter()
            .map(|s| s.device.map(|d| m.machine(&d)).transpose().map(Option::flatten))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .filter(|d| Some(d.key) == device || switched_on(&d.key))
            .map(|d| json!({ "key": hex(&d.key), "name": d.name, "here": Some(d.key) == device, "online": true, "services": [self.0.name] }))
            .collect();
        // The scopes its device runs, whose part it is: a program here may reach what they hold.
        let runs_for: Vec<String> = m.scopes()?.into_iter().filter(|s| device.is_some_and(|d| m.runs_for(&d, s))).map(|s| s.to_string()).collect();
        let name = device.map(|d| m.machine(&d)).transpose()?.flatten().map(|d| d.name).unwrap_or_default();
        Ok(json!({
            "device": { "key": device.map(|d| hex(&d)).unwrap_or_default(), "name": name, "runs_for": runs_for },
            "me": { "key": hex(&self.key()), "name": name },
            "user": self.owner_in(m)?.map(|u| m.user(&u).map(|p| json!({ "id": u, "name": p.map(|p| p.name) }))).transpose()?,
            "network": { "name": m.name()?, "root": self.0.network.root, "devices": devices, "settings": ::scope::state::settings(&m.settings()?) },
        }))
    }
}

/// Whether a node, `remote`, may ask this service (`me`) for a call by `user` from their device `key`: this service
/// serves them (its own user, or everyone, as its listing says), and the node is one of their own, calling for its
/// own user. A node of someone else's, an admin's too, never calls as them.
pub fn may_call(m: &ScopeDb, me: &Key, remote: &Key, key: &Key, user: &str) -> Result<()> {
    let service = m.service(me)?.with_context(|| format!("this service isn't one of {YOURS}'s yet"))?;
    let node = m.compute(remote)?.with_context(|| format!("only {YOURS}'s devices are answered"))?;
    allowed!(m.may(user, "use", &scopes::service(me)), "{} serves only its own user", service.service.as_deref().unwrap_or("this service"));
    let own = node.user == user && (*key == *remote || m.client(key)?.is_some_and(|c| c.user == user));
    ensure!(own, "{} may not call as that user", short(remote));
    Ok(())
}

/// A reader of who's who in the network `root`, from what it kept at `path`, and who's who as it read. One it can't read
/// starts over, and nodes bring the steps again.
fn reader_kept(root: Hash, path: &Path) -> (Reader, ScopeDb) {
    let kept: Value = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut reader = Reader::new(Some(root));
    let restarted = if kept.is_null() { Ok(()) } else { reader.restart(&kept, now_ms()) };
    match restarted.and_then(|_| whos_who(&kept)) {
        Ok(members) => (reader, members),
        Err(err) => {
            tracing::warn!("reading who's who as it was kept: {err:#}");
            (Reader::new(Some(root)), ScopeDb::empty(&scopes::Scope::Global, 0).expect("an empty scope database builds"))
        }
    }
}

/// Who's who, from what a reader `kept` (`Reader::kept`): no one, before it's read its freeze.
fn whos_who(kept: &Value) -> Result<ScopeDb> {
    match kept.get("truth").filter(|t| !t.is_null()) {
        Some(truth) => Ok(Truth::kept(truth, now_ms())?.db),
        None => ScopeDb::empty(&scopes::Scope::Global, 0),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

async fn proxy(State(s): State<Service>, req: axum::extract::Request) -> Response {
    ::services::sandbox::connect(s.0.declared.as_ref(), req).await
}

async fn state(State(s): State<Service>, headers: HeaderMap) -> Response {
    if let Err(r) = s.0.registry.ours(&headers) {
        return r;
    }
    match s.state() {
        Ok(state) => Json(state).into_response(),
        Err(err) => refused(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err:#}")),
    }
}

/// OTLP from the program, JSON or protobuf: passed to its node, which keeps what closed sets allow
/// (`::insights::otlp`), said back as taken whatever happens to it, as an exporter would only retry. A program's
/// metrics aren't kept: what's a metric is the node's own to say.
async fn otlp(State(s): State<Service>, uri: axum::http::Uri, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if let Err(r) = s.0.registry.ours(&headers) {
        return r;
    }
    let protobuf = headers.get(axum::http::header::CONTENT_TYPE).is_some_and(|t| t.as_bytes().starts_with(b"application/x-protobuf"));
    let request = if protobuf { ::insights::otlp::from_protobuf(uri.path(), &body) } else { serde_json::from_slice::<Value>(&body).ok() };
    if let Some(request) = request.filter(|r| r.get("resourceMetrics").is_none()) {
        s.tell_node(json!({ "otlp": request }), &tokio::runtime::Handle::current());
    }
    if protobuf {
        // An empty message: what OTLP answers in protobuf.
        return ([(axum::http::header::CONTENT_TYPE, "application/x-protobuf")], axum::body::Bytes::new()).into_response();
    }
    Json(json!({})).into_response()
}

async fn call(State(s): State<Service>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(r) = s.0.registry.ours(&headers) {
        return r;
    }
    match s.ask_node(body).await {
        Ok(answer) if answer.get("error").is_some() => refused(StatusCode::BAD_REQUEST, answer["error"].as_str().unwrap_or("failed")),
        Ok(answer) => Json(json!({ "ok": answer["ok"] })).into_response(),
        Err(err) => refused(StatusCode::BAD_GATEWAY, format!("{err:#}")),
    }
}

/// A blob its program keeps (`POST /api/blobs`) or reads (`GET /api/blobs/<hash>`), through its own device's oned, as
/// a service in oned's own process does: one it keeps is named in its call's answer, which the call's node keeps.
async fn put_blob(State(s): State<Service>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    blobs(&s, &headers, "POST", "/api/blobs", Some(body.to_vec())).await
}

async fn get_blob(State(s): State<Service>, headers: HeaderMap, UrlPath(hash): UrlPath<String>) -> Response {
    blobs(&s, &headers, "GET", &format!("/api/blobs/{hash}"), None).await
}

async fn blobs(s: &Service, headers: &HeaderMap, method: &'static str, path: &str, body: Option<Vec<u8>>) -> Response {
    if let Err(r) = s.0.registry.ours(headers) {
        return r;
    }
    let url = format!("http://{}{path}", ::api::local::addr());
    let asked = tokio::task::spawn_blocking(move || -> Result<(u16, Vec<u8>)> {
        let token = std::fs::read_to_string(platform::home().join(::api::local::LOCAL_TOKEN)).context("its device's oned has no local token")?;
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let resp = match body {
            Some(body) => agent.post(&url).header(::custom_build::protocol::header::LOCAL_TOKEN, token.trim()).header("content-type", "application/octet-stream").send(&body[..])?,
            None => agent.get(&url).header(::custom_build::protocol::header::LOCAL_TOKEN, token.trim()).call()?,
        };
        let status = resp.status().as_u16();
        Ok((status, resp.into_body().with_config().limit(u64::MAX).read_to_vec()?))
    })
    .await;
    match asked {
        Ok(Ok((status, bytes))) => {
            let kind = if method == "GET" && status == 200 { "application/octet-stream" } else { "application/json" };
            (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), [(axum::http::header::CONTENT_TYPE, kind)], bytes).into_response()
        }
        Ok(Err(err)) => refused(StatusCode::BAD_GATEWAY, format!("its device's oned: {err:#}")),
        Err(err) => refused(StatusCode::BAD_GATEWAY, format!("{err:#}")),
    }
}

/// Whether this part should stop: this program's binary was replaced, by an update, so the new oned starts it again;
/// or asked to.
fn stop_check() -> Result<impl Fn() -> bool> {
    // Asked to stop (oned switching it off): it stops what it started first, on a Mac too, where nothing else would.
    #[cfg(unix)]
    {
        extern "C" fn asked(_: libc::c_int) {
            ASKED.store(true, SeqCst);
        }
        // SAFETY: the handler only stores to an atomic.
        unsafe { libc::signal(libc::SIGTERM, asked as *const () as libc::sighandler_t) };
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    let id = platform::os::file_id(&std::fs::metadata(&exe)?);
    Ok(move || ASKED.load(SeqCst) || std::fs::metadata(&exe).map(|m| platform::os::file_id(&m)).ok() != Some(id))
}

static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The process id of this part's running program, for `die_with_oned`; 0 when none is.
static PROGRAM: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Ends this part, and its program asked to stop, the moment the oned that started it is gone, however it went: oned
/// keeps the other end of the pipe that is this part's stdin, and closes it by dying. A thread of its own does it, so a
/// part whose runtime or loops are stuck (a Mac out of memory) still goes, and with it its lock; polling the parent
/// from those loops didn't. Linux ends the program with it (`PR_SET_PDEATHSIG`); a Mac has nothing like that.
#[cfg(windows)]
fn die_with_oned() -> Result<()> {
    use std::io::Read;
    // Its program goes with it: oned's job takes every process it starts (`os::children_end_with_me`).
    std::thread::spawn(|| {
        while std::io::stdin().read(&mut [0u8; 64]).is_ok_and(|n| n > 0) {}
        std::process::exit(1);
    });
    Ok(())
}

#[cfg(unix)]
fn die_with_oned() -> Result<()> {
    use std::io::Read;
    use std::os::fd::{AsFd, AsRawFd};
    // The copy is closed on exec: what this part starts gets /dev/null, not oned's pipe.
    let mut pipe = std::fs::File::from(std::io::stdin().as_fd().try_clone_to_owned()?);
    let null = std::fs::File::open("/dev/null")?;
    // SAFETY: two open descriptors this process owns.
    ensure!(unsafe { libc::dup2(null.as_raw_fd(), 0) } == 0, "{}", std::io::Error::last_os_error());
    std::thread::spawn(move || {
        while pipe.read(&mut [0u8; 64]).is_ok_and(|n| n > 0) {}
        let program = PROGRAM.load(SeqCst);
        // SAFETY: a signal to this part's own child, named in `PROGRAM` only while it hasn't been reaped; and an exit
        // that runs nothing, which a stuck process might wait on.
        unsafe {
            if program > 0 {
                libc::kill(program, libc::SIGTERM);
            }
            libc::_exit(1)
        }
    });
    Ok(())
}

/// `oned <name> --runtime=<address>`: the service `name` as a part of this device's oned, which started it, reached
/// through its device's app runtime at that address, until oned stops or this program is replaced: its program (`oned::program`), started here and again whenever it stops, told where to register (`ONE_ADDR`), where the token it sends is (`ONE_HOME`,
/// the service's folder) and where its data goes (`<NAME>_DATA`). One runs at a time: a part an update's oned starts
/// waits for the one before to stop.
pub fn run(name: &str, runtime: &str) -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "one=info,warn".into());
    // What it and its program say is kept too, for `one logs --service <name>`.
    ::device_logs::keep(name);
    let _ = tracing_subscriber::fmt().with_writer(::device_logs::Tee).with_env_filter(filter).try_init();
    let dir = folder(name);
    std::fs::create_dir_all(&dir)?;
    die_with_oned()?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("lock"))?;
    // Held until this process exits, however it does. While another holds it, this waits; the oned that started this
    // one going ends it (`die_with_oned`), even here.
    lock.lock().with_context(|| name.to_string())?;
    let stop = stop_check()?;
    let program = crate::program(name);
    ensure!(program.exists(), "{name}'s program, {}, isn't there", program.display());
    let tokio = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let service = Service::open(&dir, name, runtime)?;
    let at = tokio.block_on(service.start())?;
    let data = platform::home().join(name);
    std::fs::create_dir_all(&data)?;
    host(name, &program, &data, at, &service, &stop, tokio.handle())?;
    eprintln!("{name}: stopping, for oned to start it again");
    Ok(())
}

/// Where the part says why its program can't start in its sandbox, in its folder, which oned shows on it.
pub const PROBLEM: &str = "problem";

enum Running {
    Child(std::process::Child),
    Unit(String),
    /// A Mac helper app's program (`oned::helper`), a process macOS holds responsible for itself.
    #[cfg(target_os = "macos")]
    Own(libc::pid_t),
}

impl Running {
    /// Its program's process id, if it is one of this process's children.
    fn pid(&self) -> i32 {
        match self {
            Running::Child(child) => child.id() as i32,
            Running::Unit(_) => 0,
            #[cfg(target_os = "macos")]
            Running::Own(pid) => *pid,
        }
    }

    fn done(&mut self) -> Result<Option<String>> {
        Ok(match self {
            Running::Child(child) => child.try_wait()?.map(|s| s.to_string()),
            Running::Unit(unit) => {
                let state = ::services::sandbox::systemctl(&["is-active", unit])?;
                (!["active", "activating", "reloading"].contains(&state.as_str())).then_some(state)
            }
            #[cfg(target_os = "macos")]
            Running::Own(pid) => {
                let mut status = 0;
                // SAFETY: waits without blocking for a child of this process.
                match unsafe { libc::waitpid(*pid, &mut status, libc::WNOHANG) } {
                    0 => None,
                    -1 => Some("gone".into()),
                    _ => Some(format!("wait status {status}")),
                }
            }
        })
    }

    fn end(&mut self) {
        match self {
            Running::Child(child) => end(child),
            Running::Unit(unit) => {
                if let Err(err) = ::services::sandbox::systemctl(&["stop", unit]) {
                    eprintln!("stopping {unit}: {err:#}");
                }
            }
            #[cfg(target_os = "macos")]
            Running::Own(pid) => {
                // SAFETY: signals to, and waits for, a child of this process, which it hasn't reaped.
                unsafe {
                    libc::kill(*pid, libc::SIGTERM);
                    for _ in 0..100 {
                        if libc::waitpid(*pid, std::ptr::null_mut(), libc::WNOHANG) != 0 {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    libc::kill(*pid, libc::SIGKILL);
                    libc::waitpid(*pid, std::ptr::null_mut(), 0);
                }
            }
        }
    }
}

/// Starts `command` as a process macOS holds responsible for itself, not for this one: so a Mac helper app's
/// permissions are asked for and recorded under its own name, not the Mac app's that started oned. Its id.
#[cfg(target_os = "macos")]
fn disclaimed(command: &std::process::Command) -> Result<libc::pid_t> {
    use std::ffi::{CString, OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        // libSystem's, as Chromium and LLDB start what they run.
        fn responsibility_spawnattrs_setdisclaim(attr: *mut libc::posix_spawnattr_t, disclaim: libc::c_int) -> libc::c_int;
    }
    let c = |s: &OsStr| CString::new(s.as_bytes());
    let program = c(command.get_program())?;
    let args: Vec<CString> = std::iter::once(command.get_program()).chain(command.get_args()).map(c).collect::<Result<_, _>>()?;
    let mut vars: std::collections::BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (k, v) in command.get_envs() {
        match v {
            Some(v) => vars.insert(k.into(), v.into()),
            None => vars.remove(k),
        };
    }
    let env: Vec<CString> = vars.iter().map(|(k, v)| CString::new([k.as_bytes(), b"=", v.as_bytes()].concat())).collect::<Result<_, _>>()?;
    let pointers = |all: &[CString]| all.iter().map(|a| a.as_ptr() as *mut libc::c_char).chain(std::iter::once(std::ptr::null_mut())).collect::<Vec<_>>();
    let (argv, envp) = (pointers(&args), pointers(&env));
    // SAFETY: the attributes are made, used and destroyed here; every string outlives the call.
    unsafe {
        let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
        ensure!(libc::posix_spawnattr_init(&mut attr) == 0, "posix_spawnattr_init failed");
        let mut pid = 0;
        let mut r = responsibility_spawnattrs_setdisclaim(&mut attr, 1);
        if r == 0 {
            r = libc::posix_spawn(&mut pid, program.as_ptr(), std::ptr::null(), &attr, argv.as_ptr(), envp.as_ptr());
        }
        libc::posix_spawnattr_destroy(&mut attr);
        ensure!(r == 0, "{}", std::io::Error::from_raw_os_error(r));
        Ok(pid)
    }
}

/// Starts a service's program, in its sandbox as its app declares and this machine can give it.
fn begin(name: &str, program: &Path, data: &Path, at: std::net::SocketAddr, service: &Service) -> Result<Running> {
    use ::services::sandbox::{How, Launch};
    let declared = service.0.declared.clone().unwrap_or_default();
    let launcher = platform::this_program()?;
    let mut command = match ::services::sandbox::how(service.0.declared.as_ref(), "") {
        How::Unit => {
            let launch = Launch { name: name.into(), own_network: !declared.any_host(), ports: declared.local_ports(), data: None };
            ::services::sandbox::prepare(name, program, &launcher, service.token(), &launch)?;
            let unit = ::services::sandbox::unit(name);
            // One left by an endpoint that stopped without stopping it goes first: two mustn't share its data.
            let _ = ::services::sandbox::systemctl(&["stop", &unit]);
            let _ = ::services::sandbox::systemctl(&["reset-failed", &unit]);
            ::services::sandbox::systemctl(&["start", &unit])?;
            return Ok(Running::Unit(unit));
        }
        How::User => {
            let launch = Launch { name: name.into(), own_network: !declared.any_host(), ports: declared.local_ports(), data: Some(data.to_path_buf()) };
            let dir = ::services::sandbox::prepare(name, program, &launcher, service.token(), &launch)?;
            crate::command(&[&format!("--sandboxed={}", dir.display())])?
        }
        How::Seatbelt => {
            let token = service.0.dir.join(::api::local::LOCAL_TOKEN);
            let profile = ::services::sandbox::seatbelt(&declared, at.port(), data, &token, program);
            let mut command = std::process::Command::new("/usr/bin/sandbox-exec");
            command.arg("-p").arg(profile).arg(program);
            let proxy = format!("http://{at}");
            for var in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
                command.env(var, &proxy);
            }
            command.env("NO_PROXY", "localhost,127.0.0.1").env("no_proxy", "localhost,127.0.0.1");
            if !declared.user {
                let tmp = data.join("tmp");
                std::fs::create_dir_all(&tmp)?;
                command.env("TMPDIR", tmp);
            }
            command
        }
        How::Plain(why) => {
            if let Some(why) = why {
                eprintln!("{name} runs without its sandbox: {why}");
            }
            std::process::Command::new(program)
        }
    };
    command.envs(::services::program_env(name, at, &service.0.dir, data));
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // It stops with this part, even one killed: two of a service on the same data (WhatsApp's link) mustn't run.
        // SAFETY: only an async-signal-safe call, between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(target_os = "macos")]
    if crate::helper(name).is_some() {
        return Ok(Running::Own(disclaimed(&command).with_context(|| format!("starting {}", program.display()))?));
    }
    // What it says goes to this part's stderr as before, and to its log.
    command.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = command.spawn().with_context(|| format!("starting {}", program.display()))?;
    if let Some(out) = child.stdout.take() {
        ::device_logs::tee(out);
    }
    if let Some(err) = child.stderr.take() {
        ::device_logs::tee(err);
    }
    Ok(Running::Child(child))
}

/// Runs a service's program in another language, started again whenever it stops, until `stop`. One that can't start
/// in its sandbox says why (`PROBLEM`), and is tried again a minute on.
fn host(name: &str, program: &Path, data: &Path, at: std::net::SocketAddr, service: &Service, stop: &impl Fn() -> bool, runtime: &tokio::runtime::Handle) -> Result<()> {
    let built = || std::fs::metadata(program).ok().and_then(|m| Some((platform::os::file_id(&m), m.modified().ok()?)));
    let problem = service.0.dir.join(PROBLEM);
    let before = ::services::sandbox::dir(name).join(::services::sandbox::BEFORE);
    while !stop() {
        let started = built();
        let build = std::fs::read(program).map(|bytes| ::key::hex(&::key::sha256(&bytes))).unwrap_or_default();
        let told = |event: Value| service.tell_node(event, runtime);
        let mut running = match begin(name, program, data, at, service) {
            Ok(r) => {
                let _ = std::fs::remove_file(&problem);
                PROGRAM.store(r.pid(), SeqCst);
                told(json!({ "name": "service.start", "build": build }));
                r
            }
            Err(err) => {
                eprintln!("{name}: {err:#}");
                let _ = std::fs::write(&problem, format!("Couldn’t start it in its sandbox: {err:#}"));
                for _ in 0..60 {
                    if stop() {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                continue;
            }
        };
        loop {
            if let Some(status) = running.done()? {
                PROGRAM.store(0, SeqCst);
                service.services().program_stopped(name);
                told(json!({ "name": "service.exit", "build": build, "reason": exit_reason(&status) }));
                eprintln!("{} stopped ({status}): starting it again", program.display());
                std::thread::sleep(Duration::from_secs(2));
                break;
            }
            // Its data from before, moved into its sandbox's own folder before it started, is done with once it's up.
            if service.services().has(name) && before.exists() {
                let _ = std::fs::remove_dir_all(&before);
            }
            // Stopped for good, or for its new build, put in its place by hand (the try-a-service skill).
            let renewed = built() != started;
            if stop() || renewed {
                running.end();
                PROGRAM.store(0, SeqCst);
                service.services().program_stopped(name);
                told(json!({ "name": "service.exit", "build": build, "reason": if renewed { "renewed" } else { "stopped" } }));
                if renewed {
                    eprintln!("{}: a new build is there, so it starts again", program.display());
                }
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    Ok(())
}

/// Why a program stopped, as a closed set of words: `exit-<code>`, `signal-<n>`, or systemd's state for one it ran.
fn exit_reason(status: &str) -> String {
    let number = |after: &str| status.split(after).nth(1).and_then(|s| s.split_whitespace().next()).and_then(|n| n.parse::<u32>().ok());
    match (number("exit status: "), number("signal: ")) {
        (Some(code), _) => format!("exit-{code}"),
        (_, Some(n)) => format!("signal-{n}"),
        _ => "failed".into(),
    }
}

/// Stops a program: asked, then made to, if it hasn't in ten seconds, so it closes what it keeps (WhatsApp's link).
fn end(child: &mut std::process::Child) {
    platform::os::ask_to_stop(child);
    for _ in 0..100 {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}
