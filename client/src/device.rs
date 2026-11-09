//! A native app as a device of its user (ARCHITECTURE.md, Being built: Devices), the same on every app: its own key,
//! which the app keeps in the Keychain, the keystore or the Credential Locker and hands it at start; asking to be let in,
//! with the code it shows; and once a device of theirs approved it, asking its nodes with the keys it opens
//! (`crate::keyring`), keeping what outlives it in its folder. A browser runs the same keyring in its worker.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD as B64U;
use ed25519_dalek::SigningKey;
use one::config::YOURS;
use one::keys::{self, Endpoint, Waited, hex};
use serde_json::{Value, json};

use crate::answers::{Answers, Files};
use crate::join;
use crate::bucket::Signed;
pub(crate) use crate::keyring::now;
use crate::keyring::{Address, HANDED_FOR, Keep, Keyring, error, refused};
use crate::{Body, Client, Error, Request, Response, Sent, unsent};

const TRACES_KEPT: usize = 100;

/// What a sign-in for the device whose key is `secret` names it by (`join`), before it's made: its key ‖ seal key, hex.
pub fn join_of(secret: [u8; 32]) -> String {
    let me = Endpoint::of(&SigningKey::from_bytes(&secret));
    format!("{}{}", hex(&me.key), hex(&me.seal))
}

/// A note pushed to the device whose key is `secret`, as its push carried it (base64url): → the note, and the key of
/// the device that sent it (hex), or none for one from the network's Workers. Opened here alone: nothing connects or starts.
pub fn open_note(secret: [u8; 32], pushed: &str) -> Result<(one::push::Note, Option<String>), Error> {
    let sealed = B64U.decode(pushed.trim().trim_end_matches('=').as_bytes()).map_err(|_| failed("That push isn't a note."))?;
    one::push::open_note(&keys::seal_secret(&SigningKey::from_bytes(&secret)), &sealed).map_err(|_| failed("That note isn't this device's."))
}

pub fn new_secret() -> [u8; 32] {
    rand::random()
}

fn failed(message: impl std::fmt::Display) -> Error {
    unsent(message)
}

/// Where a device is with its network, from its sign-in to its sign-out, as a browser goes the same way (cloudflare/web:
/// browser.js `join` and `leave`, web.js `start`, request.js `takenOut`). Whether this key was ever let in here is kept on disk (`joined`), as it
/// outlives the program: `removed` asks it.
enum Stage {
    /// Not reaching its nodes: not let in yet, having asked auth with `asked` if it has since it started (`ask`, then
    /// `wait`); or let in, and its connections closed until it's asked again (`disconnect`: its app in the background,
    /// or a program ending).
    Off { asked: Option<Value> },
    In(Client),
    /// Taken out, as the relay said, or signed out: nothing more is asked as it, and its app forgets its key.
    Out,
}

/// What this client is built for, as it says in its listing.
pub const PLATFORM: &str = if cfg!(target_os = "ios") {
    "iphone"
} else if cfg!(target_os = "android") {
    "android"
} else if cfg!(target_os = "macos") {
    "mac"
} else if cfg!(windows) {
    "windows"
} else {
    "linux"
};

/// A device's way in: its keys, the network it's in, and what it has opened.
pub struct Device {
    key: SigningKey,
    me: Endpoint,
    address: String,
    /// Where it keeps what outlives it (`Keep`): whether it was let in (`joined`), developer mode, and the keyring's.
    dir: PathBuf,
    bucket: Signed,
    /// Held while it connects, so it connects once.
    stage: tokio::sync::Mutex<Stage>,
    keyring: Keyring,
    /// The order things run in, as the node last said (`/v1/state`): only ever used to sort nodes it knows.
    order: Mutex<Vec<[u8; 32]>>,
    /// The device it's on, as its app says (`on`): signing in there again replaces this sign-in.
    on: Mutex<Option<String>>,
    traces: Mutex<std::collections::VecDeque<Value>>,
    /// The last answer of each call that only reads, for its app's screens (`kept`): a native app's keeps them
    /// (`keeping`), the CLI's doesn't.
    answers: Option<Answers<Files>>,
    /// Where it keeps its log (`one::logs`), which a node asking for it is answered from.
    logs: Option<PathBuf>,
}

