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
use std::collections::{BTreeMap, BTreeSet};

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

/// A step as `encode` wrote it, its signature checked, whatever place it says it's for.
fn shaped(bytes: &[u8]) -> Option<Entry> {
    let ciborium::Value::Array(parts) = ciborium::from_reader(bytes).ok()? else { return None };
    match parts.as_slice() {
        [ciborium::Value::Bytes(h), ciborium::Value::Bytes(s), ciborium::Value::Bytes(b), ..] => Entry::decode_step(h, s, b).ok(),
        _ => None,
    }
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

/// The kind of step that tells writers of the log that it's being frozen: the first the recovery key signs in the
/// ceremony, before it works out where the log ends. Changes nothing; a writer that finds one after it wrote an entry
/// looks whether the freeze that follows holds it (`Freeze::holds`), and once it has waited `node::steps`' while for
/// one that doesn't come, writes on: the ceremony writes the freeze within half that, or not at all.
pub const FREEZING: &str = "freezing";

/// The kind of step that freezes the log (`anchor`): `{ hash, logs }`, with its copy.
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
            // No log gets near it, and every reader reads to the step after the end it names.
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

    /// The last step of `global`'s log it holds: every reader replays exactly the entries up to it.
    pub fn global(&self) -> u64 {
        self.logs.get("global").copied().unwrap_or(0)
    }

    /// Whether `entry`, written at step `step` of its scope's log, counts: the freeze holds it.
    pub fn holds(&self, entry: &Entry, step: u64) -> bool {
        self.logs.get(entry.log()).is_some_and(|n| step <= *n)
    }
}

/// `global`'s log as a reader of a freeze reads it from the bucket's steps (`log_to`), never a copy, nor what a reader
/// happens to hold: the entries in each of its steps from the first, and which of them are a freeze's mark (`claimed`).
/// Every reader of a freeze replays exactly the entries up to the end it names (`entries`).
#[derive(Clone, Debug, Default)]
pub struct Log {
    read: crate::log::ReadTo,
    /// How many entries there are up to each step.
    ends: Vec<usize>,
    /// The steps that are a freeze, as the step after the log's end holds it, by position.
    marks: BTreeMap<u64, Entry>,
    /// Every step that opens to a step, by position (`steps`).
    shaped: BTreeMap<u64, Entry>,
    /// Whether the bucket held nothing after its last step when it was read.
    pub ended: bool,
    /// The network's genesis, which the entries before each step are replayed from (`take`).
    root: Option<Hash>,
    /// The newest epoch of `global` the entries in its steps so far make, as last worked out (`made`).
    made: u32,
}

impl Log {
    /// Of the network whose genesis is `root`, before its first step; with none, the one its first step holds.
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
    /// an epoch above the newest the entries in the steps before it make, never what its reader knows now, so every
    /// reader reads the same steps the same whenever it reads them.
    pub fn take(&mut self, keys: &crate::db::Keys, sealed: &[u8]) -> Result<()> {
        let at = self.last() + 1;
        let name = crate::db::step_name(&crate::log::prefix("global"), at);
        let made = self.made(sealed);
        if let Some(opened) = self.read.take(keys, made, &name, sealed, "global")?
            && let Some(step) = shaped(&opened)
        {
            if step.content.kind == FREEZE && step.log() == "global" && u64::from(step.seq()) == at {
                self.marks.insert(at, step.clone());
            }
            self.shaped.insert(at, step);
        }
        // Read by a device that doesn't know the network yet: its genesis, as its first step holds it, so the epochs
        // its entries make are worked out from it, not passed over. Never one a later step holds.
        if at == 1 && self.root.is_none() {
            self.root = self.read.entries.iter().find(|e| e.content.app == members::APP && e.content.kind == "genesis").map(Entry::hash);
        }
        self.ends.push(self.read.entries.len());
        Ok(())
    }

    /// Each of its steps that opens to a step, rather than entries, whatever it says or wherever it says it's for: a
    /// freeze's mark is one.
    pub fn steps(&self) -> impl Iterator<Item = (u64, &Entry)> {
        self.shaped.iter().map(|(at, e)| (*at, e))
    }

    /// The newest epoch of `global` the entries in its steps so far make, for `sealed`, its next step: worked out again
    /// only for a step under a newer epoch than it was last time, as every reader of the same steps does. The first is
    /// the genesis's, made with the network and sealed under, in the step that holds it, before anything else.
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

    /// A freeze's mark, already opened, as its next step (`mark`).
    pub fn push_mark(&mut self, mark: Entry) {
        self.marks.insert(self.last() + 1, mark);
        self.ends.push(self.read.entries.len());
    }

    /// Every key its entries say was ever the recovery key (`recovery_keys`).
    pub fn recovery(&self) -> Recovery {
        recovery_keys(self.root, &self.read.entries)
    }

    /// Whether it was read far enough to judge a freeze that says the log ends at `to`: past the step after it, or to
    /// its end.
    pub fn reaches(&self, to: u64) -> bool {
        self.last() > to || self.ended
    }
}

/// `global`'s log of the network at `root` from the bucket to the step after `to` (or its end, if it ends before)
/// (`Log`).
pub async fn log_to(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, root: Option<Hash>, to: u64) -> Result<Log> {
    let prefix = crate::log::prefix("global");
    let mut log = Log::new(root);
    for at in 1..=to.checked_add(1).context("no log ends there")? {
        match bucket.get(&crate::db::step_name(&prefix, at)).await? {
            Some(o) => log.take(keys, &o.bytes)?,
            None => {
                log.ended = true;
                break;
            }
        }
    }
    Ok(log)
}

/// `log`'s steps after those it holds, from the bucket to its end (`Log::ended`), as `log_to` reads them.
pub async fn log_on(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, log: &mut Log) -> Result<()> {
    let prefix = crate::log::prefix("global");
    log.ended = false;
    while let Some(o) = bucket.get(&crate::db::step_name(&prefix, log.last() + 1)).await? {
        log.take(keys, &o.bytes)?;
    }
    log.ended = true;
    Ok(())
}

/// `global`'s log far enough to judge every freeze among `steps` that a key in `recovery` signed (`wants_log`), as a
/// reader that keeps nothing finds it, from the bucket: none before there's one.
pub async fn log_for(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, steps: &[Option<Given>], root: Option<Hash>, recovery: &Recovery) -> Result<Option<Log>> {
    let mut log = None;
    while let Some(end) = wants_log(steps, root, log.as_ref(), recovery) {
        log = Some(log_to(bucket, keys, root, end).await?);
    }
    Ok(log)
}

/// Every key that was ever the network's recovery key, as far as a reader knows: the one the genesis names (the entry
/// whose hash is `root`), each a `recovery` entry signed by one of those names, and each one replay made the recovery
/// key (one an admin's own device named while there was none, `Members::recovery_lost`). Only a freeze one of them
/// signed is looked at, and its log read; which of them counts is judged at the end of the log it names (`claimed`).
pub type Recovery = BTreeSet<Key>;

/// `Recovery` from `entries`, `global`'s, in any order, of the network at `root`.
pub fn recovery_keys<'a>(root: Option<Hash>, entries: impl IntoIterator<Item = &'a Entry>) -> Recovery {
    let entries: Vec<&Entry> = entries.into_iter().filter(|e| e.content.app == members::APP && e.log() == "global").collect();
    let named = |e: &Entry, field: &str| e.content.data[field].as_str().and_then(|k| unhex(k).ok());
    let mut handed = Vec::new();
    let mut keys = Recovery::new();
    for e in &entries {
        match e.content.kind.as_str() {
            "genesis" if Some(e.hash()) == root => keys.extend(named(e, "recovery")),
            "recovery" => handed.extend(named(e, "key").map(|k| (e.author(), k))),
            _ => {}
        }
    }
    let chain = |keys: &mut Recovery, handed: &mut Vec<(Key, Key)>| {
        while let Some(i) = handed.iter().position(|(by, _)| keys.contains(by)) {
            keys.insert(handed.swap_remove(i).1);
        }
    };
    chain(&mut keys, &mut handed);
    // A `recovery` entry no such key signed: an admin's, which counts only where replay says so.
    if !handed.is_empty() {
        let replayed = members_of(root, &entries.into_iter().cloned().collect::<Vec<_>>());
        keys.extend(replayed.recovery_was.iter().chain(&replayed.recovery));
        chain(&mut keys, &mut handed);
    }
    keys
}

