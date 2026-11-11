//! What a client device knows of its network, the keys it opened, and how it asks a node with them, the same on every
//! client that runs this crate: the CLI and native apps (`device.rs`), and the remote MCP server's devices
//! (oned/cloudflare/node/src/clients.rs), and browsers (`wasm::Keyring`, in onet/web/net-worker.js).
//!
//! It trusts no node, nor the relay, for who's who: it reads `global`'s scope database's steps from the bucket itself,
//! opens them with the epochs sealed to it, and applies them by the node's own rules (`steps::Reader`). It asks only
//! endpoints those list as nodes that run its user's things, takes a 428 only from the node that answered, and only
//! for scopes they say it runs, databases that kind of request uses (`keys::asked_for`) and epochs from each one's
//! newest copy (`keys::oldest_needed`); and hands what it named to the one node each request is sent to, of the scopes
//! they say that node runs, sealed to the seal key they list for it. An epoch it's given opens only if its hash is the
//! one who's who names, whoever passed it on.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use data_encoding::BASE64URL_NOPAD as B64U;
use n0_future::time::Duration;
use ::custom_build::config::YOURS;
use ::custom_build::protocol::header;
use ::scope::scopes::{Scope, ScopeDb};
use ::scope::steps::{Reader, Welcome};
use ::{api::handed::{Handed, Handoff, Need}, key::{Hash, Opens, Signs, hex}};
use serde_json::{Value, json};

use crate::{Body, Client, Error, Request, Response, not_let_in, unsent};

const REREAD: u64 = 30_000;
/// How long a node keeps the keys a request handed it.
pub const HANDED_FOR: u64 = 3_600_000;
/// The most kinds of request whose needs are kept: the remote MCP server keeps one for each tool an MCP client calls.
const KINDS_KEPT: usize = 128;
const ROOT: &str = "root";
const WELCOME: &str = "welcome";
const NEEDS: &str = "needs.json";
const WHO: &str = "who.json";
/// The relay's count of writes to `global` as it read who's who (`read_if_moved`).
const WRITES: &str = "writes";

/// Where a device keeps what outlives one run of it, by name: the network's root, pinned the first time it reads it
/// (`root`); where the device that let it in had read to (`welcome`), until it's read from there; who's who as it last
/// read it (`who.json`), to start from; and what each kind of request needed (`needs.json`).
pub trait Keep {
    fn get(&self, name: &str) -> impl Future<Output = Option<Vec<u8>>>;
    fn put(&self, name: &str, bytes: &[u8]) -> impl Future<Output = anyhow::Result<()>>;
    fn delete(&self, name: &str) -> impl Future<Output = ()>;
}

/// What a device reaches at its network's address: the bucket, through the URL signer; the relay, which alone says
/// whether it's still let in; and sign-in, the relay and the notifications Worker as it asks them itself (`ask`).
pub trait Address {
    type Bucket: bucket::Bucket;
    fn bucket(&self) -> &Self::Bucket;
    /// False only if the relay says it no longer lets this device in.
    fn let_in(&self) -> impl Future<Output = bool>;
    /// The network's address: `https://…`.
    fn address(&self) -> &str;
    /// A request to `url`, JSON in and out, with `header` if there's one: → its status and JSON (null if it wasn't).
    fn ask(&self, method: &str, url: &str, header: Option<(&str, String)>, body: Option<Value>) -> impl Future<Output = Result<(u16, Value), Error>>;
}