impl Device {
    pub fn new(secret: [u8; 32], address: &str, dir: &str) -> Device {
        let key = SigningKey::from_bytes(&secret);
        let me = Endpoint::of(&key);
        let address = address.trim_end_matches('/').to_string();
        Device {
            bucket: Signed::new(address.clone(), secret),
            keyring: Keyring::new(me.key, keys::seal_secret(&key)),
            me,
            key,
            address,
            dir: dir.into(),
            stage: tokio::sync::Mutex::new(Stage::Off { asked: None }),
            order: Mutex::default(),
            on: Mutex::default(),
            traces: Mutex::default(),
            answers: None,
            logs: None,
        }
    }

    /// Answers a node that asks for its log, from the log kept in `dir` (`one::logs::keep_in`).
    pub fn logging(mut self, dir: &std::path::Path) -> Device {
        self.logs = Some(dir.to_path_buf());
        self
    }

    /// Says which device it's on: that device's oned key (hex), on a machine with one, or an id of the device's own,
    /// hashed, that outlives signing out. Its request to join names it, and once it's in, its listing does.
    pub fn on(&self, device: &str) {
        if keys::unhex(device).is_ok() {
            *self.on.lock().unwrap() = Some(device.to_string());
        }
    }

    pub fn keeping(mut self) -> Device {
        self.answers = Some(Answers::new(Files::new(&self.dir, &self.key)));
        self
    }

    /// Whether this device is in developer mode, which only its own Settings › Developer changes: its requests then
    /// ask for traces, which its screens show.
    pub fn developer(&self) -> bool {
        self.dir.join("developer").exists()
    }

    /// Turns developer mode on or off, here alone.
    pub fn set_developer(&self, on: bool) {
        if on {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(self.dir.join("developer"), b"");
        } else {
            let _ = std::fs::remove_file(self.dir.join("developer"));
            self.traces.lock().unwrap().clear();
        }
    }

    /// Its latest requests' traces, oldest first, as JSON: each what was asked, when, how long it took, the nodes it
    /// tried and what the one that answered did (Developer's Timing draws them: src/screen/settings/devtools.jsx). None out of developer mode.
    pub fn traces(&self) -> Option<String> {
        self.developer().then(|| Value::Array(self.traces.lock().unwrap().iter().cloned().collect()).to_string())
    }

    fn keep_trace(&self, trace: Value) {
        let mut traces = self.traces.lock().unwrap();
        if traces.len() >= TRACES_KEPT {
            traces.pop_front();
        }
        traces.push_back(trace);
    }

    pub fn key(&self) -> String {
        hex(&self.me.key)
    }

    pub fn join(&self) -> String {
        format!("{}{}", self.key(), hex(&self.me.seal))
    }

    /// The code it shows to be approved: "ABCD EFGH IJKL MNOP".
    pub fn code(&self) -> String {
        keys::spaced_code(&keys::join_code(&self.me))
    }

    /// Asks auth to let it in, as the user signed in with `token`, calling it `name` (and with someone new's invite
    /// secret, its proof).
    pub async fn ask(&self, token: &str, name: &str, invite: Option<&str>) -> Result<(), Error> {
        let on = self.on.lock().unwrap().clone();
        let asked = join::asking(&self.me, name, on.as_deref(), Some(PLATFORM), invite);
        if let Stage::Off { asked: kept } = &mut *self.stage.lock().await {
            *kept = Some(asked.clone());
        }
        join::ask(self, token, &asked).await
    }

