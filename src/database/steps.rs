//! A scope database's steps, once `global`'s log is frozen (`switch`): the database is the truth, and every change to
//! it is a step at the next free position of its own in the bucket, which the bucket keeps in one order.
//!
//! A step is one change, signed by the device that made it as its person: the same content an entry of the log carried
//! (`members`' kinds, and a scope's own), for one position, under `entry::STEP_DOMAIN`, so it's never taken for an
//! entry nor an entry for it. Its position is signed in it (its `seq`), so a step can't be written twice, or anywhere
//! but where its writer meant it: one that loses its position to another is signed again for the next. A node only
//! carries a client's step to the bucket: it can hold one back, but change none.
//!
//! The cut-over is `global`'s first step that is a freeze the network's recovery key signed (`FREEZE`, `claimed`), in
//! the one ceremony an admin runs with it (`one switch`, `Node::switch`): it names the step of `global`'s log where the
//! log ends (and of every other scope's log its last step in order), and carries the database there, its first copy.
//! The same freeze is written first as the step of the log after that end, its mark: the bucket takes a position once,
//! so a freeze names the log's real end as it was read just before, never an older one, and a recovery key replaced
//! since can't name one from before it was. Every reader replays exactly the entries in `global`'s log's steps up to
//! it (`Log`), forks only among those, so every reader replays the same ones whatever it holds or reads later; judges
//! the signer there and only there (the recovery key the genesis or a `recovery` entry up to there names), so it
//! judges the same whenever it reads it; and checks that the copy says what those entries add up to (`anchor`). Then
//! it keeps it: from then on it reads the copy it keeps and the steps after, never the entries again. A freeze the
//! recovery key signed that doesn't check out stops the reader there, as a newer release's step does, never passed
//! over for the next. Anything else shaped like a freeze is nothing, passed over before any log it names is read: a
//! reader looks only at those a key it knows was ever the recovery key signed (`recovery_keys`). A `FREEZING` step
//! the recovery key signed before it tells writers of the log to look whether what they wrote is inside it.
//! Every other scope's database starts, before its first step, from what its log's entries up to the freeze's position
//! there add up to (`scope_anchor`), which every reader of it works out itself: no writer chooses it.
//!
//! Every reader applies each step after to the database as the steps before it left it, and counts it only if all
//! three of the protocol's rules hold there:
//! 1. the network's first step and its recovery key: the freeze's copy, checked as above; and only the recovery key
//!    lets a device in by itself;
//! 2. epochs never go down: a step names its scope's epoch its writer knew, never one above the database's;
//! 3. its signer holds the permission the change needs, at that step (`allowed`): `may`, `may_give`, `may_take` and
//!    `may_make` over the database's rows; and is one of its person's own devices still listed, or for a scope's own
//!    state, a device that runs the scope's calls for them.
//!
//! A step that doesn't count is passed over, its position taken, as a database's are: nothing it says happens. What a
//! counted one changes is what replay's entry of the same content changed (`Members` and a scope's own state, read from
//! the database and built back into it), so the database a step leaves is the one replay would have made.
//!
//! Its steps are sealed as the scope's log was, with the key of the log in each epoch (`log::seal`), under their own
//! names, so whoever was handed the log's key reads them. A reader opens them itself: a node passes them on sealed.
//!
//! ```text
//! <prefix>/<n>               step n, from 1: one signed step, its header, signature and body, and a freeze's copy
//! <prefix>/copy/<n>          the database after step n, every `db::COPY_EVERY` steps
//! <prefix>/newest            the newest hint, as a database's
//! ```

use crate::config::YOURS;
use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::SigningKey;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::entry::{Content, Entry, Ext, Hash, Key, Scope, unhex};
use crate::members::{self, Cuts, Members, Principal, Role, ScopeState};
use crate::permissions;

use super::scopes::{self, ScopeDb};
use super::READS;

/// The node's database of a scope that its steps are: its `scope`.
pub const NAME: &str = "scope";

/// Where a scope's database's steps are in the bucket.
pub fn prefix(scope: &str) -> String {
    format!("{}/{}/{}", crate::bucket::folder(scope), members::APP, NAME)
}

/// A step of `scope`'s database for position `at`, signed by `key` as `user`, the scope at `epoch`.
pub fn sign(key: &SigningKey, scope: &Scope, at: u64, user: &str, epoch: u32, time: u64, content: Content) -> Result<Entry> {
    let at = u32::try_from(at).context("that database has more steps than a step can name")?;
    Ok(Entry::create_step(key, Ext { log: scope.to_string(), time, user: user.into(), epoch, previous: Vec::new() }, at, content))
}

/// A step as a reader has it: the step, and the copy a freeze carries.
#[derive(Clone)]
pub struct Given {
    pub step: Entry,
    pub copy: Option<Vec<u8>>,
}

/// A step as the bucket holds it, before it's sealed: its header, signature and body, and a freeze's copy, as CBOR.
pub fn encode(step: &Entry, copy: Option<&[u8]>) -> Vec<u8> {
    let (h, s, b) = step.encode();
    let mut parts = vec![ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b)];
    parts.extend(copy.map(|c| ciborium::Value::Bytes(c.to_vec())));
    let mut out = Vec::new();
    ciborium::into_writer(&ciborium::Value::Array(parts), &mut out).expect("cbor in memory");
    out
}

/// A step as `encode` wrote it, its signature checked, for position `at` of `scope`'s database.
pub fn decode(scope: &Scope, at: u64, bytes: &[u8]) -> Result<Given> {
    let ciborium::Value::Array(parts) = ciborium::from_reader(bytes)? else { bail!("not a step") };
    let (h, s, b, copy) = match parts.as_slice() {
        [ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b)] => (h, s, b, None),
        [ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b), ciborium::Value::Bytes(c)] => (h, s, b, Some(c.clone())),
        _ => bail!("not a step"),
    };
    let step = Entry::decode_step(h, s, b)?;
    ensure!(step.log() == scope.to_string() && u64::from(step.seq()) == at, "a step for another place");
    Ok(Given { step, copy })
}

/// A step a device of the network's own passed on opened (`encode`'s bytes): none if it isn't one. Only a service
/// endpoint takes them so, from its own device's node, holding no keys of its own.
pub fn given(scope: &Scope, at: u64, opened: Option<&[u8]>) -> Option<Given> {
    opened.and_then(|b| decode(scope, at, b).ok())
}

/// A step as `given` takes it.
pub fn encoded(given: &Given) -> Vec<u8> {
    encode(&given.step, given.copy.as_deref())
}

/// What a sealed step is to a reader with `keys`, its log's keys by epoch.
pub enum Opened {
    /// A step.
    Step(Given),
    /// Nothing that will ever be one: it doesn't open with its epoch's key, isn't a step for its place, or is under an
    /// epoch above every one made.
    Junk,
    /// Under an epoch that was made, whose key the reader doesn't have: it reads on from here once it does.
    Waits(u32),
}

/// Step `at` of `scope`'s database, sealed as the bucket holds it, opened with `keys`, under `made`, the newest epoch
/// of the scope made as the reader knows it: one above it is junk, since an epoch is always made before anything is
/// sealed under it, and a reader waits only for an epoch that was. Every reader judges it so.
pub fn open(keys: &crate::db::Keys, made: u32, scope: &Scope, at: u64, sealed: &[u8]) -> Opened {
    let name = crate::db::step_name(&prefix(&scope.to_string()), at);
    let Ok(epoch) = crate::keys::epoch_of(sealed) else { return Opened::Junk };
    let Some(key) = keys.get(&epoch) else { return if epoch > made { Opened::Junk } else { Opened::Waits(epoch) } };
    match crate::keys::open(key, &name, sealed).and_then(|b| decode(scope, at, &b)) {
        Ok(given) => Opened::Step(given),
        Err(err) => {
            tracing::debug!("{name}: {err:#}");
            Opened::Junk
        }
    }
}

/// What `read` read: the steps by position, none for one that will never be a step; and the epoch of the one it stopped
/// at, if it's under one it has no key for.
#[derive(Default)]
pub struct Read {
    pub steps: Vec<Option<Given>>,
    pub waits: Option<u32>,
}

impl Read {
    /// Whether the step it stopped at is junk: under an epoch above `made`, the newest of its scope made as the steps
    /// before it leave it (`open`), so its reader passes it over and reads on, rather than wait for it for good.
    pub fn junk_at_end(&self, made: u32) -> bool {
        self.waits.is_some_and(|e| e > made)
    }
}

