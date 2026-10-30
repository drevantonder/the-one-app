//! A node: this device's spaces, their stores and views, and its iroh endpoint.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use iroh::endpoint::{Connection, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use p2panda_core::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};

use crate::apps::{Apps, Folded, Me};
use crate::entry::{Content, Entry, Key, Order, SPACE_LOG, hex, short, unhex};
use crate::release::{self, Release};
use crate::space::{self, Members, Role};
use crate::store::Store;
use crate::sync::{self, Msg};

const JOIN_ALPN: &[u8] = b"one/join/1";
const CALL_ALPN: &[u8] = b"one/call/1";
pub const CLIENT_ALPN: &[u8] = b"one/client/1";
const BLOB_ALPN: &[u8] = b"one/blob/1";

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

#[derive(Serialize, Deserialize)]
struct DeviceFile {
    secret: String,
    name: String,
}

#[derive(Serialize, Deserialize)]
struct SpaceFile {
    person: String,
    #[serde(default)]
    root: Option<String>,
}

struct Space {
    store: Store,
    members: Members,
    person: String,
    folded: Option<Order>,
    /// Senders to every live sync session for this space, by peer device.
    sessions: HashMap<Key, mpsc::UnboundedSender<Msg>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Event {
    Changed { space: String, apps: Vec<String> },
    Peers { space: String },
    Spaces,
}

#[derive(Serialize, Deserialize)]
struct Invite {
    space: Key,
    person: Option<String>,
    /// Milliseconds since 1970 after which it no longer works.
    until: u64,
}

const INVITE_LIFE: u64 = 7 * 86_400_000;

#[derive(Serialize, Deserialize)]
struct Ticket {
    space: Key,
    inviter: EndpointAddr,
    secret: [u8; 16],
}

#[derive(Serialize, Deserialize)]
struct JoinRequest {
    secret: [u8; 16],
    name: String,
    #[serde(default)]
    client: bool,
}

#[derive(Serialize, Deserialize)]
struct JoinReply {
    space_name: String,
    person: String,
}

pub struct Inner {
    dir: PathBuf,
    key: SigningKey,
    id: Key,
    name: Mutex<String>,
    endpoint: Endpoint,
    apps: Apps,
    native: Vec<crate::native::Native>,
    spaces: Mutex<HashMap<Key, Space>>,
    invites: Mutex<HashMap<[u8; 16], Invite>>,
    events: broadcast::Sender<Event>,
    writes: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct Node(Arc<Inner>);

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node({})", short(&self.0.id))
    }
}

impl Node {
    pub async fn start(dir: &Path) -> Result<(Node, Router)> {
        std::fs::create_dir_all(dir)?;
        let device = load_device(dir)?;
        let secret = unhex(&device.secret)?;
        let key = SigningKey::from(secret);
        let id = *key.verifying_key().as_bytes();
        let endpoint = Endpoint::builder(presets::N0).secret_key(SecretKey::from_bytes(&secret)).bind().await?;
        let apps = Apps::start(dir.join("views"))?;
        let (events, _) = broadcast::channel(256);
        let node = Node(Arc::new(Inner {
            dir: dir.to_path_buf(),
            key,
            id,
            name: Mutex::new(device.name),
            endpoint: endpoint.clone(),
            apps,
            native: crate::native::all(),
            spaces: Mutex::new(HashMap::new()),
            invites: Mutex::new(load_invites(dir)),
            events,
            writes: tokio::sync::Mutex::new(()),
        }));
        for entry in std::fs::read_dir(dir.join("spaces")).into_iter().flatten().flatten() {
            if let Ok(space) = unhex(&entry.file_name().to_string_lossy()) {
                node.open_space(space).with_context(|| format!("open space {}", short(&space)))?;
                node.refold(space).await?;
            }
        }
        let router = Router::builder(endpoint)
            .accept(sync::ALPN, SyncHandler(node.clone()))
            .accept(JOIN_ALPN, JoinHandler(node.clone()))
            .accept(CALL_ALPN, CallHandler(node.clone()))
            .accept(CLIENT_ALPN, ClientHandler(node.clone()))
            .accept(BLOB_ALPN, BlobHandler(node.clone()))
            .spawn();
        tokio::spawn(node.clone().dial_loop());
        for space in node.spaces() {
            node.consider_update(space);
            for native in node.0.native.iter().filter(|n| n.sync.is_some()) {
                node.sync_native(space, native.name);
            }
        }
        Ok((node, router))
    }

    pub fn id(&self) -> Key {
        self.0.id
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.events.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.0.events.send(event);
    }

