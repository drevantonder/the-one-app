//! Who's who as the network's own signed steps say it, checked on the device with the nodes' own Rust
//! (`one::database::steps`), never as a node says it: a node holds no keys, and a hacked one could list a seal key of
//! its own as someone's device, or itself as where their things run, to be sealed or handed keys. What a device seals,
//! and to whom it hands keys, it takes only from here.
//!
//! A device starts from the freeze the network's recovery key signed, which the genesis it joined names, its copy
//! checked against its hash, or from where the device that let it in had read to (its Welcome), and applies each step
//! after itself, opened with its own keys. What it asks of who's who it asks `global`'s scope database and `may` over
//! it (`one::database::scopes::ScopeDb`, as nodes ask theirs): native apps directly, browsers as JSON (`view`).

use one::config::YOURS;
use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};

pub use one::database::scopes::ScopeDb;
use one::database::steps;
use one::database::scopes::{NETWORK, device, object, person};
use one::entry::{Hash, Key, Scope, hex};
use one::members::{Kind, Members, Role, ScopeState};

/// `global`'s scope database's steps as far as this device read them (`one::database::steps`): the database the
/// recovery key's freeze anchors, and the steps after, each opened here with the device's own keys and applied only if
/// it counts. Never what a node says of a step: a node passes them on sealed.
pub struct Global {
    root: Option<Hash>,
    /// The recovery key the genesis names, once read (`steps::recovery_of`).
    recovery: Option<Key>,
    truth: Option<steps::Truth>,
    /// Before it's read the freeze, the last step looked at, none of them its freeze.
    looked: u64,
}

impl Global {
    /// For the network whose genesis is `root`, or, for a device that's only now joined, the one genesis its log's
    /// first step holds.
    pub fn new(root: Option<Hash>) -> Global {
        Global { root, recovery: None, truth: None, looked: 0 }
    }

    /// Starts from what `kept` (`kept`'s, as a device keeps it) says: who's who, checked against the hash kept with it,
    /// or where it looked to before, and the recovery key its genesis names.
    pub fn restart(&mut self, kept: &Value) -> Result<()> {
        match kept.get("truth").filter(|t| !t.is_null()) {
            Some(t) => self.truth = Some(steps::Truth::kept(t, now())?),
            None => self.looked = kept["looked"].as_u64().unwrap_or(0),
        }
        self.recovery = kept["recovery"].as_str().and_then(|k| one::entry::unhex(k).ok());
        Ok(())
    }

    /// Where it read to, to keep and start again from (`restart`).
    pub fn kept(&mut self) -> Result<Value> {
        Ok(json!({ "truth": self.truth.as_mut().map(steps::Truth::keep).transpose()?, "looked": self.looked, "recovery": self.recovery.as_ref().map(hex) }))
    }

    /// Takes step `at`, the next, as this device opened it (none for one that will never be a step): before it's read
    /// the freeze, the freeze if the network's recovery key signed it; after, applied if it counts. Fails at a freeze the
    /// recovery key signed that doesn't check out, so it's never passed over.
    pub fn take(&mut self, at: u64, given: Option<&steps::Given>) -> Result<()> {
        ensure!(at == self.at() + 1, "step {at} isn't the next");
        match &mut self.truth {
            Some(t) => drop(t.take(at, given.map(|g| &g.step))?),
            None => {
                let recovery = self.recovery.context("this device hasn't read its network's genesis yet")?;
                self.truth = steps::Truth::first(at, given, recovery, now())?;
                self.looked = at;
            }
        }
        Ok(())
    }

    /// Whether it needs `global`'s log's first step, its genesis, to judge the steps it reads: before it's read the
    /// freeze, while it doesn't know the recovery key.
    pub fn wants_genesis(&self) -> bool {
        self.truth.is_none() && self.recovery.is_none()
    }

