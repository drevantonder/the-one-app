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

use crate::apps::{Apps, Replay, AppEntry, Me};
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
    user: String,
    /// The device whose pairing code this one joined with, to sync with until it sees itself admitted.
    #[serde(default)]
    paired_by: Option<String>,
}

struct Space {
    store: Store,
    members: Members,
    user: String,
    paired_by: Option<Key>,
    /// The build each device last said it runs.
    builds: HashMap<Key, release::Running>,
    /// What each of this user's devices announced it provides (SPEC.md, "Providers").
    provides: Vec<(Key, Value)>,
    replayed: Option<Order>,
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
struct Pairing {
    space: Key,
    user: Option<String>,
    /// Milliseconds since 1970 after which it no longer works.
    until: u64,
}

const PAIRING_LIFE: u64 = 7 * 86_400_000;

#[derive(Serialize, Deserialize)]
struct PairingCode {
    space: Key,
    from: EndpointAddr,
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
    user: String,
}

pub struct Inner {
    dir: PathBuf,
    key: SigningKey,
    id: Key,
    name: Mutex<String>,
    endpoint: Endpoint,
    apps: Apps,
    apps_names: Vec<String>,
    native: Vec<crate::native::Native>,
    pub providers: crate::providers::Providers,
    spaces: Mutex<HashMap<Key, Space>>,
    pairings: Mutex<HashMap<[u8; 16], Pairing>>,
    events: broadcast::Sender<Event>,
    writes: tokio::sync::Mutex<()>,
    /// Wakes the task that announces this device when its membership in a space may have changed.
    membership_changed: tokio::sync::Notify,
}

#[derive(Clone)]
pub struct Node(Arc<Inner>);

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node({})", short(&self.0.id))
    }
}

