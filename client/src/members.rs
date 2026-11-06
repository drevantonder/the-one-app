//! Who's who as the install's own signed entries say it, checked on the device with the nodes' own Rust (`one`'s
//! entry checks and member replay), never as a node says it: a node holds no keys, and a hacked one could list a seal
//! key of its own as someone's device, or itself as where their things run, to be sealed or handed keys. What a device
//! seals, and to whom it hands keys, it takes only from here.
//!
//! A device keeps `global`'s entries it has checked and the install's root, the genesis it saw when it joined, and
//! refuses entries of an install with another; then it asks a node only for entries it doesn't have
//! (`GET /v1/entries/global?have=`, `have`).
//!
//! What it asks of them it asks `global`'s scope database, built from that replay, and `may` over it
//! (`one::database::scopes::ScopeDb`, as nodes ask theirs): native apps directly, browsers as JSON (`view`). Replay
//! stays the truth, and this only its answers.

use one::config::YOURS;
use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};

pub use one::database::scopes::ScopeDb;
use one::database::steps;
use one::database::scopes::{INSTALL, device, object, person};
use one::entry::{Entry, Hash, Key, Scope, causal, hex, pack, unpack};
use one::members::{Cuts, Kind, Members, Role, ScopeState, is_member_entry};

/// `global`'s entries a device holds, checked, and the install they're of; and its scope database's steps
/// (`one::database::steps`) as far as it read them: once its log is frozen, the database its freeze anchors, checked
/// here once against the entries, and the steps after, each opened here with the device's own keys and applied only
/// if it counts. Never what a node says of a step: a node passes them on sealed.
pub struct Global {
    root: Option<Hash>,
    entries: BTreeMap<Hash, Entry>,
    truth: Option<steps::Truth>,
    /// Before it's frozen, the last step looked at, none of them its freeze.
    looked: u64,
    /// `global`'s log past where the freezes among its steps say it ends, read from its steps (`steps::log_to`): what
    /// each is judged and checked against.
    log: Option<steps::Log>,
}

impl Global {
    /// For the install whose genesis is `root`, or, for a device that's only now joined, whichever one genesis the
    /// first entries it's given hold.
    pub fn new(root: Option<Hash>) -> Global {
        Global { root, entries: BTreeMap::new(), truth: None, looked: 0, log: None }
    }

    /// Starts from what `kept` (`kept`'s, as a device keeps it) says: who's who once frozen, checked against the hash
    /// kept with it, or where it looked to before.
    pub fn restart(&mut self, kept: &Value) -> Result<()> {
        match kept.get("truth").filter(|t| !t.is_null()) {
            Some(t) => self.truth = Some(steps::Truth::kept(t, now())?),
            None => self.looked = kept["looked"].as_u64().unwrap_or(0),
        }
        Ok(())
    }

    /// Where it read to, to keep and start again from (`restart`).
    pub fn kept(&mut self) -> Result<Value> {
        Ok(json!({ "truth": self.truth.as_mut().map(steps::Truth::keep).transpose()?, "looked": self.looked }))
    }

    /// Takes step `at`, the next, as this device opened it (none for one that will never be a step): before the log is
    /// frozen, the freeze if the install's recovery key signed it (`steps::claimed`), judged and checked against
    /// `global`'s log past the end it names (`with_log`); after, applied if it counts. Anything else shaped like a
    /// freeze is passed over, its log never read. Fails at a freeze the recovery key signed that doesn't check out or
    /// whose log isn't here, so it's never passed over.
    pub fn take(&mut self, at: u64, given: Option<&steps::Given>) -> Result<()> {
        ensure!(at == self.at() + 1, "step {at} isn't the next");
        match &mut self.truth {
            Some(t) => drop(t.take(at, given.map(|g| &g.step))?),
            None => {
                let Some(root) = self.root else { bail!("no entries of {YOURS} yet") };
                match steps::Truth::first(at, given, Some(root), self.log.as_ref(), &self.recovery(), now()) {
                    Ok(t) => self.truth = t,
                    Err(err) => {
                        // Nothing where its mark would be when the log was read: read it again the next time.
                        if err.is::<steps::Unmarked>() {
                            self.log = None;
                        }
                        return Err(err);
                    }
                }
                self.looked = at;
            }
        }
        Ok(())
    }