    /// Lets this device in with the network's recovery key (`recovery`, its secret), read from a file on this computer,
    /// when no device is left to say yes: it asks auth first (`ask`), which says whose it is, and then the recovery key
    /// signs it listed, as `database::rules` lets it for someone already in, each use an entry in the log. The recovery
    /// key does the rest, reaching the bucket and relay as itself, since this device can't yet: reads who's who as its
    /// own, seals this device each epoch it opens of what its user may use, and tells the relay to let it in.
    pub async fn admit_by_recovery(&self, recovery: &[u8; 32]) -> Result<(), Error> {
        use one::bucket::Signer;
        use one::database::scopes::Scope;
        use one::database::{content, steps, stored};
        let signing = SigningKey::from_bytes(recovery);
        let public = signing.verifying_key().to_bytes();
        let (secret, twin) = (keys::seal_secret(&signing), keys::twin_public(&public).map_err(failed)?);
        let bucket = Signed::new(self.address.clone(), *recovery);
        let mut reader = steps::Reader::new(None);
        let (_, step_keys) = steps::read_sealed_to(&bucket, &mut reader, "recovery", &secret, &mut None, now()).await.map_err(failed)?;
        let signer = std::sync::Arc::new(Signer::reading(self.address.clone(), signing.clone()));
        let asked_at = format!("{}/api/auth/devices/requests", self.address);
        let asking = keys::signed_request_by(&signing, one::protocol::AUTH_DOMAIN, json!({ "route": "GET /api/auth/devices/requests" }), now()).await.map_err(failed)?;
        let (status, listed) = http("GET", &asked_at, Some((one::protocol::header::REQUEST, B64U.encode(&serde_json::to_vec(&asking).map_err(failed)?))), None).await?;
        if status != 200 {
            return Err(failed(error(&listed).unwrap_or_else(|| format!("{YOURS}'s auth answered {status}."))));
        }
        let mine = listed["requests"].as_array().into_iter().flatten().find(|r| r["key"].as_str() == Some(self.key().as_str())).ok_or_else(|| failed("This device's request isn't at auth: sign in again."))?;
        let (user, name) = (mine["user"].as_str().unwrap_or_default().to_string(), mine["name"].as_str().unwrap_or_default().to_string());
        {
            let who = &reader.truth().ok_or_else(|| failed(format!("{YOURS} hasn't been read yet.")))?.db;
            if who.recovery().map_err(failed)? != Some(public) {
                return Err(failed("That isn't this network's recovery key."));
            }
            if who.user(&user).map_err(failed)?.is_none() {
                return Err(failed(format!("The recovery key lets in a device of someone already in {YOURS}.")));
            }
        }
        let mut listing = content::device(&self.me, &user, &name, &[one::database::scopes::Kind::Client], None);
        listing.data["platform"] = json!(PLATFORM);
        let mut wrote = false;
        for _ in 0..20 {
            let at = reader.at() + 1;
            let name_now = stored::step_name(&steps::prefix("global"), at);
            let epoch = reader.truth().map(|t| t.db.epoch(&Scope::Global)).transpose().map_err(failed)?.unwrap_or(0);
            let step = steps::sign(&signing, &Scope::Global, at, &user, epoch, now(), listing.clone()).map_err(failed)?;
            let bytes = steps::seal(&step_keys, &name_now, &steps::encode(&step, None)).map_err(failed)?;
            if create(signer.clone(), name_now, bytes).await.map_err(failed)? {
                wrote = true;
                break;
            }
            reader.read(&bucket, &step_keys, now()).await.map_err(failed)?;
        }
        if !wrote {
            return Err(failed("Who's who changed too often to write to: try again."));
        }
        let admin = reader.truth().is_some_and(|t| t.db.may(&user, "invite", one::database::scopes::NETWORK));
        let told = keys::signed_request_by(&signing, one::protocol::RELAY, json!({ "add": self.key(), "role": "client", "admin": admin }), keys::next_at(now())).await.map_err(failed)?;
        let (status, out) = http("POST", &format!("{}/relay/tell", self.address), None, Some(told)).await?;
        if !(200..300).contains(&status) && out["code"] != one::protocol::code::ALREADY {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{YOURS}'s relay answered {status}."))));
        }
        let scopes = reader.truth().map(|t| t.db.may_scopes(&user, "use")).transpose().map_err(failed)?.unwrap_or_default();
        for scope in scopes {
            let name = scope.to_string();
            let hashes = reader.truth().map(|t| t.db.epoch_hashes(&scope)).transpose().map_err(failed)?.unwrap_or_default();
            for (n, hash) in hashes {
                let mut kept = Vec::new();
                for at in keys::recovery_epoch_names(&name, n, &twin) {
                    kept.extend(one::bucket::Bucket::get(&bucket, &at).await.map_err(failed)?.map(|o| o.bytes));
                }
                let Some(key) = keys::opened_recovery_epoch(&secret, &hash, &kept) else { continue };
                create(signer.clone(), keys::epoch_name(&name, n, &self.key()), keys::seal_to(&self.me.seal, &key).map_err(failed)?).await.map_err(failed)?;
            }
        }
        // Its request, answered as its own: nothing else will, as nothing else let it in.
        let path = format!("/api/auth/devices/requests/{}", self.key());
        let root = reader.root().map(|r| r.to_hex().to_string()).unwrap_or_default();
        let unasked = keys::signed_request_by(self, one::protocol::AUTH_DOMAIN, json!({ "route": format!("DELETE {path}") }), now()).await.map_err(failed)?;
        let header = B64U.encode(&serde_json::to_vec(&unasked).map_err(failed)?);
        if let Err(err) = http("DELETE", &format!("{}{path}?approved&root={root}", self.address), Some((one::protocol::header::REQUEST, header)), None).await {
            eprintln!("telling auth this device is in: {err}");
        }
        Ok(())
    }

