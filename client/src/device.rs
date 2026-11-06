//! A native app as a device of its person (ARCHITECTURE.md, Being built: Devices), the same on every app: its own key,
//! which the app keeps in the Keychain, the keystore or the Credential Locker and hands it at start; asking to be let in,
//! with the code it shows; and once a device of theirs approved it, asking its nodes with the keys it opens
//! (`crate::keyring`), keeping what outlives it in its folder. cloudflare/web/device.js is the same in a browser.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use data_encoding::BASE64URL_NOPAD as B64U;
use ed25519_dalek::SigningKey;
use one::config::YOURS;
use one::database::content;
use one::database::scopes::Scope;
use one::keys::{self, Endpoint, Waited, hex};
use serde_json::{Value, json};

use crate::answers::Answers;
use crate::bucket::Signed;
pub(crate) use crate::keyring::now;
use crate::keyring::{Address, Keep, Keyring, error, refused};
use crate::{Body, Client, Error, Request, Response, Sent, unsent};

/// How long a node keeps the keys a request handed it.
const HANDED_FOR: u64 = 3_600_000;
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

/// Where a device is with its network, from its sign-in to its sign-out, as a browser goes the same way (cloudflare/web/web.js:
/// `join`, `start`, `takenOut` and `signOut`). Whether this key was ever let in here is kept on disk (`joined`), as it
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
    answers: Option<Answers>,
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
        }
    }

    /// Says which device it's on: that device's oned key (hex), on a machine with one, or an id of the device's own,
    /// hashed, that outlives signing out. Its request to join names it, and once it's in, its listing does.
    pub fn on(&self, device: &str) {
        if keys::unhex(device).is_ok() {
            *self.on.lock().unwrap() = Some(device.to_string());
        }
    }

    pub fn keeping(mut self) -> Device {
        self.answers = Some(Answers::new(&self.dir, &self.key));
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

    /// The code it shows to be approved: "ABCD EFGH".
    pub fn code(&self) -> String {
        let code = keys::join_code(&self.me);
        format!("{} {}", &code[..4], &code[4..])
    }

    /// Asks auth to let it in, as the person signed in with `token`, calling it `name` (and with someone new's invite
    /// secret, its proof).
    pub async fn ask(&self, token: &str, name: &str, invite: Option<&str>) -> Result<(), Error> {
        let mut asked = json!({ "key": self.key(), "seal": hex(&self.me.seal), "name": name, "reads": one::database::READS });
        if let Some(on) = self.on.lock().unwrap().clone() {
            asked["device"] = json!(on);
        }
        if let Some(invite) = invite.filter(|i| !i.is_empty()) {
            asked["proof"] = json!(keys::invite_proof(invite.as_bytes(), &self.me));
        }
        if let Stage::Off { asked: kept } = &mut *self.stage.lock().await {
            *kept = Some(asked.clone());
        }
        let (status, out) = http("POST", &format!("{}/api/auth/devices/requests", self.address), Some(("authorization", format!("Bearer {token}"))), Some(asked)).await?;
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("Couldn't ask to be let in ({status})."))));
        }
        Ok(())
    }

    /// Whether it's been let in: it reaches a node as itself. While it waits, what auth says of its request
    /// (`keys::waited`) is acted on as every client does: once approved, the network the device that approved it said it's
    /// in (its root) is the only one it takes, kept before it reads any, so none it's shown first can be another; one
    /// gone from auth is asked again unless it's in; a sign-in that's no more fails, to sign in again.
    pub async fn wait(&self, token: &str) -> Result<bool, Error> {
        let at = format!("{}/api/auth/devices/requests/{}", self.address, self.key());
        let (status, said) = http("GET", &at, Some(("authorization", format!("Bearer {token}"))), None).await.unwrap_or((0, Value::Null));
        let gone = match keys::waited(status, &said) {
            Waited::SignedOut => return Err(failed("Sign in again.")),
            Waited::Waiting => return Ok(false),
            Waited::Approved { root, welcome } => {
                for (name, said) in [("root", root), ("welcome", welcome)] {
                    if let Some(said) = said {
                        self.put(name, said.as_bytes()).await.map_err(failed)?;
                    }
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
            let _ = http("POST", &format!("{}/api/auth/devices/requests", self.address), Some(("authorization", format!("Bearer {token}"))), Some(asked)).await;
        }
        Ok(false)
    }

    /// Reaches its network's nodes, if it's been let in: false if not yet. Then, each time, as each start of a browser
    /// does, it settles its person's own things (`settle_own`), if they aren't: what fails is tried again at the next.
    pub async fn connect(&self) -> Result<bool, Error> {
        if !self.reach().await? {
            return Ok(false);
        }
        let _ = self.settle_own().await;
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
        client.set_ranks(&self.keyring.ranks())?;
        client.set_wakes(self.keyring.cloudflare_node())?;
        client.set_online(&online)?;
        *stage = Stage::In(client);
        // Kept, with its key, so this key is known as one that was in, to its person's sign-in once the relay takes it
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

    /// As a person's first device does: says where their own things run (the machines that run everyone's), if they
    /// haven't, and makes their own scope's first epoch, if it has none (cloudflare/web/device.js `settleOwn`). What's to do
    /// is looked at again in what it reads fresh, as another device of theirs may have done it.
    async fn settle_own(&self) -> Result<(), Error> {
        let device = self.on.lock().unwrap().clone();
        let todo = || -> Result<_, Error> {
            self.keyring
                .who(|who| -> anyhow::Result<_> {
                    let me = who.machine(&self.me.key)?;
                    let user = me.as_ref().map(|d| d.user.clone()).unwrap_or_default();
                    let on = who.runs_until_chosen(&user)?;
                    // Listed before sign-ins named their device: it lists itself again, naming it.
                    let unnamed = me.filter(|d| d.device.is_none() && d.passkey.is_none() && device.is_some()).map(|d| (d.name, d.roles.into_iter().collect::<Vec<_>>()));
                    let first = who.epoch(&Scope::User(user.clone()))? == 0;
                    Ok((!on.is_empty() || unnamed.is_some() || first).then_some((user, on, unnamed, first)))
                })
                .ok_or_else(refused)?
                .map_err(failed)
        };
        if todo()?.is_none() {
            return Ok(());
        }
        self.keyring.read(self).await?;
        let Some((user, on, unnamed, first)) = todo()? else { return Ok(()) };
        if let (Some(device), Some((name, roles))) = (device, unnamed) {
            let mut listed = content::device(&self.me, &user, &name, &roles, None);
            listed.data["device"] = json!(device);
            self.write(&user, listed).await?;
        }
        if !on.is_empty() {
            self.write(&user, content::runs(&on)).await?;
        }
        if first {
            self.new_epoch(&Scope::User(user.clone()), 1, &[]).await?;
        }
        Ok(())
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
    /// as a browser's does (web.js `forgetDevice` and `signOut`): what each kind of request needed, its calls' answers
    /// and the network it was let into, which the next approval names again. Whether this key was in stays, for
    /// `removed` to say.
    async fn out(&self) {
        if let Stage::In(client) = std::mem::replace(&mut *self.stage.lock().await, Stage::Out) {
            client.close().await;
        }
        self.keyring.forget(self).await;
        self.forget_answers();
    }

    /// Signs out: this device takes itself out of its person's devices first (`leave`), reaching the network first if
    /// its connections were closed, so signing in again makes a new one that replaces it, never a second; then it's
    /// out, and forgets it was in. A network that doesn't answer in a few seconds is left: approving the new device
    /// replaces it. The app forgets this device's key either way, as a removed key can't be let in again.
    pub async fn sign_out(&self) {
        if self.was_in() {
            let leave = async {
                if self.reach().await? {
                    self.leave().await?;
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
    /// to, and its person as who's who lists it, so auth pushes it when another device of theirs asks to join. Once
    /// it's out of the network, the Worker forgets it.
    pub async fn register_push(&self, address: &one::push::Address) -> Result<(), Error> {
        let user = self.keyring.user();
        let body = one::push::register_request(&self.key, &self.me.seal, address, user.as_deref(), now()).map_err(failed)?;
        let (status, out) = http("POST", &format!("{}/push/register", self.address), None, Some(body)).await?;
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{YOURS}'s push service didn't take this device ({status})."))));
        }
        Ok(())
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

    /// A request to its network's nodes as its person (`Keyring::request`), given up on once the relay says this device
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
            return Ok(Response { body: Body::whole(bytes), ..response });
        }
        Ok(response)
    }

    /// The order things run in, from the node's `/v1/state`; a node it lists that this device didn't know of has it
    /// read `global` again (now and then), and only nodes its own reading lists are asked.
    async fn learn_order(&self, bytes: &[u8]) {
        let Ok(state) = serde_json::from_slice::<Value>(bytes) else { return };
        let mut said: Vec<(u64, [u8; 32])> = state["network"]["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["serves_me"] == true)
            .filter_map(|d| Some((d["order"].as_u64().unwrap_or(u64::MAX), keys::unhex(d["key"].as_str()?).ok()?)))
            .collect();
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

    /// What `app`'s `tool` answered last for `input` (JSON), if it only reads: its answer's JSON, and when (ms). For a
    /// screen to show at once while it asks again (bridge.js's `kept`).
    pub fn kept(&self, app: &str, tool: &str, input: &[u8]) -> Option<(Vec<u8>, u64)> {
        self.answers.as_ref()?.kept(app, tool, input)
    }

    fn forget_answers(&self) {
        if let Some(answers) = &self.answers {
            answers.forget();
        }
    }

    /// A request's JSON, handing no keys, to a node of its person's.
    async fn json(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, Error> {
        let client = self.client().await.ok_or_else(refused)?;
        self.keyring.json(&client, method, path, body).await
    }

    /// Starts epoch `n` of a scope, in the order every device does (cloudflare/web/device.js `newEpoch`): a random key,
    /// its `epoch` step naming its hash written first (which, first in causal order, makes it), then, once who's who
    /// says it did, sealed to the recovery key and to each client device of the people who open it but `except` (keys
    /// being taken out), this device among them unless it's going.
    async fn new_epoch(&self, scope: &Scope, n: u32, except: &[[u8; 32]]) -> Result<(), Error> {
        let user = self.keyring.user().ok_or_else(refused)?;
        let (recovery, to) = self
            .keyring
            .who(|who| -> anyhow::Result<_> {
                let mut to: Vec<Endpoint> = who.openers(scope)?.into_iter().filter(|c| c.seal != c.key && !except.contains(&c.key)).map(|c| Endpoint { key: c.key, seal: c.seal }).collect();
                if !to.iter().any(|e| e.key == self.me.key) && !except.contains(&self.me.key) {
                    to.push(self.me);
                }
                Ok((who.recovery()?, to))
            })
            .ok_or_else(refused)?
            .map_err(failed)?;
        let key: [u8; 32] = rand::random();
        let hash = keys::epoch_hash(&key);
        self.write(&user, content::epoch(scope, n, &to.iter().map(|e| e.key).collect::<Vec<_>>(), &hash)).await?;
        self.keyring.read(self).await?;
        if self.keyring.who(|who| who.epoch_hash(scope, n)).transpose().map_err(failed)?.flatten() != Some(hash) {
            return Err(failed("Another device made new keys for this just now. Try again."));
        }
        let sealed = |to: &[u8; 32]| keys::seal_to(to, &key).map(|s| B64U.encode(&s)).map_err(failed);
        // Not while there's none (the one there was forked `global`), until an admin names the next.
        if let Some(recovery) = recovery {
            let recovery = keys::twin_public(&recovery).map_err(failed)?;
            self.json("POST", "/v1/keys", Some(json!({ "scope": scope.to_string(), "n": n, "to": "recovery", "sealed": sealed(&recovery)? }))).await?;
        }
        for e in &to {
            self.json("POST", "/v1/keys", Some(json!({ "scope": scope.to_string(), "n": n, "to": hex(&e.key), "sealed": sealed(&e.seal)? }))).await?;
        }
        self.keyring.made_epoch(scope, n, key);
        Ok(())
    }

    /// A new epoch of each scope this person may start one of (an admin of it), sealed to none of `except`.
    async fn renew(&self, scopes: &[Scope], except: &[[u8; 32]]) -> Result<(), Error> {
        let user = self.keyring.user().ok_or_else(refused)?;
        for scope in scopes {
            let newest = self
                .keyring
                .who(|who| -> anyhow::Result<_> { Ok((who.role_in(&user, scope)? == Some(one::database::scopes::Role::Admin)).then(|| who.epoch(scope)).transpose()?.unwrap_or(0)) })
                .ok_or_else(refused)?
                .map_err(failed)?;
            if newest > 0 {
                self.new_epoch(scope, newest + 1, except).await?;
            }
        }
        Ok(())
    }

    /// Tells the relay something (`{ add, admin? }` or `{ remove }`), signed by this device.
    async fn tell(&self, fields: Value) -> Result<(), Error> {
        let url = format!("{}/relay/tell", self.address);
        let mut again = true;
        loop {
            let at = keys::next_at(now());
            let (status, out) = http("POST", &url, None, Some(keys::signed_request(&self.key, one::protocol::RELAY, fields.clone(), at))).await?;
            if (200..300).contains(&status) {
                return Ok(());
            }
            // Signed again once if another process of this device signed in the same millisecond, which the relay takes once.
            if again && out["code"] == one::protocol::code::REPLAYED {
                again = false;
                tokio::time::sleep(keys::again_in(at)).await;
                continue;
            }
            return Err(Error { code: out["code"].as_str().map(str::to_string), ..failed(error(&out).unwrap_or_else(|| format!("{YOURS} didn't take that ({status})."))) });
        }
    }

    /// Takes a key off the relay's list; one it doesn't list is out already.
    async fn take_out(&self, key: &[u8; 32]) -> Result<(), Error> {
        match self.tell(json!({ "remove": hex(key) })).await {
            Err(err) if err.code.as_deref() != Some(one::protocol::code::NOT_LISTED) => Err(err),
            _ => Ok(()),
        }
    }

    /// Takes one of this person's devices out (or, for an admin, anyone's), as device.js `remove` does: off the network
    /// first, then unlisted, it and its parts; then each scope its person opens that this person may move on gets a new
    /// epoch it doesn't have. Itself, it leaves (`leave`).
    pub async fn remove(&self, key: &[u8; 32]) -> Result<(), Error> {
        if *key == self.me.key {
            return self.leave().await;
        }
        self.keyring.read(self).await?;
        let user = self.keyring.user().ok_or_else(refused)?;
        let (keys, scopes) = self
            .keyring
            .who(|who| -> anyhow::Result<_> {
                let it = who.machine(key)?.filter(|d| !d.removed).ok_or_else(|| anyhow::anyhow!("That device isn’t listed."))?;
                anyhow::ensure!(who.may(&user, "remove", &one::database::scopes::device(key)), "Only its person or an admin takes a device out.");
                let keys: Vec<[u8; 32]> = std::iter::once(*key).chain(who.machines()?.into_iter().filter(|d| d.device == Some(*key) && d.key != *key && !d.removed).map(|d| d.key)).collect();
                let scopes = who.scopes()?.into_iter().filter(|s| who.may(&it.user, "use", &one::database::scopes::object(s))).collect::<Vec<_>>();
                Ok((keys, scopes))
            })
            .ok_or_else(refused)?
            .map_err(failed)?;
        for k in &keys {
            self.take_out(k).await?;
            self.write(&user, content::remove_device(k)).await?;
        }
        self.renew(&scopes, &keys).await
    }

    /// Takes a person out, for an admin, as device.js `removePerson` does: each of their devices off the network first,
    /// then them, then each scope they could open but their own gets a new epoch they don't have.
    pub async fn remove_person(&self, gone: &str) -> Result<(), Error> {
        self.keyring.read(self).await?;
        let user = self.keyring.user().ok_or_else(refused)?;
        if gone == user {
            return Err(failed("You can’t remove yourself. Another admin can."));
        }
        let (keys, scopes) = self
            .keyring
            .who(|who| -> anyhow::Result<_> {
                anyhow::ensure!(who.may(&user, "remove", one::database::scopes::NETWORK), "Only an admin takes someone out.");
                let keys: Vec<[u8; 32]> = who.machines()?.into_iter().filter(|d| d.user == gone && !d.removed).map(|d| d.key).collect();
                let own = Scope::User(gone.to_string());
                let scopes = who.scopes()?.into_iter().filter(|s| *s != own && who.may(gone, "use", &one::database::scopes::object(s))).collect::<Vec<_>>();
                Ok((keys, scopes))
            })
            .ok_or_else(refused)?
            .map_err(failed)?;
        for k in &keys {
            self.take_out(k).await?;
        }
        self.write(&user, content::remove_user(gone)).await?;
        self.renew(&scopes, &keys).await
    }

    /// Takes this device out of its person's devices, as it signs out, so signing in again makes a new one that
    /// replaces it, never a second. Nothing it writes after counts, so it goes in this order: its removal, then off the
    /// network. Its scopes keep their keys, as it forgets its own (ARCHITECTURE.md).
    async fn leave(&self) -> Result<(), Error> {
        self.keyring.read(self).await?;
        let Some(user) = self.keyring.user() else { return Ok(()) };
        // A node closes a removed device's connection as it applies its removal, so its answer may never come: off
        // the network all the same.
        let wrote = self.write(&user, content::remove_device(&self.me.key)).await;
        self.take_out(&self.me.key).await?;
        wrote
    }

    /// Asks the network's sign-in something as this device, signed with its key (`one/auth`), in `x-one-request`,
    /// signed for `path` without its query.
    async fn ask_sign_in(&self, method: &str, path: &str) -> Result<Value, Error> {
        let route = format!("{method} {}", path.split('?').next().unwrap_or_default());
        let signed = keys::signed_request(&self.key, one::protocol::AUTH_DOMAIN, json!({ "route": route }), now());
        let header = B64U.encode(&serde_json::to_vec(&signed).map_err(failed)?);
        let (status, out) = http(method, &format!("{}{path}", self.address), Some(("x-one-request", header)), None).await?;
        if !(200..300).contains(&status) {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{YOURS}'s sign-in answered {status}."))));
        }
        Ok(out)
    }

    /// Its person's other devices waiting to be let in, as auth says, but those taken out or turned down, each with
    /// the code it shows ("ABCD EFGH").
    async fn waiting(&self) -> Result<Vec<(Value, Endpoint, String)>, Error> {
        self.keyring.read(self).await?;
        let user = self.keyring.user().ok_or_else(refused)?;
        let asked = self.ask_sign_in("GET", "/api/auth/devices/requests").await?;
        let mut out = Vec::new();
        for r in asked["requests"].as_array().into_iter().flatten() {
            let (Ok(key), Ok(seal)) = (keys::unhex(r["key"].as_str().unwrap_or("")), keys::unhex(r["seal"].as_str().unwrap_or(""))) else { continue };
            let gone = self.keyring.who(|who| who.machine_ever(&key).map(|m| m.is_some_and(|m| m.removed))).ok_or_else(refused)?.map_err(failed)?;
            if r["user"] != json!(user) || key == self.me.key || r["service"].is_string() || gone {
                continue;
            }
            let e = Endpoint { key, seal };
            let code = keys::join_code(&e);
            out.push((r.clone(), e, format!("{} {}", &code[..4], &code[4..])));
        }
        Ok(out)
    }

    /// The one of its person's waiting with `code`, as typed; none if none is (someone new, invited by a link a node
    /// made, is let in by that node).
    async fn waiting_with(&self, code: &str) -> Result<Option<(Value, Endpoint)>, Error> {
        let code: String = code.to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let mut found: Vec<_> = self.waiting().await?.into_iter().filter(|(_, _, c)| c.replace(' ', "") == code).collect();
        if found.len() > 1 {
            return Err(failed("Two devices are waiting with that code, so neither was let in. Start again on yours, for a new code."));
        }
        Ok(found.pop().map(|(r, e, _)| (r, e)))
    }

    /// Devices waiting that this device may let in, as Settings › Devices shows them: its person's own, and someone
    /// new invited by a link a node made, as that node says.
    async fn requests(&self) -> Result<Value, Error> {
        let mut out: Vec<Value> = self.waiting().await?.into_iter().map(|(r, _, code)| json!({ "device": r["name"], "person": r["person"], "code": code, "made": r["made"], "place": r["place"], "yours": true })).collect();
        let invited = self.json("GET", "/v1/devices/requests", None).await?;
        out.extend(invited.as_array().into_iter().flatten().filter(|r| r["new"] == true).cloned());
        Ok(json!(out))
    }

    /// What `r`'s sign-in shows it signed in with, checked here (`one::sign_in_proof`); none if it shows nothing
    /// that checks out.
    async fn signed_in_with(&self, r: &Value, e: &Endpoint) -> Option<one::sign_in_proof::Account> {
        let proof: one::sign_in_proof::Proof = serde_json::from_value(r["sign_in"].clone()).ok()?;
        let (root, clients) = self.keyring.who(|who| -> anyhow::Result<_> { Ok((who.root()?, who.setting(one::sign_in_proof::CLIENTS)?)) })?.ok()?;
        let network = one::sign_in_proof::Network::new(&self.address, *root?.as_bytes(), clients.as_deref());
        let jwks = match proof.issuer().and_then(|i| one::sign_in_proof::keys_at(&i, &self.address)) {
            Some(at) => Some(http("GET", &at, None, None).await.ok().filter(|(s, _)| *s == 200)?.1),
            None => None,
        };
        one::sign_in_proof::check(&proof, e, &network, jwks.as_ref(), now()).ok()
    }

    /// Lets in one of its person's devices that asked, as device.js `approve` does: lists it (with what it signed in
    /// with, the device it's on and where it asked from), seals it each epoch it opens of the scopes they use, takes
    /// their sign-in before on the same device out, lets it onto the network, and marks its request approved with where
    /// this device read who's who to.
    async fn approve(&self, r: &Value, e: &Endpoint) -> Result<(), Error> {
        let user = self.keyring.user().ok_or_else(refused)?;
        let on = r["device"].as_str().map(keys::unhex).transpose().map_err(|_| failed("The device it's on is 64 hex characters."))?;
        let (listed, scopes, admin, before) = self
            .keyring
            .who(|who| -> anyhow::Result<_> {
                let listed = who.client(&e.key)?.is_some_and(|c| c.user == user && c.seal == e.seal);
                let scopes = who.scopes()?.into_iter().filter(|s| who.may(&user, "use", &one::database::scopes::object(s))).collect::<Vec<_>>();
                let before: Vec<[u8; 32]> = match on {
                    Some(on) => who.clients()?.into_iter().filter(|c| c.user == user && c.device == Some(on) && c.key != e.key && c.passkey.is_none()).map(|c| c.key).collect(),
                    None => Vec::new(),
                };
                Ok((listed, scopes, who.may(&user, "invite", one::database::scopes::NETWORK), before))
            })
            .ok_or_else(refused)?
            .map_err(failed)?;
        if !listed {
            let name = r["name"].as_str().unwrap_or("");
            let mut content = content::device(e, &user, name, &[one::database::scopes::Kind::Client], None);
            if let Some(on) = &on {
                content.data["device"] = json!(hex(on));
            }
            if let Some(account) = self.signed_in_with(r, e).await {
                content.data["sign_in"] = json!(account);
            }
            if r["place"].is_string() {
                content.data["place"] = r["place"].clone();
            }
            self.write(&user, content).await?;
        }
        for scope in &scopes {
            self.seal_held(scope, e).await?;
        }
        match self.tell(json!({ "add": hex(&e.key), "role": "client", "admin": admin })).await {
            Err(err) if !(listed && err.code.as_deref() == Some(one::protocol::code::ALREADY)) => return Err(err),
            _ => {}
        }
        for old in before {
            self.take_out(&old).await?;
            self.write(&user, content::remove_device(&old)).await?;
        }
        self.keyring.read(self).await?;
        let root = self.keyring.who(|who| who.root()).ok_or_else(refused)?.map_err(failed)?.ok_or_else(refused)?;
        let welcome = self.keyring.welcome().map(|w| format!("&welcome={w}")).unwrap_or_default();
        self.ask_sign_in("DELETE", &format!("/api/auth/devices/requests/{}?approved&root={}{welcome}", hex(&e.key), root.to_hex())).await?;
        Ok(())
    }

    /// Seals each epoch of `scope` sealed to this device to `to`, but those it has, each checked against the hash
    /// who's who names.
    async fn seal_held(&self, scope: &Scope, to: &Endpoint) -> Result<(), Error> {
        let mine = self.json("GET", &format!("/v1/keys?scope={}&to={}", scope, hex(&self.me.key)), None).await?;
        let secret = keys::seal_secret(&self.key);
        for (n, sealed) in mine["sealed"].as_object().into_iter().flatten() {
            let n: u32 = n.parse().map_err(failed)?;
            let key = keys::open_sealed(&secret, &B64U.decode(sealed.as_str().unwrap_or("").as_bytes()).map_err(failed)?).map_err(failed)?;
            let key: [u8; 32] = key.try_into().map_err(|_| failed("A key for this didn't check out, so it wasn't used."))?;
            if self.keyring.who(|who| who.epoch_hash(scope, n)).transpose().map_err(failed)?.flatten() != Some(keys::epoch_hash(&key)) {
                return Err(failed("A key for this didn't check out, so it wasn't used."));
            }
            let body = json!({ "scope": scope.to_string(), "n": n, "to": hex(&to.key), "sealed": B64U.encode(&keys::seal_to(&to.seal, &key).map_err(failed)?) });
            match self.json("POST", "/v1/keys", Some(body)).await {
                Err(err) if err.code.as_deref() != Some(one::protocol::code::ALREADY) => return Err(err),
                _ => {}
            }
        }
        Ok(())
    }

    /// Turns down one of its person's devices waiting: a step, so none of their devices lets it in after, then its
    /// request forgotten at auth.
    async fn refuse(&self, r: &Value, e: &Endpoint) -> Result<(), Error> {
        let user = self.keyring.user().ok_or_else(refused)?;
        self.write(&user, content::refuse(e, r["name"].as_str().unwrap_or(""))).await?;
        self.ask_sign_in("DELETE", &format!("/api/auth/devices/requests/{}", hex(&e.key))).await?;
        Ok(())
    }

    /// What Settings changes that this device signs itself, as its person's device (web.js `signs`, the same paths):
    /// → the answer, or `None` for a path it leaves to a node.
    pub async fn settings(&self, method: &str, path: &str, body: &Value) -> Option<Result<Value, Error>> {
        let s = |k: &str| body[k].as_str().unwrap_or("").to_string();
        let role = |v: &Value| serde_json::from_value::<one::database::scopes::Role>(v.clone()).map_err(failed);
        let key = |v: String| keys::unhex(&v).map_err(|_| failed("That device isn’t listed."));
        let write = async |c: Result<content::Content, Error>| -> Result<Value, Error> {
            let user = self.keyring.user().ok_or_else(refused)?;
            self.write(&user, c?).await?;
            Ok(json!({}))
        };
        Some(match (method, path) {
            ("POST", "/v1/role") => write(role(&body["role"]).map(|r| content::role(&s("user"), r))).await,
            ("POST", "/v1/remove") if body["user"].is_string() => self.remove_person(&s("user")).await.map(|()| json!({})),
            ("POST", "/v1/remove") => match key(s("key")) {
                Ok(k) => self.remove(&k).await.map(|()| json!({})),
                Err(err) => Err(err),
            },
            ("POST", "/v1/settings") => write(Ok(content::setting(&s("name"), body["value"].as_str()))).await,
            ("POST", "/v1/invite") => write(role(&body.get("role").cloned().unwrap_or(json!("member"))).map(|r| content::invite(&s("group"), &s("user"), r))).await,
            ("POST", "/v1/accept") => write(Ok(content::accept(&s("group")))).await,
            ("POST", "/v1/member") => write(if body["role"].is_null() { Ok(None) } else { role(&body["role"]).map(Some) }.map(|r| content::member(&s("group"), &s("user"), r))).await,
            ("GET", "/v1/devices/requests") => self.requests().await,
            ("POST", "/v1/devices/approve" | "/v1/devices/refuse") => match self.waiting_with(&s("code")).await {
                Ok(None) => return None,
                Ok(Some((r, _))) if path.ends_with("approve") && body["confirm"] != true => Ok(json!({ "device": r["name"], "person": r["person"], "new": false })),
                Ok(Some((r, e))) if path.ends_with("approve") => self.approve(&r, &e).await.map(|()| json!({ "device": r["name"] })),
                Ok(Some((r, e))) => self.refuse(&r, &e).await.map(|()| json!({ "device": r["name"] })),
                Err(err) => Err(err),
            },
            ("POST", "/v1/devices/forget-sign-in") => write(key(s("key")).map(|k| content::forget_sign_in(&k))).await,
            ("POST", "/v1/runs") => {
                let on: Result<Vec<[u8; 32]>, Error> = body["on"].as_array().into_iter().flatten().map(|k| key(k.as_str().unwrap_or("").to_string())).collect();
                write(on.map(|on| content::runs(&on))).await
            }
            ("POST", "/v1/group") => {
                let name = s("name").trim().to_string();
                if name.is_empty() {
                    return Some(Err(failed("Give the group a name.")));
                }
                let group = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 8]>());
                write(Ok(content::group(&group, &name))).await.map(|_| json!({ "group": group }))
            }
            ("POST", "/v1/handle") => {
                let (what, id) = if body["group"].is_string() { ("group", s("group")) } else { ("user", self.keyring.user().unwrap_or_default()) };
                write(Ok(content::handle(what, &id, s("handle").trim()))).await
            }
            ("POST", "/v1/name") => {
                let id = if body["key"].is_string() { s("key") } else { hex(&self.me.key) };
                write(Ok(content::rename("key", &id, s("name").trim()))).await
            }
            ("POST", "/v1/service") => write(key(s("device")).map(|d| content::service(&d, &s("name"), body["on"] == true, None))).await,
            _ => return None,
        })
    }

    /// Signs a step of `content` in `global`'s scope database for the next position (`one::database::steps`), and
    /// writes it, signing it again for the one after while another device's goes first.
    async fn write(&self, user: &str, content: content::Content) -> Result<(), Error> {
        let newest = format!("/v1/steps/global?from={}", one::database::steps::NUMBER_MAX);
        let next = || self.json("GET", &newest, None);
        let mut steps = next().await?;
        for _ in 0..5 {
            let (at, epoch) = (steps["at"].as_u64().unwrap_or(0), steps["epoch"].as_u64().unwrap_or(0) as u32);
            let step = one::database::steps::sign(&self.key, &Scope::Global, at + 1, user, epoch, now(), content.clone()).map_err(failed)?;
            match self.json("POST", "/v1/steps", Some(json!([step.to_json()]))).await {
                Ok(_) => return Ok(()),
                Err(err) if err.code.as_deref() == Some(one::protocol::code::TAKEN) => steps = next().await?,
                Err(err) => return Err(err),
            }
        }
        Err(failed("Who's who changed too often to write to: try again."))
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

impl Address for Device {
    type Bucket = Signed;

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