/// Reads `scope`'s steps after `from` from the bucket, opened with the keys of its log by epoch: each by position, none
/// for one that will never be a step. Stops at the first position with nothing, or one under an epoch it has no key
/// for (`Read::waits`), which its reader takes once it's applied those before (`Read::junk_at_end`); fails at a
/// position with nothing that a later step says was written, rather than read on as if the steps ended there.
pub async fn read(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, scope: &Scope, from: u64) -> Result<Read> {
    let prefix = prefix(&scope.to_string());
    let mut out = Read::default();
    loop {
        let at = from + out.steps.len() as u64 + 1;
        let Some(o) = bucket.get(&crate::db::step_name(&prefix, at)).await? else {
            missing(bucket, keys, scope, at).await?;
            break;
        };
        match open(keys, u32::MAX, scope, at, &o.bytes) {
            Opened::Step(given) => out.steps.push(Some(given)),
            Opened::Junk => out.steps.push(None),
            Opened::Waits(epoch) => {
                out.waits = Some(epoch);
                break;
            }
        }
    }
    Ok(out)
}

/// Fails if step `at` of `scope`'s database, which isn't in the bucket, was written: the next is a step (it opens with
/// `keys` and is signed for its place). Steps are written one after another, so one gone is one deleted, and reading
/// on as if it never was would pass over whatever it and those after it changed. Anything else there is anyone's to
/// write (the URL signer creates a position only after the one before it, but an older one didn't), so it says
/// nothing: the steps end at `at`, and the next writer writes there. Nor does the newest hint.
pub async fn missing(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, scope: &Scope, at: u64) -> Result<()> {
    let later = match bucket.get(&crate::db::step_name(&prefix(&scope.to_string()), at + 1)).await? {
        Some(o) => matches!(open(keys, u32::MAX, scope, at + 1, &o.bytes), Opened::Step(_)),
        None => false,
    };
    ensure!(!later, "step {at} of {scope}'s database isn't in the bucket, though later ones were written: this device reads no further until it's back");
    Ok(())
}

/// The kind of step that freezes the log (`Truth::first`): `{ hash, logs }`, with its copy. The first the network's
/// recovery key signed is who's who from there; a network made since starts with it (`first_freeze`).
pub const FREEZE: &str = "freeze";

/// The furthest a freeze may say a log ends: 2⁵³, as far as a JSON number holds exactly, past any log's end.
pub const MOST_STEPS: u64 = 1 << 53;

/// Where the log was frozen, as its freeze says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Freeze {
    /// The freeze's position.
    pub at: u64,
    /// The state hash its copy comes to, as it is (`db::state_hash`).
    pub hash: String,
    /// The last step of every scope's log that counts, by scope: `global`'s among them.
    pub logs: BTreeMap<String, u64>,
}

impl Freeze {
    /// The freeze `step` at position `at` says.
    pub fn of(step: &Entry, at: u64) -> Result<Freeze> {
        ensure!(step.content.app == members::APP && step.content.kind == FREEZE && step.scope() == Scope::Global, "not a freeze");
        Freeze::from_data(&step.content.data, at)
    }

    fn from_data(data: &Value, at: u64) -> Result<Freeze> {
        let mut logs = BTreeMap::new();
        for (scope, n) in data["logs"].as_object().context("a freeze names every scope's log")? {
            Scope::parse(scope)?;
            let n = n.as_u64().context("a log's end is a position")?;
            ensure!(n <= MOST_STEPS, "a log's end is a position a log can reach");
            logs.insert(scope.clone(), n);
        }
        ensure!(logs.get("global").is_some_and(|n| *n > 0), "a freeze names where global's log ends");
        Ok(Freeze { at, hash: data["hash"].as_str().context("a freeze names its copy's hash")?.to_string(), logs })
    }

    /// As a freeze says it, and as a reader keeps it.
    pub fn content(&self) -> Content {
        Content::new(members::APP, FREEZE, json!({ "hash": self.hash, "logs": self.logs }))
    }

    /// The last step of `global`'s log it holds.
    pub fn global(&self) -> u64 {
        self.logs.get("global").copied().unwrap_or(0)
    }
}

/// `global`'s frozen log as the bucket's steps hold it, read only: a scope's database starts, before its first step,
/// from what its own log's entries add up to with it (`scope_anchor`). The entries in each of its steps from the first.
#[derive(Clone, Debug, Default)]
pub struct Log {
    read: crate::log::ReadTo,
    /// How many entries there are up to each step.
    ends: Vec<usize>,
    /// The network's genesis, which the entries before each step are replayed from (`take`).
    root: Option<Hash>,
    /// The newest epoch of `global` the entries in its steps so far make, as last worked out (`made`).
    made: u32,
}

impl Log {
    /// Of the network whose genesis is `root`, before its first step.
    pub fn new(root: Option<Hash>) -> Log {
        Log { root, ..Default::default() }
    }

    /// The last of its steps it holds.
    pub fn last(&self) -> u64 {
        self.ends.len() as u64
    }

    /// The entries in its steps 1 to `to`.
    pub fn entries(&self, to: u64) -> &[Entry] {
        let n = to.min(self.last()).checked_sub(1).map_or(0, |i| self.ends[i as usize]);
        &self.read.entries[..n]
    }

    /// Takes its next step, sealed as the bucket holds it, opened with `keys` (`crate::log::read_to`): junk if it's under
    /// an epoch above the newest the entries in the steps before it make, so every reader reads the same steps.
    pub fn take(&mut self, keys: &crate::db::Keys, sealed: &[u8]) -> Result<()> {
        let name = crate::db::step_name(&crate::log::prefix("global"), self.last() + 1);
        let made = self.made(sealed);
        self.read.take(keys, made, &name, sealed, "global")?;
        self.ends.push(self.read.entries.len());
        Ok(())
    }

    /// The newest epoch of `global` the entries in its steps so far make, for `sealed`, its next step: worked out again
    /// only for a step under a newer epoch than it was last time. The first is the genesis's.
    fn made(&mut self, sealed: &[u8]) -> u32 {
        if crate::keys::epoch_of(sealed).is_ok_and(|e| e > self.made.max(1)) {
            self.made = made_by(self.root, &self.read.entries, &Scope::Global);
        }
        self.made.max(1)
    }

    /// A step of `entries` already opened, as a test writes them.
    pub fn push(&mut self, entries: Vec<Entry>) {
        self.read.entries.extend(entries);
        self.ends.push(self.read.entries.len());
    }
}

/// `global`'s log of the network at `root` from the bucket to step `to`, or its end if it ends before (`Log`).
pub async fn log_to(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, root: Option<Hash>, to: u64) -> Result<Log> {
    let prefix = crate::log::prefix("global");
    let mut log = Log::new(root);
    for at in 1..=to {
        match bucket.get(&crate::db::step_name(&prefix, at)).await? {
            Some(o) => log.take(keys, &o.bytes)?,
            None => break,
        }
    }
    Ok(log)
}

/// The network's recovery key, as its genesis names it: the entry whose hash is `root` among `entries`, those of
/// `global`'s log's first step. Only the first freeze it signed is who's who (`Truth::first`).
pub fn recovery_of(root: Hash, entries: &[Entry]) -> Option<Key> {
    let genesis = entries.iter().find(|e| e.hash() == root && e.content.app == members::APP && e.content.kind == "genesis")?;
    genesis.content.data["recovery"].as_str().and_then(|k| unhex(k).ok())
}

/// The network's genesis for a reader that pins none of its own: the one genesis among `entries`, none while there
/// are two; or `pinned`, whatever genesis anyone writes.
pub fn genesis_of(pinned: Option<Hash>, entries: &[Entry]) -> Option<Hash> {
    if pinned.is_some() {
        return pinned;
    }
    let mut found = entries.iter().filter(|e| e.content.app == members::APP && e.content.kind == "genesis").map(Entry::hash);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// The network's genesis (`root`, or for a reader that pins none, `genesis_of`) and the recovery key it names, read
/// from `global`'s log's first step in the bucket, opened with `keys` (`recovery_of`).
pub async fn recovery_key(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, root: Option<Hash>) -> Result<(Hash, Key)> {
    let log = log_to(bucket, keys, root, 1).await?;
    let root = genesis_of(root, log.entries(1)).with_context(|| format!("{YOURS}'s genesis isn't the first of its log's entries"))?;
    let key = recovery_of(root, log.entries(1)).with_context(|| format!("{YOURS}'s genesis isn't the first of its log's entries"))?;
    Ok((root, key))
}

/// The newest epoch of `scope` that `entries` of `global`'s make, of the network at `root`, with the forks among them.
fn made_by(root: Option<Hash>, entries: &[Entry], scope: &Scope) -> u32 {
    let ordered = crate::entry::causal(entries.to_vec(), &Default::default());
    let member_entries: Vec<Entry> = ordered.into_iter().filter(members::is_member_entry).collect();
    Members::replay(root, &member_entries, &forks(entries)).epoch(scope)
}

/// Each device that signed two entries at one seq of a log among `entries`, cut from the first.
fn forks(entries: &[Entry]) -> Cuts {
    let mut seen: BTreeMap<(Key, String, u32), Hash> = BTreeMap::new();
    let mut cuts = Cuts::new();
    for e in &crate::entry::causal(entries.to_vec(), &Default::default()) {
        if seen.insert((e.author(), e.log().to_string(), e.seq()), e.hash()).is_some_and(|h| h != e.hash()) {
            let at = cuts.entry((e.author(), e.log().to_string())).or_insert(u32::MAX);
            *at = (*at).min(e.seq());
        }
    }
    cuts
}

/// The database a copy is, as bytes.
pub fn connection(bytes: &[u8]) -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    conn.deserialize_read_exact("main", bytes, bytes.len(), false)?;
    conn.query_row("select count(*) from sqlite_schema", [], |_| Ok(()))?;
    Ok(conn)
}