    /// Whether it's been let in: it reaches a node as itself. While it waits, what auth says of its request
    /// (`keys::waited`) is acted on as every client does: once approved, the network the device that approved it said it's
    /// in (its root) is the only one it takes, kept before it reads any, so none it's shown first can be another; one
    /// gone from auth is asked again unless it's in; a sign-in that's no more fails, to sign in again.
    pub async fn wait(&self, token: &str) -> Result<bool, Error> {
        let gone = match join::waited(self, token, &self.me.key).await? {
            Waited::SignedOut => return Err(failed("Sign in again.")),
            Waited::Waiting => return Ok(false),
            Waited::Approved { .. } => false,
            Waited::Gone => true,
        };
        if self.connect().await? {
            return Ok(true);
        }
        let asked = match &*self.stage.lock().await {
            Stage::Off { asked } => asked.clone(),
            _ => None,
        };
        if gone && let Some(asked) = asked {
            let _ = join::ask(self, token, &asked).await;
        }
        Ok(false)
    }

    /// Reaches its network's nodes, if it's been let in: false if not yet. Then, each time, as each start of a browser
    /// does, it settles its user's own things (`settle_own`), if they aren't: what fails is tried again at the next.
    pub async fn connect(&self) -> Result<bool, Error> {
        if !self.reach().await? {
            return Ok(false);
        }
        if let Some(client) = self.client().await {
            let on = self.on.lock().unwrap().clone();
            if let Err(err) = self.keyring.settle_own(&client, self, on.as_deref(), Some(PLATFORM)).await {
                eprintln!("settling this user's own things: {err}");
            }
        }
        Ok(true)
    }