    /// The genesis among the entries of `global`'s log's first step, and the recovery key it names: of `root`, or for
    /// a device that pins none, the one there.
    fn genesis(&mut self, entries: &[one::entry::Entry]) -> Result<()> {
        let root = steps::genesis_of(self.root, entries).with_context(|| format!("{YOURS}'s genesis isn't the first of its log's entries"))?;
        self.recovery = Some(steps::recovery_of(root, entries).with_context(|| format!("this isn't the {YOURS} this device joined"))?);
        self.root = Some(root);
        Ok(())
    }

    /// Reads `global`'s steps after those held from `bucket`, opened with `keys`, its log's keys by epoch, and takes
    /// them, its genesis first if it needs it: one under an epoch above every one made, as the steps before it leave
    /// it, is junk, passed over.
    pub async fn read_steps(&mut self, bucket: &impl one::bucket::Bucket, keys: &one::database::stored::Keys) -> Result<()> {
        if self.wants_genesis() {
            let (root, recovery) = steps::recovery_key(bucket, keys, self.root).await?;
            (self.root, self.recovery) = (Some(root), Some(recovery));
        }
        loop {
            let read = steps::read(bucket, keys, &Scope::Global, self.at()).await?;
            for given in &read.steps {
                self.take(self.at() + 1, given.as_ref())?;
            }
            if !read.junk_at_end(self.made()) {
                return Ok(());
            }
            self.take(self.at() + 1, None)?;
        }
    }

    /// The newest epoch of `global` made, as the steps read leave it: any, before it's read the freeze.
    fn made(&self) -> u32 {
        self.truth.as_ref().map_or(u32::MAX, |t| t.db.epoch(&Scope::Global))
    }

    /// Takes steps as this device read them from the bucket itself, sealed (`{ steps: [{ at, sealed }], log }`,
    /// base64url, through the URL signer's `get`, never a node), opened here with `epochs`, `global`'s epochs' keys
    /// sealed to this device (raw, by number), each used once it's read the freeze only if who's who says it's that
    /// epoch's, and `apps`, the node's key in each epoch sealed under it (base64url, by number). One the bucket holds
    /// that doesn't open is junk, passed over as every reader passes it over; one whose bytes didn't come stops it, to
    /// be asked again. Stops at one under an epoch this device has no key for yet. Before it knows the recovery key, it
    /// takes its genesis from `log` or `entries` (`{ steps: [{ at, sealed }] }`, `global`'s log's first step); without
    /// it, → 1, the step of the log to read and give again. With `welcome` (`steps::Welcome`), before it's read the
    /// freeze, it starts from there (`add_welcomed`): what it's given for that is checked against the Welcome's hash.
    pub fn add_sealed(&mut self, given: &Value, epochs: &BTreeMap<u32, [u8; 32]>, apps: &BTreeMap<u32, Vec<u8>>) -> Result<Option<u64>> {
        let welcome = given["welcome"].as_str().and_then(steps::Welcome::parse).filter(|_| self.truth.is_none());
        // One that doesn't check out is read from the freeze on.
        if let Some(welcome) = &welcome {
            let _ = self.add_welcomed(welcome, given, epochs, apps);
        }
        if self.wants_genesis() {
            let first = [&given["log"]["steps"], &given["entries"]["steps"]].into_iter().flat_map(|s| s.as_array().into_iter().flatten()).find(|s| s["at"].as_u64() == Some(1)).cloned();
            let Some(bytes) = first.and_then(|s| data_encoding::BASE64URL_NOPAD.decode(s["sealed"].as_str()?.as_bytes()).ok()) else { return Ok(Some(1)) };
            self.genesis(&steps::genesis_entries(&all_keys(epochs, apps), &bytes)?)?;
        }
        let mut keys = one::database::stored::Keys::new();
        let key = |keys: &mut one::database::stored::Keys, g: &Global, e: u32| {
            let checked = |epoch: &[u8; 32]| g.truth.as_ref().is_none_or(|t| t.db.epoch_hash(&Scope::Global, e) == Some(one::keys::epoch_hash(epoch)));
            if !keys.contains_key(&e)
                && let Some(epoch) = epochs.get(&e).filter(|k| checked(k))
                && let Some(app) = apps.get(&e).and_then(|a| one::cloudflare::open_app_key(epoch, one::members::APP, a).ok())
            {
                keys.insert(e, one::cloudflare::database_key(&app, steps::KEY_NAME));
            }
        };
        let sealed_at = |n: u64| given["steps"].as_array().into_iter().flatten().find(|s| s["at"].as_u64() == Some(n)).and_then(|s| data_encoding::BASE64URL_NOPAD.decode(s["sealed"].as_str()?.as_bytes()).ok());
        while let Some(sealed) = sealed_at(self.at() + 1) {
            let at = self.at() + 1;
            if let Ok(e) = one::keys::epoch_of(&sealed) {
                key(&mut keys, self, e);
            }
            // Under an epoch above every one made, as the steps before it leave it, or before it's read the freeze, above
            // every one sealed to it: junk.
            let made = self.truth.as_ref().map_or_else(|| epochs.keys().next_back().copied().unwrap_or(0), |t| t.db.epoch(&Scope::Global));
            let opened = match steps::open(&keys, made, &Scope::Global, at, &sealed) {
                steps::Opened::Step(g) => Some(g),
                steps::Opened::Junk => None,
                steps::Opened::Waits(_) => break,
            };
            self.take(at, opened.as_ref())?;
        }
        // Nothing at the next position, but a step after it: the one missing was written (`steps::missing`).
        let next = self.at() + 1;
        if sealed_at(next).is_none()
            && let Some(sealed) = sealed_at(next + 1)
        {
            if let Ok(e) = one::keys::epoch_of(&sealed) {
                key(&mut keys, self, e);
            }
            ensure!(!matches!(steps::open(&keys, u32::MAX, &Scope::Global, next + 1, &sealed), steps::Opened::Step(_)), "global's step {next} isn't in the bucket, though later ones were written: this device reads no further until it's back");
        }
        Ok(None)
    }