/// A scope's database another device wrote (a copy), or one this device kept from an older release, in this release's
/// form: what it says, built again here, with what only its steps said carried over (`carry`).
pub fn normalize(copy: Connection, scope: &Scope, now: u64) -> Result<ScopeDb> {
    ensure!(version(&copy)? <= READS, Newer);
    let copy = ScopeDb::open(copy, now)?;
    let built = super::scope::build(&if *scope == Scope::Global { copy.members() } else { Members::default() }, scope, &copy.state())?;
    carry(copy.conn(), &built)?;
    ScopeDb::open(built, now)
}

/// The freeze a new network starts with, at step 1 of `global`'s database, signed by its recovery key as `user`: who's
/// who as `genesis`, its log's first entry, says it, with its log's one step. → the step, and its copy.
pub fn first_freeze(recovery: &SigningKey, genesis: &Entry, user: &str, now: u64) -> Result<Given> {
    let m = Members::replay(Some(genesis.hash()), std::slice::from_ref(genesis), &Cuts::new());
    let mut db = ScopeDb::build(&m, &Scope::Global, &ScopeState::default(), now)?;
    db.conn().pragma_update(None, "user_version", READS)?;
    let freeze = Freeze { at: 1, hash: crate::db::state_hash(db.conn())?, logs: [("global".to_string(), 1)].into() };
    let step = sign(recovery, &Scope::Global, 1, user, 1, now, freeze.content())?;
    Ok(Given { step, copy: Some(db.bytes()?) })
}

/// A scope's database before its first step (not `global`'s): what its log's entries up to the freeze's position there
/// (`entries`, read from its steps up to `Freeze::logs`') add up to with `global`'s at the freeze (`global`, its log up
/// to the freeze's end of it, of the network at `root`). Every reader works it out itself, from the frozen logs.
pub fn scope_anchor(freeze: &Freeze, root: Option<Hash>, global: &Log, scope: &Scope, entries: &[Entry], now: u64) -> Result<ScopeDb> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    ensure!(global.last() >= freeze.global(), "global's log was read to {}, not to the freeze's {}", global.last(), freeze.global());
    let entries_of_global = global.entries(freeze.global());
    let ordered_global = crate::entry::causal(entries_of_global.to_vec(), &Default::default());
    let member_entries: Vec<Entry> = ordered_global.iter().filter(|e| members::is_member_entry(e)).cloned().collect();
    let known = Members::replay(root, &member_entries, &forks(entries_of_global));
    let ordered = crate::entry::causal(entries.to_vec(), &ordered_global.iter().map(|e| (e.hash(), e.depth)).collect());
    let (_, state) = members::replay_from(&known, &member_entries, &ordered, ScopeState::default(), |_, _, _| {});
    ScopeDb::open(super::scope::build(&Members::default(), scope, &state)?, now)
}

/// The entries `scope_anchor` starts `scope`'s database from (not `global`'s): those in its log's steps up to the
/// freeze's end of it, read from the bucket with `keys`, its log's keys by epoch. A step under an epoch above the newest
/// of `scope` made at the freeze, as `global`, `global`'s log to the freeze's end of it (of the network at `root`), says,
/// is junk: every reader reads them the same whenever it reads them.
pub async fn anchor_entries(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, freeze: &Freeze, root: Option<Hash>, global: &Log, scope: &Scope) -> Result<Vec<Entry>> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    ensure!(global.last() >= freeze.global(), "global's log was read to {}, not to the freeze's {}", global.last(), freeze.global());
    let end = freeze.logs.get(&scope.to_string()).copied().unwrap_or(0);
    let made = made_by(root, global.entries(freeze.global()), scope);
    crate::log::read_to(bucket, keys, made, &crate::log::prefix(&scope.to_string()), &scope.to_string(), end).await
}

/// `global`'s scope database as a reader has it: the freeze, the last position applied (or passed over), and the
/// database there.
pub struct Truth {
    pub freeze: Freeze,
    pub at: u64,
    pub db: ScopeDb,
}

impl Truth {
    /// From the steps a reader has, by position from the first (none for one that will never be a step): from the first
    /// freeze `recovery`, the network's recovery key (`recovery_of`), signed (`first`), each step after taken. None
    /// before there's one. Fails there if it doesn't check out.
    pub fn read(steps: &[Option<Given>], recovery: Key, now: u64) -> Result<Option<Truth>> {
        let mut truth: Option<Truth> = None;
        for (i, given) in steps.iter().enumerate() {
            let at = i as u64 + 1;
            match &mut truth {
                Some(t) => drop(t.take(at, given.as_ref().map(|g| &g.step))?),
                None => truth = Truth::first(at, given.as_ref(), recovery, now)?,
            }
        }
        Ok(truth)
    }

    /// Step `at` of `global`'s database before its freeze: the freeze, if `recovery` signed it, its copy checked against
    /// the hash it names. Any other step is nothing. One the recovery key signed that doesn't check out stops the
    /// reader here, as a newer release's does (`Newer`): it's never passed over for another.
    pub fn first(at: u64, given: Option<&Given>, recovery: Key, now: u64) -> Result<Option<Truth>> {
        let Some(g) = given.filter(|g| g.step.content.kind == FREEZE && g.step.author() == recovery) else { return Ok(None) };
        let t = Truth::of_freeze(g, at, now).with_context(|| format!("{YOURS}'s freeze at step {at}, which its recovery key signed, doesn't check out: this device reads who's who no further"))?;
        Ok(Some(t))
    }

    /// The freeze `given` is at `at`, whoever signed it: its copy, if it comes to the hash it names.
    fn of_freeze(given: &Given, at: u64, now: u64) -> Result<Truth> {
        let freeze = Freeze::of(&given.step, at)?;
        let copy = connection(given.copy.as_deref().context("a freeze carries its copy")?)?;
        ensure!(crate::db::state_hash(&copy)? == freeze.hash, "a freeze's copy isn't what it says");
        Ok(Truth { freeze, at, db: normalize(copy, &Scope::Global, now)? })
    }

    /// Takes step `at`, the next: applied if it counts, passed over if not (or none). → whether it counted.
    pub fn take(&mut self, at: u64, step: Option<&Entry>) -> Result<bool> {
        ensure!(at == self.at + 1, "step {at} isn't the next");
        self.at = at;
        let Some(step) = step else { return Ok(false) };
        match apply(&self.db, None, &Scope::Global, step) {
            Ok(a) => {
                self.db = ScopeDb::open(a.db, self.db.now())?;
                Ok(true)
            }
            Err(err) if err.is::<Newer>() => {
                self.at = at - 1;
                Err(err)
            }
            Err(err) => {
                tracing::info!("global's step {at} doesn't count: {err:#}");
                Ok(false)
            }
        }
    }

    /// As a reader keeps it, to start from again: `{ freeze, freeze_at, at, hash, db }`, the database's bytes base64.
    pub fn keep(&mut self) -> Result<Value> {
        let bytes = self.db.bytes()?;
        Ok(json!({ "freeze": self.freeze.content().data, "freeze_at": self.freeze.at, "at": self.at, "hash": crate::db::state_hash(self.db.conn())?, "db": data_encoding::BASE64.encode(&bytes) }))
    }