    /// Reaches its network's nodes, if it's let in: false if it isn't (yet, or any more).
    async fn reach(&self) -> Result<bool, Error> {
        let mut stage = self.stage.lock().await;
        match &*stage {
            Stage::In(_) => return Ok(true),
            Stage::Out => return Ok(false),
            Stage::Off { .. } => {}
        }
        // The relay says who's on its network: until it lets this device in, nothing else will.
        let Some((relayed, online)) = self.computes().await? else { return Ok(false) };
        match self.keyring.read(self).await {
            Ok(()) => {}
            // Not sealed global's keys yet: its approval is still being written.
            Err(_) if relayed.is_empty() => return Ok(false),
            Err(err) => return Err(err),
        }
        *self.order.lock().unwrap() = relayed;
        let nodes = self.ordered();
        if nodes.is_empty() {
            return Err(failed("None of your devices runs your things yet."));
        }
        let client = Client::start(self.key.to_bytes(), None, &self.address, &nodes).await?;
        if let Some(dir) = self.logs.clone() {
            client.answer_with(std::sync::Arc::new(move |ask| one::logs::answer(&dir, &ask)));
        }
        client.set_ranks(&self.keyring.ranks())?;
        client.set_wakes(self.keyring.cloudflare_node())?;
        client.set_online(&online)?;
        *stage = Stage::In(client);
        // Kept, with its key, so this key is known as one that was in, to its user's sign-in once the relay takes it
        // out, not to approve; a new key in the same folder (signed in again) is one to approve.
        if !self.was_in() {
            self.put("joined", self.key().as_bytes()).await.map_err(failed)?;
        }
        Ok(true)
    }

    async fn client(&self) -> Option<Client> {
        match &*self.stage.lock().await {
            Stage::In(client) => Some(client.clone()),
            _ => None,
        }
    }