    /// Starts from `truth`, worked out from where the device that let this one in read who's who to
    /// (`steps::welcomed`, `steps::Welcome`), unless this device has read its freeze itself.
    pub fn welcomed(&mut self, truth: steps::Truth) {
        if self.truth.is_none() {
            self.truth = Some(truth);
        }
    }

    /// `add_sealed`'s start for a device that hasn't read its freeze, from its Welcome (`welcome`): `given`'s `freeze`
    /// and `start` (`{ at, sealed }`, its freeze's step and the copy before the Welcome's step, as a node passes them on
    /// with `?welcome=`) and its steps up to the Welcome's, opened with every key of `global`'s sealed to this device,
    /// each then checked against who's who as they make it. Fails if they don't come to the Welcome's hash, or a key
    /// isn't the one it says.
    fn add_welcomed(&mut self, welcome: &steps::Welcome, given: &Value, epochs: &BTreeMap<u32, [u8; 32]>, apps: &BTreeMap<u32, Vec<u8>>) -> Result<()> {
        let Some(freeze) = given.get("freeze") else { return Ok(()) };
        let keys = all_keys(epochs, apps);
        let made = keys.keys().next_back().copied().unwrap_or(0);
        let sealed = |v: &Value| v["sealed"].as_str().and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok());
        let opened = |at: u64, v: &Value| match sealed(v).map(|b| steps::open(&keys, made, &Scope::Global, at, &b)) {
            Some(steps::Opened::Step(g)) => Ok(Some(g)),
            Some(steps::Opened::Junk) => Ok(None),
            _ => bail!("global's step {at} isn't one this device opens"),
        };
        let Some(freeze) = opened(welcome.freeze, freeze)? else { bail!("the step it was welcomed with isn't a freeze") };
        let start = match given.get("start").filter(|s| s["at"].as_u64().is_some() && s["at"].as_u64() == welcome.copy()) {
            Some(s) => {
                let name = one::database::stored::copy_name(&steps::prefix("global"), s["at"].as_u64().unwrap_or(0));
                let bytes = sealed(s).ok_or_else(|| anyhow::anyhow!("no copy"))?;
                let key = keys.get(&one::keys::epoch_of(&bytes)?).ok_or_else(|| anyhow::anyhow!("its copy is under a key this device wasn't given"))?;
                Some(one::keys::open(key, &name, &bytes)?)
            }
            None => None,
        };
        let from = start.as_ref().and(welcome.copy()).unwrap_or(welcome.freeze);
        let mut after = Vec::new();
        for at in from + 1..=welcome.at {
            let s = given["steps"].as_array().into_iter().flatten().find(|s| s["at"].as_u64() == Some(at)).ok_or_else(|| anyhow::anyhow!("global's step {at} wasn't given"))?;
            after.push(opened(at, s)?);
        }
        let truth = steps::Truth::welcomed(welcome, &freeze, start.as_deref(), &after, now())?;
        for (e, epoch) in epochs {
            ensure!(truth.db.epoch_hash(&Scope::Global, *e) == Some(one::keys::epoch_hash(epoch)), "a key for this didn't check out, so it wasn't used");
        }
        self.truth = Some(truth);
        Ok(())
    }

    /// The last step taken: what to ask a node for those after (`?from=`).
    pub fn at(&self) -> u64 {
        self.truth.as_ref().map_or(self.looked, |t| t.at)
    }

    /// Whether it has read who's who.
    pub fn frozen(&self) -> bool {
        self.truth.is_some()
    }

    /// Who's who: its scope database as its steps leave it; one of nobody before it's read the freeze.
    pub fn who(&mut self) -> Result<ScopeDb> {
        match &mut self.truth {
            Some(t) => ScopeDb::open(steps::connection(&t.db.bytes()?)?, now()),
            None => ScopeDb::build(&Members::default(), &Scope::Global, &ScopeState::default(), now()),
        }
    }

    /// The network's genesis, once known.
    pub fn root(&self) -> Option<Hash> {
        self.root
    }
}