    fn space_dir(&self, space: &Key) -> PathBuf {
        self.0.dir.join("spaces").join(hex(space))
    }

    fn open_space(&self, space: Key) -> Result<()> {
        let dir = self.space_dir(&space);
        let file: SpaceFile = serde_json::from_slice(&std::fs::read(dir.join("space.json"))?)?;
        let store = Store::open(&dir.join("entries.db"), space)?;
        let members = Members::fold(&space, &store.ordered()?);
        self.0.spaces.lock().unwrap().insert(space, Space { store, members, person: file.person, folded: None, sessions: HashMap::new() });
        Ok(())
    }

    fn save_space(&self, space: &Key, file: &SpaceFile) -> Result<()> {
        let dir = self.space_dir(space);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("space.json"), serde_json::to_vec_pretty(file)?)?;
        Ok(())
    }

    pub fn device_name(&self) -> String {
        self.0.name.lock().unwrap().clone()
    }

    /// Creates a space with this device as its first admin.
    pub async fn create_space(&self, name: &str) -> Result<Key> {
        let root = SigningKey::generate();
        let space = *root.verifying_key().as_bytes();
        let person = new_person();
        self.save_space(&space, &SpaceFile { person: person.clone(), root: Some(hex(root.as_bytes())) })?;
        self.open_space(space)?;
        let genesis = space::genesis(&root, &self.0.id, name, &person, &self.device_name());
        self.append(space, genesis).await?;
        self.emit(Event::Spaces);
        Ok(space)
    }

    /// Writes an entry to this device's log in `space`.
    pub async fn append(&self, space: Key, content: Content) -> Result<Entry> {
        let _write = self.0.writes.lock().await;
        let entry = {
            let spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get(&space).context("no such space")?;
            let time = now_ms().max(s.store.max_time()? + 1);
            let prev = s.store.last(&self.0.id, SPACE_LOG)?;
            let entry = Entry::create(&self.0.key, space, time, prev.as_ref(), content);
            s.store.insert(&entry)?;
            entry
        };
        self.after(space, std::slice::from_ref(&entry), None).await?;
        Ok(entry)
    }

    /// Folds new entries and passes them on to every session but the one they came from.
    async fn after(&self, space: Key, new: &[Entry], from: Option<Key>) -> Result<()> {
        if new.is_empty() {
            return Ok(());
        }
        let (folded, rebuild, apps) = {
            let mut spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get_mut(&space).context("no such space")?;
            let msg = sync::entries(space, new);
            for (peer, tx) in &s.sessions {
                if Some(*peer) != from {
                    let _ = tx.send(msg.clone());
                }
            }
            let membership = new.iter().any(|e| e.content.app == space::APP);
            if membership {
                s.members = Members::fold(&space, &s.store.ordered()?);
            }
            let earliest = new.iter().map(Entry::order).min().unwrap();
            let rebuild = membership || s.folded.is_some_and(|f| earliest <= f);
            let entries = if rebuild { s.store.ordered()? } else { sorted(new) };
            let folded = to_fold(&s.members, &entries);
            s.folded = s.folded.max(entries.iter().map(Entry::order).max());
            let apps: Vec<String> = new.iter().map(|e| e.content.app.clone()).collect();
            (folded, rebuild, apps)
        };
        if new.iter().any(|e| e.content.app == release::APP) {
            self.consider_update(space);
        }
        for native in &self.0.native {
            if native.sync.is_some() && new.iter().any(|e| e.content.app == native.name) {
                self.sync_native(space, native.name);
            }
        }
        self.0.apps.fold(space, folded, rebuild).await?;
        self.emit(Event::Changed { space: hex(&space), apps });
        Ok(())
    }

    async fn refold(&self, space: Key) -> Result<()> {
        let folded = {
            let mut spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get_mut(&space).context("no such space")?;
            let entries = s.store.ordered()?;
            s.folded = entries.iter().map(Entry::order).max();
            to_fold(&s.members, &entries)
        };
        self.0.apps.fold(space, folded, true).await
    }

    /// Who `device` is in `space`, as apps see them.
    pub fn me_of(&self, space: &Key, device: &Key) -> Result<Me> {
        let spaces = self.0.spaces.lock().unwrap();
        let s = spaces.get(space).context("no such space")?;
        ensure!(s.members.current(&self.0.id).is_some(), "this device is not a member of the space yet");
        let m = s.members.current(device).context("not a member of this space")?;
        let name = if *device == self.0.id { self.device_name() } else { m.name.clone() };
        Ok(Me { space: hex(space), device: hex(device), person: m.person.clone(), name })
    }

    /// Runs an app operation on this device, appending whatever entries it asks for.
    pub async fn call(&self, space: Key, app: &str, op: &str, input: Value) -> Result<Value> {
        self.call_as(space, self.0.id, app, op, input).await
    }

    /// Runs an operation on this device for `caller`, another device in the space.
    pub async fn call_as(&self, space: Key, caller: Key, app: &str, op: &str, input: Value) -> Result<Value> {
        let me = self.me_of(&space, &caller)?;
        if let Some(native) = self.0.native.iter().find(|n| n.name == app) {
            ensure!(me.person == self.me_of(&space, &self.0.id)?.person, "{app} runs only for this device's own person's devices");
            let (result, appends) = (native.call)(op.to_string(), input, me, self.app_entries(space, app)?).await?;
            for content in appends {
                self.append(space, content).await?;
            }
            return Ok(result);
        }
        let outcome = self.0.apps.call(space, app, op, input, me, now_ms()).await?;
        for content in outcome.appends {
            self.append(space, content).await?;
        }
        Ok(outcome.result)
    }

    /// Runs an operation on another device in the space.
    pub async fn call_on(&self, device: Key, space: Key, app: &str, op: &str, input: Value) -> Result<Value> {
        if device == self.0.id {
            return self.call(space, app, op, input).await;
        }
        let conn = self.0.endpoint.connect(EndpointId::from_bytes(&device)?, CALL_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let request = serde_json::to_vec(&json!({ "space": hex(&space), "app": app, "op": op, "input": input }))?;
        send.write_all(&request).await?;
        send.finish()?;
        let reply: Value = serde_json::from_slice(&recv.read_to_end(16 << 20).await?)?;
        match reply.get("error") {
            Some(err) => bail!("{}", err.as_str().unwrap_or("remote error")),
            None => Ok(reply["ok"].clone()),
        }
    }

    pub async fn widgets(&self, space: Key, viewer: Key) -> Result<Vec<Value>> {
        let me = self.me_of(&space, &viewer)?;
        let mut out = Vec::new();
        for app in self.0.apps.describe().await? {
            let name = app["name"].as_str().unwrap_or_default().to_string();
            for w in app["widgets"].as_array().into_iter().flatten() {
                let widget = w["name"].as_str().unwrap_or_default();
                match self.0.apps.widget(space, &name, widget, me.clone(), now_ms()).await {
                    Ok(root) => out.push(json!({ "app": name, "widget": widget, "title": w["title"], "root": root })),
                    Err(err) => tracing::warn!("{name} widget {widget}: {err:#}"),
                }
            }
        }
        Ok(out)
    }

    /// Runs an app's hook for an agent harness on this device (SPEC.md, "Hooks"). The node remembers,
    /// per session, when the hook last ran, and tells the app as `since` (null on a session's first call).
    pub async fn hook(&self, app: &str, name: &str, event: Value) -> Result<Value> {
        let space = self.find_space(None)?;
        let session = event["session_id"].as_str().unwrap_or("").to_string();
        let path = self.0.dir.join("hooks.json");
        let mut seen: serde_json::Map<String, Value> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let key = format!("{app}/{name}/{session}");
        let since = seen.get(&key).cloned().unwrap_or(Value::Null);
        let now = now_ms();
        let me = self.me_of(&space, &self.0.id)?;
        let out = self.0.apps.hook(space, app, name, json!({ "event": event, "since": since }), me, now).await?;
        seen.insert(key, json!(now));
        let week_ago = now.saturating_sub(7 * 86_400_000);
        seen.retain(|_, t| t.as_u64().is_some_and(|t| t > week_ago));
        std::fs::write(&path, serde_json::to_vec(&seen)?)?;
        Ok(out)
    }

    pub async fn apps(&self) -> Result<Vec<Value>> {
        let mut apps = self.0.apps.describe().await?;
        apps.extend(self.0.native.iter().map(|n| (n.describe)()));
        Ok(apps)
    }

    pub fn spaces(&self) -> Vec<Key> {
        let mut keys: Vec<Key> = self.0.spaces.lock().unwrap().keys().copied().collect();
        keys.sort();
        keys
    }

    /// Everything a shell needs to draw this device's spaces.
    pub fn state(&self) -> Value {
        self.state_for(self.0.id)
    }

    /// The state as `viewer`, a device in some of this node's spaces, sees it.
    pub fn state_for(&self, viewer: Key) -> Value {
        let viewer_name = if viewer == self.0.id { self.device_name() } else { self.name_of(&viewer) };
        let spaces = self.0.spaces.lock().unwrap();
        let mut list: Vec<Value> = spaces
            .iter()
            .filter(|(_, s)| viewer == self.0.id || s.members.current(&viewer).is_some())
            .map(|(key, s)| {
                let devices: Vec<Value> = s
                    .members
                    .current_devices()
                    .map(|m| {
                        json!({
                            "device": hex(&m.device), "name": m.name, "person": m.person, "role": m.role, "client": m.client,
                            "me": m.device == viewer, "host": m.device == self.0.id,
                            "online": m.device == self.0.id || m.device == viewer || s.sessions.contains_key(&m.device),
                        })
                    })
                    .collect();
                json!({
                    "id": hex(key), "name": s.members.name, "person": s.person,
                    "member": s.members.current(&viewer).is_some(),
                    "role": s.members.current(&viewer).map(|m| m.role.clone()),
                    "devices": devices, "entries": s.store.count().unwrap_or(0),
                })
            })
            .collect();
        list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        json!({
            "brand": { "name": crate::brand::NAME, "cli": crate::brand::CLI, "accent": crate::brand::ACCENT },
            "device": { "id": hex(&viewer), "name": viewer_name },
            "host": { "id": hex(&self.0.id), "name": self.device_name() },
            "spaces": list,
        })
    }

    /// Removes a device from a space. Admins only, as SPEC.md says.
    pub async fn remove_device(&self, space: Key, device: Key) -> Result<()> {
        {
            let spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get(&space).context("no such space")?;
            ensure!(s.members.current(&self.0.id).is_some_and(|m| m.role == Role::Admin), "only admins remove devices");
            ensure!(s.members.current(&device).is_some(), "that device isn't in the space");
        }
        self.append(space, space::remove(&device)).await?;
        Ok(())
    }

    fn name_of(&self, device: &Key) -> String {
        let spaces = self.0.spaces.lock().unwrap();
        spaces.values().find_map(|s| s.members.current(device).map(|m| m.name.clone())).unwrap_or_default()
    }

    fn is_member_anywhere(&self, device: &Key) -> bool {
        self.0.spaces.lock().unwrap().values().any(|s| s.members.current(device).is_some())
    }

    /// Serves a client device (a phone) that keeps no log of its own: one JSON request per stream.
    async fn accept_client(&self, conn: Connection) -> Result<()> {
        let caller = *conn.remote_id().as_bytes();
        tracing::debug!("client {} connected", short(&caller));
        ensure!(self.is_member_anywhere(&caller), "{} is not a member of any space here", short(&caller));
        loop {
            let (mut send, mut recv) = match conn.accept_bi().await {
                Ok(s) => s,
                Err(_) => return Ok(()),
            };
            let node = self.clone();
            tokio::spawn(async move {
                let result = async {
                    let request: Value = serde_json::from_slice(&recv.read_to_end(16 << 20).await?)?;
                    let (method, path) = (request["method"].as_str().unwrap_or("GET"), request["path"].as_str().unwrap_or(""));
                    if path == "/v1/events" {
                        let mut events = node.subscribe();
                        while let Ok(event) = events.recv().await {
                            send.write_all(format!("{}\n", serde_json::to_string(&event)?).as_bytes()).await?;
                        }
                        return anyhow::Ok(());
                    }
                    let reply = match node.client_request(caller, method, path, request["body"].clone()).await {
                        Ok(ok) => ok,
                        Err(err) => json!({ "error": format!("{err:#}") }),
                    };
                    send.write_all(&serde_json::to_vec(&reply)?).await?;
                    send.finish()?;
                    Ok(())
                }
                .await;
                if let Err(err) = result {
                    tracing::debug!("client request: {err:#}");
                }
            });
        }
    }

    async fn client_request(&self, caller: Key, method: &str, path: &str, body: Value) -> Result<Value> {
        let space = |body: &Value| -> Result<Key> {
            let key = self.find_space(body["space"].as_str())?;
            ensure!(self.0.spaces.lock().unwrap()[&key].members.current(&caller).is_some(), "not a member of that space");
            Ok(key)
        };
        Ok(match (method, path) {
            ("GET", "/v1/state") => self.state_for(caller),
            ("GET", "/v1/apps") => json!(self.apps().await?),
            ("GET", "/v1/widgets") => json!(self.widgets(space(&body)?, caller).await?),
            ("POST", "/v1/call") => {
                let input = if body["input"].is_null() { json!({}) } else { body["input"].clone() };
                let ok = self.call_as(space(&body)?, caller, body["app"].as_str().unwrap_or(""), body["op"].as_str().unwrap_or(""), input).await?;
                json!({ "ok": ok })
            }
            ("POST", "/v1/invite") => {
                let key = space(&body)?;
                ensure!(body["mine"] == true, "phones invite only their person's own devices for now");
                json!({ "ticket": self.invite_for(key, caller)? })
            }
            _ => bail!("no such request: {method} {path}"),
        })
    }

    pub async fn rename_device(&self, name: &str) -> Result<()> {
        *self.0.name.lock().unwrap() = name.to_string();
        let path = self.0.dir.join("device.json");
        let mut file: DeviceFile = serde_json::from_slice(&std::fs::read(&path)?)?;
        file.name = name.to_string();
        std::fs::write(&path, serde_json::to_vec_pretty(&file)?)?;
        for space in self.spaces() {
            self.append(space, space::rename(&self.0.id, name)).await?;
        }
        Ok(())
    }

    /// Makes a one-time ticket another device can join `space` with.
    pub fn invite(&self, space: Key, mine: bool) -> Result<String> {
        let person = {
            let spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get(&space).context("no such space")?;
            let me = s.members.current(&self.0.id).context("this device is not a member")?;
            ensure!(mine || me.role == Role::Admin, "only admins invite other people; invite your own device instead");
            mine.then(|| s.person.clone())
        };
        let secret: [u8; 16] = rand::random();
        self.add_invite(secret, Invite { space, person, until: now_ms() + INVITE_LIFE })?;
        let ticket = Ticket { space, inviter: self.0.endpoint.addr(), secret };
        let mut bytes = Vec::new();
        ciborium::into_writer(&ticket, &mut bytes)?;
        Ok(data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase())
    }

    /// A ticket for another device of `caller`'s person.
    fn invite_for(&self, space: Key, caller: Key) -> Result<String> {
        let person = {
            let spaces = self.0.spaces.lock().unwrap();
            spaces[&space].members.current(&caller).context("not a member")?.person.clone()
        };
        let secret: [u8; 16] = rand::random();
        self.add_invite(secret, Invite { space, person: Some(person), until: now_ms() + INVITE_LIFE })?;
        let ticket = Ticket { space, inviter: self.0.endpoint.addr(), secret };
        let mut bytes = Vec::new();
        ciborium::into_writer(&ticket, &mut bytes)?;
        Ok(data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase())
    }

    /// Invites are kept in a file, so a ticket still works after its inviter restarts.
    fn add_invite(&self, secret: [u8; 16], invite: Invite) -> Result<()> {
        let mut invites = self.0.invites.lock().unwrap();
        invites.retain(|_, i| i.until > now_ms());
        invites.insert(secret, invite);
        self.save_invites(&invites)
    }

    fn take_invite(&self, secret: &[u8; 16]) -> Result<Option<Invite>> {
        let mut invites = self.0.invites.lock().unwrap();
        let invite = invites.remove(secret);
        self.save_invites(&invites)?;
        Ok(invite)
    }

    fn save_invites(&self, invites: &HashMap<[u8; 16], Invite>) -> Result<()> {
        let list: Vec<(String, &Invite)> = invites.iter().map(|(k, v)| (data_encoding::HEXLOWER.encode(k), v)).collect();
        std::fs::write(self.0.dir.join("invites.json"), serde_json::to_vec(&list)?)?;
        Ok(())
    }

    pub async fn join(&self, ticket: &str) -> Result<Key> {
        let bytes = data_encoding::BASE32_NOPAD.decode(ticket.trim().to_uppercase().as_bytes()).context("not a ticket")?;
        let ticket: Ticket = ciborium::from_reader(bytes.as_slice()).context("not a ticket")?;
        ensure!(!self.0.spaces.lock().unwrap().contains_key(&ticket.space), "this device is already in that space");
        let conn = self.0.endpoint.connect(ticket.inviter.clone(), JOIN_ALPN).await.context("reach the inviting device")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&serde_json::to_vec(&JoinRequest { secret: ticket.secret, name: self.device_name(), client: false })?).await?;
        send.finish()?;
        let reply: Value = serde_json::from_slice(&recv.read_to_end(1 << 16).await?)?;
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("the invite was refused"));
        }
        let reply: JoinReply = serde_json::from_value(reply)?;
        self.save_space(&ticket.space, &SpaceFile { person: reply.person, root: None })?;
        self.open_space(ticket.space)?;
        let inviter = ticket.inviter.id;
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.connect_sync(ticket.space, *inviter.as_bytes()).await {
                tracing::warn!("first sync after joining: {err:#}");
            }
        });
        self.emit(Event::Spaces);
        Ok(ticket.space)
    }

    async fn accept_join(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let request: JoinRequest = serde_json::from_slice(&recv.read_to_end(1 << 16).await?)?;
        let invite = self.take_invite(&request.secret)?.filter(|i| i.until > now_ms());
        let Some(invite) = invite else {
            send.write_all(br#"{"error":"this invite was already used or has expired"}"#).await?;
            send.finish()?;
            return Ok(());
        };
        let person = invite.person.unwrap_or_else(new_person);
        let admit = if request.client { space::admit_client(&remote, &person, &request.name) } else { space::admit(&remote, &person, &request.name, Role::Member) };
        self.append(invite.space, admit).await?;
        let space_name = self.0.spaces.lock().unwrap()[&invite.space].members.name.clone();
        send.write_all(&serde_json::to_vec(&JoinReply { space_name, person })?).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }

    async fn dial_loop(self) {
        loop {
            let wanted: Vec<(Key, Key)> = {
                let spaces = self.0.spaces.lock().unwrap();
                spaces
                    .iter()
                    .flat_map(|(space, s)| {
                        s.members
                            .current_devices()
                            .filter(|m| m.device != self.0.id && !m.client && !s.sessions.contains_key(&m.device))
                            .map(|m| (*space, m.device))
                            .collect::<Vec<_>>()
                    })
                    .collect()
            };
            for (space, device) in wanted {
                let node = self.clone();
                tokio::spawn(async move {
                    if let Err(err) = node.connect_sync(space, device).await {
                        tracing::debug!("sync with {}: {err:#}", short(&device));
                    }
                });
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    }

    async fn connect_sync(&self, space: Key, device: Key) -> Result<()> {
        let conn = tokio::time::timeout(Duration::from_secs(20), self.0.endpoint.connect(EndpointId::from_bytes(&device)?, sync::ALPN))
            .await
            .context("timed out")??;
        let (send, recv) = conn.open_bi().await?;
        self.session(space, device, send, recv, None).await
    }

    async fn accept_sync(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let (send, mut recv) = conn.accept_bi().await?;
        let first = sync::read(&mut recv).await?;
        let Msg::Hello { space, .. } = &first else { bail!("expected hello") };
        let space = *space;
        {
            let spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get(&space).context("not in that space")?;
            ensure!(s.members.current(&remote).is_some(), "{} is not a member", short(&remote));
        }
        self.session(space, remote, send, recv, Some(first)).await
    }

    /// Runs one sync session until the connection drops.
    async fn session(&self, space: Key, peer: Key, mut send: iroh::endpoint::SendStream, mut recv: iroh::endpoint::RecvStream, first: Option<Msg>) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
        {
            let mut spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get_mut(&space).context("no such space")?;
            tx.send(sync::hello(&s.store)?).ok();
            s.sessions.insert(peer, tx.clone());
        }
        self.emit(Event::Peers { space: hex(&space) });
        tracing::info!("syncing {} with {}", short(&space), short(&peer));
        let writer = async {
            while let Some(msg) = rx.recv().await {
                sync::write(&mut send, &msg).await?;
            }
            anyhow::Ok(())
        };
        let hellos = async {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let hello = { sync::hello(&self.0.spaces.lock().unwrap()[&space].store)? };
                tx.send(hello).map_err(|_| anyhow!("session closed"))?;
            }
        };
        let reader = async {
            if let Some(msg) = first {
                if let Some(reply) = self.handle(space, peer, msg).await? {
                    tx.send(reply).map_err(|_| anyhow!("session closed"))?;
                }
            }
            loop {
                let msg = sync::read(&mut recv).await?;
                if let Some(reply) = self.handle(space, peer, msg).await? {
                    tx.send(reply).map_err(|_| anyhow!("session closed"))?;
                }
            }
        };
        let result: Result<()> = tokio::select! { r = writer => r, r = hellos => r, r = reader => r };
        {
            let mut spaces = self.0.spaces.lock().unwrap();
            if let Some(s) = spaces.get_mut(&space) {
                if s.sessions.get(&peer).is_some_and(|t| t.same_channel(&tx)) {
                    s.sessions.remove(&peer);
                }
            }
        }
        self.emit(Event::Peers { space: hex(&space) });
        result
    }

    async fn handle(&self, space: Key, peer: Key, msg: Msg) -> Result<Option<Msg>> {
        let _write = self.0.writes.lock().await;
        let (new, reply) = {
            let spaces = self.0.spaces.lock().unwrap();
            sync::handle(&spaces.get(&space).context("no such space")?.store, msg)?
        };
        self.after(space, &new, Some(peer)).await?;
        Ok(reply)
    }

    async fn accept_call(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let request: Value = serde_json::from_slice(&recv.read_to_end(16 << 20).await?)?;
        let result = async {
            let space = unhex(request["space"].as_str().context("no space")?)?;
            {
                let spaces = self.0.spaces.lock().unwrap();
                ensure!(spaces.get(&space).and_then(|s| s.members.current(&remote)).is_some(), "not a member");
            }
            let (app, op) = (request["app"].as_str().unwrap_or_default(), request["op"].as_str().unwrap_or_default());
            self.call_as(space, remote, app, op, request["input"].clone()).await
        }
        .await;
        let reply = match result {
            Ok(ok) => json!({ "ok": ok }),
            Err(err) => json!({ "error": format!("{err:#}") }),
        };
        send.write_all(&serde_json::to_vec(&reply)?).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }

    /// Finds a device in a space by name or id prefix.
    pub fn find_device(&self, space: Key, name: &str) -> Result<Key> {
        let spaces = self.0.spaces.lock().unwrap();
        let s = spaces.get(&space).context("no such space")?;
        let found: Vec<Key> = s.members.current_devices().filter(|m| m.name.eq_ignore_ascii_case(name) || hex(&m.device).starts_with(name)).map(|m| m.device).collect();
        match found.as_slice() {
            [one] => Ok(*one),
            [] => bail!("no device called {name}"),
            _ => bail!("several devices match {name}"),
        }
    }

    /// Finds a space by id, name or id prefix.
    pub fn find_space(&self, name: Option<&str>) -> Result<Key> {
        let spaces = self.0.spaces.lock().unwrap();
        let mut matches: Vec<Key> = match name {
            None => spaces.keys().copied().collect(),
            Some(n) => spaces.iter().filter(|(k, s)| s.members.name.eq_ignore_ascii_case(n) || hex(k).starts_with(n)).map(|(k, _)| *k).collect(),
        };
        matches.sort();
        match matches.as_slice() {
            [one] => Ok(*one),
            [] if name.is_none() => bail!("this device is in no space yet: create one, or join with a ticket"),
            [] => bail!("no space called {}", name.unwrap()),
            _ => bail!("this device is in several spaces: say which"),
        }
    }
}