    /// Whether the relay lets this device in: → the compute endpoints on its network, which it says are there, not
    /// what they may do; and those on it now.
    async fn computes(&self) -> Result<Option<(Vec<[u8; 32]>, Vec<[u8; 32]>)>, Error> {
        let url = format!("{}/relay/computes", self.address);
        // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
        let mut at = keys::next_at(now());
        let (mut status, mut out) = http("POST", &url, None, Some(keys::signed_request(&self.key, one::protocol::RELAY, json!({ "computes": true, "reads": one::database::READS }), at))).await?;
        if out["code"] == one::protocol::code::REPLAYED {
            tokio::time::sleep(keys::again_in(at)).await;
            at = keys::next_at(now());
            (status, out) = http("POST", &url, None, Some(keys::signed_request(&self.key, one::protocol::RELAY, json!({ "computes": true, "reads": one::database::READS }), at))).await?;
        }
        // The relay answers 403 only for a key it doesn't let in.
        if status == 403 {
            return Ok(None);
        }
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{YOURS} answered {status}."))));
        }
        let keys = |v: &Value| v.as_array().into_iter().flatten().filter_map(|c| keys::unhex(c.as_str().unwrap_or("")).ok()).collect::<Vec<_>>();
        Ok(Some((keys(&out["computes"]), keys(&out["online"]))))
    }

    /// The nodes it may ask, in the order things run on them as last said, those it wasn't told of after, and the
    /// Cloudflare node after them; then by run priority, which may put the Cloudflare node first.
    fn ordered(&self) -> Vec<[u8; 32]> {
        let nodes = self.keyring.nodes();
        let order = self.order.lock().unwrap().clone();
        let last = self.keyring.cloudflare_node();
        let ranks: HashMap<[u8; 32], u32> = self.keyring.ranks().into_iter().collect();
        let mut out: Vec<[u8; 32]> = order.iter().filter(|k| nodes.contains_key(*k) && Some(**k) != last).copied().collect();
        let mut rest: Vec<[u8; 32]> = nodes.keys().filter(|k| !out.contains(k) && Some(**k) != last).copied().collect();
        rest.sort();
        out.extend(rest);
        out.extend(last.filter(|k| nodes.contains_key(k)));
        out.sort_by_key(|k| ranks.get(k).copied().unwrap_or(1));
        out
    }

    pub async fn reconnect(&self) {
        if let Some(client) = self.client().await {
            client.reconnect().await;
        }
    }

    /// Closes its connections, until it's asked again, still knowing it was in: when its app goes to the background or
    /// a program using it ends, so a node isn't left holding a connection it counts against this device.
    pub async fn disconnect(&self) {
        let mut stage = self.stage.lock().await;
        if matches!(*stage, Stage::In(_))
            && let Stage::In(client) = std::mem::replace(&mut *stage, Stage::Off { asked: None })
        {
            client.close().await;
        }
    }

    /// Out: as the relay said, or signed out. Its connections close, and what it kept as this device goes with them,
    /// as a browser's does (browser.js `forgetDevice` and `leave`): what each kind of request needed, its calls' answers
    /// and the network it was let into, which the next approval names again. Whether this key was in stays, for
    /// `removed` to say.
    async fn out(&self) {
        if let Stage::In(client) = std::mem::replace(&mut *self.stage.lock().await, Stage::Out) {
            client.close().await;
        }
        self.keyring.forget(self).await;
        if let Some(answers) = &self.answers {
            answers.forget().await;
        }
    }

    /// Signs out: this device takes itself out of its user's devices first (`leave`), reaching the network first if
    /// its connections were closed, so signing in again makes a new one that replaces it, never a second; then it's
    /// out, and forgets it was in. A network that doesn't answer in a few seconds is left: approving the new device
    /// replaces it. The app forgets this device's key either way, as a removed key can't be let in again.
    pub async fn sign_out(&self) {
        if self.was_in() {
            let leave = async {
                if self.reach().await?
                    && let Some(client) = self.client().await
                {
                    self.keyring.leave(&client, self).await?;
                }
                Ok::<_, Error>(())
            };
            if let Ok(Err(err)) = n0_future::time::timeout(Duration::from_secs(15), leave).await {
                eprintln!("signing out: {err}");
            }
        }
        self.delete("joined").await;
        self.out().await;
    }

    /// Registers where a push service reaches it (`one::push::Address`) with the network's push Worker
    /// (`<address>/push/register`), signed with its key and naming its seal key, which each note pushed to it is sealed
    /// to, and its user as who's who lists it, so auth pushes it when another device of theirs asks to join. Once
    /// it's out of the network, the Worker forgets it.
    pub async fn register_push(&self, address: &one::push::Address) -> Result<(), Error> {
        self.keyring.register_push(self, address).await
    }

    /// Whether the network took this device out: this key was in, and the relay, which alone says who's on the
    /// network, lets it in no more. Never a node's word, which may only not have read its approval yet, nor a
    /// relay that didn't answer: a key taken for out is forgotten, and can't be let in again. Once out, it's `out`.
    pub async fn removed(&self) -> bool {
        let removed = self.was_in() && matches!(self.computes().await, Ok(None));
        if removed {
            self.out().await;
        }
        removed
    }

    fn was_in(&self) -> bool {
        std::fs::read_to_string(self.dir.join("joined")).is_ok_and(|key| key.trim() == self.key())
    }

    /// A request to its network's nodes as its user (`Keyring::request`), given up on once the relay says this device
    /// is out. `/v1/state` is read for the order things run in, and a screen's calls that only read are kept.
    pub async fn fetch(&self, method: &str, path: &str, headers: Vec<(String, String)>, body: impl Into<Sent>) -> Result<Response, Error> {
        let body = body.into();
        let traced = self.developer().then(|| (now(), std::time::Instant::now(), what(path, body.bytes())));
        let client = match self.client().await {
            Some(client) => client,
            // Asked before it joined, or after its connections closed: it's in only if the relay lets it, else its own
            // screen says so.
            None if self.reach().await? => self.client().await.ok_or_else(refused)?,
            None => return Err(refused()),
        };
        let network = traced.as_ref().map(|t| t.1.elapsed().as_millis() as u64).filter(|ms| *ms > 0);
        let mut fetches = Vec::new();
        let asked = self.request(&client, method, path, headers, body, traced.as_ref().map(|_| &mut fetches));
        let answered = n0_future::future::or(asked, self.watch()).await;
        if let Some((at, started, mut what)) = traced {
            what["at"] = json!(at);
            what["ms"] = json!(started.elapsed().as_millis() as u64);
            what["network"] = json!(network);
            what["fetches"] = json!(fetches);
            match &answered {
                Ok(response) => what["status"] = json!(response.status),
                Err(_) => what["failed"] = json!(true),
            }
            self.keep_trace(what);
        }
        let err = match answered {
            Ok(response) => return Ok(response),
            Err(err) => err,
        };
        // A request that failed may be one a device taken out can't make: the relay says who's on its network, so it's
        // asked, whatever a node said. Out, this device starts over at joining, where nothing but its approval or sign-in
        // is shown.
        if !self.let_in().await {
            self.out().await;
            return Err(refused());
        }
        Err(err)
    }

    /// Never answers while the relay lets this device in; once it doesn't, it's `refused`. A request that's slow may be
    /// one a device taken out can't be answered, which would otherwise wait out every node's time.
    async fn watch(&self) -> Result<Response, Error> {
        let mut wait = 3;
        loop {
            n0_future::time::sleep(Duration::from_secs(wait)).await;
            if !self.let_in().await {
                return Err(refused());
            }
            wait = 5;
        }
    }

    async fn request(&self, client: &Client, method: &str, path: &str, headers: Vec<(String, String)>, body: Sent, fetches: Option<&mut Vec<Value>>) -> Result<Response, Error> {
        let request = Request { method: method.into(), path: path.into(), headers, body: body.clone() };
        let response = self.keyring.request(client, self, request, now() + HANDED_FOR, fetches).await?;
        if response.status == 200 && path.split('?').next() == Some("/v1/state") {
            let bytes = response.body.bytes().await?;
            self.learn_order(&bytes).await;
            return Ok(Response { body: Body::whole(bytes), ..response });
        }
        // What a screen's call answered, kept if it only reads, as the whole list of apps says which do.
        match &self.answers {
            Some(answers) => answers.passing(method, path, body.bytes(), response).await,
            None => Ok(response),
        }
    }

    /// The order things run in, from the node's `/v1/state`; a node it lists that this device didn't know of has it
    /// read `global` again (now and then), and only nodes its own reading lists are asked.
    async fn learn_order(&self, bytes: &[u8]) {
        let Ok(state) = serde_json::from_slice::<one::state::State>(bytes) else { return };
        let mut said: Vec<(usize, [u8; 32])> = state.network.iter().flat_map(|n| &n.devices).filter(|d| d.serves_me).filter_map(|d| Some((d.order, keys::unhex(&d.key).ok()?))).collect();
        said.sort();
        let said: Vec<[u8; 32]> = said.into_iter().map(|s| s.1).collect();
        *self.order.lock().unwrap() = said.clone();
        self.keyring.reread_for(self, &said).await;
        if let Some(client) = self.client().await {
            let _ = client.set_ranks(&self.keyring.ranks());
            let _ = client.set_wakes(self.keyring.cloudflare_node());
            let _ = client.set_nodes(&self.ordered());
        }
    }

    /// Renews its user's app schedules of `app` as they use it (`Keyring::renew_app_schedules`).
    pub async fn renew_app_schedules(&self, app: &str) {
        if let Some(client) = self.client().await {
            self.keyring.renew_app_schedules(&client, self, app).await;
        }
    }

    /// What `app`'s `tool` answered last for `input` (JSON), if it only reads: its answer's JSON, and when (ms). For a
    /// screen to show at once while it asks again (bridge.js's `kept`).
    pub async fn kept(&self, app: &str, tool: &str, input: &[u8]) -> Option<(Vec<u8>, u64)> {
        self.answers.as_ref()?.kept(app, tool, input).await
    }
}