/// `global`'s log's key in each of `epochs` sealed to this device (raw, by number), from `apps`, the node's key sealed
/// under each: unchecked, so what they open is used only once who's who it comes to says they're its, or its hash pins it.
fn all_keys(epochs: &BTreeMap<u32, [u8; 32]>, apps: &BTreeMap<u32, Vec<u8>>) -> one::database::stored::Keys {
    let mut keys = one::database::stored::Keys::new();
    for (e, epoch) in epochs {
        if let Some(app) = apps.get(e).and_then(|a| one::cloudflare::open_app_key(epoch, one::members::APP, a).ok()) {
            keys.insert(*e, one::cloudflare::database_key(&app, steps::KEY_NAME));
        }
    }
    keys
}

fn now() -> u64 {
    n0_future::time::SystemTime::now().duration_since(n0_future::time::SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// What a browser seals, hands and signs by, as JSON for it to keep and look up (`cloudflare/web/device.js`,
/// `net-worker.js`), each answer worked out here:
///
/// - `root`, `name`, `recovery` (hex, what each epoch is sealed to besides devices)
/// - `devices`: each listed now, `{ key, seal, user, name, roles, passkey, device }`, and the Cloudflare node as
///   `{ key, seal, roles: ["compute"] }`
/// - `removed`: the keys taken out, or turned down before they were listed, which are never sealed anything again
/// - `sign_in_clients`: the OIDC clients the network's people sign in with, its setting (`sign_in_proof::CLIENTS`)
/// - `openers`: each scope's `{ key, seal }` its epochs are sealed to
/// - `runs_for`: each node's scopes whose calls it runs: the only nodes handed a scope's keys
/// - `epochs`: each scope's newest epoch `n` and every epoch's hash
/// - `people`: each person's `uses` and `starts` (scopes they open, and whose new epochs they start), `nodes` (those
///   that run their calls), `runs_until_chosen`, `approves` (whose new devices they let in), `invites` (someone new),
///   `removes_people`, `removes` (the devices they may take out), and `app_schedules`: theirs on a device still
///   listed, `{ endpoint, scope, app, until, keys }` (each epoch's key hash), the only ones their browser renews
///
/// Of who's who as it is (`Global::who`).
pub fn view_of(db: &ScopeDb) -> Result<Value> {
    let scopes = db.scopes();
    let listed = db.machines().into_iter().filter(|d| !d.removed).collect::<Vec<_>>();
    let mut devices: Vec<Value> = listed.iter().map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal), "user": d.user, "name": d.name, "roles": d.roles, "passkey": d.passkey, "device": d.device.as_ref().map(hex), "since": d.since, "place": d.place })).collect();
    let nodes = db.nodes();
    if let Some(k) = db.cloudflare_node()
        && let Some((_, seal)) = nodes.iter().find(|(n, _)| *n == k)
    {
        devices.push(json!({ "key": hex(&k), "seal": hex(seal), "roles": ["compute"] }));
    }
    let people: Vec<String> = db.users().into_iter().filter(|u| !u.removed).map(|u| u.id).collect();
    let uses = |u: &str| scopes.iter().filter(|s| db.may(u, "use", &object(s))).collect::<Vec<_>>();
    let uses: BTreeMap<&str, Vec<&Scope>> = people.iter().map(|u| (u.as_str(), uses(u))).collect();
    // A scope someone opens, with the client devices of everyone who does.
    let mut openers = Map::new();
    for s in scopes.iter().filter(|s| uses.values().any(|on| on.contains(s))) {
        let to: Vec<Value> = listed.iter().filter(|d| d.is(Kind::Client) && uses.get(d.user.as_str()).is_some_and(|on| on.contains(&s))).map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal) })).collect();
        openers.insert(s.to_string(), json!(to));
    }
    let runs_for: Map<String, Value> = nodes.iter().map(|(k, _)| (hex(k), json!(scopes.iter().filter(|s| db.runs_for(k, s)).map(Scope::to_string).collect::<Vec<_>>()))).collect();
    let mut epochs = Map::new();
    for (s, n) in db.epochs() {
        epochs.insert(s.to_string(), json!({ "n": n, "hashes": db.epoch_hashes(&s).into_iter().map(|(n, h)| (n.to_string(), json!(h))).collect::<Map<_, _>>() }));
    }
    let schedules = db.app_schedules();
    let mut answers = Map::new();
    for u in &people {
        let own: Vec<Value> = schedules
            .iter()
            .filter(|((k, s, _), g)| g.by == *u && db.machine(k).is_some() && db.may(u, "choose-devices", &object(s)))
            .map(|((k, s, a), g)| json!({ "endpoint": hex(k), "scope": s.to_string(), "app": a, "until": g.until, "keys": g.hashes }))
            .collect();
        answers.insert(
            u.clone(),
            json!({
                "uses": uses[u.as_str()].iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                "starts": scopes.iter().filter(|s| db.role_in(u, s) == Some(Role::Admin)).map(Scope::to_string).collect::<Vec<_>>(),
                "nodes": nodes.iter().filter(|(k, _)| db.serves(k, u)).map(|(k, _)| hex(k)).collect::<Vec<_>>(),
                "runs_until_chosen": db.runs_until_chosen(u).iter().map(hex).collect::<Vec<_>>(),
                "approves": people.iter().filter(|of| db.may(u, "add-devices", &person(of))).collect::<Vec<_>>(),
                "invites": db.may(u, "invite", NETWORK),
                "removes_people": db.may(u, "remove", NETWORK),
                "removes": listed.iter().filter(|d| db.may(u, "remove", &device(&d.key))).map(|d| hex(&d.key)).collect::<Vec<_>>(),
                "app_schedules": own,
                "sign_ins": db.sign_ins(u),
            }),
        );
    }
    Ok(json!({
        "root": db.root().map(|r| r.to_hex()), "name": db.name(), "recovery": db.recovery().as_ref().map(hex),
        "devices": devices, "openers": openers,
        "removed": db.machines().iter().filter(|d| d.removed).map(|d| hex(&d.key)).collect::<Vec<_>>(),
        "sign_in_clients": db.setting(one::sign_in_proof::CLIENTS), "runs_for": runs_for, "epochs": epochs, "people": answers,
    }))
}