    /// What `keep` kept, checked against the hash it kept with it, in this release's form.
    pub fn kept(kept: &Value, now: u64) -> Result<Truth> {
        let bytes = data_encoding::BASE64.decode(kept["db"].as_str().context("no database kept")?.as_bytes())?;
        let conn = connection(&bytes)?;
        ensure!(Some(crate::db::state_hash(&conn)?.as_str()) == kept["hash"].as_str(), "who's who this device kept isn't what it was");
        let freeze = Freeze::from_data(&kept["freeze"], kept["freeze_at"].as_u64().context("no freeze kept")?)?;
        Ok(Truth { freeze, at: kept["at"].as_u64().context("no position kept")?, db: normalize(conn, &Scope::Global, now)? })
    }
}

/// After its writer wrote step `at` of a scope's database: a copy of the database it leaves (`db`'s bytes), sealed
/// as its steps are, at every `COPY_EVERY`th step; and the newest hint moved on, as a database's (`db::hint`). Only
/// saves readers work, so a link that fails it fails nothing. The hint is anyone's to write, so nothing here trusts it.
pub async fn written(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, scope: &Scope, at: u64, bytes: Vec<u8>) -> Result<()> {
    let prefix = prefix(&scope.to_string());
    let epoch = *keys.keys().next_back().context("no key")?;
    if at % crate::db::COPY_EVERY == 0 {
        let name = crate::db::copy_name(&prefix, at);
        bucket.create(&name, &crate::log::seal(keys, &name, &bytes)?).await?;
    }
    crate::db::hint(bucket, &prefix, at, at - at % crate::db::COPY_EVERY, epoch).await
}

/// Where the device that let a new one in had read `global`'s scope database to (its Welcome): its freeze's position,
/// the last step it had taken, and the database's state hash there, in its release's form. The new device starts from
/// the copy at or before that step, takes the steps after it up to there, and keeps what it made only if it comes to
/// that hash: the steps from the freeze on are never all read again. Said `<freeze>.<at>.<hash>`, as sign-in passes it on
/// beside the genesis (`cloudflare/sign-in/worker.js`), which a device that knows none of it never reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    pub freeze: u64,
    pub at: u64,
    pub hash: String,
}

impl Welcome {
    /// A Welcome as it's said: none for one that isn't. `cloudflare/shared/crypto.js`'s `waited` reads it the same way.
    pub fn parse(said: &str) -> Option<Welcome> {
        let number = |n: &str| (!n.is_empty() && n.len() <= 16 && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.parse::<u64>().ok()).flatten();
        let mut parts = said.splitn(3, '.');
        let (freeze, at, hash) = (number(parts.next()?)?, number(parts.next()?)?, parts.next()?);
        (at >= freeze && at < 1 << 53 && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).then(|| Welcome { freeze, at, hash: hash.to_string() })
    }

    /// The copy a new device starts from: the newest at or before `at` the bucket keeps (one every `db::COPY_EVERY`
    /// steps), or none, for the freeze's own.
    pub fn copy(&self) -> Option<u64> {
        let c = self.at - self.at % crate::db::COPY_EVERY;
        (c > self.freeze).then_some(c)
    }
}

impl std::fmt::Display for Welcome {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.freeze, self.at, self.hash)
    }
}

impl Truth {
    /// Where this reader is, as it welcomes a new device.
    pub fn welcome(&self) -> Result<Welcome> {
        Ok(Welcome { freeze: self.freeze.at, at: self.at, hash: crate::db::state_hash(self.db.conn())? })
    }

    /// Who's who as `welcome` says it: from `freeze`, the step at its freeze's position, and `start`, the copy at
    /// `Welcome::copy` (its bytes, opened), or none to start from the freeze's own, then `after`, the steps after that
    /// one up to the Welcome's, each taken as any reader takes it. Fails unless it comes to the Welcome's hash, in this
    /// release's form: a freeze, copy or step a node or another device swapped is never used.
    pub fn welcomed(welcome: &Welcome, freeze: &Given, start: Option<&[u8]>, after: &[Option<Given>], now: u64) -> Result<Truth> {
        ensure!(u64::from(freeze.step.seq()) == welcome.freeze, "that isn't the freeze it was welcomed with");
        let anchored = Truth::of_freeze(freeze, welcome.freeze, now).context("the step it was welcomed with isn't a freeze")?;
        let (at, db) = match (welcome.copy(), start) {
            (Some(at), Some(bytes)) => (at, normalize(connection(bytes)?, &Scope::Global, now)?),
            (_, None) => (welcome.freeze, anchored.db),
            (None, Some(_)) => bail!("no copy comes before the freeze's"),
        };
        ensure!(after.len() as u64 == welcome.at - at, "not every step up to where it was welcomed is here");
        let mut t = Truth { freeze: anchored.freeze, at, db };
        for given in after {
            t.take(t.at + 1, given.as_ref().map(|g| &g.step))?;
        }
        ensure!(crate::db::state_hash(t.db.conn())? == welcome.hash, "who's who from that copy isn't what the device that let this one in said");
        Ok(t)
    }
}

/// Who's who as `welcome` says it, read from the bucket with `keys`, `global`'s log's keys by epoch (`Truth::welcomed`):
/// from the copy before the Welcome's step, or, if that one isn't there or doesn't come to its hash, from its freeze's.
pub async fn welcomed(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, welcome: &Welcome, now: u64) -> Result<Truth> {
    async fn step(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, at: u64) -> Result<Option<Given>> {
        let o = bucket.get(&crate::db::step_name(&prefix("global"), at)).await?.with_context(|| format!("global's step {at} isn't there"))?;
        // Opened only with keys it was given, so any epoch it wasn't is junk.
        let made = keys.keys().next_back().copied().unwrap_or(0);
        Ok(match open(keys, made, &Scope::Global, at, &o.bytes) {
            Opened::Step(g) => Some(g),
            Opened::Junk => None,
            Opened::Waits(e) => bail!("global's step {at} is under epoch {e}, whose key this device wasn't given"),
        })
    }
    let prefix = prefix("global");
    let freeze = step(bucket, keys, welcome.freeze).await?.context("the step it was welcomed with isn't a freeze")?;
    let mut tried = None;
    if let Some(c) = welcome.copy() {
        let name = crate::db::copy_name(&prefix, c);
        let copy = async {
            let o = bucket.get(&name).await?.context("no copy")?;
            let key = keys.get(&crate::keys::epoch_of(&o.bytes)?).context("its copy is under a key this device wasn't given")?;
            crate::keys::open(key, &name, &o.bytes)
        };
        match copy.await {
            Ok(bytes) => {
                let mut after = Vec::new();
                for at in c + 1..=welcome.at {
                    after.push(step(bucket, keys, at).await?);
                }
                match Truth::welcomed(welcome, &freeze, Some(&bytes), &after, now) {
                    Ok(t) => return Ok(t),
                    Err(err) => tried = Some(err),
                }
            }
            Err(err) => tried = Some(err),
        }
    }
    if let Some(err) = tried {
        tracing::info!("global's copy before step {}: {err:#}: read from its freeze", welcome.at);
    }
    let mut after = Vec::new();
    for at in welcome.freeze + 1..=welcome.at {
        after.push(step(bucket, keys, at).await?);
    }
    Truth::welcomed(welcome, &freeze, None, &after, now)
}

/// What a step that counts made of the database it was applied to: the database it leaves.
pub struct Applied {
    pub db: Connection,
}

/// Applies `step` to `db`, `scope`'s database as the steps before it left it, with `global`'s as it is now (for a
/// scope other than `global`, whose permissions are `global`'s): → the database it leaves, or why it doesn't count.
pub fn apply(db: &ScopeDb, global: Option<&ScopeDb>, scope: &Scope, step: &Entry) -> Result<Applied> {
    ensure!(step.scope() == *scope, "a step of another scope");
    let who = global.unwrap_or(db);
    if step.content.app == members::APP && step.content.kind == DEFAULTS {
        return defaults(db, who, scope, step);
    }
    ensure!(version(db.conn())? <= READS, Newer);
    let mut m = who.members();
    let mut state = db.state();
    ensure!(step.epoch() <= m.epoch(scope), "it names an epoch its scope hasn't reached");
    allowed(who, scope, step)?;
    let member = members::is_member_entry(step);
    if *scope == Scope::Global && member {
        m.try_step(step)?;
        if matches!(step.content.kind.as_str(), "install" | "uninstall") {
            members::applies_in_scope(&m, &state, step)?;
            state = apply_own(&m, state, step)?;
        }
    } else {
        ensure!(m.counts(step), "{} can't write in {scope} for that person", crate::entry::short(&step.author()));
        if step.content.app == members::APP {
            state = apply_own(&m, state, step)?;
        }
    }
    let built = super::scope::build(&if *scope == Scope::Global { m } else { Members::default() }, scope, &state)?;
    carry(db.conn(), &built)?;
    named(&built, step)?;
    Ok(Applied { db: built })
}