/// What it keeps, as files in its folder.
impl Keep for Device {
    async fn get(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(name)).ok()
    }

    async fn put(&self, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(std::fs::write(self.dir.join(name), bytes)?)
    }

    async fn delete(&self, name: &str) {
        let _ = std::fs::remove_file(self.dir.join(name));
    }
}

impl keys::Signs for Device {
    fn public(&self) -> [u8; 32] {
        self.me.key
    }

    async fn sign_bytes(&self, bytes: &[u8]) -> anyhow::Result<[u8; 64]> {
        self.key.sign_bytes(bytes).await
    }
}

impl Address for Device {
    type Bucket = Signed;

    fn address(&self) -> &str {
        &self.address
    }

    async fn ask(&self, method: &str, url: &str, header: Option<(&str, String)>, body: Option<Value>) -> Result<(u16, Value), Error> {
        http(method, url, header, body).await
    }

    fn bucket(&self) -> &Signed {
        &self.bucket
    }

    /// Who the relay says is on its network now is raced first from then on.
    async fn let_in(&self) -> bool {
        match self.computes().await {
            Ok(None) => false,
            Ok(Some((_, online))) => {
                if let Some(client) = self.client().await {
                    let _ = client.set_online(&online);
                }
                true
            }
            _ => true,
        }
    }
}

