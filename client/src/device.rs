//! A native app as a device of its person (ARCHITECTURE.md, Being built: Devices), the same on every app: its own key,
//! which the app keeps in the Keychain, the keystore or the Credential Locker and hands it at start; asking to be let in,
//! with the code it shows; and once a device of theirs approved it, its scopes' epochs, sealed to it, from which it hands
//! each request the database keys it needed last time (`x-one-keys`), sent again once with those a node's 428 names.
//! kernel/web/device.js is the same in a browser.
//!
//! It trusts no node, nor the relay, for who's who: it reads `global`'s entries from the bucket itself, opens them with
//! the epochs sealed to it, and replays them by the node's own rules (`crate::members`, as browsers and the MCP
//! Worker's devices do). It asks only endpoints those list
//! as compute that run its person's things, takes a 428 only from the node that answered, naming the seal key they list
//! for it, and only for scopes they say it runs, databases that kind of request uses (`keys::may_hand`) and epochs
//! from each one's newest copy (`keys::oldest_needed`), and hands what it named only to that node, sealed to that key.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use one::entry::{Entry, Ext, Hash, Scope, hex};
use one::keys::{self, Endpoint, Handed, Handoff, Need};
use one::members::{self, Members};
use data_encoding::BASE64URL_NOPAD as B64U;
use serde_json::{Value, json};

use crate::bucket::Signed;
use crate::{Body, Client, Error, Request, Response, not_let_in, unsent};

/// How long a node keeps the keys a request handed it.
const HANDED_FOR: u64 = 3_600_000;
/// The most keys one kind of request is handed: a 428 naming more is refused.
const MOST_NEEDED: usize = 16;
/// How often, at most, it reads `global`'s entries again for a node it didn't know.
const REREAD: u64 = 30_000;

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
    members: Members,
    /// Its person.
    user: String,
    /// The nodes it may ask, by key: their seal keys; and the serverless node, asked last, if the install has one.
    nodes: HashMap<[u8; 32], [u8; 32]>,
    serverless: Option<[u8; 32]>,
    /// When it was read.
    at: u64,
}