/// A scope's database (not `global`'s) as `take_scope` left it: the last position it applied or passed over, the
/// database there, and whether it stopped at the step after, a newer release's (`Newer`).
pub struct TookScope {
    pub at: u64,
    pub db: ScopeDb,
    pub newer: bool,
}

/// Takes `read`, `scope`'s steps after `at` by position (none for one that will never be a step), into `db`, its
/// database there (not `global`'s): each applied if it counts with `global`, `global`'s database as it is now, passed
/// over if not. A newer release's step stops it before it, as it stops `global`'s readers (`Truth::take`): it's never
/// passed over, which a reader of that release wouldn't do.
pub fn take_scope(mut at: u64, mut db: ScopeDb, global: &ScopeDb, scope: &Scope, read: &[Option<Given>], now: u64) -> Result<TookScope> {
    for step in read {
        if let Some(given) = step {
            match apply(&db, Some(global), scope, &given.step) {
                Ok(a) => db = ScopeDb::open(a.db, now)?,
                Err(err) if err.is::<Newer>() => return Ok(TookScope { at, db, newer: true }),
                Err(err) => tracing::info!("{scope}'s step {} doesn't count: {err:#}", at + 1),
            }
        }
        at += 1;
    }
    Ok(TookScope { at, db, newer: false })
}

/// The kind of step that moves a database on to a newer release's types, roles, rules and tables
/// (`permissions::DEFAULTS`, `scope::SCHEMA`): `{ reads, types, roles, rules }`, `reads` that release's
/// `READS`, which the database keeps as its version. An admin's device writes one once every device reads it, and
/// only above the database's version: a release never puts back what a newer one said, and two
/// on different releases never write turn about. Readers never run their own release's: each database keeps the rows
/// its freeze and steps said. One that reads less stops at it (`Newer`): update it.
pub const DEFAULTS: &str = "defaults";

/// A database's version: the `READS` of the release whose types, roles, rules and tables it has.
pub fn version(db: &Connection) -> Result<u32> {
    Ok(db.query_row("pragma user_version", [], |r| r.get(0))?)
}

/// What a reader that reads less than a step's release says: it stops there, and its device needs updating.
#[derive(Debug)]
pub struct Newer;

impl std::fmt::Display for Newer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "update this device: {YOURS}'s who's who is from a newer release than this one reads")
    }
}

impl std::error::Error for Newer {}

/// This release's types, roles and rules as a `DEFAULTS` step says them, with its version.
pub fn release_defaults() -> Result<Content> {
    let db = Connection::open_in_memory()?;
    db.execute_batch(permissions::SCHEMA)?;
    permissions::defaults(&db)?;
    let mut data = rules_of(&db)?;
    data["reads"] = json!(READS);
    Ok(Content::new(members::APP, DEFAULTS, data))
}

/// A database's types, roles and rules, as a `DEFAULTS` step says them.
pub fn rules_of(db: &Connection) -> Result<serde_json::Value> {
    let rows = |sql: &str| -> Result<Vec<serde_json::Value>> {
        let mut q = db.prepare(sql)?;
        let n = q.column_count();
        let out = q.query_map([], |r| (0..n).map(|i| r.get::<_, rusqlite::types::Value>(i)).collect::<rusqlite::Result<Vec<_>>>())?;
        out.map(|r| {
            Ok(serde_json::Value::Array(
                r?.into_iter()
                    .map(|v| match v {
                        rusqlite::types::Value::Integer(i) => serde_json::json!(i),
                        rusqlite::types::Value::Text(t) => serde_json::json!(t),
                        _ => serde_json::Value::Null,
                    })
                    .collect(),
            ))
        })
        .collect()
    };
    Ok(serde_json::json!({
        "types": rows("SELECT name, permissions FROM types ORDER BY name")?,
        "roles": rows("SELECT type, name, permissions, gives FROM roles WHERE builtin = 1 ORDER BY type, name")?,
        "rules": rows(&format!("SELECT subject, relation, object FROM relations WHERE {RULE} ORDER BY 1, 2, 3"))?,
    }))
}

/// A row of `relations` that's a rule: the release's, never given or taken (`permissions`).
const RULE: &str = "(substr(subject, 1, 1) IN ('#', '@') OR instr(subject, '*') > 0 OR instr(object, '*') > 0)";

/// A `DEFAULTS` step: by an admin's device, the release's types, roles and rules as it says them, in place of those
/// the database had; the network's own roles stay.
fn defaults(db: &ScopeDb, who: &ScopeDb, scope: &Scope, step: &Entry) -> Result<Applied> {
    // Whose it is first: one that doesn't count is passed over, whatever release it names.
    ensure!(who.client(&step.author()).is_some_and(|c| c.user == step.user()), "only a person's own device writes for them");
    ensure!(who.may(step.user(), "change-roles", scopes::NETWORK), "only the network's admins say the release's roles");
    let reads = u32::try_from(step.content.data["reads"].as_u64().context("a release's roles name its version")?)?;
    ensure!(reads <= READS, Newer);
    ensure!(reads > version(db.conn())?, "a release's roles only ever move on");
    let out = super::scope::build(&if *scope == Scope::Global { who.members() } else { Members::default() }, scope, &db.state())?;
    carry(db.conn(), &out)?;
    let data = &step.content.data;
    let list = |k: &str| data[k].as_array().cloned().unwrap_or_default();
    let tx = out.unchecked_transaction()?;
    tx.execute_batch(&format!("DELETE FROM types; DELETE FROM roles WHERE builtin = 1; DELETE FROM relations WHERE {RULE};"))?;
    for t in list("types") {
        tx.execute("INSERT OR REPLACE INTO types VALUES (?, ?)", rusqlite::params![t[0].as_str(), t[1].as_str()])?;
    }
    for r in list("roles") {
        tx.execute("INSERT OR REPLACE INTO roles VALUES (?, ?, ?, ?, 1)", rusqlite::params![r[0].as_str(), r[1].as_str(), r[2].as_str(), r[3].as_str().unwrap_or("[]")])?;
    }
    for r in list("rules") {
        let (subject, object) = (r[0].as_str().unwrap_or(""), r[2].as_str().unwrap_or(""));
        if subject.starts_with(['#', '@']) || subject.contains('*') || object.contains('*') {
            tx.execute("INSERT OR IGNORE INTO relations VALUES (?, ?, ?, NULL)", rusqlite::params![subject, r[1].as_str(), object])?;
        }
    }
    tx.pragma_update(None, "user_version", reads)?;
    tx.commit()?;
    Ok(Applied { db: out })
}

/// What only steps say, which building from replay's members leaves out, copied from the database before: what they
/// named, and the release's types, roles and rules the steps said (`DEFAULTS`), not this reader's own release's.
fn carry(from: &Connection, to: &Connection) -> Result<()> {
    to.pragma_update(None, "user_version", version(from)?)?;
    to.execute_batch(&format!("DELETE FROM types; DELETE FROM roles WHERE builtin = 1; DELETE FROM relations WHERE {RULE};"))?;
    let mut rules = from.prepare(&format!("SELECT subject, relation, object, until FROM relations WHERE {RULE}"))?;
    let mut rows = rules.query([])?;
    while let Some(r) = rows.next()? {
        to.execute("INSERT OR IGNORE INTO relations VALUES (?, ?, ?, ?)", rusqlite::params![r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<i64>>(3)?])?;
    }
    for table in ["types", "roles"] {
        let mut q = from.prepare(&format!("SELECT * FROM {table}{}", if table == "roles" { " WHERE builtin = 1" } else { "" }))?;
        let n = q.column_count();
        let insert = format!("INSERT OR REPLACE INTO {table} VALUES ({})", vec!["?"; n].join(", "));
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let values: Vec<rusqlite::types::Value> = (0..n).map(|i| row.get(i)).collect::<rusqlite::Result<_>>()?;
            to.execute(&insert, rusqlite::params_from_iter(values))?;
        }
    }
    for table in ["blobs", "releases"] {
        let mut q = from.prepare(&format!("SELECT * FROM {table}"))?;
        let n = q.column_count();
        let insert = format!("INSERT INTO {table} VALUES ({})", vec!["?"; n].join(", "));
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let values: Vec<rusqlite::types::Value> = (0..n).map(|i| row.get(i)).collect::<rusqlite::Result<_>>()?;
            to.execute(&insert, rusqlite::params_from_iter(values))?;
        }
    }
    Ok(())
}