/// What a request asked, as a trace names it: a call's app and tool, a screen's app and name, else its path. Never
/// its input or query.
fn what(path: &str, body: &[u8]) -> Value {
    let at = path.split('?').next().unwrap_or(path);
    if at == "/v1/call" {
        let b = serde_json::from_slice::<Value>(body).unwrap_or_default();
        return json!({ "what": "call", "app": b["app"], "tool": b["tool"] });
    }
    match at.strip_prefix("/v1/screens/").map(|s| s.split('/')) {
        Some(mut s) => json!({ "what": "page", "app": s.next(), "screen": s.next() }),
        None => json!({ "what": "request", "path": at }),
    }
}

/// A request to the network's address (auth, the relay), JSON in and out: → its status and JSON.
async fn http(method: &str, url: &str, header: Option<(&str, String)>, body: Option<Value>) -> Result<(u16, Value), Error> {
    let (method, url, header) = (method.to_string(), url.to_string(), header.map(|(n, v)| (n.to_string(), v)));
    tokio::task::spawn_blocking(move || {
        let mut request = ureq::http::Request::builder().method(method.as_str()).uri(&url);
        if let Some((name, value)) = header {
            request = request.header(name, value);
        }
        let response = match body {
            Some(body) => crate::bucket::AGENT.run(request.header("content-type", "application/json").body(keys::canonical(&body)).map_err(failed)?),
            None => crate::bucket::AGENT.run(request.body(()).map_err(failed)?),
        };
        let mut response = response.map_err(|e| failed(format!("Couldn't reach {YOURS}: {e}")))?;
        let status = response.status().as_u16();
        Ok((status, response.body_mut().read_json().unwrap_or(Value::Null)))
    })
    .await
    .map_err(failed)?
}

/// Creates an object in the bucket, by a URL the URL signer signed for whoever `signer` signs as (a create only if
/// absent, which every endpoint of the network may ask for): → whether it was created.
async fn create(signer: std::sync::Arc<one::bucket::Signer>, name: String, bytes: Vec<u8>) -> anyhow::Result<bool> {
    tokio::task::spawn_blocking(move || {
        let said = signer.ask(json!({ "create": [name] }))?;
        let url = said["create"][&name].as_str().ok_or_else(|| anyhow::anyhow!("the URL signer didn't sign that"))?.to_string();
        let response = crate::bucket::AGENT.put(&url).header("if-none-match", "*").config().http_status_as_error(false).build().send(&bytes[..])?;
        match response.status().as_u16() {
            200 | 201 => Ok(true),
            409 | 412 => Ok(false),
            status => anyhow::bail!("{YOURS}'s storage said {status}"),
        }
    })
    .await?
}