fn sorted(entries: &[Entry]) -> Vec<Entry> {
    let mut v = entries.to_vec();
    v.sort_by_key(Entry::order);
    v
}

impl Node {
    /// Publishes `binary` (a build of this node for `target`) as the space's newest release. Admins only.
    pub async fn publish(&self, space: Key, binary: &Path, commit: &str, target: &str) -> Result<Release> {
        {
            let spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get(&space).context("no such space")?;
            ensure!(s.members.current(&self.0.id).is_some_and(|m| m.role == Role::Admin), "only admins publish releases");
        }
        let bytes = std::fs::read(binary).with_context(|| format!("read {}", binary.display()))?;
        let blob = release::put_blob(&self.0.dir, &bytes)?;
        let r = Release { commit: commit.into(), target: target.into(), blob, size: bytes.len() as u64 };
        self.append(space, release::content(&r)).await?;
        Ok(r)
    }

    /// Updates this node if an admin published a newer build for its platform.
    fn consider_update(&self, space: Key) {
        let latest = {
            let spaces = self.0.spaces.lock().unwrap();
            let Some(s) = spaces.get(&space) else { return };
            match s.store.ordered() {
                Ok(entries) => release::latest(&s.members, &entries, crate::brand::TARGET),
                Err(_) => return,
            }
        };
        let Some((r, author)) = latest else { return };
        let marker = self.0.dir.join("release");
        if r.commit == crate::brand::BUILD || std::fs::read_to_string(&marker).is_ok_and(|m| m.trim() == r.blob) {
            return;
        }
        let node = self.clone();
        tokio::spawn(async move {
            match node.apply(space, &r, author).await {
                Ok(exe) => {
                    let _ = std::fs::write(&marker, &r.blob);
                    tracing::info!("updated {} to {}; restarting", exe.display(), r.commit);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    std::process::exit(0);
                }
                Err(err) => tracing::warn!("update to {}: {err:#}", r.commit),
            }
        });
    }