/// What a counted step names that no other table keeps: its blobs, and a release.
fn named(db: &Connection, step: &Entry) -> Result<()> {
    let pinned = step.content.app == members::APP && step.content.kind != "keep";
    for blob in &step.content.blobs {
        db.execute("INSERT INTO blobs VALUES (?1, ?2) ON CONFLICT (blob) DO UPDATE SET pinned = max(pinned, ?2)", rusqlite::params![blob, pinned])?;
    }
    if step.content.app == members::APP && members::RELEASES.contains(&step.content.kind.as_str()) {
        db.execute(
            "INSERT INTO releases VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![step.seq(), step.content.kind, serde_json::to_string(&step.content)?, crate::entry::hex(&step.author()), i64::try_from(step.time()).unwrap_or(i64::MAX)],
        )?;
    }
    Ok(())
}

/// A scope's own state after `step`, a node entry of its own: replay's rules for it, and what it does.
fn apply_own(m: &Members, mut state: ScopeState, step: &Entry) -> Result<ScopeState> {
    members::apply_in_scope(m, &mut state, step)?;
    Ok(state)
}

/// Rule 3: whether `step`'s signer, as its person, holds the permission its change needs in `db` (`global`'s), before
/// it. Replay's own rules for the change are checked too, as it's applied (`apply`): this is the engine's.
pub fn allowed(db: &ScopeDb, scope: &Scope, step: &Entry) -> Result<()> {
    let data = &step.content.data;
    let w = step.user();
    let text = |k: &str| data[k].as_str().map(str::to_string).with_context(|| format!("no {k}"));
    let may = |verb: &str, object: &str| db.may(w, verb, object);
    let role = |k: &str| -> Result<Role> { Ok(serde_json::from_value(data[k].clone())?) };
    let ok = match (step.content.app.as_str(), step.content.kind.as_str()) {
        // Rule 1: the recovery key lets a device in, and hands over to the next; nothing else.
        _ if db.recovery() == Some(step.author()) => matches!(step.content.kind.as_str(), "device" | "recovery") && *scope == Scope::Global,
        (members::APP, "genesis") => false,
        (members::APP, "device") => {
            let user = text("user")?;
            let key = unhex(&text("key")?)?;
            let kinds: Vec<members::Kind> = serde_json::from_value(data["roles"].clone()).unwrap_or_default();
            let theirs = kinds.iter().any(|k| matches!(k, members::Kind::Client | members::Kind::Service));
            let verb = if theirs { "add-devices" } else { "add-machines" };
            match (db.user_ever(&user), db.machine_ever(&key)) {
                // Someone new, with their first device: given a role in the network by someone who may give it.
                (None, None) => db.may_give(w, &user, super::scope::role(role("role")?), scopes::NETWORK),
                (Some(_), None) => db.user(&user).is_some() && maker(db, w, verb, &scopes::person(&user), &scopes::person(&user), "owner", &scopes::device(&key)),
                // Listed again by its person's own device, to name the device it's part of.
                (Some(_), Some(_)) => may("add-devices", &scopes::person(&user)),
                (None, Some(_)) => false,
            }
        }
        (members::APP, "role") => {
            let user = text("user")?;
            let was = db.user(&user).context("no such person")?.role;
            let to = role("role")?;
            to == was || (db.may_give(w, &user, super::scope::role(to), scopes::NETWORK) && db.may_take(w, &user, super::scope::role(was), scopes::NETWORK))
        }
        (members::APP, "remove") => match text("user") {
            Ok(user) => db.user(&user).is_some_and(|u| db.may_take(w, &user, super::scope::role(u.role), scopes::NETWORK)),
            Err(_) => may("remove", &scopes::device(&unhex(&text("key")?)?)),
        },
        // A person's own say over their devices: turning one down before it's listed, and forgetting what one of
        // theirs signed in with (replay checks it's theirs).
        (members::APP, "refuse" | "forget-sign-in") => may("add-devices", &scopes::person(w)),
        (members::APP, "epoch") => {
            let s = Scope::parse(&text("scope")?)?;
            may("remove", &scopes::object(&s))
        }
        (members::APP, "runs") => may("keep", &scopes::person(w)),
        (members::APP, "grant") | (members::APP, "vault") => may("keep", &scopes::object(&Scope::parse(&text("scope")?)?)),
        (members::APP, "group") => may("make-groups", scopes::NETWORK),
        (members::APP, "invite") => db.may_give(w, &text("user")?, super::scope::role(role("role")?), &scopes::group(&text("group")?)),
        // Taking up an invite: the role the invite gave, which was checked as it was given.
        (members::APP, "accept") => db.roles().get(&(w.to_string(), Scope::Group(text("group")?))).is_some_and(|(_, invited)| *invited),
        (members::APP, "member") => {
            let (g, user) = (text("group")?, text("user")?);
            let object = scopes::group(&g);
            let was = db.group_role(&user, &g).or_else(|| db.group(&g).and_then(|grp| grp.invited.get(&user).copied()));
            let to: Option<Role> = serde_json::from_value(data["role"].clone())?;
            let took = was.is_none_or(|r| Some(r) == to || db.may_take(w, &user, super::scope::role(r), &object));
            took && to.is_none_or(|r| Some(r) == was || db.may_give(w, &user, super::scope::role(r), &object))
        }
        (members::APP, "rename") => {
            if let Ok(key) = text("key") {
                may("rename", &scopes::device(&unhex(&key)?))
            } else if let Ok(user) = text("user") {
                may("rename", &scopes::person(&user))
            } else {
                may("rename", &scopes::group(&text("group")?))
            }
        }
        (members::APP, "setting") => may("change-settings", scopes::NETWORK),
        // Only while there's no recovery key (the one there was forked `global`): an admin names the next.
        (members::APP, "recovery") => db.recovery().is_none() && may("change-settings", scopes::NETWORK),
        (members::APP, "app") => {
            let name = text("name")?;
            let owner = Principal::parse(&text("owner")?)?;
            let to = scopes::owner_object(&owner);
            match db.app(&name) {
                None => {
                    let holders = match &owner {
                        Principal::User(u) => scopes::person(u),
                        Principal::Group(g) => format!("{}#admin", scopes::group(g)),
                        Principal::Admins => format!("{}#admin", scopes::NETWORK),
                    };
                    maker(db, w, "add-apps", &to, &holders, "owner", &scopes::app(&name))
                }
                // Deleting, restoring, erasing or giving it away; or saying where its code is.
                Some(a) => {
                    let moves = a.owner != owner;
                    let retires = data.get("deleted").is_some() || data["erased"] == true || moves;
                    (!retires || may("delete", &scopes::app(&name))) && (!moves || may("add-apps", &to)) && (data.get("source").is_none() || may("deploy", &scopes::app(&name)))
                }
            }
        }
        (members::APP, "deploy") => may("deploy", &scopes::app(&text("app")?)),
        (members::APP, "service") => {
            let k = unhex(&text("device")?)?;
            let on = data["on"] == true;
            let adds = |s: &str| Scope::parse(s).is_ok_and(|s| may("add-apps", &scopes::object(&s)));
            // Its scope's adders switch it off there (removing the app, or moving its service), whoever's node it is.
            let was = db.switch(&k, &text("name")?).and_then(|s| s.scope);
            let theirs = match db.cloudflare_node() == Some(k) {
                true => may("change-settings", scopes::NETWORK),
                false => may("switch", &scopes::device(&k)),
            };
            (theirs || (!on && was.as_deref().is_some_and(adds))) && (!on || data["scope"].as_str().is_none_or(adds))
        }
        (members::APP, "install" | "uninstall" | "schedule" | "vault-request") => may("add-apps", &scopes::object(scope)) || runs_it(db, scope, step),
        (members::APP, "keep" | "forget") => may("use", &scopes::object(scope)) || runs_it(db, scope, step),
        // A release, which every node checks is signed with the release key before it takes it.
        (members::APP, kind) if members::RELEASES.contains(&kind) => true,
        (members::APP, "arrange") => *scope == Scope::User(w.to_string()) && may("use", &scopes::object(scope)),
        // An app's own entries, and the release's: as replay counts them, by who signed them.
        (members::APP, _) => false,
        _ => true,
    };
    ensure!(ok, "{} may not {} there", w, step.content.kind);
    Ok(())
}

/// Whether `maker` may make `object`, new, with its first row `(subject, relation, object)` (`permissions::may_make`).
fn maker(db: &ScopeDb, maker: &str, verb: &str, container: &str, subject: &str, relation: &str, object: &str) -> bool {
    permissions::may_make(db.conn(), &scopes::person(maker), verb, container, subject, relation, object, db.now()).unwrap_or(false)
}

