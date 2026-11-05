//! A native app as a device of its person (ARCHITECTURE.md, Being built: Devices), the same on every app: its own key,
//! which the app keeps in the Keychain, the keystore or the Credential Locker and hands it at start; asking to be let in,
//! with the code it shows; and once a device of theirs approved it, its scopes' epochs, sealed to it, from which it hands
//! each request the database keys it needed last time (`x-one-keys`), sent again once with those a node's 428 names.
//! cloudflare/web/device.js is the same in a browser.
//!
//! It trusts no node, nor the relay, for who's who: it reads `global`'s entries from the bucket itself, opens them with
//! the epochs sealed to it, and replays them by the node's own rules (`crate::members`, as browsers and the MCP
//! Worker's devices do). It asks only endpoints those list
//! as compute that run its person's things, takes a 428 only from the node that answered, naming the seal key they list
//! for it, and only for scopes they say it runs, databases that kind of request uses (`keys::may_hand`) and epochs
//! from each one's newest copy (`keys::oldest_needed`); and hands what it named to the one node each request is sent
//! to, of the scopes they say that node runs, sealed to the seal key they list for it.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use one::entry::{Entry, Ext, Hash, Scope, hex};
use one::keys::{self, Endpoint, Handed, Handoff, Need, Waited};
use one::members;
use data_encoding::BASE64URL_NOPAD as B64U;
use serde_json::{Value, json};

use crate::answers::Answers;
use crate::members::ScopeDb;
use crate::bucket::Signed;
use crate::{Body, Client, Error, Request, Response, Sent, TRACE_HEADER, not_let_in, unsent};

/// How long a node keeps the keys a request handed it.
const HANDED_FOR: u64 = 3_600_000;
/// How often, at most, it reads `global`'s entries again for a node it didn't know.
const REREAD: u64 = 30_000;
/// How many requests' traces it keeps, in developer mode, for screens to show.
const TRACES_KEPT: usize = 100;

/// What a sign-in for the device whose key is `secret` names it by (`join`), before it's made: its key ‖ seal key, hex.
pub fn join_of(secret: [u8; 32]) -> String {
    let me = Endpoint::of(&SigningKey::from_bytes(&secret));
    format!("{}{}", hex(&me.key), hex(&me.seal))
}

/// A note pushed to the device whose key is `secret`, as its push carried it (base64url): → the note, and the key of
/// the device that sent it (hex), or none for one from the install's Workers. Opened here alone: nothing connects or starts.
pub fn open_note(secret: [u8; 32], pushed: &str) -> Result<(one::push::Note, Option<String>), Error> {
    let sealed = B64U.decode(pushed.trim().trim_end_matches('=').as_bytes()).map_err(|_| failed("That push isn't a note."))?;
    one::push::open_note(&keys::seal_secret(&SigningKey::from_bytes(&secret)), &sealed).map_err(|_| failed("That note isn't this device's."))
}