/// The network's genesis, for a reader that holds no genesis of its own and reads `global`'s `entries` as the bucket
/// has them (the Cloudflare node): `pinned`, as the device that deployed it holds it, whatever genesis a device writes
/// into a step of the log; or, unpinned (deployed before its network was), the one genesis among them, and none
/// while there are two, until it's deployed again beside its network.
pub fn genesis_of(pinned: Option<Hash>, entries: &[Entry]) -> Option<Hash> {
    if pinned.is_some() {
        return pinned;
    }
    let mut found = entries.iter().filter(|e| e.content.app == members::APP && e.content.kind == "genesis").map(Entry::hash);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// The newest epoch of `scope` that `entries` of `global`'s make, of the network at `root`, with the forks among them.
fn made_by(root: Option<Hash>, entries: &[Entry], scope: &Scope) -> u32 {
    members_of(root, entries).epoch(scope)
}

/// What `entries` of `global`'s (in any order) replay to, of the network at `root`, with the forks among them.
fn members_of(root: Option<Hash>, entries: &[Entry]) -> Members {
    let ordered = crate::entry::causal(entries.to_vec(), &Default::default());
    let member_entries: Vec<Entry> = ordered.into_iter().filter(members::is_member_entry).collect();
    Members::replay(root, &member_entries, &forks(entries))
}

/// Each device that signed two entries at one seq of a log among `entries`, cut from the first: the forks a reader
/// that holds just those finds, as a node's store finds them (`Store::forks`). Only among those whose every entry they
/// follow is there (`entry::causal`): the store keeps an entry, a fork's too, only once it is, and one that follows
/// what no one holds waits for good.
pub fn forks(entries: &[Entry]) -> Cuts {
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

/// What `entries` of `global`'s (in any order) replay to, of the network at `root`: in causal order, with only the forks
/// among them.
fn replay_all(root: Option<Hash>, entries: &[Entry]) -> Replayed {
    let ordered = crate::entry::causal(entries.to_vec(), &Default::default());
    let member_entries: Vec<Entry> = ordered.iter().filter(|e| members::is_member_entry(e)).cloned().collect();
    let known = Members::replay(root, &member_entries, &forks(entries));
    let (members, state) = members::replay(&known, &member_entries, &ordered, |_, _, _| {});
    Replayed { known, members, member_entries, state, ordered }
}

/// What `replay_all` makes: the members as the member entries replay (`known`), and as the whole replay leaves them
/// with its state; the member entries, and every entry, in order, each with its depth.
struct Replayed {
    known: Members,
    members: Members,
    member_entries: Vec<Entry>,
    state: ScopeState,
    ordered: Vec<Entry>,
}

/// Who's who before the freeze, as a reader that keeps nothing of its own works it out from `ordered`, `global`'s
/// entries it holds in replay order, of the network at `root`: with the forks among them, as a node replays them.
pub fn view(root: Option<Hash>, ordered: &[Entry], now: u64) -> Result<ScopeDb> {
    let member_entries: Vec<Entry> = ordered.iter().filter(|e| members::is_member_entry(e)).cloned().collect();
    ScopeDb::build(&Members::replay(root, &member_entries, &forks(ordered)), &Scope::Global, &ScopeState::default(), now)
}

/// Who's who as a reader that keeps nothing of its own works it out from `ordered`, `global`'s entries it holds in
/// replay order, of the network at `root`, its scope database's steps by position from the first, and `global`'s log
/// far enough to judge the freezes among them (`log_for`, with `recovery_keys` of `ordered`): the steps' once its log is
/// frozen (`Truth::read`), replay's before.
pub fn who(steps: &[Option<Given>], root: Option<Hash>, ordered: &[Entry], log: Option<&Log>, now: u64) -> Result<ScopeDb> {
    match Truth::read(steps, root, log, &recovery_keys(root, ordered), now)? {
        Some(t) => Ok(t.db),
        None => view(root, ordered, now),
    }
}

/// The freeze `given`, step `at` of `global`'s database, says, if it's one a key in `recovery` signed: what a reader
/// reads `global`'s log to the step after the end of (`log_to`) before it judges whether that key was the recovery key
/// there (`claimed`). Anyone else's is nothing, and the end it names is never read.
pub fn candidate(given: Option<&Given>, at: u64, recovery: &Recovery) -> Option<Freeze> {
    Freeze::of(&given.filter(|g| g.step.content.kind == FREEZE && recovery.contains(&g.step.author()))?.step, at).ok()
}

/// A freeze whose mark isn't in the bucket yet: nothing at the step of `global`'s log after its end (`claimed`). The
/// ceremony writes its mark before its freeze, so it's there unless the recovery key's holder wrote otherwise: the
/// reader stops and looks again, and whatever lands there first decides it.
#[derive(Debug)]
pub struct Unmarked {
    pub at: u64,
}

impl std::fmt::Display for Unmarked {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{YOURS}'s freeze waits for step {} of its log to say whether it's where the log ends", self.at)
    }
}

impl std::error::Error for Unmarked {}

/// Whether `given`, step `at` of `global`'s database before its freeze, is a freeze the network's recovery key signed:
/// the freeze it says. Only one a key in `recovery` signed is looked at (`candidate`). Judged only at the end of
/// `global`'s log it names, from `log`, its steps up to there, replayed with the forks among them only, so every
/// reader judges it the same whenever it reads it: its signer is the recovery key the genesis or a `recovery` entry up
/// to there names; and only if the step after that end is its mark, the same freeze signed by the same key for that
/// step of the log: the bucket takes a position once, so a freeze that names an end behind one written since, a
/// replaced recovery key's to before it was, never has one. One that isn't is nothing, passed over. One that is binds:
/// it checks out (`anchor`) or the reader stops there. Fails while `log` doesn't reach the step after its end, or
/// nothing is there yet (`Unmarked`).
pub fn claimed(given: Option<&Given>, at: u64, root: Option<Hash>, log: &Log, recovery: &Recovery) -> Result<Option<Freeze>> {
    let (Some(freeze), Some(g)) = (candidate(given, at, recovery), given) else { return Ok(None) };
    let end = freeze.global();
    ensure!(log.reaches(end), "who's who waits for global's log to step {}, after where its freeze says it ends", end + 1);
    if log.last() < end {
        return Ok(None);
    }
    if log.last() == end {
        bail!(Unmarked { at: end + 1 });
    }
    let step = &g.step;
    let marked = log.marks.get(&(end + 1)).is_some_and(|m| m.author() == step.author() && m.content == step.content);
    Ok((marked && freezes(root, log, end, step.author()).is_ok()).then_some(freeze))
}

/// Whether who's who where `global`'s log ends at `end`, from `log`, lets a freeze `signer` signed count: its signer
/// is the recovery key there, and an admin's own device said the gate was open before, so nothing written before this
/// release's ceremony, while a key since replaced was the recovery key, is ever one. → why not.
pub fn freezes(root: Option<Hash>, log: &Log, end: u64, signer: Key) -> Result<()> {
    let there = members_of(root, log.entries(end));
    ensure!(there.recovery == Some(signer), "the key that signed it isn't {YOURS}'s recovery key where its log ends, at step {end}");
    ensure!(there.gate(), "no one who's still an admin where its log ends, at step {end}, opened the gate before it");
    Ok(())
}

/// Of `steps`, `global`'s database's by position from the first, the step of `global`'s log a reader reads to the step
/// after, from `log` as it has it, so it judges every freeze among them a key in `recovery` signed, up to the one the
/// recovery key signed (`claimed`): none once it can.
pub fn wants_log(steps: &[Option<Given>], root: Option<Hash>, log: Option<&Log>, recovery: &Recovery) -> Option<u64> {
    let mut furthest = None;
    for (i, g) in steps.iter().enumerate() {
        let Some(freeze) = candidate(g.as_ref(), i as u64 + 1, recovery) else { continue };
        match log.filter(|l| l.reaches(freeze.global())) {
            Some(l) if furthest.is_none() => {
                if !matches!(claimed(g.as_ref(), i as u64 + 1, root, l, recovery), Ok(None)) {
                    return None;
                }
            }
            Some(_) => {}
            None => furthest = furthest.max(Some(freeze.global())),
        }
    }
    furthest
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

/// Whether `own` says everything `copy` says: each of the copy's tables, by the columns both have, holds the same rows.
/// Only what the copy says is compared, so a later release that works out more from the entries (a column, a table)
/// still finds an older release's copy is what they add up to.
fn says(copy: &Connection, own: &Connection) -> Result<bool> {
    let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    let tables = |c: &Connection| -> Result<BTreeSet<String>> {
        let mut q = c.prepare("select name from sqlite_schema where type = 'table' and name not like 'sqlite\\_%' escape '\\'")?;
        Ok(q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    };
    let columns = |c: &Connection, t: &str| -> Result<Vec<String>> {
        let mut q = c.prepare("select name from pragma_table_info(?1) order by cid")?;
        Ok(q.query_map([t], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    };
    let rows = |c: &Connection, t: &str, cols: &[String]| -> Result<Vec<Vec<rusqlite::types::Value>>> {
        let list = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
        let mut q = c.prepare(&format!("select {list} from {} order by {list}", quote(t)))?;
        let n = q.column_count();
        let out = q.query_map([], |r| (0..n).map(|i| r.get(i)).collect::<rusqlite::Result<Vec<_>>>())?.collect::<rusqlite::Result<_>>()?;
        Ok(out)
    };
    let theirs = tables(own)?;
    for t in tables(copy)?.iter().filter(|t| theirs.contains(*t)) {
        let has = columns(own, t)?;
        let cols: Vec<String> = columns(copy, t)?.into_iter().filter(|c| has.contains(c)).collect();
        if !cols.is_empty() && rows(copy, t, &cols)? != rows(own, t, &cols)? {
            tracing::info!("the freeze's copy says otherwise in {t}");
            return Ok(false);
        }
    }
    Ok(true)
}

/// The freeze `given` is, at position `at`, as checked here against `log`, `global`'s log read past the end it names
/// (`log_to`), of the network whose genesis is `root`: its copy comes to the hash it names; the recovery key the
/// entries up to there name signed it; and they add up to what the copy says (`says`). → the database it anchors, this
/// release's form of its copy. Everyone replays exactly those entries, forks among them only, so later entries, forks
/// or releases change nothing.
pub fn anchor(given: &Given, at: u64, root: Option<Hash>, log: &Log, now: u64) -> Result<ScopeDb> {
    let step = &given.step;
    let freeze = Freeze::of(step, at)?;
    ensure!(log.last() >= freeze.global(), "global's log was read to {}, not to the freeze's {}", log.last(), freeze.global());
    let copy = connection(given.copy.as_deref().context("a freeze carries its copy")?)?;
    ensure!(crate::db::state_hash(&copy)? == freeze.hash, "a freeze's copy isn't what it says");
    ensure!(version(&copy)? <= READS, Newer);
    let r = replay_all(root, log.entries(freeze.global()));
    let own = super::scope::build(&r.members, &Scope::Global, &r.state)?;
    carry(&copy, &own)?;
    let own = ScopeDb::open(own, now)?;
    ensure!(own.recovery() == Some(step.author()), "only {YOURS}'s recovery key freezes the log");
    ensure!(says(&copy, own.conn())?, "who's who at {YOURS}'s freeze isn't what the log says it was");
    normalize(copy, &Scope::Global, now)
}

/// The freeze the ceremony writes, at `logs`' ends (every scope's log but `global`'s, by its last step in the bucket)
/// and `to`, the end of `global`'s log, whose steps hold `entries`, of the network at `root`: its content, and its
/// copy. It's written as the step of the log after `to` first, its mark (`claimed`).
pub fn freeze(root: Option<Hash>, entries: &[Entry], to: u64, mut logs: BTreeMap<String, u64>, now: u64) -> Result<(Content, Vec<u8>)> {
    logs.insert("global".into(), to);
    let r = replay_all(root, entries);
    let mut db = ScopeDb::build(&r.members, &Scope::Global, &r.state, now)?;
    db.conn().pragma_update(None, "user_version", READS)?;
    let freeze = Freeze { at: 0, hash: crate::db::state_hash(db.conn())?, logs };
    Ok((freeze.content(), db.bytes()?))
}

/// A scope's database before its first step (not `global`'s): what its log's entries up to the freeze's position there
/// (`entries`, read from its steps up to `Freeze::logs`') add up to with `global`'s at the freeze (`global`, its log up
/// to the freeze's end of it, of the network at `root`). Every reader works it out itself.
pub fn scope_anchor(freeze: &Freeze, root: Option<Hash>, global: &Log, scope: &Scope, entries: &[Entry], now: u64) -> Result<ScopeDb> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    ensure!(global.last() >= freeze.global(), "global's log was read to {}, not to the freeze's {}", global.last(), freeze.global());
    let r = replay_all(root, global.entries(freeze.global()));
    let ordered = crate::entry::causal(entries.to_vec(), &r.ordered.iter().map(|e| (e.hash(), e.depth)).collect());
    let (_, state) = members::replay_from(&r.known, &r.member_entries, &ordered, ScopeState::default(), |_, _, _| {});
    ScopeDb::open(super::scope::build(&Members::default(), scope, &state)?, now)
}

/// The entries `scope_anchor` starts `scope`'s database from (not `global`'s): those in its log's steps up to the
/// freeze's end of it, read from the bucket with `keys`, its log's keys by epoch. A step under an epoch above the newest
/// of `scope` made at the freeze, as `global`, `global`'s log to the freeze's end of it (of the network at `root`), says,
/// is junk, never one made since: every reader reads them the same whenever it reads them.
pub async fn anchor_entries(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, freeze: &Freeze, root: Option<Hash>, global: &Log, scope: &Scope) -> Result<Vec<Entry>> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    ensure!(global.last() >= freeze.global(), "global's log was read to {}, not to the freeze's {}", global.last(), freeze.global());
    let end = freeze.logs.get(&scope.to_string()).copied().unwrap_or(0);
    let made = made_by(root, global.entries(freeze.global()), scope);
    crate::log::read_to(bucket, keys, made, &crate::log::prefix(&scope.to_string()), &scope.to_string(), end).await
}

/// `global`'s scope database as a reader has it once its log is frozen: the freeze, the last position applied (or
/// passed over), and the database there.
pub struct Truth {
    pub freeze: Freeze,
    pub at: u64,
    pub db: ScopeDb,
}

impl Truth {
    /// From the steps a reader has, by position from the first (none for one that will never be a step), and `log`,
    /// `global`'s log read far enough to judge each freeze among them a key in `recovery` signed (`wants_log`): none
    /// before the one the recovery key signed. Fails there if it doesn't check out, or `log` doesn't reach it.
    pub fn read(steps: &[Option<Given>], root: Option<Hash>, log: Option<&Log>, recovery: &Recovery, now: u64) -> Result<Option<Truth>> {
        let mut truth: Option<Truth> = None;
        for (i, given) in steps.iter().enumerate() {
            let at = i as u64 + 1;
            match &mut truth {
                Some(t) => drop(t.take(at, given.as_ref().map(|g| &g.step))?),
                None => truth = Truth::first(at, given.as_ref(), root, log, recovery, now)?,
            }
        }
        Ok(truth)
    }

    /// `read`, for a reader that answers while it reads, and keeps nothing (the Cloudflare node): none while it hasn't
    /// read `global`'s log far enough to judge a freeze among `steps` (`wants_log`), or nothing is where its mark would
    /// be yet (`Unmarked`), when who's who and the epochs made are still the entries' (`view`), until it has. Fails, as
    /// `read` does, at a freeze the recovery key signed that doesn't check out, or a newer release's step.
    pub fn judged(steps: &[Option<Given>], root: Option<Hash>, log: Option<&Log>, recovery: &Recovery, now: u64) -> Result<Option<Truth>> {
        if wants_log(steps, root, log, recovery).is_some() {
            return Ok(None);
        }
        match Truth::read(steps, root, log, recovery, now) {
            Err(err) if err.is::<Unmarked>() => Ok(None),
            other => other,
        }
    }

    /// `read`, for a service endpoint, which holds no keys and takes its steps from its own device's node: the first
    /// freeze `recovery`, the network's recovery key as the entries it holds say, signed, its copy checked against the
    /// hash it names, unchecked against the log, which it can't read. The recovery key vouches for it; anything else
    /// shaped like a freeze is nothing. One it signed that doesn't check out stops it, as it stops every reader.
    pub fn read_given(steps: &[Option<Given>], recovery: Option<Key>, now: u64) -> Result<Option<Truth>> {
        let recovery = Recovery::from_iter(recovery);
        let mut truth: Option<Truth> = None;
        for (i, given) in steps.iter().enumerate() {
            let at = i as u64 + 1;
            match (&mut truth, candidate(given.as_ref(), at, &recovery), given) {
                (Some(t), _, _) => drop(t.take(at, given.as_ref().map(|g| &g.step))?),
                (None, Some(freeze), Some(g)) => {
                    let copy = connection(g.copy.as_deref().context("a freeze carries its copy")?)?;
                    ensure!(crate::db::state_hash(&copy)? == freeze.hash, "{YOURS}'s freeze at step {at}, which its recovery key signed, isn't what it says");
                    truth = Some(Truth { freeze, at, db: normalize(copy, &Scope::Global, now)? });
                }
                _ => {}
            }
        }
        Ok(truth)
    }

    /// Step `at` of `global`'s database, before its freeze: the freeze, if the recovery key signed it (`claimed`,
    /// judged from `log`, `global`'s log read past the end it names, and looked at only if a key in `recovery`
    /// signed it), checked against the same (`anchor`). Any other step is nothing. A freeze the recovery key signed
    /// that doesn't check out stops the reader here, as a newer release's does (`Newer`): it's never passed over for
    /// another, which a reader that it did check out for would never take.
    pub fn first(at: u64, given: Option<&Given>, root: Option<Hash>, log: Option<&Log>, recovery: &Recovery, now: u64) -> Result<Option<Truth>> {
        let (Some(wants), Some(g)) = (candidate(given, at, recovery), given) else { return Ok(None) };
        let log = log.with_context(|| format!("who's who waits for global's log to step {}, after where its freeze says it ends", wants.global() + 1))?;
        let Some(freeze) = claimed(given, at, root, log, recovery)? else { return Ok(None) };
        let db = anchor(g, at, root, log, now).with_context(|| format!("{YOURS}'s freeze at step {at}, which its recovery key signed, doesn't check out: this device reads who's who no further"))?;
        Ok(Some(Truth { freeze, at, db }))
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

    /// Who's who as `welcome` says it: from `freeze`, the step at its freeze's position, which must be a freeze the
    /// recovery key signed that checks out (`first`, with `log`, `global`'s log read past the end it names, of the
    /// network at `root`, whose entries say which keys were ever the recovery key), and `start`, the copy at `Welcome::copy` (its bytes, opened), or none to start from the
    /// freeze's own, then `after`, the steps after that one up to the Welcome's, each taken as any reader takes it.
    /// Fails unless it comes to the Welcome's hash, in this release's form: a freeze, copy or step a node or another
    /// device swapped is never used.
    #[allow(clippy::too_many_arguments)]
    pub fn welcomed(welcome: &Welcome, freeze: &Given, root: Option<Hash>, log: Option<&Log>, start: Option<&[u8]>, after: &[Option<Given>], now: u64) -> Result<Truth> {
        ensure!(u64::from(freeze.step.seq()) == welcome.freeze, "that isn't the freeze it was welcomed with");
        let recovery = log.map(Log::recovery).unwrap_or_default();
        let anchored = Truth::first(welcome.freeze, Some(freeze), root, log, &recovery, now)?.context("the step it was welcomed with isn't a freeze its recovery key signed")?;
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

/// Who's who as `welcome` says it, read from the bucket with `keys`, `global`'s log's keys by epoch, of the network at
/// `root`, with `global`'s log read past the end its freeze names (`Truth::welcomed`): from the copy before the
/// Welcome's step, or, if that one isn't there or doesn't come to its hash, from its freeze's.
pub async fn welcomed(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, root: Option<Hash>, welcome: &Welcome, now: u64) -> Result<Truth> {
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
    let end = Freeze::of(&freeze.step, welcome.freeze).context("the step it was welcomed with isn't a freeze")?.global();
    let log = log_to(bucket, keys, root, end).await?;
    let log = Some(&log);
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
                match Truth::welcomed(welcome, &freeze, root, log, Some(&bytes), &after, now) {
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
    Truth::welcomed(welcome, &freeze, root, log, None, &after, now)
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

    /// `global`'s entries, each written after every one before it, as they'd be in its log's steps, one a step; and
    /// the freezes' marks among them.
    #[derive(Clone, Default)]
    struct History {
        entries: Vec<Entry>,
        heads: std::collections::HashMap<Key, crate::entry::Head>,
        log: Log,
    }

    impl History {
        fn write(&mut self, key: &SigningKey, user: &str, content: Content) -> Entry {
            self.write_seeing(key, user, content, self.entries.len())
        }

        /// As `write`, from a device that has read only the first `seen` entries: it follows only those.
        fn write_seeing(&mut self, key: &SigningKey, user: &str, content: Content, seen: usize) -> Entry {
            let me = key.verifying_key().to_bytes();
            let previous = self.entries[..seen].iter().filter(|e| e.author() != me).map(Entry::hash).collect();
            let e = Entry::create(key, Ext { log: "global".into(), time: 0, user: user.into(), epoch: 1, previous }, self.heads.get(&me).copied(), content);
            self.heads.insert(me, e.head());
            self.entries.push(e.clone());
            self.log.push(vec![e.clone()]);
            e
        }

        /// An entry signed again at the seq of `key`'s last: a fork.
        fn fork(&mut self, key: &SigningKey, user: &str, content: Content) {
            let me = key.verifying_key().to_bytes();
            let mine: Vec<Entry> = self.entries.iter().filter(|e| e.author() == me).cloned().collect();
            let last = mine.last().unwrap();
            let head = mine.len().checked_sub(2).map(|i| mine[i].head());
            let again = Entry::create(key, Ext { log: "global".into(), time: 1, user: user.into(), epoch: 1, previous: last.previous().to_vec() }, head, content);
            assert_eq!(again.seq(), last.seq());
            self.entries.push(again.clone());
            self.log.push(vec![again]);
        }

        /// Its log as a reader reads it from the bucket now, to its end.
        fn log(&self) -> Log {
            Log { ended: true, ..self.log.clone() }
        }

        /// A freeze `key` signs as `user` at step `at` of `global`'s database, of the log up to step `to`, as the
        /// ceremony writes one, without its mark.
        fn freeze_to(&self, key: &SigningKey, user: &str, at: u64, to: u64) -> Given {
            let (content, copy) = freeze(Some(self.root()), self.log.entries(to), to, BTreeMap::new(), 0).unwrap();
            Given { step: sign(key, &Scope::Global, at, user, 1, 0, content).unwrap(), copy: Some(copy) }
        }

        /// A freeze of the log to its end, with its mark written there first, as the ceremony writes one.
        fn freeze(&mut self, key: &SigningKey, user: &str, at: u64) -> Given {
            let given = self.freeze_to(key, user, at, self.log.last());
            self.mark(key, user, &given);
            given
        }

        /// `given`'s mark, at the log's end.
        fn mark(&mut self, key: &SigningKey, user: &str, given: &Given) {
            self.log.push_mark(sign(key, &Scope::Global, self.log.last() + 1, user, 1, 0, given.step.content.clone()).unwrap());
        }

        fn root(&self) -> Hash {
            self.entries[0].hash()
        }

        /// Every key its entries say was ever the recovery key, as a reader that holds them all works it out.
        fn recovery(&self) -> Recovery {
            recovery_keys(Some(self.root()), &self.entries)
        }

        /// What a reader of `steps` with the log as it is now takes: none before a freeze counts.
        fn read(&self, steps: &[Given]) -> Result<Option<Truth>> {
            let steps: Vec<Option<Given>> = steps.iter().cloned().map(Some).collect();
            Truth::read(&steps, Some(self.root()), Some(&self.log()), &self.recovery(), 0)
        }
    }

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";

    /// Andre's Mac starts the network, its recovery key kept offline, and lists Bob's laptop, Bob an admin too.
    fn two_admins() -> (History, SigningKey, SigningKey, SigningKey) {
        use crate::keys::Endpoint;
        use crate::members::{Kind, device, genesis};
        let (mac, laptop, recovery) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[7; 32]));
        let mut h = History { log: Log::new(None), ..Default::default() };
        h.write(&mac, ANDRE, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &recovery.verifying_key().to_bytes(), &"e".repeat(64)));
        h.log.root = Some(h.root());
        h.write(&mac, ANDRE, device(&Endpoint::of(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Admin))));
        h.write(&mac, ANDRE, members::gate());
        (h, mac, laptop, recovery)
    }

    /// `two_admins`, before an admin's device said the gate is open.
    fn before_the_gate() -> (History, SigningKey, SigningKey, SigningKey) {
        let (mut h, mac, laptop, recovery) = two_admins();
        h.entries.pop();
        h.heads.insert(mac.verifying_key().to_bytes(), h.entries[1].head());
        h.log = Log { root: h.log.root, ..Log::default() };
        h.log.push(vec![h.entries[0].clone()]);
        h.log.push(vec![h.entries[1].clone()]);
        (h, mac, laptop, recovery)
    }

    fn key_of(k: &SigningKey) -> String {
        crate::entry::hex(&k.verifying_key().to_bytes())
    }

    /// A recovery key that forked `global` is none: R1, replaced by R2, signs another handover at the same seq, so every
    /// reader cuts it from there and its handover to R2 never counts. It isn't the recovery key again: it freezes
    /// nothing, and nothing is sealed to it. With none, an admin's own device names R3, once, and R3's freeze counts.
    #[test]
    fn a_recovery_key_that_forked_is_none_and_an_admin_names_the_next() {
        let (mut h, mac, laptop, r1) = two_admins();
        let (r2, thief, r3) = (SigningKey::from_bytes(&[8; 32]), SigningKey::from_bytes(&[6; 32]), SigningKey::from_bytes(&[5; 32]));
        h.write(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&r2) })));
        h.fork(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&thief) })));
        let root = Some(h.root());
        let m = members_of(root, &h.entries);
        assert_eq!(m.recovery, None, "no recovery key");
        assert!(m.recovery_was.contains(&r1.verifying_key().to_bytes()), "R1 is one no more");
        let db = ScopeDb::build(&m, &Scope::Global, &ScopeState::default(), 0).unwrap();
        assert_eq!(db.recovery(), None, "so no epoch is sealed to it");
        let r1s = h.freeze(&r1, ANDRE, 1);
        assert!(claimed(Some(&r1s), 1, root, &h.log(), &h.recovery()).unwrap().is_none(), "R1 freezes nothing");
        let thiefs = h.freeze(&thief, ANDRE, 2);
        assert!(claimed(Some(&thiefs), 2, root, &h.log(), &h.recovery()).unwrap().is_none(), "nor does the key it forked to");
        // Bob, an admin, names R3 from his own device; Andre's, after, names none: there's one.
        h.write(&laptop, BOB, members::node("recovery", json!({ "key": key_of(&r3) })));
        h.write(&mac, ANDRE, members::node("recovery", json!({ "key": key_of(&r2) })));
        let m = members_of(root, &h.entries);
        assert_eq!(m.recovery, Some(r3.verifying_key().to_bytes()), "an admin named R3");
        assert!(h.recovery().contains(&r3.verifying_key().to_bytes()), "and its freeze is looked at");
        let r3s = h.freeze(&r3, ANDRE, 3);
        assert_eq!(h.read(&[r1s, thiefs, r3s]).unwrap().unwrap().at, 3, "R3's is the freeze");
    }

    /// Where `e` sorts among `entries` in replay order.
    fn place(entries: &[Entry], e: &Entry) -> usize {
        crate::entry::causal(entries.to_vec(), &Default::default()).iter().position(|o| o.hash() == e.hash()).unwrap()
    }

    /// R1 hands over to R2, and forks only after, so its handover counts. Bob's laptop, idle since before it, names KA
    /// in an entry that sorts before the handover: R1 hadn't yet passed its last entry before its fork there, so it
    /// counts for nothing, R2 stays the recovery key, and KA freezes nothing.
    #[test]
    fn a_key_that_forked_after_its_handover_lets_no_one_name_the_next_before_it() {
        let (mut h, _, laptop, r1) = two_admins();
        let (r2, thief, ka) = (SigningKey::from_bytes(&[8; 32]), SigningKey::from_bytes(&[6; 32]), SigningKey::from_bytes(&[4; 32]));
        let handover = h.write(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&r2) })));
        h.write(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&thief) })));
        h.fork(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&ka) })));
        // Bob's laptop had read Andre's first two entries only.
        let named = h.write_seeing(&laptop, BOB, members::node("recovery", json!({ "key": key_of(&ka) })), 2);
        assert!(place(&h.entries, &named) < place(&h.entries, &handover), "it sorts before the handover");
        let m = members_of(Some(h.root()), &h.entries);
        assert_eq!(m.recovery, Some(r2.verifying_key().to_bytes()), "R2 is the recovery key");
        let kas = h.freeze(&ka, BOB, 1);
        let r2s = h.freeze(&r2, ANDRE, 2);
        assert_eq!(h.read(&[kas, r2s]).unwrap().unwrap().at, 2, "R2's is the freeze");
    }

    /// R1 forks having handed over to no one: an admin's naming that sorts before R1's last entry before its fork
    /// counts for nothing, and one after it, from a device that has read the fork, names R3.
    #[test]
    fn an_admin_names_the_next_only_after_the_forked_key_s_last_entry_before_its_fork() {
        let (mut h, mac, laptop, r1) = two_admins();
        let (thief, ka, r3) = (SigningKey::from_bytes(&[6; 32]), SigningKey::from_bytes(&[4; 32]), SigningKey::from_bytes(&[5; 32]));
        // R1's first: one that changes nothing (its "new" key is a device's).
        let first = h.write(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&mac) })));
        h.write(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&thief) })));
        h.fork(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&thief) })));
        let early = h.write_seeing(&laptop, BOB, members::node("recovery", json!({ "key": key_of(&ka) })), 2);
        assert!(place(&h.entries, &early) < place(&h.entries, &first));
        let m = members_of(Some(h.root()), &h.entries);
        assert_eq!(m.recovery, None, "not KA: R1 is taken away, and no one named the next");
        h.write(&mac, ANDRE, members::node("recovery", json!({ "key": key_of(&r3) })));
        assert_eq!(members_of(Some(h.root()), &h.entries).recovery, Some(r3.verifying_key().to_bytes()), "an admin who read the fork names R3");
    }

    /// Bob, an admin when his laptop last read the log, opens the gate in an entry that sorts before Andre demotes
    /// him: once replayed, he's no admin, so the gate isn't open, and the recovery key's freeze counts for nothing.
    #[test]
    fn a_gate_from_someone_demoted_opens_nothing() {
        let (mut h, mac, laptop, r1) = before_the_gate();
        h.write(&mac, ANDRE, members::setting("name", Some("x")));
        let demoted = h.write(&mac, ANDRE, members::role(BOB, Role::Member));
        let gate = h.write_seeing(&laptop, BOB, members::gate(), 2);
        assert!(place(&h.entries, &gate) < place(&h.entries, &demoted), "it sorts before his demotion");
        assert!(!members_of(Some(h.root()), &h.entries).gate());
        let early = h.freeze(&r1, ANDRE, 1);
        assert!(h.read(std::slice::from_ref(&early)).unwrap().is_none());
        h.write(&mac, ANDRE, members::gate());
        let later = h.freeze(&r1, ANDRE, 2);
        assert_eq!(h.read(&[early, later]).unwrap().unwrap().at, 2, "an admin's gate opens it");
    }

    /// A device's second entry at one seq that follows an entry no one holds is no fork, to a node's store or to a
    /// reader of the entries, since neither ever places it; one that follows what's there is one, to both.
    #[test]
    fn a_fork_is_the_same_to_a_node_s_store_and_a_reader_of_the_entries() {
        let (mut h, mac, _, _) = two_admins();
        let me = mac.verifying_key().to_bytes();
        let mine: Vec<Entry> = h.entries.iter().filter(|e| e.author() == me).cloned().collect();
        let ext = Ext { log: "global".into(), time: 1, user: ANDRE.into(), epoch: 1, previous: vec![Hash::from_bytes([9; 32])] };
        let unplaced = Entry::create(&mac, ext, Some(mine[mine.len() - 2].head()), members::setting("name", Some("x")));
        assert_eq!(unplaced.seq(), mine.last().unwrap().seq());
        let store = |entries: &[Entry]| {
            let store = crate::store::Store::memory().unwrap();
            store.take(entries.to_vec()).unwrap();
            store.forks().unwrap()
        };
        let entries = [h.entries.clone(), vec![unplaced]].concat();
        assert!(store(&entries).is_empty() && forks(&entries).is_empty(), "no fork to either");
        h.fork(&mac, ANDRE, members::setting("name", Some("y")));
        let cut = BTreeMap::from([((me, "global".to_string()), mine.last().unwrap().seq())]);
        assert_eq!((store(&h.entries), forks(&h.entries)), (cut.clone(), cut), "the same fork to both");
    }

    /// Once frozen, with no recovery key, the scope database keeps the same: an admin's own device names the next, and
    /// only while there's none.
    #[test]
    fn with_no_recovery_key_an_admin_s_step_names_the_next() {
        let (mut h, mac, laptop, r1) = two_admins();
        let (r2, r3) = (SigningKey::from_bytes(&[8; 32]), SigningKey::from_bytes(&[5; 32]));
        h.write(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&r2) })));
        h.fork(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&r1) })));
        let (content, copy) = freeze(Some(h.root()), &h.entries, h.log.last(), BTreeMap::new(), 0).unwrap();
        let db = ScopeDb::open(connection(&copy).unwrap(), 0).unwrap();
        let mut t = Truth { freeze: Freeze::from_data(&content.data, 1).unwrap(), at: 1, db };
        assert_eq!(t.db.recovery(), None);
        let step = |key: &SigningKey, user: &str, at: u64, content: Content| sign(key, &Scope::Global, at, user, 1, 0, content).unwrap();
        assert!(!t.take(2, Some(&step(&r1, ANDRE, 2, members::node("recovery", json!({ "key": key_of(&r1) }))))).unwrap(), "not R1");
        assert!(t.take(3, Some(&step(&laptop, BOB, 3, members::node("recovery", json!({ "key": key_of(&r3) }))))).unwrap(), "an admin's");
        assert_eq!(t.db.recovery(), Some(r3.verifying_key().to_bytes()));
        assert!(!t.take(4, Some(&step(&mac, ANDRE, 4, members::node("recovery", json!({ "key": key_of(&r2) }))))).unwrap(), "only while there's none");
    }

    /// A freeze counts only if the log it names ends after an admin's own device said the gate is open: one R1 planted
    /// before, marked and all, while it was the recovery key, counts for nothing once it's replaced, nor ever.
    #[test]
    fn a_freeze_from_before_the_gate_counts_for_nothing() {
        let (mut h, mac, _, r1) = before_the_gate();
        let planted = h.freeze(&r1, ANDRE, 1);
        assert!(claimed(Some(&planted), 1, Some(h.root()), &h.log(), &h.recovery()).unwrap().is_none(), "before the gate");
        let r2 = SigningKey::from_bytes(&[8; 32]);
        h.write(&r1, ANDRE, members::node("recovery", json!({ "key": key_of(&r2) })));
        h.write(&mac, ANDRE, members::gate());
        let theirs = h.freeze(&r2, ANDRE, 2);
        assert_eq!(h.read(&[planted, theirs]).unwrap().unwrap().at, 2, "R2's, after the gate, is the freeze");
    }

    /// Only an admin's own device opens the gate.
    #[test]
    fn only_an_admin_opens_the_gate() {
        let (mut h, mac, laptop, r1) = before_the_gate();
        h.write(&mac, ANDRE, members::role(BOB, Role::Member));
        h.write(&laptop, BOB, members::gate());
        assert!(!members_of(Some(h.root()), &h.entries).gate(), "not a member's");
        let f = h.freeze(&r1, ANDRE, 1);
        assert!(h.read(&[f]).unwrap().is_none());
    }

    /// A genesis a device writes into the log is never the network's for a reader that holds none of its own (the
    /// Cloudflare node): the one pinned is, and unpinned, none while there are two.
    #[test]
    fn a_planted_genesis_is_never_the_networks() {
        use crate::keys::Endpoint;
        use crate::members::{Kind, genesis};
        let (h, _, laptop, _) = two_admins();
        let mine = SigningKey::from_bytes(&[3; 32]);
        let planted = Entry::create(&laptop, Ext { log: "global".into(), time: 0, user: BOB.into(), epoch: 1, previous: Vec::new() }, None, genesis("Mine", BOB, "Bob", &Endpoint::of(&laptop), "laptop", &[Kind::Client], &mine.verifying_key().to_bytes(), &"f".repeat(64)));
        let mut entries = vec![planted.clone()];
        entries.extend(h.entries.iter().cloned());
        assert_eq!(genesis_of(Some(h.root()), &entries), Some(h.root()), "the one pinned");
        assert_eq!(genesis_of(None, &entries), None, "unpinned, neither");
        assert_eq!(genesis_of(None, &h.entries), Some(h.root()));
        assert_eq!(members_of(genesis_of(Some(h.root()), &entries), &entries).recovery, Some(SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes()));
    }

    /// A freeze naming a log's end past 2⁵³ isn't one, so no reader reads to the step after it, which wraps.
    #[test]
    fn a_freeze_naming_an_end_past_any_log_is_none() {
        let (mut h, _, _, recovery) = two_admins();
        let mut far = h.freeze(&recovery, ANDRE, 1);
        let mut data = far.step.content.data.clone();
        data["logs"]["global"] = json!(u64::MAX);
        far.step = sign(&recovery, &Scope::Global, 1, ANDRE, 1, 0, Content::new(members::APP, FREEZE, data.clone())).unwrap();
        assert!(Freeze::of(&far.step, 1).is_err());
        assert_eq!(wants_log(&[Some(far.clone())], Some(h.root()), None, &h.recovery()), None, "no log is read for it");
        assert!(Truth::read(&[Some(far)], Some(h.root()), None, &h.recovery(), 0).unwrap().is_none());
        data["logs"]["global"] = json!(MOST_STEPS);
        assert!(Freeze::from_data(&data, 1).is_ok() && Freeze::from_data(&json!({ "hash": "", "logs": { "global": MOST_STEPS + 1 } }), 1).is_err());
    }

    /// An admin's own device can't freeze the log: one Bob's laptop plants while he's an admin, marked and all, is
    /// nothing, so it undoes nothing written after it, and the recovery key's, later, is the freeze.
    #[test]
    fn an_admin_s_planted_freeze_counts_for_nothing_and_a_later_demotion_holds() {
        let (mut h, mac, laptop, recovery) = two_admins();
        let planted = h.freeze(&laptop, BOB, 1);
        h.write(&mac, ANDRE, members::role(BOB, Role::Member));
        assert!(claimed(Some(&planted), 1, Some(h.root()), &h.log(), &h.recovery()).unwrap().is_none(), "an admin's freeze is nothing");
        let theirs = h.freeze(&recovery, ANDRE, 2);
        let truth = h.read(&[planted, theirs]).unwrap().unwrap();
        assert_eq!((truth.at, truth.db.user(BOB).map(|u| u.role)), (2, Some(Role::Member)), "the recovery key's is the freeze, and Bob's demotion holds");
    }

    /// Anything shaped like a freeze that a key that was never the recovery key signed is passed over before the log
    /// it names is read: one naming a log end of 10¹² holds no reader up, as it waits for no log.
    #[test]
    fn a_junk_freeze_naming_a_far_log_end_holds_no_reader_up() {
        let (mut h, _, laptop, recovery) = two_admins();
        let mut junk = h.freeze_to(&laptop, BOB, 1, h.log.last());
        let mut data = junk.step.content.data.clone();
        data["logs"]["global"] = json!(1_000_000_000_000u64);
        junk.step = sign(&laptop, &Scope::Global, 1, BOB, 1, 0, Content::new(members::APP, FREEZE, data)).unwrap();
        let root = Some(h.root());
        assert_eq!(wants_log(&[Some(junk.clone())], root, None, &h.recovery()), None, "no log is read for it");
        assert!(Truth::read(&[Some(junk.clone())], root, None, &h.recovery(), 0).unwrap().is_none() && Truth::judged(&[Some(junk.clone())], root, None, &h.recovery(), 0).unwrap().is_none());
        let theirs = h.freeze(&recovery, ANDRE, 2);
        assert_eq!(wants_log(&[Some(junk.clone()), Some(theirs.clone())], root, None, &h.recovery()), Some(theirs.step.content.data["logs"]["global"].as_u64().unwrap()), "only the recovery key's");
        assert_eq!(h.read(&[junk, theirs]).unwrap().unwrap().at, 2);
    }

    /// A recovery key replaced by a `recovery` entry freezes nothing: not at the log's end, where it isn't the
    /// recovery key, nor from before it was replaced, which has no step free for its mark. The one it was replaced by
    /// does.
    #[test]
    fn a_replaced_recovery_key_s_freeze_counts_for_nothing() {
        let (mut h, _, _, old) = two_admins();
        let new = SigningKey::from_bytes(&[8; 32]);
        let before = h.log.last();
        h.write(&old, ANDRE, members::node("recovery", json!({ "key": crate::entry::hex(&new.verifying_key().to_bytes()) })));
        assert_eq!(h.recovery(), Recovery::from([old.verifying_key().to_bytes(), new.verifying_key().to_bytes()]), "both are looked at");
        let from_before = h.freeze_to(&old, ANDRE, 1, before);
        assert!(claimed(Some(&from_before), 1, Some(h.root()), &h.log(), &h.recovery()).unwrap().is_none(), "from before it was replaced");
        let at_end = h.freeze(&old, ANDRE, 1);
        assert!(claimed(Some(&at_end), 1, Some(h.root()), &h.log(), &h.recovery()).unwrap().is_none(), "from the log's end");
        let theirs = h.freeze(&new, ANDRE, 2);
        assert_eq!(h.read(&[at_end, theirs]).unwrap().unwrap().at, 2, "the new one's is the freeze");
    }

    /// A service endpoint, which takes its steps from its node and can't read the log, takes only the freeze the
    /// recovery key signed, its copy checked against the hash it names: an admin's is nothing, and the recovery key's
    /// whose copy isn't what it says stops it.
    #[test]
    fn a_service_takes_only_the_recovery_key_s_freeze() {
        let (mut h, mac, _, recovery) = two_admins();
        let admins = h.freeze(&mac, ANDRE, 1);
        let key = Some(recovery.verifying_key().to_bytes());
        assert!(Truth::read_given(&[Some(admins.clone())], key, 0).unwrap().is_none(), "an admin's is nothing");
        let theirs = h.freeze(&recovery, ANDRE, 2);
        assert_eq!(Truth::read_given(&[Some(admins.clone()), Some(theirs.clone())], key, 0).unwrap().unwrap().at, 2);
        let mut swapped = theirs;
        let mut db = connection(swapped.copy.as_deref().unwrap()).unwrap();
        db.execute_batch("UPDATE devices SET name = 'mine' WHERE name = 'laptop'").unwrap();
        swapped.copy = Some(db.serialize("main").unwrap().to_vec());
        assert!(Truth::read_given(&[Some(swapped)], key, 0).is_err(), "a copy that isn't what it says stops it");
    }

    /// A freeze is judged at the end of the log it names only, so every reader takes the same one whenever it reads it:
    /// a fork, a demotion or anything else written to the log after it changes nothing about it.
    #[test]
    fn a_freeze_counts_the_same_whenever_it_s_read() {
        let (mut h, mac, laptop, recovery) = two_admins();
        h.write(&laptop, BOB, members::setting("name", Some("Ours")));
        let freeze = h.freeze(&recovery, ANDRE, 1);
        let soon = h.read(std::slice::from_ref(&freeze)).unwrap().unwrap();
        // Then, in the log after it: Bob demotes Andre, and Bob's laptop and Andre's Mac, restored from older backups,
        // fork.
        h.write(&laptop, BOB, members::role(ANDRE, Role::Member));
        h.fork(&laptop, BOB, members::setting("name", Some("Forked")));
        h.fork(&mac, ANDRE, members::setting("name", Some("Forked")));
        let late = h.read(&[freeze]).unwrap().unwrap();
        let hash = |t: &Truth| crate::db::state_hash(t.db.conn()).unwrap();
        assert_eq!(hash(&late), hash(&soon), "the same who's who, whenever it's read");
    }

    /// The recovery key's freeze whose copy isn't what the log says stops its reader there: never passed over for the
    /// next, which a reader it did check out for would never take. As does one whose mark isn't there yet, or whose
    /// log it hasn't read.
    #[test]
    fn a_recovery_key_s_freeze_that_doesn_t_check_out_stops_its_reader() {
        let (mut h, _, _, recovery) = two_admins();
        let forged = forged(&h, &recovery, 1);
        let unmarked = h.log();
        h.mark(&recovery, ANDRE, &forged);
        let next = h.freeze(&recovery, ANDRE, 2);
        assert!(h.read(&[forged.clone(), next]).is_err(), "a copy that isn't what the log says stops it");
        let err = Truth::read(&[Some(forged.clone())], Some(h.root()), Some(&unmarked), &h.recovery(), 0).err().unwrap();
        assert!(err.is::<Unmarked>(), "nothing where its mark would be stops it, until something is: {err:#}");
        assert!(Truth::read(&[Some(forged)], Some(h.root()), None, &h.recovery(), 0).is_err(), "as does one whose log it hasn't read");
    }

    /// The recovery key's freeze of `h`'s log to its end at `at`, its copy saying Bob's laptop is named otherwise.
    fn forged(h: &History, recovery: &SigningKey, at: u64) -> Given {
        let given = h.freeze_to(recovery, ANDRE, at, h.log.last());
        let mut db = connection(given.copy.as_deref().unwrap()).unwrap();
        db.execute_batch("UPDATE devices SET name = 'mine' WHERE name = 'laptop'").unwrap();
        let mut data = given.step.content.data.clone();
        data["hash"] = json!(crate::db::state_hash(&db).unwrap());
        Given { step: sign(recovery, &Scope::Global, at, ANDRE, 1, 0, Content::new(members::APP, FREEZE, data)).unwrap(), copy: Some(db.serialize("main").unwrap().to_vec()) }
    }

    /// A reader that answers while it reads, and keeps nothing (the Cloudflare node), takes who's who and the epochs
    /// made from the entries while it hasn't read `global`'s log far enough to judge the recovery key's freeze, or its
    /// mark isn't there yet; only one that doesn't check out stops it.
    #[test]
    fn a_reader_that_answers_as_it_reads_judges_a_freeze_once_it_can() {
        let (mut h, _, _, recovery) = two_admins();
        let root = Some(h.root());
        let mut bad = h.clone();
        let theirs = h.freeze_to(&recovery, ANDRE, 1, h.log.last());
        assert!(Truth::read(&[Some(theirs.clone())], root, None, &h.recovery(), 0).is_err(), "a reader that keeps what it reads waits for the log");
        assert!(Truth::judged(&[Some(theirs.clone())], root, None, &h.recovery(), 0).unwrap().is_none(), "before the log is read");
        assert!(Truth::judged(&[Some(theirs.clone())], root, Some(&h.log()), &h.recovery(), 0).unwrap().is_none(), "nor while nothing is where its mark would be");
        h.mark(&recovery, ANDRE, &theirs);
        assert_eq!(Truth::judged(&[Some(theirs)], root, Some(&h.log()), &h.recovery(), 0).unwrap().unwrap().at, 1);
        let given = forged(&bad, &recovery, 1);
        bad.mark(&recovery, ANDRE, &given);
        assert!(Truth::judged(&[Some(given)], root, Some(&bad.log()), &bad.recovery(), 0).is_err(), "one that doesn't check out stops it");
    }

    /// A step of `global`'s log under an epoch the entries before it hadn't made is junk to every reader, whenever it
    /// reads it: one that holds that epoch's key, made since, reads the log as one that doesn't. So does each scope's log
    /// up to a freeze's end of it, under the epochs made at the freeze, whatever was made since.
    #[test]
    fn the_log_under_a_freeze_reads_the_same_whenever_it_s_read() {
        use crate::bucket::Bucket;
        let (mut h, mac, laptop, _) = two_admins();
        let (k1, k2) = ([1; 32], [2; 32]);
        let user = Scope::User(ANDRE.into());
        let (kx1, kx2) = ([11; 32], [12; 32]);
        let mac_key = mac.verifying_key().to_bytes();
        let mut global: Vec<(u32, [u8; 32], Vec<Entry>)> = vec![(1, k1, h.entries.clone())];
        // An admin seals a step under epoch 2 with a key they chose, before epoch 2 is made.
        let early = Entry::create(&laptop, Ext { log: "global".into(), time: 0, user: BOB.into(), epoch: 1, previous: Vec::new() }, None, members::setting("name", Some("Early")));
        global.push((2, k2, vec![early]));
        global.push((1, k1, vec![h.write(&mac, ANDRE, members::setting("name", Some("Ours")))]));
        global.push((1, k1, vec![h.write(&mac, ANDRE, members::epoch(&user, 1, &[mac_key], &crate::keys::epoch_hash(&[21; 32])))]));
        global.push((1, k1, vec![h.write(&mac, ANDRE, members::epoch(&Scope::Global, 2, &[mac_key], &crate::keys::epoch_hash(&[22; 32])))]));
        global.push((2, k2, vec![h.write(&mac, ANDRE, members::setting("name", Some("After")))]));
        let note = |n: u32| {
            let content = Content::new("notes", "note", json!({ "n": n }));
            Entry::create(&mac, Ext { log: user.to_string(), time: 0, user: ANDRE.into(), epoch: 1, previous: Vec::new() }, None, content)
        };
        // The scope's log: a step under its epoch 2, made only after the freeze, between two under epoch 1.
        let theirs: Vec<(u32, [u8; 32], Vec<Entry>)> = vec![(1, kx1, vec![note(1)]), (2, kx2, vec![note(2)]), (1, kx1, vec![note(3)])];
        let bucket = crate::bucket::Memory::default();
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            for (scope, steps) in [("global".to_string(), &global), (user.to_string(), &theirs)] {
                for (i, (epoch, key, entries)) in steps.iter().enumerate() {
                    let name = crate::db::step_name(&crate::log::prefix(&scope), i as u64 + 1);
                    bucket.create(&name, &crate::keys::seal(*epoch, key, &name, &crate::entry::pack(&entries.iter().collect::<Vec<_>>()))).await.unwrap();
                }
            }
            let names = |log: &Log, to: u64| log.entries(to).iter().filter_map(|e| e.content.data["value"].as_str().map(str::to_string)).collect::<Vec<_>>();
            // Before epoch 2 is made, a reader holds only epoch 1's key; after, both.
            let before: crate::db::Keys = [(1, k1)].into();
            let after: crate::db::Keys = [(1, k1), (2, k2)].into();
            let early = log_to(&bucket, &before, Some(h.root()), 3).await.unwrap();
            let late = log_to(&bucket, &after, Some(h.root()), 3).await.unwrap();
            assert_eq!(names(&early, 4), vec!["Ours"]);
            assert_eq!(names(&late, 4), names(&early, 4), "the same steps, whenever they're read");
            let whole = log_to(&bucket, &after, Some(h.root()), 10).await.unwrap();
            assert_eq!(names(&whole, whole.last()), vec!["Ours", "After"], "and one under epoch 2 once it's made");
            let new = log_to(&bucket, &after, None, 10).await.unwrap();
            assert_eq!(names(&new, new.last()), names(&whole, whole.last()), "by a device that doesn't know the network yet too");
            let freeze = Freeze { at: 1, hash: String::new(), logs: [("global".to_string(), 5), (user.to_string(), 3)].into() };
            let ns = |entries: Vec<Entry>| entries.iter().map(|e| e.content.data["n"].as_u64().unwrap()).collect::<Vec<_>>();
            let early = anchor_entries(&bucket, &[(1, kx1)].into(), &freeze, Some(h.root()), &whole, &user).await.unwrap();
            let late = anchor_entries(&bucket, &[(1, kx1), (2, kx2)].into(), &freeze, Some(h.root()), &whole, &user).await.unwrap();
            assert_eq!((ns(early), ns(late)), (vec![1, 3], vec![1, 3]), "a scope's log under the epochs made at the freeze");
        });
    }

    /// A newer release's step in a scope's own database stops its reader before it, as in `global`'s: nothing after it
    /// is taken as if it weren't there.
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