/// Whether a step of a scope's own state was signed by a device that runs the scope's calls, for a person who may use
/// the scope: a node writes what a call it ran did.
fn runs_it(db: &ScopeDb, scope: &Scope, step: &Entry) -> bool {
    db.runs_for(&step.author(), scope) && db.may(step.user(), "use", &scopes::object(scope))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// That `entries`, replay's in its order, applied as steps from the genesis on, leave the database replay's
    /// `m` builds: what each counted for and changed is replay's.
    /// Histories written one after another (each entry after every one before it): those where replay's order is
    /// the one the steps would have, with nothing written at once, which a database's one order never has.
    pub(crate) fn agrees(entries: &[Entry], m: &Members) {
        let mut before: std::collections::HashMap<crate::entry::Hash, std::collections::HashSet<crate::entry::Hash>> = Default::default();
        for (i, e) in entries.iter().enumerate() {
            let mut all: std::collections::HashSet<_> = e.follows_all().collect();
            for f in e.follows_all() {
                all.extend(before.get(&f).cloned().unwrap_or_default());
            }
            if !entries[..i].iter().all(|p| all.contains(&p.hash())) {
                return;
            }
            before.insert(e.hash(), all);
        }
        // Nor a removal that says its remover had seen less than it had, which replay takes back from before it: a
        // step's place is where it was written, so what came before a removal counts.
        let last_removal = entries.iter().rposition(|e| e.content.kind == "remove").unwrap_or(0);
        if entries[..last_removal].iter().any(|e| m.cut(e)) {
            return;
        }
        let genesis = &entries[..1];
        let start = Members::replay(Some(genesis[0].hash()), genesis, &Default::default());
        let mut db = ScopeDb::build(&start, &Scope::Global, &ScopeState::default(), 0).unwrap();
        for e in &entries[1..] {
            match apply(&db, None, &Scope::Global, e) {
                Ok(a) => db = ScopeDb::open(a.db, 0).unwrap(),
                Err(err) => tracing::debug!("{} doesn't count: {err:#}", e.content.kind),
            }
        }
        let replayed = ScopeDb::build(m, &Scope::Global, &ScopeState::default(), 0).unwrap();
        let hash = |d: &ScopeDb| crate::db::state_hash(d.conn()).unwrap();
        if hash(&db) != hash(&replayed) {
            let dump = |d: &ScopeDb| {
                let mut out = Vec::new();
                for t in ["install", "principals", "invites", "devices", "relations", "epochs", "sealed", "owed", "keeps", "apps", "deploys", "services", "settings", "vault", "vault_opens", "recovery", "runs", "runs_chosen"] {
                    let mut q = d.conn().prepare(&format!("SELECT * FROM {t} ORDER BY 1")).unwrap();
                    let n = q.column_count();
                    let rows: Vec<String> = q.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap())).collect::<Vec<_>>().join(", "))).unwrap().map(Result::unwrap).collect();
                    out.push(format!("{t}: {rows:#?}"));
                }
                out.join("\n")
            };
            panic!("steps left\n{}\nreplay made\n{}", dump(&db), dump(&replayed));
        }
    }

    /// `global`'s entries, each written after every one before it.
    #[derive(Clone, Default)]
    struct History {
        entries: Vec<Entry>,
        heads: std::collections::HashMap<Key, crate::entry::Head>,
    }

    impl History {
        fn write(&mut self, key: &SigningKey, user: &str, content: Content) -> Entry {
            let me = key.verifying_key().to_bytes();
            let previous = self.entries.iter().filter(|e| e.author() != me).map(Entry::hash).collect();
            let e = Entry::create(key, Ext { log: "global".into(), time: 0, user: user.into(), epoch: 1, previous }, self.heads.get(&me).copied(), content);
            self.heads.insert(me, e.head());
            self.entries.push(e.clone());
            e
        }

        fn root(&self) -> Hash {
            self.entries[0].hash()
        }
    }

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";

    /// Andre's Mac starts the network, its recovery key kept offline, and lists Bob's laptop, Bob an admin too.
    fn two_admins() -> (History, SigningKey, SigningKey, SigningKey) {
        use crate::keys::Endpoint;
        use crate::members::{Kind, device, genesis};
        let (mac, laptop, recovery) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[7; 32]));
        let mut h = History::default();
        h.write(&mac, ANDRE, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &recovery.verifying_key().to_bytes(), &"e".repeat(64)));
        h.write(&mac, ANDRE, device(&Endpoint::of(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Admin))));
        (h, mac, laptop, recovery)
    }

    /// Who's who is the first freeze the recovery key its genesis names signed, its copy checked against the hash it
    /// names: anyone else's, an admin's own device's too, is nothing, and one it signed whose copy isn't what it says
    /// stops the reader there, never passed over. A device let in starts from its Welcome, whose hash pins it.
    #[test]
    fn a_reader_takes_only_the_recovery_key_s_freeze() {
        let (h, mac, _, recovery) = two_admins();
        let key = recovery_of(h.root(), &h.entries).unwrap();
        assert_eq!(key, recovery.verifying_key().to_bytes(), "the genesis names it");
        assert_eq!(recovery_of(h.entries[1].hash(), &h.entries), None, "and only the genesis does");
        let theirs = first_freeze(&recovery, &h.entries[0], ANDRE, 0).unwrap();
        let mut admins = first_freeze(&mac, &h.entries[0], ANDRE, 0).unwrap();
        assert!(Truth::read(&[Some(admins.clone())], key, 0).unwrap().is_none(), "an admin's is nothing");
        admins.step = sign(&mac, &Scope::Global, 1, ANDRE, 1, 0, admins.step.content.clone()).unwrap();
        let at2 = Given { step: sign(&recovery, &Scope::Global, 2, ANDRE, 1, 0, theirs.step.content.clone()).unwrap(), copy: theirs.copy.clone() };
        let t = Truth::read(&[Some(admins), Some(at2.clone())], key, 0).unwrap().unwrap();
        assert_eq!(t.at, 2);
        assert_eq!(t.db.recovery(), Some(key));
        let mut swapped = at2;
        let mut db = connection(swapped.copy.as_deref().unwrap()).unwrap();
        db.execute_batch("UPDATE principals SET name = 'Mallory'").unwrap();
        swapped.copy = Some(db.serialize("main").unwrap().to_vec());
        assert!(Truth::read(&[None, Some(swapped.clone())], key, 0).is_err(), "a copy that isn't what it says stops it");
        // A Welcome's hash pins who's who, whoever signed the freeze it names.
        let mut t = Truth::read(&[Some(theirs.clone())], key, 0).unwrap().unwrap();
        let step = sign(&mac, &Scope::Global, 2, ANDRE, 1, 0, members::setting("name", Some("Ours"))).unwrap();
        assert!(t.take(2, Some(&step)).unwrap());
        let w = t.welcome().unwrap();
        let after = [Some(Given { step, copy: None })];
        assert_eq!(crate::db::state_hash(Truth::welcomed(&w, &theirs, None, &after, 0).unwrap().db.conn()).unwrap(), w.hash);
        assert!(Truth::welcomed(&Welcome { hash: "0".repeat(64), ..w.clone() }, &theirs, None, &after, 0).is_err(), "nor another hash");
        assert!(Truth::welcomed(&w, &theirs, None, &[None], 0).is_err(), "nor a step held back");
    }

    #[test]
    fn a_scope_s_reader_stops_at_a_newer_release_s_step() {
        let (h, mac, _, _) = two_admins();
        let m = Members::replay(Some(h.root()), &h.entries, &Default::default());
        let global = ScopeDb::build(&m, &Scope::Global, &ScopeState::default(), 0).unwrap();
        let scope = Scope::User(ANDRE.into());
        let db = || ScopeDb::open(super::super::scope::build(&Members::default(), &scope, &ScopeState::default()).unwrap(), 0).unwrap();
        let mut newer = release_defaults().unwrap();
        newer.data["reads"] = json!(READS + 1);
        let newer = Some(Given { step: sign(&mac, &scope, 2, ANDRE, 1, 0, newer).unwrap(), copy: None });
        let took = take_scope(0, db(), &global, &scope, &[None, newer, None], 0).unwrap();
        assert!(took.newer && took.at == 1, "stopped before it");
        let took = take_scope(0, db(), &global, &scope, &[None, None], 0).unwrap();
        assert!(!took.newer && took.at == 2);
    }

    /// A newest hint is anyone's to write: one far above the last step jams no reader, which reads on to the steps'
    /// own end. Nor does anything else past a position with nothing, unless it's a step; nor a step under an epoch
    /// above every one made, which a reader passes over once it's applied those before it.
    #[test]
    fn keyless_objects_jam_no_reader() {
        use crate::bucket::Bucket;
        let bucket = crate::bucket::Memory::default();
        let prefix = prefix("global");
        let keys: crate::db::Keys = [(1, [9; 32])].into();
        let (_, mac, _, _) = two_admins();
        let step = |at: u64| {
            let name = crate::db::step_name(&prefix, at);
            crate::log::seal(&keys, &name, &encode(&sign(&mac, &Scope::Global, at, ANDRE, 1, 0, members::setting("name", Some("x"))).unwrap(), None)).unwrap()
        };
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            bucket.create(&crate::db::newest_name(&prefix), br#"{"at": 1000000000, "copy": 0, "epoch": 1}"#).await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0).await.unwrap().steps.is_empty(), "nothing written, nothing read, and no failure");
            bucket.create(&crate::db::step_name(&prefix, 3), b"x").await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0).await.unwrap().steps.is_empty(), "nor junk after a position with nothing");
            bucket.create(&crate::db::step_name(&prefix, 2), &step(2)).await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0).await.is_err(), "but a step after one with nothing stops it: that one was written");
            // Junk at 1, under an epoch no one made.
            let junk = crate::db::step_name(&prefix, 1);
            bucket.create(&junk, &crate::keys::seal(0xFFFF_FFFF, &[3; 32], &junk, b"junk")).await.unwrap();
            let r = read(&bucket, &keys, &Scope::Global, 0).await.unwrap();
            assert!(r.steps.is_empty() && r.junk_at_end(1) && !r.junk_at_end(u32::MAX), "it waits only for an epoch that was made");
            assert!(matches!(open(&keys, 1, &Scope::Global, 1, &bucket.get(&junk).await.unwrap().unwrap().bytes), Opened::Junk));
            let r = read(&bucket, &keys, &Scope::Global, 1).await.unwrap();
            assert!(r.steps.len() == 2 && r.steps[0].is_some() && r.steps[1].is_none(), "and reads on past it, passing over the junk after");
        });
    }

    /// A release's roles that don't count stop no one: whatever release it names, a `DEFAULTS` step by a device that
    /// isn't an admin's own is passed over; only an admin's of a newer release stops a reader.
    #[test]
    fn only_an_admin_s_newer_release_stops_a_reader() {
        let (h, mac, laptop, _) = two_admins();
        let mut h = h;
        h.write(&mac, ANDRE, members::role(BOB, Role::Member));
        let m = Members::replay(Some(h.root()), &h.entries, &Default::default());
        let db = ScopeDb::build(&m, &Scope::Global, &ScopeState::default(), 0).unwrap();
        let step = |key: &SigningKey, user: &str| {
            let mut content = release_defaults().unwrap();
            content.data["reads"] = json!(999);
            sign(key, &Scope::Global, 1, user, 1, 0, content).unwrap()
        };
        let stranger = SigningKey::from_bytes(&[9; 32]);
        for (key, user) in [(&laptop, BOB), (&stranger, ANDRE)] {
            let err = apply(&db, None, &Scope::Global, &step(key, user)).err().unwrap();
            assert!(!err.is::<Newer>(), "passed over, not a newer release: {err:#}");
        }
        assert!(apply(&db, None, &Scope::Global, &step(&mac, ANDRE)).err().unwrap().is::<Newer>());
    }

    /// A release's types, roles and rules move a database on only to a newer release's, by an admin's own device, and a
    /// reader of an older one stops there rather than read on as if it didn't; and from the first that reads the scope
    /// database, each `READS` is held to the tables, types, roles and rules it reads, so a release that changes them
    /// reads one more.
    #[test]
    fn a_release_s_roles_only_move_on_and_an_older_reader_stops_at_a_newer_one() {
        use crate::keys::Endpoint;
        use crate::members::{Kind, genesis};
        const ANDRE: &str = "a000000000000001";
        let mac = SigningKey::from_bytes(&[1; 32]);
        let g = Entry::create(&mac, Ext { log: "global".into(), time: 0, user: ANDRE.into(), epoch: 0, previous: Vec::new() }, None, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)));
        let m = Members::replay(Some(g.hash()), std::slice::from_ref(&g), &Default::default());
        let db = ScopeDb::build(&m, &Scope::Global, &ScopeState::default(), 0).unwrap();
        let step = |at: u64, reads: u32| {
            let mut content = release_defaults().unwrap();
            content.data["reads"] = json!(reads);
            sign(&mac, &Scope::Global, at, ANDRE, 1, 0, content).unwrap()
        };
        let newer = apply(&db, None, &Scope::Global, &step(1, READS + 1)).err().unwrap();
        assert!(newer.is::<Newer>(), "a newer release's stops this reader");
        let db = ScopeDb::open(apply(&db, None, &Scope::Global, &step(1, READS)).unwrap().db, 0).unwrap();
        assert_eq!(version(db.conn()).unwrap(), READS);
        assert!(apply(&db, None, &Scope::Global, &step(2, READS)).is_err(), "the same release's again changes nothing");
        // What each `READS` reads, from the first that reads the scope database (2): add `(READS, hash)`
        // when a release changes them, and read one more.
        const PINNED: &[(u32, &str)] = &[(2, "1a26a7b14a1b8998d5edcf6b6d00db14a698c6109eb865f7bd8e7c86dcd295a4"), (3, "e614e4c705368711133be27dd44ff51c24ff89da7fe059001ad3990f0b454a67")];
        let mut h = blake3::Hasher::new();
        h.update(super::super::scope::SCHEMA.as_bytes());
        h.update(permissions::SCHEMA.as_bytes());
        h.update(rules_of(db.conn()).unwrap().to_string().as_bytes());
        let hash = h.finalize().to_hex().to_string();
        assert!(PINNED.contains(&(READS, hash.as_str())), "the scope database's tables, types, roles or rules changed: read one more in READS, and pin ({}, \"{hash}\")", READS + 1);
    }

    /// Who may write what as a step, by the rows at that step: a member makes a group but changes no setting, a removed
    /// device's next step counts for nothing, nor one naming an epoch not reached; and a step is never an entry.
    #[test]
    fn a_step_counts_only_if_its_signer_may_make_that_change_there() {
        use crate::keys::Endpoint;
        use crate::members::{Kind, device, genesis, group, node, remove_device, setting};
        const ANDRE: &str = "a000000000000001";
        const BOB: &str = "b000000000000002";
        let (mac, laptop, phone) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[3; 32]));
        let g = Entry::create(&mac, Ext { log: "global".into(), time: 0, user: ANDRE.into(), epoch: 0, previous: Vec::new() }, None, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)));
        let m = Members::replay(Some(g.hash()), std::slice::from_ref(&g), &Default::default());
        let mut db = ScopeDb::build(&m, &Scope::Global, &ScopeState::default(), 0).unwrap();
        let mut at = 0;
        let mut step = |db: &mut ScopeDb, key: &SigningKey, user: &str, epoch: u32, content: Content| -> bool {
            at += 1;
            let s = sign(key, &Scope::Global, at, user, epoch, 0, content).unwrap();
            let s = decode(&Scope::Global, at, &encode(&s, None)).unwrap().step;
            match apply(db, None, &Scope::Global, &s) {
                Ok(a) => {
                    *db = ScopeDb::open(a.db, 0).unwrap();
                    true
                }
                Err(_) => false,
            }
        };
        assert!(step(&mut db, &mac, ANDRE, 1, device(&Endpoint::of(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Member)))));
        assert!(step(&mut db, &laptop, BOB, 1, group("c000000000000003", "Band")), "a member makes a group");
        assert!(!step(&mut db, &laptop, BOB, 1, setting("name", Some("Bob's"))), "but changes no setting");
        assert!(!step(&mut db, &laptop, BOB, 1, device(&Endpoint::of(&phone), ANDRE, "phone", &[Kind::Client], None)), "nor lists another's device");
        assert!(!step(&mut db, &mac, ANDRE, 2, setting("name", Some("Ours"))), "nor names an epoch not reached");
        assert!(step(&mut db, &mac, ANDRE, 1, remove_device(&laptop.verifying_key().to_bytes(), None)));
        assert!(!step(&mut db, &laptop, BOB, 1, node("rename", json!({ "user": BOB, "name": "Robert" }))), "a removed device's next step counts for nothing");
        assert!(db.group("c000000000000003").is_some() && db.setting("name").is_none());
        // Signed for one place, it's nowhere else; and an entry, signed the same way, is never a step.
        let s = sign(&mac, &Scope::Global, 9, ANDRE, 1, 0, setting("name", Some("x"))).unwrap();
        assert!(decode(&Scope::Global, 10, &encode(&s, None)).is_err() && decode(&Scope::User(ANDRE.into()), 9, &encode(&s, None)).is_err());
        assert!(decode(&Scope::Global, 0, &encode(&g, None)).is_err());
    }
}