    async fn apply(&self, space: Key, r: &Release, author: Key) -> Result<PathBuf> {
        let path = release::blob_path(&self.0.dir, &r.blob)?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                let mut peers = vec![author];
                peers.extend(self.peers(space).into_iter().filter(|p| *p != author));
                let mut last = anyhow!("no device had it");
                let mut found = None;
                for peer in peers {
                    match self.fetch_blob(peer, &r.blob).await {
                        Ok(b) => {
                            found = Some(b);
                            break;
                        }
                        Err(err) => last = err,
                    }
                }
                let bytes = found.ok_or(last)?;
                release::put_blob(&self.0.dir, &bytes)?;
                bytes
            }
        };
        release::install(&bytes)
    }

    /// Every entry an app has in a space, by members, oldest first.
    fn app_entries(&self, space: Key, app: &str) -> Result<Vec<Folded>> {
        let spaces = self.0.spaces.lock().unwrap();
        let s = spaces.get(&space).context("no such space")?;
        Ok(to_fold(&s.members, &s.store.ordered()?).into_iter().filter(|e| e.app == app).collect())
    }

    fn sync_native(&self, space: Key, app: &'static str) {
        let Some(sync) = self.0.native.iter().find(|n| n.name == app).and_then(|n| n.sync) else { return };
        let Ok(entries) = self.app_entries(space, app) else { return };
        tokio::task::spawn_blocking(move || sync(&entries));
    }

    fn peers(&self, space: Key) -> Vec<Key> {
        let spaces = self.0.spaces.lock().unwrap();
        spaces.get(&space).map(|s| s.members.current_devices().map(|m| m.device).filter(|d| *d != self.0.id).collect()).unwrap_or_default()
    }

    async fn fetch_blob(&self, peer: Key, hash: &str) -> Result<Vec<u8>> {
        let conn = self.0.endpoint.connect(EndpointId::from_bytes(&peer)?, BLOB_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(hash.as_bytes()).await?;
        send.finish()?;
        let bytes = recv.read_to_end(1 << 30).await?;
        ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "the blob didn't match its hash");
        Ok(bytes)
    }

    async fn accept_blob(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        ensure!(self.is_member_anywhere(&remote), "not a member");
        let (mut send, mut recv) = conn.accept_bi().await?;
        let hash = String::from_utf8(recv.read_to_end(64).await?)?;
        let bytes = std::fs::read(release::blob_path(&self.0.dir, &hash)?).context("no such blob here")?;
        send.write_all(&bytes).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }
}