    /// Of `read`, steps after those held, the step of `global`'s log this device reads to the step after, to judge the
    /// freezes among them (`steps::wants_log`), if it hasn't yet: read it (`steps::log_to`), and give it (`with_log`),
    /// before taking them. `steps::MOST_STEPS` for its end.
    pub fn wants_log(&self, read: &[Option<steps::Given>]) -> Result<Option<u64>> {
        if self.truth.is_some() {
            return Ok(None);
        }
        // A step shaped like a freeze: `global`'s log read to its end first, as a node reads it, so every key that was
        // ever the recovery key is known (`recovery`), whatever entries this device holds, and a freeze isn't passed
        // over for good because they're behind.
        if read.iter().flatten().any(|g| g.step.content.kind == steps::FREEZE) && self.log.as_ref().is_none_or(|l| !l.ended) {
            return Ok(Some(steps::MOST_STEPS));
        }
        // Placed where they are: the ones before were looked at already, and none was a freeze the recovery key signed.
        let mut placed: Vec<Option<steps::Given>> = vec![None; self.looked as usize];
        placed.extend(read.iter().cloned());
        Ok(steps::wants_log(&placed, self.root, self.log.as_ref(), &self.recovery()))
    }

    /// Every key the entries this device holds, and those in `global`'s log as read from its steps, say was ever the
    /// install's recovery key (`steps::recovery_keys`): only a freeze one of them signed is looked at.
    fn recovery(&self) -> steps::Recovery {
        let read = self.log.iter().flat_map(|l| l.entries(l.last()));
        steps::recovery_keys(self.root, self.entries.values().chain(read))
    }

    /// `global`'s log past the end a freeze names, read from its steps (`wants_log`).
    pub fn with_log(&mut self, log: steps::Log) {
        self.log = Some(log);
    }

    /// Takes steps read from the bucket with this device's own keys (`steps::read`), after those held, with `global`'s
    /// log past the end the freezes among them name (`wants_log`).
    pub fn add_read(&mut self, read: Vec<Option<steps::Given>>) -> Result<()> {
        for given in read {
            self.take(self.at() + 1, given.as_ref())?;
        }
        Ok(())
    }

    /// Reads `global`'s steps after those held from `bucket`, opened with `keys`, its log's keys by epoch, with its log
    /// as far as the freezes among them need (`wants_log`), and takes them: one under an epoch above every one made, as
    /// the steps before it leave it, is junk, passed over.
    pub async fn read_steps(&mut self, bucket: &impl one::bucket::Bucket, keys: &one::db::Keys) -> Result<()> {
        loop {
            let read = steps::read(bucket, keys, &Scope::Global, self.at()).await?;
            while let Some(end) = self.wants_log(&read.steps)? {
                self.with_log(steps::log_to(bucket, keys, self.root, end).await?);
            }
            self.add_read(read.steps.clone())?;
            if !read.junk_at_end(self.who()?.epoch(&Scope::Global)) {
                return Ok(());
            }
            self.take(self.at() + 1, None)?;
        }
    }