/// A new device's key: 32 random bytes, which its app keeps.
pub fn new_secret() -> [u8; 32] {
    rand::random()
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn failed(message: impl std::fmt::Display) -> Error {
    unsent(message)
}

/// What a request says when this device isn't one of the install's (yet, or any more).
fn refused() -> Error {
    not_let_in("This device hasn't been let in yet.")
}

/// The install as this device read and checked it itself.
struct Known {
    who: ScopeDb,
    /// Its person.
    user: String,
    /// The nodes it may ask, by key: their seal keys; and the serverless node, asked last, if the install has one.
    nodes: HashMap<[u8; 32], [u8; 32]>,
    serverless: Option<[u8; 32]>,
    /// When it was read.
    at: u64,
}

/// Where a device is with its install, from its sign-in to its sign-out, as a browser goes the same way (cloudflare/web/web.js:
/// `join`, `start`, `takenOut` and `signOut`). Whether this key was ever let in here is kept on disk (`joined`), as it
/// outlives the program: `removed` asks it.
enum Stage {
    /// Not reaching its nodes: not let in yet, having asked auth with `asked` if it has since it started (`ask`, then
    /// `wait`); or let in, and its connections closed until it's asked again (`disconnect`: its app in the background,
    /// or a program ending).
    Off { asked: Option<Value> },
    /// Reaching its nodes as one of its person's devices.
    In(Client),
    /// Taken out, as the relay said, or signed out: nothing more is asked as it, and its app forgets its key.
    Out,
}

/// A device's way in: its keys, the install it's in, and what it has opened.
pub struct Device {
    key: SigningKey,
    me: Endpoint,
    address: String,
    /// Where it keeps the install's genesis, once it first read it, and what each kind of request needed.
    dir: PathBuf,
    /// Held while it connects, so it connects once.
    stage: tokio::sync::Mutex<Stage>,
    /// Who's who, as it last read it itself.
    known: Mutex<Option<Known>>,
    /// The order things run in, as the node last said (`/v1/state`): only ever used to sort nodes it knows.
    order: Mutex<Vec<[u8; 32]>>,
    /// The device it's on, as its app says (`on`): signing in there again replaces this sign-in.
    on: Mutex<Option<String>>,
    /// Each scope's epochs opened, by (scope, n), and apps' keys by (scope, app, n).
    epochs: Mutex<HashMap<(String, u32), [u8; 32]>>,
    app_keys: Mutex<HashMap<(String, String, u32), [u8; 32]>>,
    /// What each kind of request needed, by its route (`route`): the last 428's, and the node that sent it (hex).
    needs: Mutex<BTreeMap<String, (String, Vec<Need>)>>,
    /// Held while a kind of request learns from a 428, by its route: another of its kind waits for it, and goes with
    /// what it learned.
    learning: Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    /// In developer mode, its latest requests' traces, oldest first (`traces`).
    traces: Mutex<std::collections::VecDeque<Value>>,
    /// The last answer of each call that only reads, for its app's screens (`kept`): a native app's keeps them
    /// (`keeping`), the CLI's doesn't.
    answers: Option<Answers>,
}

impl Device {
    /// The device whose key is `secret`, of the install at `address`, keeping what it needs to in `dir`.
    pub fn new(secret: [u8; 32], address: &str, dir: &str) -> Device {
        let needs = std::fs::read(PathBuf::from(dir).join("needs.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let key = SigningKey::from_bytes(&secret);
        Device {
            me: Endpoint::of(&key),
            key,
            address: address.trim_end_matches('/').to_string(),
            dir: dir.into(),
            stage: tokio::sync::Mutex::new(Stage::Off { asked: None }),
            known: Mutex::default(),
            order: Mutex::default(),
            on: Mutex::default(),
            epochs: Mutex::default(),
            app_keys: Mutex::default(),
            needs: Mutex::new(needs),
            learning: Mutex::default(),
            traces: Mutex::default(),
            answers: None,
        }
    }

    /// Says which device it's on: that device's oned key (hex), on a machine with one, or an id of the device's own,
    /// hashed, that outlives signing out. Its request to join names it, and once it's in, its listing does.
    pub fn on(&self, device: &str) {
        if keys::unhex(device).is_ok() {
            *self.on.lock().unwrap() = Some(device.to_string());
        }
    }

    /// The same device, keeping the last answer of each call that only reads (`kept`): a native app's.
    pub fn keeping(mut self) -> Device {
        self.answers = Some(Answers::new(&self.dir, &self.key));
        self
    }

    // Developer mode.

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
    /// tried and what the one that answered did (Developer's Timing draws them: src/screen/devtools.html). None out of developer mode.
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

    /// Its key, hex: its id.
    pub fn key(&self) -> String {
        hex(&self.me.key)
    }

    /// What a sign-in for it names it by (`join`): its key ‖ seal key, hex.
    pub fn join(&self) -> String {
        format!("{}{}", self.key(), hex(&self.me.seal))
    }

    /// The code it shows to be approved: "ABCD EFGH".
    pub fn code(&self) -> String {
        let code = keys::join_code(&self.me);
        format!("{} {}", &code[..4], &code[4..])
    }

    fn opens(&self) -> [u8; 32] {
        keys::seal_secret(&self.key)
    }

    // Joining.

    /// Asks auth to let it in, as the person signed in with `token`, calling it `name` (and with someone new's invite
    /// secret, its proof).
    pub async fn ask(&self, token: &str, name: &str, invite: Option<&str>) -> Result<(), Error> {
        let mut asked = json!({ "key": self.key(), "seal": hex(&self.me.seal), "name": name });
        if let Some(on) = self.on.lock().unwrap().clone() {
            asked["device"] = json!(on);
        }
        if let Some(invite) = invite.filter(|i| !i.is_empty()) {
            asked["proof"] = json!(keys::invite_proof(invite.as_bytes(), &self.me));
        }
        if let Stage::Off { asked: kept } = &mut *self.stage.lock().await {
            *kept = Some(asked.clone());
        }
        let (status, out) = http("POST", &format!("{}/api/auth/devices/requests", self.address), Some(token), Some(asked)).await?;
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("Couldn't ask to be let in ({status})."))));
        }
        Ok(())
    }

    /// Whether it's been let in: it reaches a node as itself. While it waits, what auth says of its request
    /// (`keys::waited`) is acted on as every client does: once approved, the install the device that approved it said it's
    /// in (its genesis) is the only one it takes, kept before it reads any, so none it's shown first can be another; one
    /// gone from auth is asked again unless it's in; a sign-in that's no more fails, to sign in again.
    pub async fn wait(&self, token: &str) -> Result<bool, Error> {
        let at = format!("{}/api/auth/devices/requests/{}", self.address, self.key());
        let (status, said) = http("GET", &at, Some(token), None).await.unwrap_or((0, Value::Null));
        let gone = match keys::waited(status, &said) {
            Waited::SignedOut => return Err(failed("Sign in again.")),
            Waited::Waiting => return Ok(false),
            Waited::Approved(genesis) => {
                if let Some(genesis) = genesis {
                    let _ = std::fs::create_dir_all(&self.dir);
                    std::fs::write(self.dir.join("genesis"), genesis).map_err(failed)?;
                }
                false
            }
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
            let _ = http("POST", &format!("{}/api/auth/devices/requests", self.address), Some(token), Some(asked)).await;
        }
        Ok(false)
    }

    /// Reaches its install's nodes, if it's been let in: false if not yet. Then, each time, as each start of a browser
    /// does, it settles its person's own things (`settle_own`), if they aren't: what fails is tried again at the next.
    pub async fn connect(&self) -> Result<bool, Error> {
        if !self.reach().await? {
            return Ok(false);
        }
        let _ = self.settle_own().await;
        Ok(true)
    }

    /// Reaches its install's nodes, if it's let in: false if it isn't (yet, or any more).
    async fn reach(&self) -> Result<bool, Error> {
        let mut stage = self.stage.lock().await;
        match &*stage {
            Stage::In(_) => return Ok(true),
            Stage::Out => return Ok(false),
            Stage::Off { .. } => {}
        }
        // The relay says who's on its network: until it lets this device in, nothing else will.
        let Some((relayed, online)) = self.computes().await? else { return Ok(false) };
        let nodes = match self.read_members().await {
            Ok(nodes) => nodes,
            // Not sealed global's keys yet: its approval is still being written.
            Err(_) if relayed.is_empty() => return Ok(false),
            Err(err) => return Err(err),
        };
        *self.order.lock().unwrap() = relayed;
        let nodes = self.ordered(&nodes);
        if nodes.is_empty() {
            return Err(failed("None of your devices runs your things yet."));
        }
        let client = Client::start(self.key.to_bytes(), None, &self.address, &nodes).await?;
        if let Some(online) = online {
            client.set_online(&online)?;
        }
        *stage = Stage::In(client);
        // Kept, with its key, so this key is known as one that was in, to its person's sign-in once the relay takes it
        // out, not to approve; a new key in the same folder (signed in again) is one to approve.
        if !self.was_in() {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(self.dir.join("joined"), self.key());
        }
        Ok(true)
    }

    /// Its client, while it's in.
    async fn client(&self) -> Option<Client> {
        match &*self.stage.lock().await {
            Stage::In(client) => Some(client.clone()),
            _ => None,
        }
    }

    /// Whether the relay lets this device in: → the compute endpoints on its network, which it says are there, not
    /// what they may do; and those on it now, if it says (an older relay doesn't).
    async fn computes(&self) -> Result<Option<(Vec<[u8; 32]>, Option<Vec<[u8; 32]>>)>, Error> {
        let url = format!("{}/relay/computes", self.address);
        // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
        let mut at = keys::next_at(now());
        let (mut status, mut out) = http("POST", &url, None, Some(keys::signed_request(&self.key, "one/relay", json!({ "computes": true }), at))).await?;
        if error(&out).is_some_and(|e| keys::taken_already(&e)) {
            tokio::time::sleep(keys::again_in(at)).await;
            at = keys::next_at(now());
            (status, out) = http("POST", &url, None, Some(keys::signed_request(&self.key, "one/relay", json!({ "computes": true }), at))).await?;
        }
        // The relay answers 403 only for a key it doesn't let in.
        if status == 403 {
            return Ok(None);
        }
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("Your One App's network answered {status}."))));
        }
        let keys = |v: &Value| v.as_array().into_iter().flatten().filter_map(|c| keys::unhex(c.as_str().unwrap_or("")).ok()).collect::<Vec<_>>();
        Ok(Some((keys(&out["computes"]), out.get("online").map(keys))))
    }

    /// Whether the relay still lets this device in: false only if it says it doesn't. Who it says is on its network
    /// now is raced first from then on.
    async fn let_in(&self) -> bool {
        match self.computes().await {
            Ok(None) => false,
            Ok(Some((_, Some(online)))) => {
                if let Some(client) = self.client().await {
                    let _ = client.set_online(&online);
                }
                true
            }
            _ => true,
        }
    }

    /// `global`'s entries, read from the bucket and opened with the epochs sealed to this device, checked and replayed:
    /// → the nodes it may ask, with their seal keys. The install's genesis is kept the first time, and must be the same
    /// after.
    async fn read_members(&self) -> Result<HashMap<[u8; 32], [u8; 32]>, Error> {
        let bucket = Signed::new(self.address.clone(), self.key.to_bytes());
        let me = self.key();
        let mut epochs: BTreeMap<u32, [u8; 32]> = BTreeMap::new();
        let mut n = 1;
        loop {
            let read = async |n: u32| -> Result<Option<[u8; 32]>, Error> {
                let Some(o) = one::bucket::Bucket::get(&bucket, &keys::epoch_name("global", n, &me)).await.map_err(failed)? else { return Ok(None) };
                one::cloudflare::open_sealed(&self.opens(), &o.bytes).ok().and_then(|k| k.try_into().ok()).map(Some).ok_or_else(|| failed("A key for this didn't open."))
            };
            match read(n).await? {
                Some(key) => drop(epochs.insert(n, key)),
                None if epochs.is_empty() && n < 8 => {}
                None => break,
            }
            n += 1;
            if n > 8 && epochs.is_empty() {
                return Err(failed("This device hasn't been given your One App's keys yet."));
            }
        }
        let mut log_keys = one::db::Keys::new();
        for (n, epoch) in &epochs {
            let Some(o) = one::bucket::Bucket::get(&bucket, &keys::app_key_name("global", members::APP, *n)).await.map_err(failed)? else { continue };
            let app = one::cloudflare::open_app_key(epoch, members::APP, &o.bytes).map_err(failed)?;
            log_keys.insert(*n, one::cloudflare::database_key(&app, one::log::NAME));
        }
        let read = one::log::open(&bucket, &log_keys, &one::log::prefix("global"), "global").await.map_err(failed)?;
        // Replayed as every client device replays them (crate::members), of the install whose genesis it kept.
        let pinned = self.dir.join("genesis");
        let kept = std::fs::read_to_string(&pinned).ok().and_then(|had| Hash::from_hex(had.trim()).ok());
        let mut global = crate::members::Global::new(kept);
        global.add_entries(read.entries).map_err(|_| failed("This isn't the One App this device joined."))?;
        // Its scope database's steps too, opened with the same keys (`one::database::steps`): who's who once it's frozen.
        global.add_read(one::database::steps::read(&bucket, &log_keys, &Scope::Global, 0).await.map_err(failed)?).map_err(failed)?;
        let who = global.who().map_err(failed)?;
        if kept.is_none()
            && let Some(root) = global.root()
        {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(&pinned, root.to_hex());
        }
        // Every epoch it was sealed must be the one the entries made.
        for (n, key) in &epochs {
            if who.epoch_hash(&Scope::Global, *n) != Some(keys::epoch_hash(key)) {
                return Err(failed("A key for this didn't check out, so it wasn't used."));
            }
            self.epochs.lock().unwrap().insert(("global".into(), *n), *key);
        }
        let user = who.machine(&self.me.key).map(|d| d.user).ok_or_else(|| failed("This device isn't listed yet."))?;
        // The nodes that run something of its person's, and the serverless node, which runs anyone's calls.
        let serverless = who.serverless();
        let mut nodes: HashMap<[u8; 32], [u8; 32]> = HashMap::new();
        for (key, seal) in who.nodes() {
            if Some(key) == serverless || who.serves(&key, &user) {
                nodes.insert(key, seal);
            }
        }
        *self.known.lock().unwrap() = Some(Known { who, user, nodes: nodes.clone(), serverless, at: now() });
        Ok(nodes)
    }

    /// `nodes`, in the order things run on them as last said, those it wasn't told of after, and the serverless node
    /// last of all.
    fn ordered(&self, nodes: &HashMap<[u8; 32], [u8; 32]>) -> Vec<[u8; 32]> {
        let order = self.order.lock().unwrap().clone();
        let last = self.known.lock().unwrap().as_ref().and_then(|k| k.serverless);
        let mut out: Vec<[u8; 32]> = order.iter().filter(|k| nodes.contains_key(*k) && Some(**k) != last).copied().collect();
        let mut rest: Vec<[u8; 32]> = nodes.keys().filter(|k| !out.contains(k) && Some(**k) != last).copied().collect();
        rest.sort();
        out.extend(rest);
        out.extend(last.filter(|k| nodes.contains_key(k)));
        out
    }

    /// As a person's first device does: says where their own things run (the machines that run everyone's), if they
    /// haven't, and makes their own scope's first epoch, if it has none (cloudflare/web/device.js `settleOwn`). What's to do
    /// is looked at again in what it reads fresh, as another device of theirs may have done it.
    async fn settle_own(&self) -> Result<(), Error> {
        let device = self.on.lock().unwrap().clone();
        let todo = || -> Result<_, Error> {
            let known = self.known.lock().unwrap();
            let who = &known.as_ref().ok_or_else(refused)?.who;
            let me = who.machine(&self.me.key);
            let user = me.as_ref().map(|d| d.user.clone()).unwrap_or_default();
            let on = who.runs_until_chosen(&user);
            // Listed before sign-ins named their device: it lists itself again, naming it.
            let unnamed = me.filter(|d| d.device.is_none() && d.passkey.is_none() && device.is_some()).map(|d| (d.name, d.roles.into_iter().collect::<Vec<_>>()));
            let first = who.epoch(&Scope::User(user.clone())) == 0;
            Ok((!on.is_empty() || unnamed.is_some() || first).then_some((user, on, unnamed, first)))
        };
        if todo()?.is_none() {
            return Ok(());
        }
        self.read_members().await?;
        let Some((user, on, unnamed, first)) = todo()? else { return Ok(()) };
        if let (Some(device), Some((name, roles))) = (device, unnamed) {
            let mut listed = members::device(&self.me, &user, &name, &roles, None);
            listed.data["device"] = json!(device);
            self.write("global", &user, listed).await?;
        }
        if !on.is_empty() {
            self.write("global", &user, members::runs(&on)).await?;
        }
        if first {
            self.first_epoch(&format!("user:{user}"), &user).await?;
        }
        Ok(())
    }

    /// Starts again with a fresh connection, once the network's back.
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
    /// as a browser's does (web.js `forgetDevice` and `signOut`): what each kind of request needed, its calls' answers
    /// and the install it was let into, which the next approval names again. Whether this key was in stays, for
    /// `removed` to say.
    async fn out(&self) {
        if let Stage::In(client) = std::mem::replace(&mut *self.stage.lock().await, Stage::Out) {
            client.close().await;
        }
        *self.known.lock().unwrap() = None;
        self.needs.lock().unwrap().clear();
        for kept in ["needs.json", "genesis"] {
            let _ = std::fs::remove_file(self.dir.join(kept));
        }
        self.forget_answers();
    }

    /// Signs out: this device takes itself out of its person's devices first, by asking their node to (as Settings ›
    /// Devices does for another), reaching it first if its connections were closed, so signing in again makes a new one
    /// that replaces it, never a second; then it's out, and forgets it was in. A node that doesn't answer in a few
    /// seconds is left: approving the new device replaces it. The app forgets this device's key either way, as a
    /// removed key can't be let in again.
    pub async fn sign_out(&self) {
        if self.was_in() {
            let leave = async {
                if self.reach().await? {
                    self.json("POST", "/v1/remove", Some(json!({ "key": self.key() }))).await?;
                }
                Ok::<_, Error>(())
            };
            let _ = n0_future::time::timeout(Duration::from_secs(15), leave).await;
        }
        let _ = std::fs::remove_file(self.dir.join("joined"));
        self.out().await;
    }

    /// Registers where a push service reaches it (`one::push::Address`) with the install's push Worker
    /// (`<address>/push/register`), signed with its key and naming its seal key, which each note pushed to it is sealed
    /// to. Once it's out of the install, the Worker forgets it.
    pub async fn register_push(&self, address: &one::push::Address) -> Result<(), Error> {
        let body = one::push::register_request(&self.key, &self.me.seal, address, now()).map_err(failed)?;
        let (status, out) = http("POST", &format!("{}/push/register", self.address), None, Some(body)).await?;
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("Your One App's push service didn't take this device ({status})."))));
        }
        Ok(())
    }

    /// Whether the install took this device out: this key was in, and the relay, which alone says who's on the
    /// install's network, lets it in no more. Never a node's word, which may only not have read its approval yet, nor a
    /// relay that didn't answer: a key taken for out is forgotten, and can't be let in again. Once out, it's `out`.
    pub async fn removed(&self) -> bool {
        let removed = self.was_in() && matches!(self.computes().await, Ok(None));
        if removed {
            self.out().await;
        }
        removed
    }

    /// Whether this key was let in on this device once.
    fn was_in(&self) -> bool {
        std::fs::read_to_string(self.dir.join("joined")).is_ok_and(|key| key.trim() == self.key())
    }

    // Requests.

    /// A request to its install's nodes as its person, with the keys it needed last time; one a node needs others for
    /// (428) is sent again with them, a few times, as a node may find what it needs as it goes (the serverless node,
    /// its logs' keys first, then a call's). `/v1/state` is read for the order things run in.
    pub async fn fetch(&self, method: &str, path: &str, headers: Vec<(String, String)>, body: impl Into<Sent>) -> Result<Response, Error> {
        let body = body.into();
        // In developer mode, what it did, from the start.
        let traced = self.developer().then(|| (now(), std::time::Instant::now(), what(path, body.bytes())));
        // Asked before it joined, or after its connections closed: it's in only if the relay lets it, else its own screen
        // says so.
        if self.client().await.is_none() && !self.reach().await? {
            return Err(refused());
        }
        let network = traced.as_ref().map(|t| t.1.elapsed().as_millis() as u64).filter(|ms| *ms > 0);
        let mut fetches = Vec::new();
        let asked = self.request(method, path, headers, body, traced.as_ref().map(|_| &mut fetches));
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

    /// `fetch`'s request, each send's trace kept in `fetches` if it's asked for one.
    async fn request(&self, method: &str, path: &str, mut headers: Vec<(String, String)>, body: Sent, mut fetches: Option<&mut Vec<Value>>) -> Result<Response, Error> {
        let route = route(path, body.bytes());
        if fetches.is_some() {
            headers.push((TRACE_HEADER.into(), "1".into()));
        }
        let (mut needed, mut behind) = (0, 0);
        loop {
            // Another of its kind learning what it needs: this one waits, and goes with that.
            let learning = self.learning.lock().unwrap().get(&route).cloned();
            if let Some(learning) = learning {
                drop(learning.lock().await);
            }
            let sent_with = self.needs.lock().unwrap().get(&route).cloned();
            let opening = fetches.is_some().then(Notes::new);
            let handed = self.handed(&route, opening.as_ref()).await;
            let request = Request { method: method.into(), path: path.into(), headers: headers.clone(), body: body.clone() };
            let sent = self.send(request, &handed).await;
            if let Some(fetches) = fetches.as_deref_mut() {
                let mut fetched = match &sent {
                    Ok(response) => response.trace.clone().unwrap_or_default(),
                    Err(err) => err.trace.clone().unwrap_or_default(),
                };
                // The keys it opened to hand with it, if it asked for any (after the app started, say).
                if let Some(opened) = opening.and_then(Notes::opened) {
                    fetched["handed"] = opened;
                }
                fetches.push(fetched);
            }
            let response = sent?;
            if response.status == 428 && needed < 3 {
                needed += 1;
                let node = response.node;
                let answer: Value = serde_json::from_slice(&response.body.bytes().await?).unwrap_or_default();
                // What it did before sending it again, each thing it asked for timed, in developer mode.
                let notes = fetches.is_some().then(Notes::new);
                let learned = self.learn(&route, &node, &answer, sent_with, notes.as_ref()).await;
                if let (Some(notes), Some(last)) = (notes, fetches.as_deref_mut().and_then(|f| f.last_mut())) {
                    last["client"] = notes.done();
                }
                learned?;
                continue;
            }
            if response.status == 403 {
                let bytes = response.body.bytes().await?;
                // A node that doesn't list this device may not have read its approval yet: the relay says if it's out,
                // else it's asked again in a moment, once it has.
                if serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| v["code"] == "not_let_in") {
                    if !self.let_in().await {
                        return Err(refused());
                    }
                    if behind < keys::BEHIND.len() {
                        n0_future::time::sleep(Duration::from_millis(keys::BEHIND[behind])).await;
                        behind += 1;
                        continue;
                    }
                    return Err(failed("Your devices haven't caught up with this one yet. Try again in a moment."));
                }
                return Ok(Response { status: 403, headers: response.headers, node: response.node, body: Body::whole(bytes), trace: response.trace });
            }
            if response.status == 200 && path.split('?').next() == Some("/v1/state") {
                let bytes = response.body.bytes().await?;
                self.learn_order(&bytes).await;
                return Ok(Response { status: 200, headers: response.headers, node: response.node, body: Body::whole(bytes), trace: response.trace });
            }
            // What a screen's call answered, kept if it only reads, as the whole list of apps says which do.
            if let Some(answers) = &self.answers
                && response.status == 200
                && ((method == "GET" && path == "/v1/apps") || (method == "POST" && path == "/v1/call"))
            {
                let bytes = response.body.bytes().await?;
                if path == "/v1/apps" {
                    answers.learn(&bytes);
                } else {
                    answers.keep(body.bytes(), &bytes);
                }
                return Ok(Response { status: 200, headers: response.headers, node: response.node, body: Body::whole(bytes), trace: response.trace });
            }
            return Ok(response);
        }
    }

    /// A request saying whose it is (`x-one-user`), which a node checks against this device's key, with the keys
    /// `handed` of the scopes its entries say runs on the node the client sends it to, for that node alone, sealed to
    /// the seal key they list for it: never to a node it only raced a connection to, nor one it doesn't list.
    async fn send(&self, mut request: Request, handed: &[Handed]) -> Result<Response, Error> {
        let client = self.client().await.ok_or_else(refused)?;
        let user = self.known.lock().unwrap().as_ref().map(|k| k.user.clone()).unwrap_or_default();
        request.headers.push(("x-one-user".into(), user));
        let me = self.key();
        client.fetch_with(request, |node| std::future::ready(self.sealed_for(node, handed, &me))).await
    }

    /// `handed`, of the scopes this device's entries say `node` runs, sealed to it (`x-one-keys`): none if it has none
    /// of them, or isn't listed.
    fn sealed_for(&self, node: &[u8; 32], handed: &[Handed], me: &str) -> Vec<(String, String)> {
        let known = self.known.lock().unwrap();
        let Some((k, seal)) = known.as_ref().and_then(|k| Some((k, k.nodes.get(node)?))) else { return Vec::new() };
        let keys: Vec<Handed> = handed.iter().filter(|h| Scope::parse(&h.scope).is_ok_and(|s| k.who.runs_for(node, &s))).cloned().collect();
        if keys.is_empty() {
            return Vec::new();
        }
        let handoff = Handoff { for_: me.to_string(), until: now() + HANDED_FOR, keys };
        keys::seal_handoff(seal, &handoff).map(|k| vec![("x-one-keys".to_string(), k)]).unwrap_or_default()
    }

    /// A request's JSON, without handing keys: what this device asks to open them.
    async fn json(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, Error> {
        let mut headers = Vec::new();
        if body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        let body = body.map(|b| keys::canonical(&b)).unwrap_or_default();
        let response = self.send(Request { method: method.into(), path: path.into(), headers, body: body.into() }, &[]).await?;
        let status = response.status;
        let out: Value = serde_json::from_slice(&response.body.bytes().await?).unwrap_or_default();
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{path} answered {status}"))));
        }
        Ok(out)
    }

    /// The order things run in, and each node's run priority, from the node's `/v1/state`; a node it lists that this device didn't know of has it
    /// read `global` again (now and then), and only nodes its own reading lists are asked.
    async fn learn_order(&self, bytes: &[u8]) {
        let Ok(state) = serde_json::from_slice::<Value>(bytes) else { return };
        let mut said: Vec<(u64, [u8; 32])> = state["install"]["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["serves_me"] == true)
            .filter_map(|d| Some((d["order"].as_u64().unwrap_or(u64::MAX), keys::unhex(d["key"].as_str()?).ok()?)))
            .collect();
        said.sort();
        let said: Vec<[u8; 32]> = said.into_iter().map(|s| s.1).collect();
        *self.order.lock().unwrap() = said.clone();
        let (mut nodes, stale) = match self.known.lock().unwrap().as_ref() {
            Some(k) => (k.nodes.clone(), said.iter().any(|n| !k.nodes.contains_key(n)) && now() > k.at + REREAD),
            None => return,
        };
        if stale && let Ok(read) = self.read_members().await {
            nodes = read;
        }
        // Run priority, where the node says it: a number, or "last".
        let ranks: Vec<([u8; 32], u32)> = state["install"]["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| Some((keys::unhex(d["key"].as_str()?).ok()?, d["priority"].as_u64().map(|n| n as u32).or_else(|| (d["priority"] == "last").then_some(u32::MAX))?)))
            .collect();
        if let Some(client) = self.client().await {
            let _ = client.set_ranks(&ranks);
            let _ = client.set_nodes(&self.ordered(&nodes));
        }
    }

    /// What `app`'s `op` answered last for `input` (JSON), if it only reads: its answer's JSON, and when (ms). For a
    /// screen to show at once while it asks again (bridge.js's `kept`).
    pub fn kept(&self, app: &str, op: &str, input: &[u8]) -> Option<(Vec<u8>, u64)> {
        self.answers.as_ref()?.kept(app, op, input)
    }

    fn forget_answers(&self) {
        if let Some(answers) = &self.answers {
            answers.forget();
        }
    }

    // Keys.

    /// The keys a kind of request needed last time, opened: none it can't open. Each node it's sent to is handed only
    /// those of the scopes it runs (`sealed_for`).
    async fn handed(&self, route: &str, notes: Option<&Notes>) -> Vec<Handed> {
        let Some((_, needs)) = self.needs.lock().unwrap().get(route).cloned() else { return Vec::new() };
        self.fetch_keys(&needs, notes).await;
        let mut handed = Vec::new();
        for n in needs {
            if let Ok(key) = self.database_key(&n).await {
                handed.push(Handed { key, scope: n.scope, epoch: n.epoch, app: n.app, database: n.database });
            }
        }
        handed
    }

    /// A node's 428, taken only from the node that answered (`node`), naming itself and the seal key its entries list,
    /// asking only keys of scopes they say it runs, and of databases that kind of request uses (`keys::may_hand`): what
    /// the request needed, remembered for that kind of request and that node, in place of what it needed before. Fails,
    /// remembering nothing, if it asks for more, or this device can't open them. One of a kind of request that another
    /// learned for since it was sent (`sent_with`, what it went with) learns nothing more.
    async fn learn(&self, route: &str, node: &[u8; 32], answer: &Value, sent_with: Option<(String, Vec<Need>)>, notes: Option<&Notes>) -> Result<(), Error> {
        let lock = self.learning.lock().unwrap().entry(route.to_string()).or_default().clone();
        let _learning = lock.lock().await;
        if self.needs.lock().unwrap().get(route) != sent_with.as_ref() {
            return Ok(());
        }
        let mut need = keys::asked_for(route, &hex(node), answer).ok_or_else(|| failed("A device asked for keys that request doesn't use. Nothing was handed."))?;
        if self.known.lock().unwrap().as_ref().is_none_or(|k| !k.nodes.contains_key(node)) {
            return Err(failed("A device that isn't one of yours asked for keys. Nothing was handed."));
        }
        let scopes: Vec<Scope> = need.iter().map(|n| Scope::parse(&n.scope)).collect::<Result<_, _>>().map_err(|_| failed("A device asked for keys of something that isn't. Nothing was handed."))?;
        let runs = |device: &Device| device.known.lock().unwrap().as_ref().is_some_and(|k| scopes.iter().all(|s| k.who.runs_for(node, s)));
        // What it runs may be newer than this device read: read again, now and then, before refusing.
        if !runs(self) && self.known.lock().unwrap().as_ref().is_some_and(|k| now() > k.at + REREAD) {
            noted(notes, json!({ "step": "members" }), self.read_members()).await?;
        }
        if !runs(self) {
            return Err(failed("A device that doesn't run that asked for its keys. Nothing was handed."));
        }
        // Of each database, only epochs from the one its newest copy is under (`keys::oldest_needed`): older ones open
        // only its history.
        let bucket = Signed::new(self.address.clone(), self.key.to_bytes());
        let prefix = |n: &Need| format!("{}/{}/{}", one::bucket::folder(&n.scope), n.app, n.database);
        // Each database's newest hint, all at once.
        let mut each: BTreeMap<String, &Scope> = BTreeMap::new();
        for (n, scope) in need.iter().zip(&scopes) {
            each.entry(prefix(n)).or_insert(scope);
        }
        let asked = n0_future::join_all(each.keys().map(|p| {
            let (bucket, name) = (&bucket, one::db::newest_name(p));
            async move { one::bucket::Bucket::get(bucket, &name).await }
        }));
        // The keys' own fetching goes at the same time: it only opens what's there, whichever epochs are handed.
        let (hints, ()) = n0_future::future::zip(noted(notes, json!({ "step": "newest", "databases": each.len() }), asked), self.fetch_keys(&need, notes)).await;
        let mut oldest: BTreeMap<String, u32> = BTreeMap::new();
        for ((p, scope), hint) in each.iter().zip(hints) {
            let hint = hint.map_err(failed)?;
            let newest = self.known.lock().unwrap().as_ref().map_or(0, |k| k.who.epoch(scope));
            oldest.insert(p.clone(), keys::oldest_needed(hint.as_ref().map(|o| &o.bytes[..]), newest));
        }
        need.retain(|n| n.epoch >= oldest[&prefix(n)]);
        if need.is_empty() {
            return Err(failed("A device asked for older keys than it needs. Nothing was handed."));
        }
        for n in &need {
            self.database_key(n).await?;
        }
        let mut needs = self.needs.lock().unwrap();
        needs.insert(route.to_string(), (hex(node), need));
        if let Ok(bytes) = serde_json::to_vec(&*needs) {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(self.dir.join("needs.json"), bytes);
        }
        Ok(())
    }

    /// What `need`'s keys are opened from that this device hasn't opened yet, asked for all at once: each scope's epochs
    /// sealed to it, and each app's keys in them, one request each. What doesn't open here is left for `database_key`,
    /// which asks again, one at a time, as it always did.
    async fn fetch_keys(&self, need: &[Need], notes: Option<&Notes>) {
        let scopes: std::collections::BTreeSet<&str> = need.iter().filter(|n| !self.epochs.lock().unwrap().contains_key(&(n.scope.clone(), n.epoch))).map(|n| n.scope.as_str()).collect();
        let apps: std::collections::BTreeSet<(&str, &str)> = need.iter().filter(|n| !self.app_keys.lock().unwrap().contains_key(&(n.scope.clone(), n.app.clone(), n.epoch))).map(|n| (n.scope.as_str(), n.app.as_str())).collect();
        let asked = |path: String, step: Value| async move { noted(notes, step, self.json("GET", &path, None)).await };
        let epochs = n0_future::join_all(scopes.iter().map(|scope| asked(format!("/v1/keys?scope={}&to={}", query(scope), self.key()), json!({ "step": "keys", "of": "epochs", "scope": kind(scope) }))));
        let sealed = n0_future::join_all(apps.iter().map(|(scope, app)| asked(format!("/v1/keys?scope={}&app={}", query(scope), query(app)), json!({ "step": "keys", "of": app, "scope": kind(scope) }))));
        let (epochs, sealed) = n0_future::future::zip(epochs, sealed).await;
        for (scope, kept) in scopes.iter().zip(epochs) {
            if let Ok(kept) = kept {
                self.open_epochs(scope, &kept);
            }
        }
        for ((scope, app), kept) in apps.iter().zip(sealed) {
            for (m, sealed) in kept.iter().flat_map(|k| k["sealed"].as_object()).flatten() {
                let Ok(m) = m.parse::<u32>() else { continue };
                let Some(epoch) = self.epochs.lock().unwrap().get(&(scope.to_string(), m)).copied() else { continue };
                let opened = B64U.decode(sealed.as_str().unwrap_or("").trim().as_bytes()).ok().and_then(|b| one::cloudflare::open_app_key(&epoch, app, &b).ok());
                if let Some(key) = opened {
                    self.app_keys.lock().unwrap().insert((scope.to_string(), app.to_string(), m), key);
                }
            }
        }
    }

    /// Opens each of a scope's epochs in `kept` (what `/v1/keys` gave, sealed to this device) whose hash is the one the
    /// entries this device read itself gave.
    fn open_epochs(&self, scope: &str, kept: &Value) {
        let Ok(parsed) = Scope::parse(scope) else { return };
        let opened: Vec<(u32, [u8; 32])> = kept["sealed"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(m, sealed)| {
                let m = m.parse::<u32>().ok()?;
                let key = B64U.decode(sealed.as_str().unwrap_or("").as_bytes()).ok().and_then(|s| one::cloudflare::open_sealed(&self.opens(), &s).ok())?;
                Some((m, <[u8; 32]>::try_from(key).ok()?))
            })
            .collect();
        let made = |m: u32| self.known.lock().unwrap().as_ref().and_then(|k| k.who.epoch_hash(&parsed, m));
        for (m, key) in opened {
            if made(m) == Some(keys::epoch_hash(&key)) {
                self.epochs.lock().unwrap().insert((scope.to_string(), m), key);
            }
        }
    }

    async fn database_key(&self, n: &Need) -> Result<[u8; 32], Error> {
        Ok(one::cloudflare::database_key(&self.app_key(&n.scope, &n.app, n.epoch).await?, &n.database))
    }

    /// An app's key in epoch `n` of a scope: the one sealed under the epoch, or one made now if it has none yet.
    async fn app_key(&self, scope: &str, app: &str, n: u32) -> Result<[u8; 32], Error> {
        let id = (scope.to_string(), app.to_string(), n);
        if let Some(key) = self.app_keys.lock().unwrap().get(&id) {
            return Ok(*key);
        }
        let epoch = self.epoch(scope, n).await?;
        let kept = self.json("GET", &format!("/v1/keys?scope={}&app={}", query(scope), query(app)), None).await?;
        let sealed = match kept["sealed"][n.to_string()].as_str() {
            Some(s) => s.to_string(),
            None => {
                let made: [u8; 32] = rand::random();
                let sealed = B64U.encode(&one::cloudflare::seal_app_key(&epoch, app, &made));
                let out = self.json("POST", "/v1/keys", Some(json!({ "scope": scope, "n": n, "app": app, "sealed": sealed }))).await?;
                out["sealed"].as_str().unwrap_or_default().to_string()
            }
        };
        let bytes = B64U.decode(sealed.trim().as_bytes()).ok().ok_or_else(|| failed(format!("{app}'s key isn't one")))?;
        let key = one::cloudflare::open_app_key(&epoch, app, &bytes).map_err(|_| failed(format!("{app}'s key didn't open")))?;
        self.app_keys.lock().unwrap().insert(id, key);
        Ok(key)
    }

    /// Epoch `n` of a scope: opened from what's sealed to this device, if its hash is the one the entries this device
    /// read itself gave.
    async fn epoch(&self, scope: &str, n: u32) -> Result<[u8; 32], Error> {
        if let Some(key) = self.epochs.lock().unwrap().get(&(scope.to_string(), n)) {
            return Ok(*key);
        }
        Scope::parse(scope).map_err(failed)?;
        let kept = self.json("GET", &format!("/v1/keys?scope={}&to={}", query(scope), self.key()), None).await?;
        for fresh in [false, true] {
            if fresh {
                self.read_members().await?;
            }
            self.open_epochs(scope, &kept);
            if let Some(key) = self.epochs.lock().unwrap().get(&(scope.to_string(), n)) {
                return Ok(*key);
            }
        }
        Err(failed("This device can't open that yet."))
    }

    /// Starts this person's own scope's first epoch, as a newcomer's first device does, in the order every device does
    /// (cloudflare/web/device.js `newEpoch`, src/node/devices.rs `new_epoch`): a random key, its `epoch` entry naming its
    /// hash written first (which, first in causal order, makes it), then, once the entries say it did, sealed to the
    /// recovery key, to this device and to their other devices.
    async fn first_epoch(&self, scope: &str, user: &str) -> Result<(), Error> {
        let parsed = Scope::parse(scope).map_err(failed)?;
        let to = |device: &Device| -> Result<(Option<[u8; 32]>, Vec<Endpoint>), Error> {
            let known = device.known.lock().unwrap();
            let who = &known.as_ref().ok_or_else(refused)?.who;
            let mut to = vec![device.me];
            to.extend(who.openers(&parsed).into_iter().filter(|c| c.key != device.me.key && c.seal != c.key).map(|c| Endpoint { key: c.key, seal: c.seal }));
            Ok((who.recovery(), to))
        };
        let (recovery, to) = to(self)?;
        let recovery = recovery.ok_or_else(|| failed("Your One App has no recovery key to keep its keys with."))?;
        let key: [u8; 32] = rand::random();
        let hash = keys::epoch_hash(&key);
        self.write("global", user, members::epoch(&parsed, 1, &to.iter().map(|e| e.key).collect::<Vec<_>>(), &hash)).await?;
        self.read_members().await?;
        if self.known.lock().unwrap().as_ref().and_then(|k| k.who.epoch_hash(&parsed, 1)) != Some(hash) {
            return Err(failed("Another device made new keys for this just now. Try again."));
        }
        let recovery = one::cloudflare::twin_public(&recovery).map_err(failed)?;
        let sealed = |to: &[u8; 32]| one::cloudflare::seal_to(to, &key).map(|s| B64U.encode(&s)).map_err(failed);
        self.json("POST", "/v1/keys", Some(json!({ "scope": scope, "n": 1, "to": "recovery", "sealed": sealed(&recovery)? }))).await?;
        for e in &to {
            self.json("POST", "/v1/keys", Some(json!({ "scope": scope, "n": 1, "to": hex(&e.key), "sealed": sealed(&e.seal)? }))).await?;
        }
        self.epochs.lock().unwrap().insert((scope.to_string(), 1), key);
        Ok(())
    }

    /// Signs an entry of `content` in `log`, after what the node has, and writes it.
    async fn write(&self, log: &str, user: &str, content: one::entry::Content) -> Result<(), Error> {
        // Once `global`'s log is frozen, a step of its scope database for the next position (`one::database::steps`).
        if log == "global"
            && let Ok(steps) = self.json("GET", "/v1/steps/global?from=9007199254740991", None).await
        {
            let mut at = steps["at"].as_u64().unwrap_or(0);
            for _ in 0..5 {
                let heads = self.json("GET", "/v1/entries?scope=global", None).await?;
                let epoch = heads["epoch"].as_u64().unwrap_or(0) as u32;
                let step = one::database::steps::sign(&self.key, &Scope::Global, at + 1, user, epoch, now(), content.clone()).map_err(failed)?;
                match self.json("POST", "/v1/steps", Some(json!([step.to_json()]))).await {
                    Ok(_) => return Ok(()),
                    Err(err) if err.to_string().contains("taken") => at = self.json("GET", "/v1/steps/global?from=9007199254740991", None).await?["at"].as_u64().unwrap_or(at + 1),
                    Err(err) => return Err(err),
                }
            }
            return Err(failed("Who's who changed too often to write to: try again."));
        }
        let heads = self.json("GET", &format!("/v1/entries?scope={}", query(log)), None).await?;
        let last = &heads["logs"][self.key()];
        let prev = match (last["seq"].as_u64(), last["hash"].as_str()) {
            (Some(seq), Some(hash)) => Some((seq as u32, Hash::from_hex(hash).map_err(failed)?)),
            _ => None,
        };
        let previous: Vec<Hash> = heads["previous"].as_array().into_iter().flatten().filter_map(|h| Hash::from_hex(h.as_str()?).ok()).filter(|h| Some(*h) != prev.map(|p| p.1)).collect();
        let ext = Ext { log: log.to_string(), time: now(), user: user.to_string(), epoch: heads["epoch"].as_u64().unwrap_or(0) as u32, previous };
        let entry = Entry::create(&self.key, ext, prev, content);
        self.json("POST", "/v1/entries", Some(json!([entry.to_json()]))).await?;
        Ok(())
    }
}

/// What a request did between a node's 428 and sending it again, each thing it asked for timed (developer mode's
/// traces): `{ ms, steps }`, as a browser's says it (cloudflare/web/web.js).
struct Notes {
    started: std::time::Instant,
    steps: Mutex<Vec<Value>>,
}

impl Notes {
    fn new() -> Notes {
        Notes { started: std::time::Instant::now(), steps: Mutex::default() }
    }

    fn done(self) -> Value {
        json!({ "ms": self.started.elapsed().as_millis() as u64, "steps": self.steps.into_inner().unwrap() })
    }

    /// As keys opened before a request was sent: until the last thing asked for was answered, if anything was.
    fn opened(self) -> Option<Value> {
        let steps = self.steps.into_inner().unwrap();
        let ms = steps.iter().map(|s| s["at"].as_u64().unwrap_or(0) + s["ms"].as_u64().unwrap_or(0)).max()?;
        Some(json!({ "ms": ms, "steps": steps }))
    }
}

/// `asked`, timed as `step` in `notes`, if there are any.
async fn noted<T>(notes: Option<&Notes>, mut step: Value, asked: impl Future<Output = T>) -> T {
    let at = std::time::Instant::now();
    let out = asked.await;
    if let Some(notes) = notes {
        step["at"] = json!(at.duration_since(notes.started).as_millis() as u64);
        step["ms"] = json!(at.elapsed().as_millis() as u64);
        notes.steps.lock().unwrap().push(step);
    }
    out
}

/// A scope as a trace names it: its kind alone, never whose.
fn kind(scope: &str) -> &'static str {
    match scope {
        "global" => "everyone",
        s if s.starts_with("group:") => "group",
        _ => "user",
    }
}