/// The entries app folds see: by members, minus membership itself.
fn to_fold(members: &Members, entries: &[Entry]) -> Vec<Folded> {
    entries
        .iter()
        .filter(|e| e.content.app != space::APP && e.content.app != release::APP)
        .filter_map(|e| {
            let m = members.active(&e.author(), e.order())?;
            Some(Folded {
                app: e.content.app.clone(),
                kind: e.content.kind.clone(),
                author: hex(&e.author()),
                person: m.person.clone(),
                name: m.name.clone(),
                time: e.time(),
                id: format!("{}:{}", short(&e.author()), e.seq()),
                data: e.content.data.clone(),
            })
        })
        .collect()
}

fn load_invites(dir: &Path) -> HashMap<[u8; 16], Invite> {
    let list: Vec<(String, Invite)> = std::fs::read(dir.join("invites.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    list.into_iter()
        .filter_map(|(k, v)| Some((data_encoding::HEXLOWER.decode(k.as_bytes()).ok()?.try_into().ok()?, v)))
        .collect()
}

fn new_person() -> String {
    data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>())
}

fn load_device(dir: &Path) -> Result<DeviceFile> {
    let path = dir.join("device.json");
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    let file = DeviceFile { secret: hex(&rand::random::<[u8; 32]>()), name: hostname() };
    std::fs::write(&path, serde_json::to_vec_pretty(&file)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "device".into())
}

#[derive(Clone, Debug)]
struct SyncHandler(Node);
#[derive(Clone, Debug)]
struct JoinHandler(Node);
#[derive(Clone, Debug)]
struct CallHandler(Node);
#[derive(Clone, Debug)]
struct ClientHandler(Node);
#[derive(Clone, Debug)]
struct BlobHandler(Node);

impl ProtocolHandler for BlobHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_blob(conn).await.map_err(accept_err)
    }
}

impl ProtocolHandler for ClientHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_client(conn).await.map_err(accept_err)
    }
}

fn accept_err(err: anyhow::Error) -> AcceptError {
    AcceptError::from_boxed(err.into())
}

impl ProtocolHandler for SyncHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_sync(conn).await.map_err(accept_err)
    }
}

impl ProtocolHandler for JoinHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_join(conn).await.map_err(accept_err)
    }
}

impl ProtocolHandler for CallHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.0.accept_call(conn).await.map_err(accept_err)
    }
}