impl Node {
    /// Starts this device's node with the apps it runs: JavaScript ones and native ones.
    pub async fn start(dir: &Path, builtin: crate::native::Builtin) -> Result<(Node, Router)> {
        std::fs::create_dir_all(dir)?;
        let device = load_device(dir)?;
        let secret = unhex(&device.secret)?;
        let key = SigningKey::from(secret);
        let id = *key.verifying_key().as_bytes();
        let endpoint = Endpoint::builder(presets::N0).secret_key(SecretKey::from_bytes(&secret)).bind().await?;
        let apps_names: Vec<String> = builtin.apps.iter().map(|(n, _)| n.clone()).collect();
        // Views used to live here, before apps ran in workerd.
        let _ = std::fs::remove_dir_all(dir.join("views"));
        let apps = Apps::start(dir.join("runtime"), builtin.apps).await?;
        let (events, _) = broadcast::channel(256);
        let node = Node(Arc::new(Inner {
            dir: dir.to_path_buf(),
            key,
            id,
            name: Mutex::new(device.name),
            endpoint: endpoint.clone(),
            apps,
            apps_names,
            native: builtin.native,
            providers: Default::default(),
            spaces: Mutex::new(HashMap::new()),
            pairings: Mutex::new(load_pairings(dir)),
            events,
            writes: tokio::sync::Mutex::new(()),
            membership_changed: tokio::sync::Notify::new(),
        }));
        for entry in std::fs::read_dir(dir.join("spaces")).into_iter().flatten().flatten() {
            if let Ok(space) = unhex(&entry.file_name().to_string_lossy()) {
                node.open_space(space).with_context(|| format!("open space {}", short(&space)))?;
                node.replay_all(space).await?;
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
        let announcer = node.clone();
        tokio::spawn(async move {
            loop {
                announcer.0.membership_changed.notified().await;
                for space in announcer.spaces() {
                    if let Err(err) = announcer.announce(space).await {
                        tracing::warn!("announce: {err:#}");
                    }
                }
            }
        });
        for space in node.spaces() {
            node.consider_update(space);
            if let Err(err) = node.announce(space).await {
                tracing::warn!("announce: {err:#}");
            }
            for native in node.0.native.iter().filter(|n| n.sync.is_some()) {
                node.sync_native(space, native.name);
            }
        }
        Ok((node, router))
    }

    pub fn id(&self) -> Key {
        self.0.id
    }

    pub fn providers(&self) -> &crate::providers::Providers {
        &self.0.providers
    }

    /// Registers a provider and announces it to every space this device is in.
    pub async fn provide(&self, app: &str, describe: Value) -> Result<()> {
        self.0.providers.register(app, describe);
        for space in self.spaces() {
            self.announce(space).await?;
        }
        Ok(())
    }

    /// Tells a space what this device is: the build it runs and the providers it has, where the
    /// space doesn't know yet. Runs when the node starts, when a provider registers, and when this
    /// device's membership changes, so a device that just created or joined a space announces itself.
    async fn announce(&self, space: Key) -> Result<()> {
        let missing: Vec<Content> = {
            let spaces = self.0.spaces.lock().unwrap();
            let Some(s) = spaces.get(&space).filter(|s| s.members.current(&self.0.id).is_some()) else { return Ok(()) };
            let mut missing: Vec<Content> = self
                .0
                .providers
                .describe()
                .into_iter()
                .filter(|d| !s.provides.iter().any(|(dev, v)| *dev == self.0.id && v == d))
                .map(|d| crate::providers::announcement(&d))
                .collect();
            if s.builds.get(&self.0.id).map(|r| r.commit.as_str()) != Some(crate::brand::BUILD) {
                missing.push(release::running());
            }
            missing
        };
        for content in missing {
            self.append(space, content).await?;
        }
        Ok(())
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
        let members = Members::replay(&space, &store.ordered()?);
        let paired_by = file.paired_by.and_then(|h| unhex(&h).ok());
        self.0.spaces.lock().unwrap().insert(space, Space { store, members, user: file.user, paired_by, builds: HashMap::new(), provides: Vec::new(), replayed: None, sessions: HashMap::new() });
        Ok(())
    }

    fn save_space(&self, space: &Key, file: &SpaceFile) -> Result<()> {
        let dir = self.space_dir(space);
        std::fs::create_dir_all(&dir)?;
        write_private(&dir.join("space.json"), &serde_json::to_vec_pretty(file)?)?;
        Ok(())
    }

    pub fn device_name(&self) -> String {
        self.0.name.lock().unwrap().clone()
    }

    /// Creates a space with this device as its first admin.
    pub async fn create_space(&self, name: &str) -> Result<Key> {
        let root = SigningKey::generate();
        let space = *root.verifying_key().as_bytes();
        let user = new_user();
        self.save_space(&space, &SpaceFile { user: user.clone(), paired_by: None })?;
        self.open_space(space)?;
        let genesis = space::genesis(&root, &self.0.id, name, &user, &self.device_name());
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
            let time = now_ms().max(s.store.max_time()?.saturating_add(1));
            let prev = s.store.last(&self.0.id, SPACE_LOG)?;
            let entry = Entry::create(&self.0.key, space, time, prev.as_ref(), content);
            s.store.insert(&entry)?;
            entry
        };
        self.after(space, std::slice::from_ref(&entry), None).await?;
        Ok(entry)
    }

    /// Replays new entries into the apps and passes them on to every session but the one they came from.
    async fn after(&self, space: Key, new: &[Entry], from: Option<Key>) -> Result<()> {
        if new.is_empty() {
            return Ok(());
        }
        let (jobs, apps) = {
            let mut spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get_mut(&space).context("no such space")?;
            let membership = new.iter().any(|e| e.content.app == space::APP);
            if membership {
                s.members = Members::replay(&space, &s.store.ordered()?);
                let members = &s.members;
                s.sessions.retain(|peer, _| members.current(peer).is_some());
            }
            s.builds.extend(release::builds(new));
            if membership || new.iter().any(|e| e.content.app == release::APP && e.content.kind == "provides") {
                s.provides = crate::providers::announced(&s.members, &s.store.ordered()?, &s.user);
            }
            let msg = sync::entries(space, new);
            for (peer, tx) in &s.sessions {
                if Some(*peer) != from {
                    let _ = tx.send(msg.clone());
                }
            }
            // Membership changes who counts, so every view is rebuilt. Otherwise an app is rebuilt only
            // if an entry arrived late (before something already replayed); the rest just replay what's new.
            let late: std::collections::BTreeSet<String> =
                new.iter().filter(|e| membership || s.replayed.is_some_and(|f| e.order() <= f)).map(|e| e.content.app.clone()).collect();
            let all = if membership || !late.is_empty() { for_apps(&s.members, &s.store.ordered()?) } else { Vec::new() };
            let fresh = for_apps(&s.members, &sorted(new));
            let mut apps: Vec<String> = new.iter().map(|e| e.content.app.clone()).collect();
            apps.sort();
            apps.dedup();
            let jobs: Vec<Replay> = if membership {
                self.0.apps_names.iter().map(|app| Replay { app: app.clone(), fresh: true, entries: all.iter().filter(|e| &e.app == app).cloned().collect() }).collect()
            } else {
                apps.iter()
                    .map(|app| match late.contains(app) {
                        true => Replay { app: app.clone(), fresh: true, entries: all.iter().filter(|e| &e.app == app).cloned().collect() },
                        false => Replay { app: app.clone(), fresh: false, entries: fresh.iter().filter(|e| &e.app == app).cloned().collect() },
                    })
                    .collect()
            };
            s.replayed = s.replayed.max(new.iter().map(Entry::order).max());
            (jobs, apps)
        };
        if new.iter().any(|e| e.content.app == release::APP) {
            self.consider_update(space);
        }
        if new.iter().any(|e| e.content.app == space::APP) {
            self.0.membership_changed.notify_one();
        }
        for native in &self.0.native {
            if native.sync.is_some() && new.iter().any(|e| e.content.app == native.name) {
                self.sync_native(space, native.name);
            }
        }
        self.0.apps.replay(space, jobs).await?;
        self.emit(Event::Changed { space: hex(&space), apps });
        Ok(())
    }

    async fn replay_all(&self, space: Key) -> Result<()> {
        let all = {
            let mut spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get_mut(&space).context("no such space")?;
            let entries = s.store.ordered()?;
            s.replayed = entries.iter().map(Entry::order).max();
            s.builds = release::builds(&entries);
            s.provides = crate::providers::announced(&s.members, &entries, &s.user);
            for_apps(&s.members, &entries)
        };
        let jobs = self.0.apps_names.iter().map(|app| Replay { app: app.clone(), fresh: true, entries: all.iter().filter(|e| &e.app == app).cloned().collect() }).collect();
        self.0.apps.replay(space, jobs).await
    }

    /// Who `device` is in `space`, as apps see them.
    pub fn me_of(&self, space: &Key, device: &Key) -> Result<Me> {
        let spaces = self.0.spaces.lock().unwrap();
        let s = spaces.get(space).context("no such space")?;
        ensure!(s.members.current(&self.0.id).is_some(), "this device is not a member of the space yet");
        let m = s.members.current(device).context("not a member of this space")?;
        let name = if *device == self.0.id { self.device_name() } else { m.name.clone() };
        Ok(Me { space: hex(space), device: hex(device), user: m.user.clone(), name })
    }

    /// Runs an app operation, appending whatever entries it asks for: here, or, for a device feature
    /// that only another of this user's devices provides, on that device.
    pub async fn call(&self, space: Key, app: &str, op: &str, input: Value) -> Result<Value> {
        let here = self.0.apps_names.iter().any(|a| a == app) || self.0.native.iter().any(|n| n.name == app) || self.0.providers.has(app);
        if !here {
            if let Some(device) = self.provider_of(space, app)? {
                return self.call_on(device, space, app, op, input).await;
            }
        }
        self.call_as(space, self.0.id, app, op, input).await
    }

    /// Another of this user's devices that provides `app`, preferring one that's online.
    fn provider_of(&self, space: Key, app: &str) -> Result<Option<Key>> {
        let spaces = self.0.spaces.lock().unwrap();
        let s = spaces.get(&space).context("no such space")?;
        let mut devices: Vec<Key> = s.provides.iter().filter(|(d, v)| *d != self.0.id && v["name"] == app && s.members.current(d).is_some()).map(|(d, _)| *d).collect();
        devices.sort_by_key(|d| !s.sessions.contains_key(d));
        Ok(devices.first().copied())
    }

    /// Runs an operation on this device for `caller`, another device in the space.
    pub async fn call_as(&self, space: Key, caller: Key, app: &str, op: &str, input: Value) -> Result<Value> {
        let me = self.me_of(&space, &caller)?;
        let native = self.0.native.iter().find(|n| n.name == app);
        if native.is_some() || self.0.providers.has(app) {
            ensure!(me.user == self.me_of(&space, &self.0.id)?.user, "{app} runs only for this device's own user's devices");
            let (result, appends) = match native {
                Some(n) => (n.call)(op.to_string(), input, me, if n.entries { self.app_entries(space, app)? } else { vec![] }).await?,
                None => self.0.providers.call(app, op, input, me).await?,
            };
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
            return self.call_as(space, self.0.id, app, op, input).await;
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

    /// An app screen as a whole page: the app's HTML with the design kit, the bridge, and who's looking.
    pub async fn screen(&self, space: Key, viewer: Key, app: &str, name: &str, params: Value) -> Result<String> {
        let body = self.0.apps.screen(app, name).await?;
        let me = self.me_of(&space, &viewer)?;
        let context = json!({ "app": app, "screen": name, "params": params, "space": hex(&space), "me": me });
        // `</` can't appear inside the inline script, so it can't end it early.
        let context = context.to_string().replace("</", "<\\/");
        Ok(format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
             <style>{kit}:root{{--accent:{accent}}}</style><script>window.__one={context};{bridge}</script></head><body>{body}</body></html>",
            kit = include_str!("screen/kit.css"),
            accent = crate::brand::ACCENT,
            bridge = include_str!("screen/bridge.js"),
        ))
    }

    pub async fn apps(&self) -> Result<Vec<Value>> {
        let mut apps = self.0.apps.describe().await?;
        apps.extend(self.0.native.iter().map(|n| (n.describe)()));
        apps.extend(self.0.providers.describe());
        let spaces = self.0.spaces.lock().unwrap();
        for s in spaces.values() {
            for (_, describe) in s.provides.clone() {
                if !apps.iter().any(|a| a["name"] == describe["name"]) {
                    apps.push(describe);
                }
            }
        }
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
                let builds = &s.builds;
                let devices: Vec<Value> = s
                    .members
                    .current_devices()
                    .map(|m| {
                        json!({
                            "device": hex(&m.device), "name": m.name, "user": m.user, "role": m.role, "client": m.client,
                            "build": builds.get(&m.device).map(|r| &r.commit), "target": builds.get(&m.device).map(|r| &r.target),
                            "me": m.device == viewer, "host": m.device == self.0.id,
                            "online": m.device == self.0.id || m.device == viewer || s.sessions.contains_key(&m.device),
                        })
                    })
                    .collect();
                json!({
                    "id": hex(key), "name": s.members.name, "user": s.user,
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
            if !self.is_member_anywhere(&caller) {
                return Ok(());
            }
            let node = self.clone();
            tokio::spawn(async move {
                let result = async {
                    let request: Value = serde_json::from_slice(&recv.read_to_end(16 << 20).await?)?;
                    let (method, path) = (request["method"].as_str().unwrap_or("GET"), request["path"].as_str().unwrap_or(""));
                    if path == "/v1/events" {
                        let mut events = node.subscribe();
                        while let Ok(event) = events.recv().await {
                            if !node.is_member_anywhere(&caller) {
                                break;
                            }
                            let space = match &event {
                                Event::Changed { space, .. } | Event::Peers { space } => Some(space.clone()),
                                Event::Spaces => None,
                            };
                            if space.is_some_and(|s| node.state_for(caller)["spaces"].as_array().is_none_or(|l| !l.iter().any(|x| x["id"] == s))) {
                                continue;
                            }
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
            ("GET", "/v1/screen") => {
                let (app, name) = (body["app"].as_str().unwrap_or(""), body["screen"].as_str().unwrap_or("main"));
                json!({ "html": self.screen(space(&body)?, caller, app, name, body["params"].clone()).await? })
            }
            ("POST", "/v1/call") => {
                let input = if body["input"].is_null() { json!({}) } else { body["input"].clone() };
                let ok = self.call_as(space(&body)?, caller, body["app"].as_str().unwrap_or(""), body["op"].as_str().unwrap_or(""), input).await?;
                json!({ "ok": ok })
            }
            ("POST", "/v1/device") => {
                self.rename_client(caller, body["name"].as_str().unwrap_or("").trim()).await?;
                json!({})
            }
            ("POST", "/v1/pair") => {
                let key = space(&body)?;
                json!({ "code": self.pair_as(key, caller, body["new_user"] == true)? })
            }
            _ => bail!("no such request: {method} {path}"),
        })
    }

    pub async fn rename_device(&self, name: &str) -> Result<()> {
        *self.0.name.lock().unwrap() = name.to_string();
        let path = self.0.dir.join("device.json");
        let mut file: DeviceFile = serde_json::from_slice(&std::fs::read(&path)?)?;
        file.name = name.to_string();
        write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
        for space in self.spaces() {
            self.append(space, space::rename(&self.0.id, name)).await?;
        }
        Ok(())
    }

    /// Renames a client in every space where this device is an admin: a client writes no entries, and SPEC.md lets only it or an admin rename it.
    async fn rename_client(&self, client: Key, name: &str) -> Result<()> {
        ensure!(!name.is_empty(), "a device needs a name");
        let spaces: Vec<Key> = {
            let spaces = self.0.spaces.lock().unwrap();
            spaces
                .iter()
                .filter(|(_, s)| s.members.current(&client).is_some() && s.members.current(&self.0.id).is_some_and(|m| m.role == Role::Admin))
                .map(|(key, _)| *key)
                .collect()
        };
        ensure!(!spaces.is_empty(), "only an admin device can rename a phone, and {} isn't one", self.device_name());
        for space in spaces {
            self.append(space, space::rename(&client, name)).await?;
        }
        Ok(())
    }

    /// Makes a one-time pairing code another device joins `space` with.
    pub fn pair(&self, space: Key, new_user: bool) -> Result<String> {
        self.pair_as(space, self.0.id, new_user)
    }

    /// A pairing code asked for by `asker` (this device, or a phone it serves): for another of the
    /// asker's user's devices, or, if `new_user`, for someone new, which only admins may add.
    fn pair_as(&self, space: Key, asker: Key, new_user: bool) -> Result<String> {
        let mine = !new_user;
        let user = {
            let spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get(&space).context("no such space")?;
            let me = s.members.current(&self.0.id).context("this device is not a member")?;
            let theirs = s.members.current(&asker).context("not a member")?.user.clone();
            ensure!(mine || (me.role == Role::Admin && asker == self.0.id), "only admins add new users; pair one of your own devices instead");
            ensure!(!mine || me.role == Role::Admin || me.user == theirs, "this device can't admit that user's devices; ask one of theirs, or an admin's");
            mine.then_some(theirs)
        };
        let secret: [u8; 16] = rand::random();
        self.add_pairing(secret, Pairing { space, user, until: now_ms() + PAIRING_LIFE })?;
        let code = PairingCode { space, from: self.0.endpoint.addr(), secret };
        let mut bytes = Vec::new();
        ciborium::into_writer(&code, &mut bytes)?;
        Ok(data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase())
    }

    /// Pairing codes are kept in a file, so one still works after the device that made it restarts.
    fn add_pairing(&self, secret: [u8; 16], pairing: Pairing) -> Result<()> {
        let mut pairings = self.0.pairings.lock().unwrap();
        pairings.retain(|_, p| p.until > now_ms());
        pairings.insert(secret, pairing);
        self.save_pairings(&pairings)
    }

    fn take_pairing(&self, secret: &[u8; 16]) -> Result<Option<Pairing>> {
        let mut pairings = self.0.pairings.lock().unwrap();
        let pairing = pairings.remove(secret);
        self.save_pairings(&pairings)?;
        Ok(pairing)
    }

    fn save_pairings(&self, pairings: &HashMap<[u8; 16], Pairing>) -> Result<()> {
        let list: Vec<(String, &Pairing)> = pairings.iter().map(|(k, v)| (data_encoding::HEXLOWER.encode(k), v)).collect();
        write_private(&self.0.dir.join("pairings.json"), &serde_json::to_vec(&list)?)?;
        Ok(())
    }

    pub async fn join(&self, code: &str) -> Result<Key> {
        let bytes = data_encoding::BASE32_NOPAD.decode(code.trim().to_uppercase().as_bytes()).context("that isn't a pairing code")?;
        let code: PairingCode = ciborium::from_reader(bytes.as_slice()).context("that isn't a pairing code")?;
        ensure!(!self.0.spaces.lock().unwrap().contains_key(&code.space), "this device is already in that space");
        let conn = self.0.endpoint.connect(code.from.clone(), JOIN_ALPN).await.context("couldn't reach the device that made the pairing code")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&serde_json::to_vec(&JoinRequest { secret: code.secret, name: self.device_name(), client: false })?).await?;
        send.finish()?;
        let reply: Value = serde_json::from_slice(&recv.read_to_end(1 << 16).await?)?;
        if let Some(err) = reply.get("error") {
            bail!("{}", err.as_str().unwrap_or("the pairing code was refused"));
        }
        let reply: JoinReply = serde_json::from_value(reply)?;
        self.save_space(&code.space, &SpaceFile { user: reply.user, paired_by: Some(hex(code.from.id.as_bytes())) })?;
        self.open_space(code.space)?;
        let paired_by = code.from.id;
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.connect_sync(code.space, *paired_by.as_bytes()).await {
                tracing::warn!("first sync after joining: {err:#}");
            }
        });
        self.emit(Event::Spaces);
        Ok(code.space)
    }

    async fn accept_join(&self, conn: Connection) -> Result<()> {
        let remote = *conn.remote_id().as_bytes();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let request: JoinRequest = serde_json::from_slice(&recv.read_to_end(1 << 16).await?)?;
        let pairing = self.take_pairing(&request.secret)?.filter(|p| p.until > now_ms());
        let Some(pairing) = pairing else {
            send.write_all(br#"{"error":"this pairing code was already used or has expired"}"#).await?;
            send.finish()?;
            return Ok(());
        };
        let user = pairing.user.unwrap_or_else(new_user);
        let admit = if request.client { space::admit_client(&remote, &user, &request.name) } else { space::admit(&remote, &user, &request.name, Role::Member) };
        self.append(pairing.space, admit).await?;
        if self.0.spaces.lock().unwrap()[&pairing.space].members.current(&remote).is_none() {
            send.write_all(br#"{"error":"this device can no longer admit that one"}"#).await?;
            send.finish()?;
            return Ok(());
        }
        let space_name = self.0.spaces.lock().unwrap()[&pairing.space].members.name.clone();
        send.write_all(&serde_json::to_vec(&JoinReply { space_name, user })?).await?;
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
                        let mut devices: Vec<Key> = s.members.current_devices().filter(|m| m.device != self.0.id && !m.client).map(|m| m.device).collect();
                        // Still joining: only the device that made the pairing code knows we're in.
                        if s.members.current(&self.0.id).is_none() {
                            devices.extend(s.paired_by);
                        }
                        devices.into_iter().filter(|d| !s.sessions.contains_key(d)).map(|d| (*space, d)).collect::<Vec<_>>()
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
                for reply in self.handle(space, peer, msg).await? {
                    tx.send(reply).map_err(|_| anyhow!("session closed"))?;
                }
            }
            loop {
                let msg = sync::read(&mut recv).await?;
                for reply in self.handle(space, peer, msg).await? {
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

    async fn handle(&self, space: Key, peer: Key, msg: Msg) -> Result<Vec<Msg>> {
        let _write = self.0.writes.lock().await;
        let (new, reply) = {
            let spaces = self.0.spaces.lock().unwrap();
            let s = spaces.get(&space).context("no such space")?;
            let joining = s.members.current(&self.0.id).is_none();
            ensure!(s.members.current(&peer).is_some() || (joining && s.paired_by == Some(peer)), "{} is no longer a member", short(&peer));
            sync::handle(&s.store, msg, now_ms())?
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
            [] if name.is_none() => bail!("this device is in no space yet: create one, or join with a pairing code"),
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
    /// Publishes `binary` (a build of this node for `target`, with the release key's `signature` over it) as the space's
    /// newest release.
    pub async fn publish(&self, space: Key, binary: &Path, commit: &str, target: &str, committed: u64, signature: &str) -> Result<Release> {
        let bytes = std::fs::read(binary).with_context(|| format!("read {}", binary.display()))?;
        release::verify(crate::brand::RELEASE_KEY, &bytes, signature).context("no node would take it")?;
        let blob = release::put_blob(&self.0.dir, &bytes)?;
        let r = Release { commit: commit.into(), target: target.into(), blob, size: bytes.len() as u64, committed, signature: signature.into() };
        self.append(space, release::content(&r)).await?;
        Ok(r)
    }

    /// Updates this node if a newer build for its platform was published, signed with the release key.
    fn consider_update(&self, space: Key) {
        let latest = {
            let spaces = self.0.spaces.lock().unwrap();
            let Some(s) = spaces.get(&space) else { return };
            match s.store.ordered() {
                Ok(entries) => release::latest(crate::brand::RELEASE_KEY, &entries, crate::brand::TARGET),
                Err(_) => return,
            }
        };
        let Some((r, author)) = latest else { return };
        let marker = self.0.dir.join("release");
        if !release::wanted(&r, crate::brand::BUILD, crate::brand::COMMITTED) || std::fs::read_to_string(&marker).is_ok_and(|m| m.trim() == r.blob) {
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
                Err(err) => tracing::warn!("ignoring the release of {}: {err:#}", r.commit),
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
                self.fetch_from(peers, &r.blob, r.size).await?
            }
        };
        release::verify(crate::brand::RELEASE_KEY, &bytes, &r.signature)?;
        release::install(r, &bytes)
    }

    /// Keeps `bytes` as a blob, which every member may then fetch. See SPEC.md, "Blobs".
    pub fn put_blob(&self, bytes: &[u8]) -> Result<String> {
        release::put_blob(&self.0.dir, bytes)
    }

    /// A blob from this device, else from any device online in one of its spaces.
    pub async fn blob(&self, hash: &str) -> Result<Vec<u8>> {
        if let Ok(bytes) = std::fs::read(release::blob_path(&self.0.dir, hash)?) {
            return Ok(bytes);
        }
        let mut peers: Vec<Key> = self.0.spaces.lock().unwrap().values().flat_map(|s| s.sessions.keys().copied()).collect();
        peers.sort();
        peers.dedup();
        self.fetch_from(peers, hash, 1 << 30).await
    }

    /// Fetches a blob from the first of `peers` that has it, and keeps it.
    async fn fetch_from(&self, peers: Vec<Key>, hash: &str, size: u64) -> Result<Vec<u8>> {
        let mut last = anyhow!("no device had it");
        for peer in peers {
            match self.fetch_blob(peer, hash, size).await {
                Ok(bytes) => {
                    release::put_blob(&self.0.dir, &bytes)?;
                    return Ok(bytes);
                }
                Err(err) => last = err,
            }
        }
        Err(last)
    }

    /// Every entry an app has in a space, by members, oldest first.
    fn app_entries(&self, space: Key, app: &str) -> Result<Vec<AppEntry>> {
        let spaces = self.0.spaces.lock().unwrap();
        let s = spaces.get(&space).context("no such space")?;
        Ok(for_apps(&s.members, &s.store.ordered()?).into_iter().filter(|e| e.app == app).collect())
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

    async fn fetch_blob(&self, peer: Key, hash: &str, size: u64) -> Result<Vec<u8>> {
        let conn = self.0.endpoint.connect(EndpointId::from_bytes(&peer)?, BLOB_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(hash.as_bytes()).await?;
        send.finish()?;
        let bytes = recv.read_to_end(size as usize).await?;
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

/// The entries apps see: by members, minus membership itself.
fn for_apps(members: &Members, entries: &[Entry]) -> Vec<AppEntry> {
    entries
        .iter()
        .filter(|e| e.content.app != space::APP && e.content.app != release::APP)
        .filter_map(|e| {
            let m = members.active(&e.author(), e.order())?;
            Some(AppEntry {
                app: e.content.app.clone(),
                kind: e.content.kind.clone(),
                author: hex(&e.author()),
                user: m.user.clone(),
                name: m.name_at(e.order()).to_string(),
                time: e.time(),
                id: format!("{}:{}", short(&e.author()), e.seq()),
                data: e.content.data.clone(),
            })
        })
        .collect()
}

/// Writes a file only this account can read: device keys, pairing secrets.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn load_pairings(dir: &Path) -> HashMap<[u8; 16], Pairing> {
    let list: Vec<(String, Pairing)> = std::fs::read(dir.join("pairings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    list.into_iter()
        .filter_map(|(k, v)| Some((data_encoding::HEXLOWER.decode(k.as_bytes()).ok()?.try_into().ok()?, v)))
        .collect()
}

fn new_user() -> String {
    data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>())
}

fn load_device(dir: &Path) -> Result<DeviceFile> {
    let path = dir.join("device.json");
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    let file = DeviceFile { secret: hex(&rand::random::<[u8; 32]>()), name: hostname() };
    write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
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