/// What a request is, to remember what it needed: an app's call by its app, as is an app's own request (`/<app>/…`,
/// its `_http` call), a screen by its app, anything else by its path.
fn route(path: &str, body: &[u8]) -> String {
    let at = path.split('?').next().unwrap_or(path);
    if let Some((app, rest)) = at.strip_prefix('/').and_then(|p| p.split_once('/'))
        && !app.is_empty() && !rest.is_empty() && !matches!(app, "v1" | "mcp" | "hooks" | "skills")
    {
        return format!("call {app}");
    }
    if at == "/v1/call" {
        let app = serde_json::from_slice::<Value>(body).ok().and_then(|b| b["app"].as_str().map(str::to_string)).unwrap_or_default();
        return format!("call {app}");
    }
    match at.strip_prefix("/v1/screens/").and_then(|s| s.split('/').next()) {
        Some(app) => format!("/v1/screens/{app}"),
        None => at.to_string(),
    }
}

/// What a request asked, as a trace names it: a call's app and operation, a screen's app and name, else its path. Never
/// its input or query.
fn what(path: &str, body: &[u8]) -> Value {
    let at = path.split('?').next().unwrap_or(path);
    if at == "/v1/call" {
        let b = serde_json::from_slice::<Value>(body).unwrap_or_default();
        return json!({ "what": "call", "app": b["app"], "op": b["op"] });
    }
    match at.strip_prefix("/v1/screens/").map(|s| s.split('/')) {
        Some(mut s) => json!({ "what": "page", "app": s.next(), "screen": s.next() }),
        None => json!({ "what": "request", "path": at }),
    }
}

fn query(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn error(out: &Value) -> Option<String> {
    out["error"].as_str().or(out["message"].as_str()).map(str::to_string)
}

/// A request to the install's address (auth, the relay), JSON in and out: → its status and JSON.
async fn http(method: &str, url: &str, token: Option<&str>, body: Option<Value>) -> Result<(u16, Value), Error> {
    let (method, url, token) = (method.to_string(), url.to_string(), token.map(str::to_string));
    tokio::task::spawn_blocking(move || {
        let mut request = ureq::http::Request::builder().method(method.as_str()).uri(&url);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = match body {
            Some(body) => crate::bucket::AGENT.run(request.header("content-type", "application/json").body(keys::canonical(&body)).map_err(failed)?),
            None => crate::bucket::AGENT.run(request.body(()).map_err(failed)?),
        };
        let mut response = response.map_err(|e| failed(format!("Couldn't reach your One App: {e}")))?;
        let status = response.status().as_u16();
        Ok((status, response.body_mut().read_json().unwrap_or(Value::Null)))
    })
    .await
    .map_err(failed)?
}