pub fn now() -> u64 {
    n0_future::time::SystemTime::now().duration_since(n0_future::time::SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn failed(message: impl std::fmt::Display) -> Error {
    unsent(message)
}

pub fn refused() -> Error {
    not_let_in("This device hasn't been let in yet.")
}

/// Who's who as this device read and checked it itself.
struct Known {
    reader: Reader,
    /// The keys `global`'s steps are sealed under, by epoch.
    steps: database::stored::Keys,
    user: String,
    /// The nodes it may ask, by key: their seal keys; and the Cloudflare node, if the network has one.
    nodes: HashMap<[u8; 32], [u8; 32]>,
    cloudflare_node: Option<[u8; 32]>,
    /// Each node's run priority, as the network's admins set it (scope/src/rank.rs): the Cloudflare node's too.
    ranks: HashMap<[u8; 32], u32>,
    at: u64,
}

impl Known {
    fn who(&self) -> &ScopeDb {
        &self.reader.truth().expect("known once who's who is read").db
    }

    /// What `reader` says, for the device whose key is `key`: fails `not_let_in` until it lists it as a client of its
    /// user's.
    fn of(reader: Reader, steps: database::stored::Keys, key: &[u8; 32], at: u64) -> Result<Known, Error> {
        let who = &reader.truth().ok_or_else(|| not_let_in("This device isn't listed yet."))?.db;
        let user = who.client(key).map_err(failed)?.map(|d| d.user).ok_or_else(|| not_let_in("This device isn't listed yet."))?;
        // The nodes that run something of its user's, and the Cloudflare node, which runs anyone's calls.
        let cloudflare_node = who.cloudflare_node().map_err(failed)?;
        let mut nodes: HashMap<[u8; 32], [u8; 32]> = HashMap::new();
        for (key, seal) in who.nodes().map_err(failed)? {
            if Some(key) == cloudflare_node || who.serves(&key, &user).map_err(failed)? {
                nodes.insert(key, seal);
            }
        }
        let set = ::scope::rank::set(&who.settings().map_err(failed)?);
        let ranks = nodes
            .keys()
            .map(|k| {
                let id = if Some(*k) == cloudflare_node { ::scope::rank::CLOUDFLARE.to_string() } else { hex(&who.device_of(k)) };
                (*k, match ::scope::rank::priority(&set, &id) { ::scope::rank::Priority::Number(n) => n, ::scope::rank::Priority::Last => u32::MAX })
            })
            .collect();
        Ok(Known { reader, steps, user, nodes, cloudflare_node, ranks, at })
    }
}

pub struct Keyring<O = [u8; 32]> {
    key: [u8; 32],
    /// What opens what's sealed to it: its seal key's secret, or in a browser, the WebCrypto key that can't be read.
    opens: O,
    known: Mutex<Option<Known>>,
    /// Held while it reads who's who, so it reads once at a time.
    reading: tokio::sync::Mutex<()>,
    epochs: Mutex<HashMap<(String, u32), [u8; 32]>>,
    app_keys: Mutex<HashMap<(String, String, u32), [u8; 32]>>,
    /// What each kind of request needed (`Kind::name`): the last 428's, and the node that sent it (hex); none until
    /// read from where it's kept.
    needs: Mutex<Option<BTreeMap<String, (String, Vec<Need>)>>>,
    /// Held while a kind of request learns from a 428: another of its kind waits for it, and goes with what it learned.
    learning: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// When it last read who's who to renew app schedules (`settings::renew_app_schedules`).
    renew_checked: Mutex<u64>,
    /// The nodes a node said that it read who's who again for (`reread_for`): each only once.
    reread: Mutex<std::collections::HashSet<[u8; 32]>>,
}

impl<O: Opens> Keyring<O> {
    /// The keyring of the device whose key is `key`, opening what's sealed to it with `opens`.
    pub fn new(key: [u8; 32], opens: O) -> Keyring<O> {
        Keyring {
            key,
            opens,
            known: Mutex::default(),
            reading: tokio::sync::Mutex::default(),
            epochs: Mutex::default(),
            app_keys: Mutex::default(),
            needs: Mutex::default(),
            learning: Mutex::default(),
            renew_checked: Mutex::default(),
            reread: Mutex::default(),
        }
    }

    /// What opens what's sealed to it.
    pub fn opens(&self) -> &O {
        &self.opens
    }

    /// Whether it's been `every` (ms) since this was last true.
    pub(crate) fn renew_checked(&self, every: u64) -> bool {
        let mut at = self.renew_checked.lock().unwrap();
        let due = now() > *at + every;
        if due {
            *at = now();
        }
        due
    }

    /// Reads who's who from the bucket: the first time from the network's first freeze, or where the device that let it
    /// in had read to, after that what's new. The network's root is kept the first time, and must be the same after.
    /// Fails `not_let_in` until it lists this device as a client of its user's, with its keys sealed to it.
    pub async fn read(&self, on: &(impl Address + Keep)) -> Result<(), Error> {
        let _reading = self.reading.lock().await;
        let (bucket, now, me) = (on.bucket(), now(), hex(&self.key));
        let pinned = on.get(ROOT).await.and_then(|r| Hash::from_hex(String::from_utf8_lossy(&r).trim()).ok());
        let mut reader = Reader::new(pinned);
        // From a copy of what it knows, which is asked meanwhile, or what it knew when it last ran, whose keys it opens
        // again.
        let had = self.known.lock().unwrap().as_mut().and_then(|k| Some((k.reader.kept().ok()?, k.steps.clone())));
        let had = match had {
            Some(had) => Some(had),
            None => on.get(WHO).await.and_then(|w| serde_json::from_slice(&w).ok()).map(|kept| (kept, Default::default())),
        };
        let (epochs, steps) = match had {
            Some((kept, steps)) if reader.restart(&kept, now).is_ok() => {
                if !reader.whole()
                    && let Ok(Some((fresh, _))) = ::scope::steps::read_again(bucket, reader.root(), reader.at(), &me, &self.opens, now).await
                {
                    reader = fresh;
                }
                if !steps.is_empty() {
                    reader.read(bucket, &steps, now).await.map_err(failed)?;
                }
                let newest = reader.truth().map(|t| t.db.epoch(&Scope::Global)).transpose().map_err(failed)?.unwrap_or(0);
                match steps.keys().next_back().is_some_and(|n| *n >= newest) {
                    true => (BTreeMap::new(), steps),
                    false => ::scope::steps::read_sealed_to(bucket, &mut reader, &me, &self.opens, &mut None, now).await.map_err(failed)?,
                }
            }
            _ => {
                let mut welcome = on.get(WELCOME).await.and_then(|w| Welcome::parse(String::from_utf8_lossy(&w).trim()));
                let had_welcome = welcome.is_some();
                let read = ::scope::steps::read_sealed_to(bucket, &mut reader, &me, &self.opens, &mut welcome, now).await.map_err(failed)?;
                if had_welcome && welcome.is_none() {
                    on.delete(WELCOME).await;
                }
                if read.0.is_empty() {
                    return Err(not_let_in(format!("This device hasn't been given {YOURS}'s keys yet.")));
                }
                read
            }
        };
        if pinned.is_none()
            && let Some(root) = reader.root()
        {
            on.put(ROOT, root.to_hex().as_bytes()).await.map_err(failed)?;
        }
        let mut known = Known::of(reader, steps, &self.key, now)?;
        // Kept only to start from next time.
        if let Ok(kept) = known.reader.kept().map(|k| k.to_string()) {
            let _ = on.put(WHO, kept.as_bytes()).await;
        }
        self.epochs.lock().unwrap().extend(epochs.into_iter().map(|(n, key)| (("global".to_string(), n), key)));
        *self.known.lock().unwrap() = Some(known);
        Ok(())
    }

    /// Who's who as it last read it when it ran before, if it kept it: → whether it knows who's who now.
    pub async fn restore(&self, on: &impl Keep) -> bool {
        if self.known.lock().unwrap().is_some() {
            return true;
        }
        let Some(kept) = on.get(WHO).await.and_then(|w| serde_json::from_slice::<Value>(&w).ok()) else { return false };
        let pinned = on.get(ROOT).await.and_then(|r| Hash::from_hex(String::from_utf8_lossy(&r).trim()).ok());
        let mut reader = Reader::new(pinned);
        let Ok(known) = reader.restart(&kept, now()).map_err(failed).and_then(|_| Known::of(reader, Default::default(), &self.key, 0)) else { return false };
        self.known.lock().unwrap().get_or_insert(known);
        true
    }

    /// Who's who as it last read it when it ran before, at once, if it kept it; else read from the bucket.
    pub async fn load(&self, on: &(impl Address + Keep)) -> Result<(), Error> {
        if self.restore(on).await {
            return Ok(());
        }
        self.read(on).await
    }


    /// Who's who as it last read it, if the relay's count of writes to `global` (`writes`, as its computes say) is still
    /// what it was then: else read from the bucket, keeping that count. As nodes go by it (`app_runtime`): only a hint, as
    /// the bucket is the truth, so with none it reads.
    pub async fn read_if_moved(&self, on: &(impl Address + Keep), writes: Option<u64>) -> Result<(), Error> {
        let kept = on.get(WRITES).await.and_then(|b| String::from_utf8(b).ok()?.trim().parse::<u64>().ok());
        if writes.is_some() && kept == writes && self.restore(on).await {
            return Ok(());
        }
        self.read(on).await?;
        if let Some(writes) = writes {
            let _ = on.put(WRITES, writes.to_string().as_bytes()).await;
        }
        Ok(())
    }

    /// Reads who's who again if a node said of `nodes` one it doesn't know, the first time one's said: one who's who
    /// still doesn't list after that isn't one to ask.
    pub async fn reread_for(&self, on: &(impl Address + Keep), nodes: &[[u8; 32]]) {
        let Some(known) = self.known.lock().unwrap().as_ref().map(|k| k.nodes.keys().copied().collect::<std::collections::HashSet<_>>()) else { return };
        let new = {
            let mut reread = self.reread.lock().unwrap();
            nodes.iter().filter(|n| !known.contains(*n)).fold(false, |new, n| reread.insert(*n) || new)
        };
        if new {
            let _ = self.read(on).await;
        }
    }

    /// Who's who, as it last read it, for `f`: none before it has.
    pub fn who<R>(&self, f: impl FnOnce(&ScopeDb) -> R) -> Option<R> {
        self.known.lock().unwrap().as_ref().map(|k| f(k.who()))
    }

    /// Where it read who's who to, as it welcomes a device it approves.
    pub fn welcome(&self) -> Option<Welcome> {
        self.known.lock().unwrap().as_ref()?.reader.truth()?.welcome().ok()
    }

    /// Its user, as who's who lists this device.
    pub fn user(&self) -> Option<String> {
        self.known.lock().unwrap().as_ref().map(|k| k.user.clone())
    }

    /// The nodes it may ask, by key, with their seal keys.
    pub fn nodes(&self) -> HashMap<[u8; 32], [u8; 32]> {
        self.known.lock().unwrap().as_ref().map(|k| k.nodes.clone()).unwrap_or_default()
    }

    /// This device's own app runtime among the nodes it may ask: the compute that names it as its device.
    pub fn own_runtime(&self) -> Option<[u8; 32]> {
        let known = self.known.lock().unwrap();
        let k = known.as_ref()?;
        k.nodes.keys().copied().find(|n| *n != self.key && k.who().device_of(n) == self.key)
    }

    /// Where who's who is read to, `global`'s newest epoch, and the keys its steps are sealed under: what the next step
    /// is written with.
    pub(crate) fn next_step(&self) -> Option<(u64, u32, database::stored::Keys)> {
        let known = self.known.lock().unwrap();
        let k = known.as_ref()?;
        Some((k.reader.at(), k.who().epoch(&Scope::Global).ok()?, k.steps.clone()))
    }

    pub fn cloudflare_node(&self) -> Option<[u8; 32]> {
        self.known.lock().unwrap().as_ref().and_then(|k| k.cloudflare_node)
    }

    /// The nodes it may ask, in the order things run on them: `first`, as last said, then those it wasn't told of, the
    /// Cloudflare node last of them; then by run priority, which may put the Cloudflare node first.
    pub fn ordered(&self, first: &[[u8; 32]]) -> Vec<[u8; 32]> {
        let nodes = self.nodes();
        let last = self.cloudflare_node();
        let ranks: HashMap<[u8; 32], u32> = self.ranks().into_iter().collect();
        let mut out: Vec<[u8; 32]> = first.iter().filter(|k| nodes.contains_key(*k) && Some(**k) != last).copied().collect();
        let mut rest: Vec<[u8; 32]> = nodes.keys().filter(|k| !out.contains(k) && Some(**k) != last).copied().collect();
        rest.sort();
        out.extend(rest);
        out.extend(last.filter(|k| nodes.contains_key(k)));
        out.sort_by_key(|k| ranks.get(k).copied().unwrap_or(1));
        out
    }

    /// Every node it knows failed: who's who read again, now and then, in case a device has a runtime of its own by now.
    /// → whether that listed a node it hadn't known of, which it now asks too.
    async fn found_nodes(&self, client: &Client, on: &(impl Address + Keep)) -> bool {
        let stale = self.known.lock().unwrap().as_ref().is_some_and(|k| now() > k.at + REREAD);
        let had = self.nodes();
        if !stale || self.read(on).await.is_err() || self.nodes().keys().all(|k| had.contains_key(k)) {
            return false;
        }
        let _ = client.set_ranks(&self.ranks());
        let _ = client.set_wakes(self.cloudflare_node());
        client.set_nodes(&self.ordered(&[])).is_ok()
    }

    pub fn ranks(&self) -> Vec<([u8; 32], u32)> {
        self.known.lock().unwrap().as_ref().map(|k| k.ranks.iter().map(|(k, r)| (*k, *r)).collect()).unwrap_or_default()
    }

    /// An epoch this device just made, whose hash who's who names.
    pub fn made_epoch(&self, scope: &Scope, n: u32, key: [u8; 32]) {
        self.epochs.lock().unwrap().insert((scope.to_string(), n), key);
    }

    /// Forgets all it knew and kept but the network's root, as a device that's out does: the next approval names the
    /// network again.
    pub async fn forget(&self, on: &impl Keep) {
        *self.known.lock().unwrap() = None;
        *self.needs.lock().unwrap() = Some(BTreeMap::new());
        self.epochs.lock().unwrap().clear();
        self.app_keys.lock().unwrap().clear();
        for kept in [NEEDS, ROOT, WELCOME, WHO, WRITES] {
            on.delete(kept).await;
        }
    }

    /// A request to its network's nodes as its user, with the keys its kind needed last time, sealed to the node it's
    /// sent to until `until` (ms). One a node needs others for (428) is sent again with them, a few times, as a node may
    /// find what it needs as it goes (the Cloudflare node, its logs' keys first, then a call's); one a node refuses as
    /// it doesn't list this device yet, again in a moment while the relay lets it in.
    /// One this device answers itself (`answer`: what it signs as its user) never reaches a node, and the API's document
    /// (`one::openapi`) is made here from the apps a node lists for it.
    pub async fn request(&self, client: &Client, on: &(impl Address + Keep + Signs), request: Request, until: u64) -> Result<Response, Error> {
        if let Some(answer) = self.answer(client, on, &request.method, &request.path, request.body.bytes()).await {
            let (status, out) = match answer {
                Ok(out) => (200, out),
                Err(err) if err.not_let_in => return Err(err),
                Err(err) => (400, json!({ "error": err.to_string(), "code": err.code })),
            };
            let headers = vec![("content-type".to_string(), "application/json".to_string())];
            return Ok(Response { node: [0; 32], status, headers, body: Body::whole(out.to_string().into_bytes()), trace: None });
        }
        if request.method == "GET" && request.path.split('?').next() == Some(::api::path::OPENAPI) {
            return self.document(client, on, request, until).await;
        }
        self.to_nodes(client, on, request, until).await
    }

    /// The API's document for this device's user, from the apps a node lists for them.
    async fn document(&self, client: &Client, on: &(impl Address + Keep), mut request: Request, until: u64) -> Result<Response, Error> {
        request.path = ::api::path::APPS.into();
        let response = self.to_nodes(client, on, request, until).await?;
        if response.status != 200 {
            return Ok(response);
        }
        let apps: Vec<Value> = serde_json::from_slice(&response.body.bytes().await?).map_err(|err| failed(format!("{} answered with a reply that wasn't JSON: {err}", ::api::path::APPS)))?;
        let body = one::openapi::document(&apps, false).to_string().into_bytes();
        let headers = vec![("content-type".to_string(), "application/json".to_string())];
        Ok(Response { headers, body: Body::whole(body), ..response })
    }

    /// `request`, of its nodes.
    async fn to_nodes(&self, client: &Client, on: &(impl Address + Keep), request: Request, until: u64) -> Result<Response, Error> {
        let kind = Kind::of(&request.path, request.body.bytes());
        if self.needs.lock().unwrap().is_none() {
            let kept = on.get(NEEDS).await.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            self.needs.lock().unwrap().get_or_insert(kept);
        }
        let (mut needed, mut behind, mut reread) = (0, 0, false);
        loop {
            let learning = self.learning.lock().unwrap().get(&kind.name).cloned();
            if let Some(learning) = learning {
                drop(learning.lock().await);
            }
            let sent_with = self.needs_of(&kind.name);
            let handed = self.handed(client, on, sent_with.as_ref()).await?;
            let again = Request { method: request.method.clone(), path: request.path.clone(), headers: request.headers.clone(), body: request.body.clone() };
            let sent = self.send(client, again, &handed, until).await;
            if sent.as_ref().is_err_and(|err| !err.sent) && !reread {
                reread = true;
                if self.found_nodes(client, on).await {
                    continue;
                }
            }
            let response = sent?;
            if response.status == 428 && needed < 3 {
                needed += 1;
                let node = response.node;
                let answer: Value = serde_json::from_slice(&response.body.bytes().await?).map_err(|err| failed(format!("the node asked for keys in a reply that wasn't JSON: {err}")))?;
                self.learn(client, on, &kind, &node, &answer, sent_with).await?;
                continue;
            }
            if response.status == 403 {
                let bytes = response.body.bytes().await?;
                // A node that doesn't list this device may not have read its approval yet: the relay says if it's out,
                // else it's asked again in a moment, once it has.
                if serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| v["code"] == ::custom_build::protocol::code::NOT_LET_IN) {
                    if !on.let_in().await {
                        return Err(refused());
                    }
                    if behind < ::api::handed::BEHIND.len() {
                        n0_future::time::sleep(Duration::from_millis(::api::handed::BEHIND[behind])).await;
                        behind += 1;
                        continue;
                    }
                    return Err(failed("Your devices haven't caught up with this one yet. Try again in a moment."));
                }
                return Ok(Response { body: Body::whole(bytes), ..response });
            }
            return Ok(response);
        }
    }

    fn needs_of(&self, kind: &str) -> Option<(String, Vec<Need>)> {
        self.needs.lock().unwrap().as_ref().and_then(|n| n.get(kind).cloned())
    }

    /// A request saying whose it is (`x-one-user`), which a node checks against this device's key, with the keys
    /// `handed` of the scopes who's who says run on the node the client sends it to, for that node alone, sealed to the
    /// seal key it lists for it: never to a node it only raced a connection to, nor one it doesn't list.
    async fn send(&self, client: &Client, mut request: Request, handed: &[Handed], until: u64) -> Result<Response, Error> {
        let user = self.user().ok_or_else(refused)?;
        request.headers.push((header::USER.into(), user));
        client.fetch_with(request, |node| std::future::ready(self.sealed_for(node, handed, until))).await
    }

    /// `handed`, of the scopes who's who says `node` runs, sealed to it (`x-one-keys`): none if it has none of them, or
    /// isn't listed.
    fn sealed_for(&self, node: &[u8; 32], handed: &[Handed], until: u64) -> Vec<(String, String)> {
        let known = self.known.lock().unwrap();
        let Some((k, seal)) = known.as_ref().and_then(|k| Some((k, k.nodes.get(node)?))) else { return Vec::new() };
        let keys: Vec<Handed> = handed.iter().filter(|h| Scope::parse(&h.scope).is_ok_and(|s| k.who().runs_for(node, &s))).cloned().collect();
        if keys.is_empty() {
            return Vec::new();
        }
        let handoff = Handoff { for_: hex(&self.key), until, keys };
        ::api::handed::seal_handoff(seal, &handoff).map(|k| vec![(header::KEYS.to_string(), k)]).unwrap_or_default()
    }

    /// A request's JSON, handing no keys: what this device asks to open them, and to write.
    pub async fn json(&self, client: &Client, method: &str, path: &str, body: Option<Value>) -> Result<Value, Error> {
        let mut headers = Vec::new();
        if body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        let body = body.map(|b| ::key::canonical(&b)).unwrap_or_default();
        let response = self.send(client, Request { method: method.into(), path: path.into(), headers, body: body.into() }, &[], 0).await?;
        json_of(response, path).await
    }

    /// A request's JSON, handing the keys its kind needs (`request`): what a device writes as its user.
    pub(crate) async fn asked(&self, client: &Client, on: &(impl Address + Keep), method: &str, path: &str, body: Option<Value>) -> Result<Value, Error> {
        let mut headers = Vec::new();
        if body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        let body = body.map(|b| ::key::canonical(&b)).unwrap_or_default();
        let response = self.to_nodes(client, on, Request { method: method.into(), path: path.into(), headers, body: body.into() }, now() + HANDED_FOR).await?;
        json_of(response, path).await
    }


    /// The keys a kind of request needed last time (`needs`), opened: none it can't open. Each node it's sent to is
    /// handed only those of the scopes it runs (`sealed_for`).
    async fn handed(&self, client: &Client, on: &(impl Address + Keep), needs: Option<&(String, Vec<Need>)>) -> Result<Vec<Handed>, Error> {
        let Some((_, needs)) = needs else { return Ok(Vec::new()) };
        self.fetch_keys(client, needs).await?;
        let mut handed = Vec::new();
        for n in needs {
            if let Ok(key) = self.database_key(client, on, n).await {
                handed.push(Handed { key, scope: n.scope.clone(), epoch: n.epoch, app: n.app.clone(), database: n.database.clone() });
            }
        }
        Ok(handed)
    }

    /// A node's 428, taken only from the node that answered (`node`), naming itself, asking only keys of scopes who's
    /// who says it runs, and of databases that kind of request uses (`keys::asked_for`): what the request needed,
    /// remembered for that kind of request and that node, in place of what it needed before. Fails, remembering
    /// nothing, if it asks for more, or this device can't open them. One of a kind of request that another learned for
    /// since it was sent (`sent_with`, what it went with) learns nothing more.
    async fn learn(&self, client: &Client, on: &(impl Address + Keep), kind: &Kind, node: &[u8; 32], answer: &Value, sent_with: Option<(String, Vec<Need>)>) -> Result<(), Error> {
        let lock = self.learning.lock().unwrap().entry(kind.name.clone()).or_default().clone();
        let _learning = lock.lock().await;
        if self.needs_of(&kind.name) != sent_with {
            return Ok(());
        }
        let mut need = ::api::handed::asked_for(&kind.route, &hex(node), answer).ok_or_else(|| failed("A device asked for keys that request doesn't use. Nothing was handed."))?;
        if !self.nodes().contains_key(node) {
            return Err(failed("A device that isn't one of yours asked for keys. Nothing was handed."));
        }
        let scopes: Vec<Scope> = need.iter().map(|n| Scope::parse(&n.scope)).collect::<Result<_, _>>().map_err(|_| failed("A device asked for keys of something that isn't. Nothing was handed."))?;
        let runs = || self.who(|who| scopes.iter().all(|s| who.runs_for(node, s))).unwrap_or(false);
        // What it runs may be newer than this device read: read again, now and then, before refusing.
        if !runs() && self.known.lock().unwrap().as_ref().is_some_and(|k| now() > k.at + REREAD) {
            self.read(on).await?;
        }
        if !runs() {
            return Err(failed("A device that doesn't run that asked for its keys. Nothing was handed."));
        }
        // Of each database, only epochs from the one its newest copy is under (`keys::oldest_needed`): older ones open
        // only its history.
        let bucket = on.bucket();
        let prefix = |n: &Need| bucket::names::database_prefix(&n.scope, &n.app, &n.database);
        let mut each: BTreeMap<String, &Scope> = BTreeMap::new();
        for (n, scope) in need.iter().zip(&scopes) {
            each.entry(prefix(n)).or_insert(scope);
        }
        let asked = n0_future::join_all(each.keys().map(|p| {
            let name = bucket::names::newest_name(p);
            async move { bucket::Bucket::get(bucket, &name).await }
        }));
        // The keys' own fetching goes at the same time: it only opens what's there, whichever epochs are handed.
        let (hints, fetched) = n0_future::future::zip(asked, self.fetch_keys(client, &need)).await;
        fetched?;
        let mut oldest: BTreeMap<String, u32> = BTreeMap::new();
        for ((p, scope), hint) in each.iter().zip(hints) {
            let hint = hint.map_err(failed)?;
            let newest = self.who(|who| who.epoch(scope)).transpose().map_err(failed)?.unwrap_or(0);
            let database = p.rsplit('/').next().unwrap_or_default();
            oldest.insert(p.clone(), ::api::handed::oldest_needed(database, hint.as_ref().map(|o| &o.bytes[..]), newest));
        }
        need.retain(|n| n.epoch >= oldest[&prefix(n)]);
        if need.is_empty() {
            return Err(failed("A device asked for older keys than it needs. Nothing was handed."));
        }
        for n in &need {
            self.database_key(client, on, n).await?;
        }
        let kept = {
            let mut needs = self.needs.lock().unwrap();
            let needs = needs.get_or_insert_default();
            needs.insert(kind.name.clone(), (hex(node), need));
            if needs.len() > KINDS_KEPT
                && let Some(other) = needs.keys().find(|k| **k != kind.name).cloned()
            {
                needs.remove(&other);
            }
            serde_json::to_vec(&*needs).unwrap_or_default()
        };
        // Kept only to save the next run of this device a 428.
        let _ = on.put(NEEDS, &kept).await;
        Ok(())
    }

    /// What `need`'s keys are opened from that this device hasn't opened yet, asked for all at once: each scope's epochs
    /// sealed to it, and each app's keys in them, one request each. What doesn't open here is left for `database_key`,
    /// which asks again, one at a time.
    async fn fetch_keys(&self, client: &Client, need: &[Need]) -> Result<(), Error> {
        let scopes: BTreeSet<&str> = need.iter().filter(|n| !self.epochs.lock().unwrap().contains_key(&(n.scope.clone(), n.epoch))).map(|n| n.scope.as_str()).collect();
        let apps: BTreeSet<(&str, &str)> = need.iter().filter(|n| !self.app_keys.lock().unwrap().contains_key(&(n.scope.clone(), n.app.clone(), n.epoch))).map(|n| (n.scope.as_str(), n.app.as_str())).collect();
        let me = hex(&self.key);
        let asked = |path: String| async move { self.json(client, "GET", &path, None).await };
        let epochs = n0_future::join_all(scopes.iter().map(|scope| asked(format!("/api/keys?scope={}&to={me}", query(scope)))));
        let sealed = n0_future::join_all(apps.iter().map(|(scope, app)| asked(format!("/api/keys?scope={}&app={}", query(scope), query(app)))));
        let (epochs, sealed) = n0_future::future::zip(epochs, sealed).await;
        for (scope, kept) in scopes.iter().zip(epochs) {
            if let Ok(kept) = kept {
                self.open_epochs(scope, &kept).await?;
            }
        }
        for ((scope, app), kept) in apps.iter().zip(sealed) {
            for (m, sealed) in kept.iter().flat_map(|k| k["sealed"].as_object()).flatten() {
                let Ok(m) = m.parse::<u32>() else { continue };
                let Some(epoch) = self.epochs.lock().unwrap().get(&(scope.to_string(), m)).copied() else { continue };
                let opened = B64U.decode(sealed.as_str().unwrap_or("").trim().as_bytes()).ok().and_then(|b| ::key::open_app_key(&epoch, app, &b).ok());
                if let Some(key) = opened {
                    self.app_keys.lock().unwrap().insert((scope.to_string(), app.to_string(), m), key);
                }
            }
        }
        Ok(())
    }

    /// Opens each of a scope's epochs in `kept` (what `/api/keys` gave, sealed to this device) whose hash is the one
    /// who's who names.
    async fn open_epochs(&self, scope: &str, kept: &Value) -> Result<(), Error> {
        let Ok(parsed) = Scope::parse(scope) else { return Ok(()) };
        let mut opened: Vec<(u32, [u8; 32])> = Vec::new();
        for (m, sealed) in kept["sealed"].as_object().into_iter().flatten() {
            let (Ok(m), Ok(sealed)) = (m.parse::<u32>(), B64U.decode(sealed.as_str().unwrap_or("").as_bytes())) else { continue };
            if let Some(key) = ::key::open_sealed_by(&self.opens, &sealed).await.ok().and_then(|k| <[u8; 32]>::try_from(k).ok()) {
                opened.push((m, key));
            }
        }
        for (m, key) in opened {
            let made = self.who(|who| who.epoch_hash(&parsed, m)).transpose().map_err(failed)?.flatten();
            if made == Some(::key::epoch_hash(&key)) {
                self.epochs.lock().unwrap().insert((scope.to_string(), m), key);
            }
        }
        Ok(())
    }

    /// A database's key in an epoch.
    async fn database_key(&self, client: &Client, on: &(impl Address + Keep), n: &Need) -> Result<[u8; 32], Error> {
        Ok(::key::database_key(&self.app_key(client, on, &n.scope, &n.app, n.epoch).await?, &n.app, &n.database))
    }

    /// An app's key in epoch `n` of a scope: the one sealed under the epoch, or one made now if it has none yet.
    pub(crate) async fn app_key(&self, client: &Client, on: &(impl Address + Keep), scope: &str, app: &str, n: u32) -> Result<[u8; 32], Error> {
        let id = (scope.to_string(), app.to_string(), n);
        if let Some(key) = self.app_keys.lock().unwrap().get(&id) {
            return Ok(*key);
        }
        let epoch = self.epoch(client, on, scope, n).await?;
        let kept = self.json(client, "GET", &format!("/api/keys?scope={}&app={}", query(scope), query(app)), None).await?;
        let sealed = match kept["sealed"][n.to_string()].as_str() {
            Some(s) => s.to_string(),
            None => {
                let made: [u8; 32] = rand::random();
                let sealed = B64U.encode(&::key::seal_app_key(&epoch, app, &made));
                let out = self.json(client, "POST", "/api/keys", Some(json!({ "scope": scope, "n": n, "app": app, "sealed": sealed }))).await?;
                out["sealed"].as_str().unwrap_or_default().to_string()
            }
        };
        let bytes = B64U.decode(sealed.trim().as_bytes()).ok().ok_or_else(|| failed(format!("{app}'s key isn't one")))?;
        let key = ::key::open_app_key(&epoch, app, &bytes).map_err(|_| failed(format!("{app}'s key didn't open")))?;
        self.app_keys.lock().unwrap().insert(id, key);
        Ok(key)
    }

    /// Epoch `n` of a scope: opened from what's sealed to this device, if its hash is the one who's who names, read
    /// again once if it isn't yet.
    async fn epoch(&self, client: &Client, on: &(impl Address + Keep), scope: &str, n: u32) -> Result<[u8; 32], Error> {
        if let Some(key) = self.epochs.lock().unwrap().get(&(scope.to_string(), n)) {
            return Ok(*key);
        }
        Scope::parse(scope).map_err(failed)?;
        let kept = self.json(client, "GET", &format!("/api/keys?scope={}&to={}", query(scope), hex(&self.key)), None).await?;
        for fresh in [false, true] {
            if fresh {
                self.read(on).await?;
            }
            self.open_epochs(scope, &kept).await?;
            if let Some(key) = self.epochs.lock().unwrap().get(&(scope.to_string(), n)) {
                return Ok(*key);
            }
        }
        Err(failed("This device can't open that yet."))
    }
}

/// What a request is, to remember what it needed: its route (`keys::Route`) and the name it's remembered by.
struct Kind {
    route: ::api::handed::Route,
    name: String,
}

impl Kind {
    /// An MCP request is remembered by its method and tool (a batch by none), anything else by its route.
    fn of(path: &str, body: &[u8]) -> Kind {
        let route = ::api::handed::Route::of(path, || serde_json::from_slice::<Value>(body).ok().and_then(|b| b["app"].as_str().map(str::to_string)).unwrap_or_default());
        let name = match &route {
            ::api::handed::Route::Path(p) if p == ::api::path::MCP => match serde_json::from_slice::<Value>(body).ok().filter(|m| m["method"].is_string()) {
                Some(m) => {
                    let tool = m["params"]["name"].as_str().filter(|_| m["method"] == "tools/call").map(|t| format!(" {t}")).unwrap_or_default();
                    format!("{route} {}{tool}", m["method"].as_str().unwrap_or_default()).chars().take(160).collect()
                }
                None => route.to_string(),
            },
            _ => route.to_string(),
        };
        Kind { route, name }
    }
}

/// A node's answer as JSON: its error, with its code, unless it's 200.
async fn json_of(response: Response, path: &str) -> Result<Value, Error> {
    let status = response.status;
    let out: Value = match serde_json::from_slice(&response.body.bytes().await?) {
        Ok(out) => out,
        // A proxy's own error page isn't JSON: the status says it.
        Err(_) if status != 200 => Value::Null,
        Err(err) => return Err(failed(format!("{path} answered with a reply that wasn't JSON: {err}"))),
    };
    if status != 200 {
        let said = error(&out).unwrap_or_else(|| format!("{path} answered {status}"));
        return Err(Error { code: out["code"].as_str().map(str::to_string), ..failed(said) });
    }
    Ok(out)
}

pub fn query(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn error(out: &Value) -> Option<String> {
    out["error"].as_str().or(out["message"].as_str()).map(str::to_string)
}