/// A device's way in: its keys, the install it's in, and what it has opened.
pub struct Device {
    key: SigningKey,
    me: Endpoint,
    address: String,
    /// Where it keeps the install's genesis, once it first read it, and what each kind of request needed.
    dir: PathBuf,
    client: tokio::sync::Mutex<Option<Client>>,
    known: Mutex<Option<Known>>,
    /// The order things run in, as the node last said (`/v1/state`): only ever used to sort nodes it knows.
    order: Mutex<Vec<[u8; 32]>>,
    /// What it asked auth to let it in with, to ask again once that waited too long.
    asked: Mutex<Option<Value>>,
    /// Each scope's epochs opened, by (scope, n), and apps' keys by (scope, app, n).
    epochs: Mutex<HashMap<(String, u32), [u8; 32]>>,
    app_keys: Mutex<HashMap<(String, String, u32), [u8; 32]>>,
    /// What each kind of request needed, by its route (`route`): the last 428's, and the node that sent it (hex), the
    /// only one they're handed to.
    needs: Mutex<BTreeMap<String, (String, Vec<Need>)>>,
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
            client: tokio::sync::Mutex::default(),
            known: Mutex::default(),
            order: Mutex::default(),
            asked: Mutex::default(),
            epochs: Mutex::default(),
            app_keys: Mutex::default(),
            needs: Mutex::new(needs),
        }
    }

    /// Its key, hex: its id.
    pub fn key(&self) -> String {
        hex(&self.me.key)
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
        if let Some(invite) = invite.filter(|i| !i.is_empty()) {
            asked["proof"] = json!(keys::invite_proof(invite.as_bytes(), &self.me));
        }
        *self.asked.lock().unwrap() = Some(asked.clone());
        let (status, out) = http("POST", &format!("{}/api/auth/devices/requests", self.address), Some(token), Some(asked)).await?;
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("Couldn't ask to be let in ({status})."))));
        }
        Ok(())
    }

    /// Whether it's been let in: it reaches a node as itself. While it waits, a request that waited too long or was
    /// turned down is asked again. Once approved, the install the device that approved it said it's in (its genesis) is
    /// the only one it takes, kept before it reads any: none it's shown first can be another.
    pub async fn wait(&self, token: &str) -> Result<bool, Error> {
        let pinned = self.dir.join("genesis");
        let asked_at = format!("{}/api/auth/devices/requests/{}", self.address, self.key());
        let mut said = None;
        if !pinned.exists() {
            let (status, out) = http("GET", &asked_at, Some(token), None).await?;
            if status == 200 && out["approved"] != true {
                return Ok(false);
            }
            if let Some(genesis) = out["genesis"].as_str().and_then(|g| Hash::from_hex(g).ok()) {
                let _ = std::fs::create_dir_all(&self.dir);
                std::fs::write(&pinned, genesis.to_hex()).map_err(failed)?;
            }
            said = Some(status);
        }
        if self.connect().await? {
            return Ok(true);
        }
        let status = match said {
            Some(status) => status,
            None => http("GET", &asked_at, Some(token), None).await?.0,
        };
        let asked = self.asked.lock().unwrap().clone();
        if status == 404
            && let Some(asked) = asked
        {
            let (status, out) = http("POST", &format!("{}/api/auth/devices/requests", self.address), Some(token), Some(asked)).await?;
            if status == 401 {
                return Err(failed(error(&out).unwrap_or_else(|| "Sign in again.".into())));
            }
        }
        Ok(false)
    }

    /// Reaches its install's nodes, if it's been let in: false if not yet. Once in, it's in for good.
    pub async fn connect(&self) -> Result<bool, Error> {
        let mut held = self.client.lock().await;
        if held.is_some() {
            return Ok(true);
        }
        // The relay says who's on its network: until it lets this device in, nothing else will.
        let Some(relayed) = self.computes().await? else { return Ok(false) };
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
        *held = Some(client);
        drop(held);
        // Kept, so a device the install no longer lists is known as one that was in, to its person's sign-in, not to approve.
        if !self.dir.join("joined").exists() {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(self.dir.join("joined"), b"");
        }
        self.settle_own().await?;
        Ok(true)
    }

    /// Whether the relay lets this device in: → the compute endpoints on its network, which it says are there, not
    /// what they may do.
    async fn computes(&self) -> Result<Option<Vec<[u8; 32]>>, Error> {
        let asked = keys::signed_request(&self.key, "one/relay", json!({ "computes": true }), now());
        let (status, out) = http("POST", &format!("{}/relay/computes", self.address), None, Some(asked)).await?;
        if status == 403 || status == 401 {
            return Ok(None);
        }
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("Your One App's network answered {status}."))));
        }
        Ok(Some(out["computes"].as_array().into_iter().flatten().filter_map(|c| keys::unhex(c.as_str().unwrap_or("")).ok()).collect()))
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
                one::kernel::open_sealed(&self.opens(), &o.bytes).ok().and_then(|k| k.try_into().ok()).map(Some).ok_or_else(|| failed("A key for this didn't open."))
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
            let app = one::kernel::open_app_key(epoch, members::APP, &o.bytes).map_err(failed)?;
            log_keys.insert(*n, one::kernel::database_key(&app, one::log::NAME));
        }
        let read = one::log::open(&bucket, &log_keys, &one::log::prefix("global"), "global").await.map_err(failed)?;
        // Replayed as every client device replays them (crate::members), of the install whose genesis it kept.
        let pinned = self.dir.join("genesis");
        let kept = std::fs::read_to_string(&pinned).ok().and_then(|had| Hash::from_hex(had.trim()).ok());
        let mut global = crate::members::Global::new(kept);
        global.add_entries(read.entries).map_err(|_| failed("This isn't the One App this device joined."))?;
        let m = global.members().map_err(failed)?;
        if kept.is_none()
            && let Some(root) = global.root()
        {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(&pinned, root.to_hex());
        }
        // Every epoch it was sealed must be the one the entries made.
        for (n, key) in &epochs {
            if m.epoch_hashes.get(&(Scope::Global, *n)) != Some(&keys::epoch_hash(key)) {
                return Err(failed("A key for this didn't check out, so it wasn't used."));
            }
            self.epochs.lock().unwrap().insert(("global".into(), *n), *key);
        }
        let user = m.machine(&self.me.key).map(|d| d.user.clone()).ok_or_else(|| failed("This device isn't listed yet."))?;
        let mut nodes: HashMap<[u8; 32], [u8; 32]> = m.computes().filter(|c| m.serves(&c.key, &user)).map(|c| (c.key, c.seal)).collect();
        // The serverless node, as an admin's setting names it, runs anyone's calls: its seal key is its key's twin.
        let serverless = m.settings.get("serverless").and_then(|k| keys::unhex(k).ok());
        if let Some(key) = serverless
            && let Ok(seal) = one::kernel::twin_public(&key)
        {
            nodes.insert(key, seal);
        }
        *self.known.lock().unwrap() = Some(Known { members: m, user, nodes: nodes.clone(), serverless, at: now() });
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
    /// haven't, and makes their own scope's first epoch, if it has none.
    async fn settle_own(&self) -> Result<(), Error> {
        let (user, runs, epoch, on) = {
            let known = self.known.lock().unwrap();
            let m = &known.as_ref().expect("read").members;
            let user = m.machine(&self.me.key).map(|d| d.user.clone()).unwrap_or_default();
            let on: Vec<[u8; 32]> = m.computes().filter(|c| c.user != user && m.runs_for(&c.key, &Scope::Global)).map(|c| c.key).collect();
            (user.clone(), m.runs.contains_key(&user), m.epoch(&Scope::User(user)), on)
        };
        if !runs && !on.is_empty() {
            self.write("global", &user, members::runs(&on)).await?;
        }
        if epoch == 0 {
            self.first_epoch(&format!("user:{user}"), &user).await?;
        }
        Ok(())
    }

    /// Starts again with a fresh connection, once the network's back.
    pub async fn reconnect(&self) {
        if let Some(client) = self.client.lock().await.as_ref() {
            client.reconnect().await;
        }
    }

    /// Closes its connections, until it's asked again: when its person signs out, which also forgets it was in.
    pub async fn close(&self) {
        let _ = std::fs::remove_file(self.dir.join("joined"));
        self.disconnect().await;
    }

    /// Closes its connections, until it's asked again, still knowing it was in: when a program using it ends, so a
    /// node isn't left holding a connection it counts against this device.
    pub async fn disconnect(&self) {
        if let Some(client) = self.client.lock().await.take() {
            client.close().await;
        }
    }

    /// Signs out: this device takes itself out of its person's devices first, by asking their node to (as Settings ›
    /// Devices does for another), so signing in again makes a new one that replaces it, never a second; then it closes.
    /// A node that doesn't answer in a few seconds is left: approving the new device replaces it. The app forgets this
    /// device's key either way, as a removed key can't be let in again.
    pub async fn sign_out(&self) {
        if self.dir.join("joined").exists() {
            let _ = n0_future::time::timeout(Duration::from_secs(15), self.json("POST", "/v1/remove", Some(json!({ "key": self.key() })))).await;
        }
        self.close().await;
    }

    /// Whether the install took this device out: it was in, and isn't listed now.
    pub fn removed(&self) -> bool {
        self.dir.join("joined").exists()
    }

    // Requests.

    /// A request to its install's nodes as its person, with the keys it needed last time; one a node needs others for
    /// (428) is sent again with them, a few times, as a node may find what it needs as it goes (the serverless node,
    /// its logs' keys first, then a call's). `/v1/state` is read for the order things run in.
    pub async fn fetch(&self, method: &str, path: &str, headers: Vec<(String, String)>, body: Vec<u8>) -> Result<Response, Error> {
        // Asked before it joined, or after it was taken out: it's in only if the relay lets it, else its own screen says so.
        let joined = self.client.lock().await.is_some();
        if !joined && !self.connect().await? {
            return Err(refused());
        }
        let err = match n0_future::future::or(self.request(method, path, headers, body), self.watch()).await {
            Ok(response) => return Ok(response),
            Err(err) => err,
        };
        // A request that failed may be one a device taken out can't make: the relay says who's on its network, so it's
        // asked. Out, this device starts over at joining, where nothing but its approval or sign-in is shown.
        if err.not_let_in || matches!(self.computes().await, Ok(None)) {
            if let Some(client) = self.client.lock().await.take() {
                client.close().await;
            }
            *self.known.lock().unwrap() = None;
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
            if matches!(self.computes().await, Ok(None)) {
                return Err(refused());
            }
            wait = 5;
        }
    }

    async fn request(&self, method: &str, path: &str, headers: Vec<(String, String)>, body: Vec<u8>) -> Result<Response, Error> {
        let route = route(path, &body);
        for tries in 0..4 {
            let handed = self.handed(&route).await;
            let request = Request { method: method.into(), path: path.into(), headers: headers.clone(), body: body.clone() };
            let response = self.send(request, handed.as_ref()).await?;
            if response.status == 428 && tries < 3 {
                let node = response.node;
                let answer: Value = serde_json::from_slice(&response.body.bytes().await?).unwrap_or_default();
                self.learn(&route, &node, &answer).await?;
                continue;
            }
            if response.status == 403 {
                let bytes = response.body.bytes().await?;
                if serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| v["code"] == "not_let_in") {
                    return Err(refused());
                }
                return Ok(Response { status: 403, headers: response.headers, node: response.node, body: Body::whole(bytes) });
            }
            if response.status == 200 && path.split('?').next() == Some("/v1/state") {
                let bytes = response.body.bytes().await?;
                self.learn_order(&bytes).await;
                return Ok(Response { status: 200, headers: response.headers, node: response.node, body: Body::whole(bytes) });
            }
            return Ok(response);
        }
        Err(failed("Your device needed keys it didn't name. Try again."))
    }

    /// A request saying whose it is (`x-one-user`), which a node checks against this device's key, with keys `handed`
    /// to the node that asked for them alone, sealed to the seal key its entries list: any other is asked without.
    async fn send(&self, mut request: Request, handed: Option<&([u8; 32], Vec<Handed>)>) -> Result<Response, Error> {
        let client = self.client.lock().await.clone().ok_or_else(refused)?;
        let (seals, user) = self.known.lock().unwrap().as_ref().map(|k| (k.nodes.clone(), k.user.clone())).unwrap_or_default();
        request.headers.push(("x-one-user".into(), user));
        let me = self.key();
        client
            .fetch_with(request, |node| {
                let Some((_, keys)) = handed.filter(|(to, keys)| to == node && !keys.is_empty()) else { return Vec::new() };
                let Some(seal) = seals.get(node) else { return Vec::new() };
                let handoff = Handoff { for_: me.clone(), until: now() + HANDED_FOR, keys: keys.clone() };
                keys::seal_handoff(seal, &handoff).map(|k| vec![("x-one-keys".to_string(), k)]).unwrap_or_default()
            })
            .await
    }

    /// A request's JSON, without handing keys: what this device asks to open them.
    async fn json(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, Error> {
        let mut headers = Vec::new();
        if body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        let body = body.map(|b| keys::canonical(&b)).unwrap_or_default();
        let response = self.send(Request { method: method.into(), path: path.into(), headers, body }, None).await?;
        let status = response.status;
        let out: Value = serde_json::from_slice(&response.body.bytes().await?).unwrap_or_default();
        if status != 200 {
            return Err(failed(error(&out).unwrap_or_else(|| format!("{path} answered {status}"))));
        }
        Ok(out)
    }

    /// The order things run in, from the node's `/v1/state`; a node it lists that this device didn't know of has it
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
        if let Some(client) = self.client.lock().await.as_ref() {
            let _ = client.set_nodes(&self.ordered(&nodes));
        }
    }

    // Keys.

    /// The keys a kind of request needed last time, opened, and the node that asked for them, which alone is handed
    /// them: none it can't open, nor of a scope its entries no longer say that node runs.
    async fn handed(&self, route: &str) -> Option<([u8; 32], Vec<Handed>)> {
        let (node, needs) = self.needs.lock().unwrap().get(route).cloned()?;
        let node: [u8; 32] = keys::unhex(&node).ok()?;
        let runs = |scope: &str| Scope::parse(scope).is_ok_and(|s| self.known.lock().unwrap().as_ref().is_some_and(|k| k.nodes.contains_key(&node) && k.members.runs_for(&node, &s)));
        let mut handed = Vec::new();
        for n in needs.into_iter().filter(|n| runs(&n.scope)) {
            if let Ok(key) = self.database_key(&n).await {
                handed.push(Handed { key, scope: n.scope, epoch: n.epoch, app: n.app, database: n.database });
            }
        }
        Some((node, handed))
    }

    /// A node's 428, taken only from the node that answered (`node`), naming itself and the seal key its entries list,
    /// asking only keys of scopes they say it runs, and of databases that kind of request uses (`keys::may_hand`): what
    /// the request needed, remembered for that kind of request and that node, in place of what it needed before. Fails,
    /// remembering nothing, if it asks for more, or this device can't open them.
    async fn learn(&self, route: &str, node: &[u8; 32], answer: &Value) -> Result<(), Error> {
        let listed = self.known.lock().unwrap().as_ref().and_then(|k| k.nodes.get(node).copied());
        let to = keys::unhex(answer["to"].as_str().unwrap_or("")).ok();
        let seal = keys::unhex(answer["seal"].as_str().unwrap_or("")).ok();
        if listed.is_none() || to != Some(*node) || seal != listed {
            return Err(failed("A device that isn't one of yours asked for keys. Nothing was handed."));
        }
        let mut need: Vec<Need> = serde_json::from_value(answer["need"].clone()).unwrap_or_default();
        need.sort();
        need.dedup();
        if need.is_empty() || need.len() > MOST_NEEDED || !need.iter().all(|n| keys::may_hand(route, n)) {
            return Err(failed("A device asked for keys that request doesn't use. Nothing was handed."));
        }
        let scopes: Vec<Scope> = need.iter().map(|n| Scope::parse(&n.scope)).collect::<Result<_, _>>().map_err(|_| failed("A device asked for keys of something that isn't. Nothing was handed."))?;
        let runs = |device: &Device| device.known.lock().unwrap().as_ref().is_some_and(|k| scopes.iter().all(|s| k.members.runs_for(node, s)));
        // What it runs may be newer than this device read: read again, now and then, before refusing.
        if !runs(self) && self.known.lock().unwrap().as_ref().is_some_and(|k| now() > k.at + REREAD) {
            self.read_members().await?;
        }
        if !runs(self) {
            return Err(failed("A device that doesn't run that asked for its keys. Nothing was handed."));
        }
        // Of each database, only epochs from the one its newest copy is under (`keys::oldest_needed`): older ones open
        // only its history.
        let bucket = Signed::new(self.address.clone(), self.key.to_bytes());
        let prefix = |n: &Need| format!("{}/{}/{}", one::bucket::folder(&n.scope), n.app, n.database);
        let mut oldest: BTreeMap<String, u32> = BTreeMap::new();
        for (n, scope) in need.iter().zip(&scopes) {
            if oldest.contains_key(&prefix(n)) {
                continue;
            }
            let hint = one::bucket::Bucket::get(&bucket, &one::db::newest_name(&prefix(n))).await.map_err(failed)?;
            let newest = self.known.lock().unwrap().as_ref().map_or(0, |k| k.members.epoch(scope));
            oldest.insert(prefix(n), keys::oldest_needed(hint.as_ref().map(|o| &o.bytes[..]), newest));
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

    async fn database_key(&self, n: &Need) -> Result<[u8; 32], Error> {
        Ok(one::kernel::database_key(&self.app_key(&n.scope, &n.app, n.epoch).await?, &n.database))
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
                let sealed = B64U.encode(&one::kernel::seal_app_key(&epoch, app, &made));
                let out = self.json("POST", "/v1/keys", Some(json!({ "scope": scope, "n": n, "app": app, "sealed": sealed }))).await?;
                out["sealed"].as_str().unwrap_or_default().to_string()
            }
        };
        let bytes = B64U.decode(sealed.trim().as_bytes()).ok().ok_or_else(|| failed(format!("{app}'s key isn't one")))?;
        let key = one::kernel::open_app_key(&epoch, app, &bytes).map_err(|_| failed(format!("{app}'s key didn't open")))?;
        self.app_keys.lock().unwrap().insert(id, key);
        Ok(key)
    }

    /// Epoch `n` of a scope: opened from what's sealed to this device, if its hash is the one the entries this device
    /// read itself gave.
    async fn epoch(&self, scope: &str, n: u32) -> Result<[u8; 32], Error> {
        if let Some(key) = self.epochs.lock().unwrap().get(&(scope.to_string(), n)) {
            return Ok(*key);
        }
        let parsed = Scope::parse(scope).map_err(failed)?;
        let kept = self.json("GET", &format!("/v1/keys?scope={}&to={}", query(scope), self.key()), None).await?;
        for fresh in [false, true] {
            if fresh {
                self.read_members().await?;
            }
            let hashes = self.known.lock().unwrap().as_ref().map(|k| k.members.epoch_hashes.clone()).unwrap_or_default();
            for (m, sealed) in kept["sealed"].as_object().into_iter().flatten() {
                let Ok(m) = m.parse::<u32>() else { continue };
                let opened = B64U.decode(sealed.as_str().unwrap_or("").as_bytes()).ok().and_then(|s| one::kernel::open_sealed(&self.opens(), &s).ok()).and_then(|k| <[u8; 32]>::try_from(k).ok());
                if let Some(key) = opened
                    && hashes.get(&(parsed.clone(), m)) == Some(&keys::epoch_hash(&key))
                {
                    self.epochs.lock().unwrap().insert((scope.to_string(), m), key);
                }
            }
            if let Some(key) = self.epochs.lock().unwrap().get(&(scope.to_string(), n)) {
                return Ok(*key);
            }
        }
        Err(failed("This device can't open that yet."))
    }

    /// Starts this person's own scope's first epoch, as a newcomer's first device does: a random key, sealed to the
    /// recovery key (which takes it), to this device and their other devices, then an `epoch` entry naming its hash.
    async fn first_epoch(&self, scope: &str, user: &str) -> Result<(), Error> {
        let (recovery, to) = {
            let known = self.known.lock().unwrap();
            let m = &known.as_ref().expect("read").members;
            let recovery = m.recovery.ok_or_else(|| failed("Your One App has no recovery key to keep its keys with."))?;
            let mut to = vec![self.me];
            to.extend(m.clients().filter(|c| c.user == user && c.key != self.me.key && c.seal != c.key).map(|c| Endpoint { key: c.key, seal: c.seal }));
            (recovery, to)
        };
        let key: [u8; 32] = rand::random();
        let recovery = one::kernel::twin_public(&recovery).map_err(failed)?;
        let sealed = |to: &[u8; 32]| one::kernel::seal_to(to, &key).map(|s| B64U.encode(&s)).map_err(failed);
        self.json("POST", "/v1/keys", Some(json!({ "scope": scope, "n": 1, "to": "recovery", "sealed": sealed(&recovery)? }))).await?;
        for e in &to {
            self.json("POST", "/v1/keys", Some(json!({ "scope": scope, "n": 1, "to": hex(&e.key), "sealed": sealed(&e.seal)? }))).await?;
        }
        let parsed = Scope::parse(scope).map_err(failed)?;
        self.write("global", user, members::epoch(&parsed, 1, &to.iter().map(|e| e.key).collect::<Vec<_>>(), &keys::epoch_hash(&key))).await?;
        self.epochs.lock().unwrap().insert((scope.to_string(), 1), key);
        Ok(())
    }

    /// Signs an entry of `content` in `log`, after what the node has, and writes it.
    async fn write(&self, log: &str, user: &str, content: one::entry::Content) -> Result<(), Error> {
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

/// What a request is, to remember what it needed: an app's call by its app, a screen by its app, anything else by
/// its path.
fn route(path: &str, body: &[u8]) -> String {
    let at = path.split('?').next().unwrap_or(path);
    if at == "/v1/call" {
        let app = serde_json::from_slice::<Value>(body).ok().and_then(|b| b["app"].as_str().map(str::to_string)).unwrap_or_default();
        return format!("call {app}");
    }
    match at.strip_prefix("/v1/screens/").and_then(|s| s.split('/').next()) {
        Some(app) => format!("/v1/screens/{app}"),
        None => at.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A device hands keys only to a node its own reading of `global` lists, which runs every scope it asks keys of,
    /// sealed to the seal key listed for it, only what its 428 named of the databases that kind of request uses, and only
    /// to the node that asked.
    #[tokio::test]
    async fn keys_go_only_to_listed_nodes_as_their_428_names() {
        let device = Device::new(new_secret(), "http://127.0.0.1:1", &std::env::temp_dir().join(format!("one-client-{}", hex(&new_secret()))).display().to_string());
        let (node, seal, other) = ([1; 32], [2; 32], [3; 32]);
        *device.known.lock().unwrap() = Some(Known { members: Members::default(), user: String::new(), nodes: HashMap::from([(node, seal)]), serverless: None, at: now() });
        let need = |n: usize| (0..n).map(|i| json!({ "scope": "global", "epoch": 1, "app": "board", "database": format!("d{i}") })).collect::<Vec<_>>();
        let answer = |to: &[u8; 32], seal: &[u8; 32], n: usize| json!({ "to": hex(to), "seal": hex(seal), "need": need(n) });
        let refused = async |from: &[u8; 32], a: Value| device.learn("call board", from, &a).await.is_err();
        assert!(refused(&other, answer(&other, &seal, 1)).await, "a node its entries don't list");
        assert!(refused(&node, answer(&other, &seal, 1)).await, "a 428 naming another node than the one that answered");
        assert!(refused(&node, answer(&node, &other, 1)).await, "a seal key other than the one listed for it");
        assert!(refused(&node, answer(&node, &seal, MOST_NEEDED + 1)).await, "more keys than a request needs");
        assert!(refused(&node, answer(&node, &seal, 1)).await, "a node the entries don't say runs the scope");
        let other_app = json!({ "to": hex(&node), "seal": hex(&seal), "need": [{ "scope": "global", "epoch": 1, "app": "notes", "database": "main" }] });
        assert!(device.learn("call board", &node, &other_app).await.unwrap_err().to_string().contains("doesn't use"), "another app's database");
        assert!(device.needs.lock().unwrap().is_empty(), "nothing refused is remembered, so nothing is handed");
        // What a node asked for goes to it alone.
        device.needs.lock().unwrap().insert("call board".into(), (hex(&other), vec![]));
        assert_eq!(device.handed("call board").await.map(|h| h.0), Some(other), "remembered with the node that asked");
    }
}