    /// Takes steps as this device read them from the bucket itself, sealed (`{ steps: [{ at, sealed }], log }`,
    /// base64url, through the URL signer's `get`, never a node), opened here with `epochs`, `global`'s epochs' keys
    /// sealed to this device (raw, by number), each used only once who's who as the steps before it leave it says it's
    /// that epoch's, and `apps`, the node's key in each epoch sealed under it (base64url, by number). One the bucket
    /// holds that doesn't open is junk, passed over as every reader passes it over; one whose bytes didn't come stops
    /// it, to be asked again. Stops at one under an epoch this device has no key for yet, and at a freeze a key that was
    /// ever the recovery key signed until `log`, `global`'s log's steps to the end it names (`{ to, steps }`), is given: → that end, to
    /// read them to, and give again. With `welcome` (`steps::Welcome`), before it's read to its freeze, it starts from
    /// there (`add_welcomed`): what it's given for that is checked against the Welcome's hash, wherever it came from.
    pub fn add_sealed(&mut self, given: &Value, epochs: &BTreeMap<u32, [u8; 32]>, apps: &BTreeMap<u32, Vec<u8>>) -> Result<Option<u64>> {
        if let Some(log) = given["entries"]["steps"].as_array() {
            self.add_log(log, epochs, apps)?;
        }
        let welcome = given["welcome"].as_str().and_then(steps::Welcome::parse).filter(|_| self.truth.is_none());
        let mut keys = one::db::Keys::new();
        let key = |keys: &mut one::db::Keys, who: &ScopeDb, e: u32| {
            if !keys.contains_key(&e)
                && let Some(epoch) = epochs.get(&e)
                && who.epoch_hash(&Scope::Global, e) == Some(one::keys::epoch_hash(epoch))
                && let Some(app) = apps.get(&e).and_then(|a| one::cloudflare::open_app_key(epoch, one::members::APP, a).ok())
            {
                keys.insert(e, one::cloudflare::database_key(&app, one::log::NAME));
            }
        };
        if self.truth.is_none()
            && let Some(log) = given["log"].as_object()
            && let Some(to) = log.get("to").and_then(Value::as_u64)
        {
            // Opened with the keys of every epoch the entries made, which its steps are sealed under; or, for a device
            // welcomed with no entries, every one sealed to it, each checked once who's who is worked out. Either way, a
            // step under an epoch the entries before it hadn't made is junk (`steps::Log::take`).
            let log_keys = match &welcome {
                Some(_) => welcome_keys(epochs, apps),
                None => {
                    let who = self.who()?;
                    let mut keys = one::db::Keys::new();
                    for e in who.epoch_hashes(&Scope::Global).into_keys() {
                        key(&mut keys, &who, e);
                    }
                    keys
                }
            };
            let sealed: BTreeMap<u64, &str> = log.get("steps").and_then(Value::as_array).into_iter().flatten().filter_map(|s| Some((s["at"].as_u64()?, s["sealed"].as_str()?))).collect();
            let mut read = steps::Log::new(self.root);
            // To its end (`wants_log`'s `MOST_STEPS`): those there, in order.
            let to = match to >= steps::MOST_STEPS {
                true => (0..).find(|n| !sealed.contains_key(&(n + 1))).unwrap_or(0),
                false => to,
            };
            for at in 1..=to + 1 {
                match sealed.get(&at).and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok()) {
                    Some(bytes) => read.take(&log_keys, &bytes)?,
                    None if at == to + 1 => read.ended = true,
                    None => bail!("global's log's step {at} didn't come"),
                }
            }
            self.log = Some(read);
        }
        // Welcomed: from the copy a node passed on with them.
        if let Some(welcome) = &welcome {
            match self.add_welcomed(welcome, given, epochs, apps) {
                Ok(Some(end)) => return Ok(Some(end)),
                Ok(None) => {}
                // One that doesn't check out is read from the freeze on.
                Err(_) => {}
            }
        }
        for s in given["steps"].as_array().into_iter().flatten() {
            let Some(at) = s["at"].as_u64().filter(|at| *at == self.at() + 1) else { continue };
            let Some(sealed) = s["sealed"].as_str().and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok()) else { break };
            let who = self.who()?;
            if let Ok(e) = one::keys::epoch_of(&sealed) {
                key(&mut keys, &who, e);
            }
            // Under an epoch above every one made, as the steps before it leave it: junk.
            let opened = match steps::open(&keys, who.epoch(&Scope::Global), &Scope::Global, at, &sealed) {
                steps::Opened::Step(g) => Some(g),
                steps::Opened::Junk => None,
                steps::Opened::Waits(_) => break,
            };
            if let Some(end) = self.wants_log(std::slice::from_ref(&opened))? {
                return Ok(Some(end));
            }
            self.take(at, opened.as_ref())?;
        }
        // Nothing at the next position, but a step after it: the one missing was written (`steps::missing`).
        let given_at = |n: u64| given["steps"].as_array().into_iter().flatten().find(|s| s["at"].as_u64() == Some(n));
        let next = self.at() + 1;
        if given_at(next).is_none()
            && let Some(sealed) = given_at(next + 1).and_then(|s| s["sealed"].as_str()).and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok())
            && let Ok(who) = self.who()
        {
            if let Ok(e) = one::keys::epoch_of(&sealed) {
                key(&mut keys, &who, e);
            }
            ensure!(!matches!(steps::open(&keys, u32::MAX, &Scope::Global, next + 1, &sealed), steps::Opened::Step(_)), "global's step {next} isn't in the bucket, though later ones were written: this device reads no further until it's back");
        }
        Ok(None)
    }

    /// `global`'s entries as its log's steps, read from the bucket itself (`[{ at, sealed }]`, base64url, from the
    /// first): for a device with none kept, whose first node can't give them until it's handed keys this device can't
    /// yet tell it may have (the Cloudflare node). Opened with every key of `global`'s sealed to it (`epochs`, `apps`),
    /// each then checked against the epoch hashes the entries say, or none is taken.
    fn add_log(&mut self, sealed: &[Value], epochs: &BTreeMap<u32, [u8; 32]>, apps: &BTreeMap<u32, Vec<u8>>) -> Result<()> {
        let keys = welcome_keys(epochs, apps);
        let sealed: BTreeMap<u64, Vec<u8>> = sealed.iter().filter_map(|s| Some((s["at"].as_u64()?, data_encoding::BASE64URL_NOPAD.decode(s["sealed"].as_str()?.as_bytes()).ok()?))).collect();
        let mut read = steps::Log::new(self.root);
        while let Some(bytes) = sealed.get(&(read.last() + 1)) {
            read.take(&keys, bytes)?;
        }
        let mut with = Global::new(self.root);
        with.add_entries(self.entries.values().cloned().chain(read.entries(read.last()).iter().cloned()).collect())?;
        let who = with.who()?;
        for (e, epoch) in epochs.iter().filter(|(e, _)| keys.contains_key(e)) {
            ensure!(who.epoch_hash(&Scope::Global, *e) == Some(one::keys::epoch_hash(epoch)), "a key for this didn't check out, so it wasn't used");
        }
        self.add_entries(read.entries(read.last()).to_vec())
    }

    /// Starts from `truth`, worked out from where the device that let this one in read who's who to
    /// (`steps::welcomed`, `steps::Welcome`), unless this device has read to its freeze itself.
    pub fn welcomed(&mut self, truth: steps::Truth) {
        if self.truth.is_none() {
            self.truth = Some(truth);
        }
    }

    /// `add_sealed`'s start for a device that hasn't read to its freeze, from its Welcome (`welcome`): `given`'s
    /// `freeze` and `start` (`{ at, sealed }`, its freeze's step and the copy before the Welcome's step, as a node passes
    /// them on with `?welcome=`) and its steps up to the Welcome's, opened with every key of `global`'s sealed to this
    /// device, each then checked against who's who as they make it. Fails if they don't come to the Welcome's hash, or
    /// a key isn't the one it says.
    fn add_welcomed(&mut self, welcome: &steps::Welcome, given: &Value, epochs: &BTreeMap<u32, [u8; 32]>, apps: &BTreeMap<u32, Vec<u8>>) -> Result<Option<u64>> {
        let (Some(freeze), true) = (given.get("freeze"), self.truth.is_none()) else { return Ok(None) };
        let keys = welcome_keys(epochs, apps);
        let made = keys.keys().next_back().copied().unwrap_or(0);
        let sealed = |v: &Value| v["sealed"].as_str().and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok());
        let opened = |at: u64, v: &Value| match sealed(v).map(|b| steps::open(&keys, made, &Scope::Global, at, &b)) {
            Some(steps::Opened::Step(g)) => Ok(Some(g)),
            Some(steps::Opened::Junk) => Ok(None),
            _ => bail!("global's step {at} isn't one this device opens"),
        };
        let Some(freeze) = opened(welcome.freeze, freeze)? else { bail!("the step it was welcomed with isn't a freeze") };
        let end = steps::Freeze::of(&freeze.step, welcome.freeze).context("the step it was welcomed with isn't a freeze")?.global();
        if self.log.as_ref().is_none_or(|l| !l.reaches(end)) {
            return Ok(Some(end));
        }
        let start = match given.get("start").filter(|s| s["at"].as_u64().is_some() && s["at"].as_u64() == welcome.copy()) {
            Some(s) => {
                let name = one::db::copy_name(&steps::prefix("global"), s["at"].as_u64().unwrap_or(0));
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
        let truth = steps::Truth::welcomed(welcome, &freeze, self.root, self.log.as_ref(), start.as_deref(), &after, now())?;
        for (e, epoch) in epochs {
            ensure!(truth.db.epoch_hash(&Scope::Global, *e) == Some(one::keys::epoch_hash(epoch)), "a key for this didn't check out, so it wasn't used");
        }
        self.truth = Some(truth);
        Ok(None)
    }

    /// The last step taken: what to ask a node for those after (`?from=`).
    pub fn at(&self) -> u64 {
        self.truth.as_ref().map_or(self.looked, |t| t.at)
    }

    /// Whether `global`'s log is frozen here: who's who is written as steps.
    pub fn frozen(&self) -> bool {
        self.truth.is_some()
    }

    /// Who's who: once the log is frozen, its scope database as its steps leave it; before, built from replay's
    /// members.
    pub fn who(&mut self) -> Result<ScopeDb> {
        match &mut self.truth {
            Some(t) => ScopeDb::open(steps::connection(&t.db.bytes()?)?, now()),
            None => who(&self.members()?),
        }
    }

    /// Each device that signed two entries at one seq, cut from the first.
    fn forks(&self) -> Cuts {
        steps::forks(&self.entries.values().cloned().collect::<Vec<_>>())
    }

    /// The install's genesis, once known.
    pub fn root(&self) -> Option<Hash> {
        self.root
    }

    /// Adds a pack's entries (`entry::pack`), each checked as from anyone: signature, body and content.
    pub fn add(&mut self, packed: &[u8]) -> Result<()> {
        self.add_entries(unpack(packed)?)
    }

    /// Adds entries already checked as from anyone (`Entry::decode`): refused, all of them, if one isn't of `global`,
    /// or starts another install than this one.
    pub fn add_entries(&mut self, entries: Vec<Entry>) -> Result<()> {
        for e in &entries {
            ensure!(e.scope() == Scope::Global, "a node gave an entry of {} as one of everyone's", e.log());
        }
        let geneses: Vec<Hash> = entries.iter().chain(self.entries.values()).filter(|e| e.content.app == one::members::APP && e.content.kind == "genesis").map(Entry::hash).collect();
        let root = match self.root {
            Some(root) => root,
            None => match geneses.as_slice() {
                [only, rest @ ..] if rest.iter().all(|g| g == only) => *only,
                [] => bail!("those aren't any install's entries: none starts it"),
                _ => bail!("those entries start more than one install"),
            },
        };
        ensure!(geneses.iter().all(|g| *g == root), "those entries are another install's, not {YOURS}'s");
        self.root = Some(root);
        for e in entries {
            self.entries.entry(e.hash()).or_insert(e);
        }
        Ok(())
    }

    /// Every entry held, packed, to keep.
    pub fn pack(&self) -> Vec<u8> {
        pack(&self.entries.values().collect::<Vec<_>>())
    }

    /// What to ask a node for only what's new: `<key>.<seq>,…`, the next seq of each author's log held.
    pub fn have(&self) -> String {
        let mut next: BTreeMap<Key, u32> = BTreeMap::new();
        for e in self.entries.values() {
            let n = next.entry(e.author()).or_default();
            *n = (*n).max(e.seq() + 1);
        }
        next.iter().map(|(k, n)| format!("{}.{n}", hex(k))).collect::<Vec<_>>().join(",")
    }

    /// The members, replayed as a node replays them: in causal order, each entry counting only as `one::members`
    /// says, a device that signed two entries at one seq cut from there and removed. Refused if they aren't the
    /// install's, or its genesis isn't among them.
    pub fn members(&self) -> Result<Members> {
        let Some(root) = self.root else { bail!("no entries of {YOURS} yet") };
        let forks = self.forks();
        let ordered: Vec<Entry> = causal(self.entries.values().cloned().collect(), &HashMap::new()).into_iter().filter(is_member_entry).collect();
        let m = Members::replay(Some(root), &ordered, &forks);
        ensure!(!m.users.is_empty(), "{YOURS}'s first entry isn't among those");
        Ok(m)
    }
}

/// `global`'s log's key in each of `epochs` sealed to this device (raw, by number), from `apps`, the node's key sealed
/// under each (`Global::add_sealed`): unchecked, so what they open is used only once who's who it comes to says
/// they're its.
fn welcome_keys(epochs: &BTreeMap<u32, [u8; 32]>, apps: &BTreeMap<u32, Vec<u8>>) -> one::db::Keys {
    let mut keys = one::db::Keys::new();
    for (e, epoch) in epochs {
        if let Some(app) = apps.get(e).and_then(|a| one::cloudflare::open_app_key(epoch, one::members::APP, a).ok()) {
            keys.insert(*e, one::cloudflare::database_key(&app, one::log::NAME));
        }
    }
    keys
}

/// `global`'s scope database, built from replay's members, as of now.
pub fn who(m: &Members) -> Result<ScopeDb> {
    ScopeDb::build(m, &Scope::Global, &ScopeState::default(), now())
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
/// - `sign_in_clients`: the OIDC clients the install's people sign in with, its setting (`sign_in_proof::CLIENTS`)
/// - `openers`: each scope's `{ key, seal }` its epochs are sealed to
/// - `runs_for`: each node's scopes whose calls it runs: the only nodes handed a scope's keys
/// - `epochs`: each scope's newest epoch `n` and every epoch's hash
/// - `people`: each person's `uses` and `starts` (scopes they open, and whose new epochs they start), `nodes` (those
///   that run their calls), `runs_until_chosen`, `approves` (whose new devices they let in), `invites` (someone new),
///   `removes_people`, `removes` (the devices they may take out), and `keeps`: their app schedules on a device still
///   listed (named so for older clients), `{ endpoint, scope, app, until, keys }` (each epoch's key hash), the only ones their browser renews
pub fn view(m: &Members) -> Result<Value> {
    view_of(&who(m)?)
}

/// `view`, of who's who as it is (`Global::who`).
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
        let keeps: Vec<Value> = schedules
            .iter()
            .filter(|((k, s, _), g)| g.by == *u && db.machine(k).is_some() && db.may(u, "keep", &object(s)))
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
                "invites": db.may(u, "invite", INSTALL),
                "removes_people": db.may(u, "remove", INSTALL),
                "removes": listed.iter().filter(|d| db.may(u, "remove", &device(&d.key))).map(|d| hex(&d.key)).collect::<Vec<_>>(),
                "keeps": keeps,
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

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use one::entry::{Content, Ext, Head};
    use one::keys::Endpoint;
    use one::members::{self, Role};

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";
    const CAROL: &str = "c000000000000003";
    const GROUP: &str = "d000000000000004";

    /// Signs entries of `global`, each after every entry before it.
    #[derive(Default)]
    struct Install {
        entries: Vec<Entry>,
        heads: HashMap<Key, Head>,
    }

    impl Install {
        fn write(&mut self, key: &SigningKey, user: &str, content: Content) {
            let me = key.verifying_key().to_bytes();
            let previous = self.entries.iter().filter(|e| e.author() != me).map(Entry::hash).collect();
            let e = Entry::create(key, Ext { log: "global".into(), time: 0, user: user.into(), epoch: 0, previous }, self.heads.get(&me).copied(), content);
            self.heads.insert(me, e.head());
            self.entries.push(e);
        }
    }

    /// What a browser seals to, hands keys and approves by is what replay says, of everyone, every scope and every
    /// device: an admin and a member, one taken out, a group, a device taken out, a Cloudflare node, someone who chose
    /// their things run nowhere else and someone who never chose. (`ScopeDb`'s own reads are checked in `one`.)
    #[test]
    fn a_browsers_answers_are_replays() {
        let key = |n: u8| SigningKey::from_bytes(&[n; 32]);
        let (mac, phone, boxed, laptop, bobs_box, carols, cloudflare_node) = (key(1), key(2), key(3), key(4), key(5), key(6), key(7));
        let at = |k: &SigningKey| Endpoint::of(k);
        let hash = |c: char| c.to_string().repeat(64);
        let mut i = Install::default();
        i.write(&mac, ANDRE, members::genesis("Home", ANDRE, "Andre", &at(&mac), "mac", &[Kind::Client], &[9; 32], &hash('e')));
        i.write(&mac, ANDRE, members::device(&at(&phone), ANDRE, "phone", &[Kind::Client], None));
        i.write(&mac, ANDRE, members::device(&at(&boxed), ANDRE, "box", &[Kind::Compute], None));
        i.write(&mac, ANDRE, members::device(&at(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Member))));
        i.write(&mac, ANDRE, members::device(&at(&carols), CAROL, "phone", &[Kind::Client], Some(("Carol", Role::Member))));
        i.write(&mac, ANDRE, members::setting("serverless", Some(&hex(&cloudflare_node.verifying_key().to_bytes()))));
        i.write(&laptop, BOB, members::device(&at(&bobs_box), BOB, "bob's box", &[Kind::Compute], None));
        i.write(&laptop, BOB, members::runs(&[]));
        i.write(&laptop, BOB, members::epoch(&Scope::User(BOB.into()), 1, &[laptop.verifying_key().to_bytes()], &hash('b')));
        i.write(&laptop, BOB, members::group(GROUP, "Band"));
        i.write(&laptop, BOB, members::invite(GROUP, ANDRE, Role::Member));
        i.write(&mac, ANDRE, members::accept(GROUP));
        i.write(&laptop, BOB, members::epoch(&Scope::Group(GROUP.into()), 1, &[laptop.verifying_key().to_bytes(), mac.verifying_key().to_bytes()], &hash('d')));
        i.write(&mac, ANDRE, members::remove_device(&phone.verifying_key().to_bytes(), None));
        i.write(&mac, ANDRE, members::remove_user(CAROL, &BTreeMap::from([(carols.verifying_key().to_bytes(), None)])));
        let stranger = key(8);
        i.write(&mac, ANDRE, members::refuse(&at(&stranger), "MacBook Pro"));
        let mut g = Global::new(None);
        g.add_entries(i.entries.clone()).unwrap();
        let m = g.members().unwrap();
        assert!(m.user(BOB).is_some() && m.removed_user(CAROL) && m.machine(&phone.verifying_key().to_bytes()).is_none(), "the install is as written");
        let nodes_: Vec<Key> = m.computes().map(|c| c.key).chain(m.cloudflare_node()).collect();

        // What a browser looks up is what replay says.
        let k = view(&m).unwrap();
        let names = |v: &Value| v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
        let people: Vec<&str> = m.users.values().filter(|u| !u.removed).map(|u| u.id.as_str()).collect();
        assert_eq!(k["people"].as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(), people, "no one taken out");
        for u in people {
            let a = &k["people"][u];
            let scopes = |f: &dyn Fn(&Scope) -> bool| m.scopes().iter().filter(|s| f(s)).map(Scope::to_string).collect::<Vec<_>>();
            assert_eq!(names(&a["uses"]), scopes(&|s| m.may_use(u, s)), "{u} opens");
            assert_eq!(names(&a["starts"]), scopes(&|s| m.role_in(u, s) == Some(Role::Admin)), "{u} starts epochs of");
            assert_eq!(names(&a["nodes"]), nodes_.iter().filter(|n| m.serves(n, u)).map(hex).collect::<Vec<_>>(), "{u}'s nodes");
            assert_eq!(names(&a["approves"]), vec![u.to_string()], "{u} lets in their own devices");
            let removes: Vec<String> = m.machines.keys().filter(|k| m.machine(k).is_some() && m.looks_after(u, k)).map(hex).collect();
            assert_eq!(names(&a["removes"]), removes, "{u} takes out");
            assert_eq!(a["invites"], json!(m.is_admin(u)), "{u} invites");
            assert_eq!(a["removes_people"], json!(m.is_admin(u)), "{u} takes people out");
        }
        let devices: Vec<String> = k["devices"].as_array().unwrap().iter().map(|d| d["key"].as_str().unwrap().to_string()).collect();
        assert_eq!(devices, m.machines.keys().filter(|k| m.machine(k).is_some()).chain(m.cloudflare_node().as_ref()).map(hex).collect::<Vec<_>>());
        // Never sealed anything again: a device taken out, one turned down before it was listed, and a removed person's.
        let mut removed = names(&k["removed"]);
        removed.sort();
        let mut out: Vec<String> = [&phone, &carols, &stranger].iter().map(|d| hex(&d.verifying_key().to_bytes())).collect();
        out.sort();
        assert_eq!(removed, out);
        for s in &m.scopes() {
            assert_eq!(k["epochs"][s.to_string()]["n"].as_u64().unwrap_or(0) as u32, m.epoch(s), "{s}'s epoch");
            for n in 1..=m.epoch(s) {
                assert_eq!(k["epochs"][s.to_string()]["hashes"][n.to_string()].as_str(), m.epoch_hash(s, n), "{s}'s epoch {n}");
            }
        }
    }

    /// A device whose entries are behind the log (it holds none of R1's handover to R2) reads `global`'s log to its end
    /// before it judges a step shaped like a freeze, so R2's freeze counts, never passed over for good.
    #[test]
    fn a_device_behind_the_log_still_takes_the_recovery_key_s_freeze() {
        let key = |n: u8| SigningKey::from_bytes(&[n; 32]);
        let (mac, r1, r2) = (key(1), key(9), key(8));
        let mut i = Install::default();
        i.write(&mac, ANDRE, members::genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &r1.verifying_key().to_bytes(), &"e".repeat(64)));
        i.write(&mac, ANDRE, members::gate());
        let held = i.entries.clone();
        i.write(&r1, ANDRE, members::node("recovery", json!({ "key": hex(&r2.verifying_key().to_bytes()) })));
        let root = i.entries[0].hash();
        let mut log = steps::Log::new(Some(root));
        for e in &i.entries {
            log.push(vec![e.clone()]);
        }
        let (content, copy) = steps::freeze(Some(root), &i.entries, log.last(), Default::default(), 0).unwrap();
        log.push_mark(steps::sign(&r2, &Scope::Global, log.last() + 1, ANDRE, 1, 0, content.clone()).unwrap());
        log.ended = true;
        let freeze = steps::Given { step: steps::sign(&r2, &Scope::Global, 1, ANDRE, 1, 0, content).unwrap(), copy: Some(copy) };
        let mut g = Global::new(Some(root));
        g.add_entries(held).unwrap();
        assert_eq!(g.wants_log(&[Some(freeze.clone())]).unwrap(), Some(steps::MOST_STEPS), "the log to its end first");
        g.with_log(log);
        assert_eq!(g.wants_log(&[Some(freeze.clone())]).unwrap(), None);
        g.take(1, Some(&freeze)).unwrap();
        assert!(g.frozen(), "R2's freeze counts");
    }

    /// A device with nothing kept reads `global`'s entries from its log's steps in the bucket itself, opened with the
    /// keys sealed to it, and takes none if a key isn't the one the entries made: a node that seals it another can't
    /// have it read a log of its own.
    #[test]
    fn a_new_device_reads_the_log_from_the_bucket_only_with_keys_the_entries_made() {
        let key = |n: u8| SigningKey::from_bytes(&[n; 32]);
        let (mac, laptop) = (key(1), key(2));
        let (epoch, app): ([u8; 32], [u8; 32]) = ([4; 32], [5; 32]);
        let mut i = Install::default();
        i.write(&mac, ANDRE, members::genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[9; 32], &one::keys::epoch_hash(&epoch)));
        i.write(&mac, ANDRE, members::device(&Endpoint::of(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Member))));
        let root = i.entries[0].hash();
        let keys: one::db::Keys = [(1, one::cloudflare::database_key(&app, one::log::NAME))].into();
        let log: Vec<Value> = i.entries.iter().enumerate().map(|(n, e)| {
            let name = one::db::step_name(&one::log::prefix("global"), n as u64 + 1);
            json!({ "at": n + 1, "sealed": data_encoding::BASE64URL_NOPAD.encode(&one::log::seal(&keys, &name, &pack(&[e])).unwrap()) })
        }).collect();
        let given = json!({ "entries": { "steps": log } });
        let apps = BTreeMap::from([(1, one::cloudflare::seal_app_key(&epoch, one::members::APP, &app))]);
        let mut g = Global::new(Some(root));
        g.add_sealed(&given, &BTreeMap::from([(1, epoch)]), &apps).unwrap();
        assert!(g.members().unwrap().user(BOB).is_some(), "who's who from the bucket");
        // Another epoch key, and the node's key sealed under it, opening a log of the node's own.
        let (other, mal) = ([6; 32], key(3));
        let mut theirs = Install::default();
        theirs.write(&mal, ANDRE, members::genesis("Home", ANDRE, "Andre", &Endpoint::of(&mal), "mac", &[Kind::Client], &[9; 32], &one::keys::epoch_hash(&other)));
        let keys: one::db::Keys = [(1, one::cloudflare::database_key(&app, one::log::NAME))].into();
        let name = one::db::step_name(&one::log::prefix("global"), 1);
        let forged = json!({ "entries": { "steps": [{ "at": 1, "sealed": data_encoding::BASE64URL_NOPAD.encode(&one::log::seal(&keys, &name, &pack(&[&theirs.entries[0]])).unwrap()) }] } });
        let apps = BTreeMap::from([(1, one::cloudflare::seal_app_key(&other, one::members::APP, &app))]);
        let mut g = Global::new(Some(root));
        assert!(g.add_sealed(&forged, &BTreeMap::from([(1, other)]), &apps).is_err(), "not a log of another install's");
        // The log's own steps, opened, but with an epoch key that isn't the one the genesis made.
        let mut g = Global::new(Some(root));
        assert!(g.add_sealed(&given, &BTreeMap::from([(1, other)]), &apps).is_err(), "nor with a key the entries didn't make");
        assert!(g.members().is_err(), "and none is taken");
    }

    /// A browser renews only the app schedules its own replay holds: ones a node lists, or signs for someone else's scope,
    /// to have their app's keys sealed to it, are never among them.
    #[test]
    fn only_replayed_keeps_are_renewed() {
        let key = |n: u8| SigningKey::from_bytes(&[n; 32]);
        let (mac, laptop, bobs_box, hacked) = (key(1), key(2), key(3), key(4));
        let at = |k: &SigningKey| Endpoint::of(k);
        let pk = |k: &SigningKey| k.verifying_key().to_bytes();
        let (bobs, soon) = (Scope::User(BOB.into()), u64::MAX / 2);
        let keys = BTreeMap::from([(1, "b".repeat(64))]);
        let mut i = Install::default();
        i.write(&mac, ANDRE, members::genesis("Home", ANDRE, "Andre", &at(&mac), "mac", &[Kind::Client], &[9; 32], &"e".repeat(64)));
        i.write(&mac, ANDRE, members::device(&at(&hacked), ANDRE, "box", &[Kind::Compute], None));
        i.write(&mac, ANDRE, members::device(&at(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Member))));
        i.write(&laptop, BOB, members::device(&at(&bobs_box), BOB, "bob's box", &[Kind::Compute], None));
        i.write(&laptop, BOB, members::app_schedules(&pk(&bobs_box), &bobs, "notes", soon, &keys));
        // The hacked node sets itself app schedules of Bob's notes, as Bob would.
        i.write(&hacked, BOB, members::app_schedules(&pk(&hacked), &bobs, "notes", soon, &keys));
        let mut g = Global::new(None);
        g.add_entries(i.entries).unwrap();
        let k = view(&g.members().unwrap()).unwrap();
        let endpoints = |u: &str| k["people"][u]["keeps"].as_array().unwrap().iter().map(|g| g["endpoint"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(endpoints(BOB), vec![hex(&pk(&bobs_box))], "Bob renews only the app schedules he signed");
        assert!(endpoints(ANDRE).is_empty(), "nor does anyone else renew one");
    }
}
