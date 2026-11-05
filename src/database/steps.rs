//! A scope database's steps, once `global`'s log is frozen (`switch`): the database is the truth, and every change to
//! it is a step at the next free position of its own in the bucket, which the bucket keeps in one order.
//!
//! A step is one change, signed by the device that made it as its person: the same content an entry of the log carried
//! (`members`' kinds, and a scope's own), for one position, under `entry::STEP_DOMAIN`, so it's never taken for an
//! entry nor an entry for it. Its position is signed in it (its `seq`), so a step can't be written twice, or anywhere
//! but where its writer meant it: one that loses its position to another is signed again for the next. A node only
//! carries a client's step to the bucket: it can hold one back, but change none.
//!
//! The cut-over is `global`'s first step that is a freeze an admin's own device signed (`FREEZE`, `claimed`): it names
//! the step of `global`'s log where the log ends (and of every other scope's log the last step in the bucket), and
//! carries the database there, its first copy. Every reader replays exactly the entries in `global`'s log's steps up to
//! it (`Log`), forks only among those, so every reader replays the same ones whatever it holds or reads later; checks
//! the signer was an admin's own device there, and still is in everything the reader holds; and that the copy says
//! what those entries add up to (`anchor`). Then it keeps it: from then on it reads the copy it keeps and the steps
//! after, never the entries again. A freeze an admin signed that doesn't check out stops the reader there, as a newer
//! release's step does, never passed over for the next; anyone else's is nothing. A `FREEZING` step before it tells
//! writers of the log to look whether what they wrote is inside it.
//! Every other scope's database starts, before its first step, from what its log's entries up to the freeze's position
//! there add up to (`scope_anchor`), which every reader of it works out itself: no writer chooses it.
//!
//! Every reader applies each step after to the database as the steps before it left it, and counts it only if all
//! three of the protocol's rules hold there:
//! 1. the install's first step and its recovery key: the freeze's copy, checked as above; and only the recovery key
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
use super::switch;

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

/// A step a device of the install's own passed on opened (`encode`'s bytes): none if it isn't one. Only a service
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
    /// Nothing that will ever be one: it doesn't open with its epoch's key, or isn't a step for its place.
    Junk,
    /// Under an epoch whose key the reader doesn't have: it reads on from here once it does.
    Waits(u32),
}

/// Step `at` of `scope`'s database, sealed as the bucket holds it, opened with `keys`.
pub fn open(keys: &crate::db::Keys, scope: &Scope, at: u64, sealed: &[u8]) -> Opened {
    let name = crate::db::step_name(&prefix(&scope.to_string()), at);
    let Ok(epoch) = crate::keys::epoch_of(sealed) else { return Opened::Junk };
    let Some(key) = keys.get(&epoch) else { return Opened::Waits(epoch) };
    match crate::keys::open(key, &name, sealed).and_then(|b| decode(scope, at, &b)) {
        Ok(given) => Opened::Step(given),
        Err(err) => {
            tracing::debug!("{name}: {err:#}");
            Opened::Junk
        }
    }
}

/// Reads `scope`'s steps after `from` from the bucket, opened with the keys of its log by epoch: each by position, none
/// for one that will never be a step. Stops at the first position with nothing, or one under an epoch it has no key
/// for, which waits for it; fails at a position with nothing that a later step says was written, rather than read on
/// as if the steps ended there.
pub async fn read(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, scope: &Scope, from: u64) -> Result<Vec<Option<Given>>> {
    let prefix = prefix(&scope.to_string());
    let mut out = Vec::new();
    loop {
        let at = from + out.len() as u64 + 1;
        let Some(o) = bucket.get(&crate::db::step_name(&prefix, at)).await? else {
            missing(bucket, &prefix, at).await?;
            break;
        };
        match open(keys, scope, at, &o.bytes) {
            Opened::Step(given) => out.push(Some(given)),
            Opened::Junk => out.push(None),
            Opened::Waits(_) => break,
        }
    }
    Ok(out)
}

/// Fails if step `at` at `prefix`, which isn't in the bucket, was written: a later one is. Steps are written one after
/// another, so one gone is one deleted, and reading on as if it never was would pass over whatever it and those after
/// it changed. The newest hint is anyone's to write, so it says nothing here: one above the last step is junk.
pub async fn missing(bucket: &impl crate::bucket::Bucket, prefix: &str, at: u64) -> Result<()> {
    let later = bucket.get(&crate::db::step_name(prefix, at + 1)).await?.is_some();
    ensure!(!later, "step {at} of {prefix} isn't in the bucket, though later ones were written: this device reads no further until it's back");
    Ok(())
}

/// The kind of step that tells writers of the log that it's being frozen: the first a device of an admin's writes,
/// before it works out where the log ends. Changes nothing; a writer that finds one after it wrote an entry looks
/// whether the freeze that follows holds it (`Freeze::holds`), and once it has waited `node::steps`' while for one
/// that doesn't come, writes on: its admin's device writes the freeze within half that, or not at all.
pub const FREEZING: &str = "freezing";

/// The kind of step that freezes the log (`anchor`): `{ hash, logs }`, with its copy.
pub const FREEZE: &str = "freeze";

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
            logs.insert(scope.clone(), n.as_u64().context("a log's end is a position")?);
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

/// `global`'s log up to a freeze's end of it: the entries in its steps 1 to `to`, read from the steps themselves
/// (`log_to`), never a copy, nor what a reader happens to hold. Every reader of the freeze replays exactly these.
#[derive(Clone, Debug, Default)]
pub struct Log {
    pub to: u64,
    pub entries: Vec<Entry>,
}

/// `global`'s log up to step `to`, from the bucket (`Log`).
pub async fn log_to(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, to: u64) -> Result<Log> {
    let entries = crate::log::read_to(bucket, keys, &crate::log::prefix("global"), "global", to).await?;
    Ok(Log { to, entries })
}

/// `global`'s log to the end the freeze an admin signed among `steps` names (`wants_log`), as a reader that keeps
/// nothing and holds `ordered`, `global`'s entries in replay order (`view`), finds it, from the bucket: none before
/// there's one.
pub async fn log_for(bucket: &impl crate::bucket::Bucket, keys: &crate::db::Keys, steps: &[Option<Given>], root: Option<Hash>, ordered: &[Entry], now: u64) -> Result<Option<Log>> {
    let end = {
        let (view, cuts) = view(root, ordered, now)?;
        wants_log(steps, &view, &cuts)
    };
    match end {
        Some(end) => Ok(Some(log_to(bucket, keys, end).await?)),
        None => Ok(None),
    }
}

/// Each device that signed two entries at one seq of a log among `entries`, cut from the first: the forks a reader
/// that holds just those finds, as a node's store finds them (`Store::forks`).
pub fn forks(entries: &[Entry]) -> Cuts {
    let mut seen: BTreeMap<(Key, String, u32), Hash> = BTreeMap::new();
    let mut cuts = Cuts::new();
    for e in entries {
        if seen.insert((e.author(), e.log().to_string(), e.seq()), e.hash()).is_some_and(|h| h != e.hash()) {
            let at = cuts.entry((e.author(), e.log().to_string())).or_insert(u32::MAX);
            *at = (*at).min(e.seq());
        }
    }
    cuts
}

/// What `entries` of `global`'s (in any order) replay to, of the install at `root`: in causal order, with only the forks
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
/// entries it holds in replay order, of the install at `root`: with the forks among them, as a node replays them.
/// → it, and those forks: what a freeze's signer is checked against (`claimed`).
pub fn view(root: Option<Hash>, ordered: &[Entry], now: u64) -> Result<(ScopeDb, Cuts)> {
    let member_entries: Vec<Entry> = ordered.iter().filter(|e| members::is_member_entry(e)).cloned().collect();
    let cuts = forks(ordered);
    Ok((ScopeDb::build(&Members::replay(root, &member_entries, &cuts), &Scope::Global, &ScopeState::default(), now)?, cuts))
}

/// Who's who as a reader that keeps nothing of its own works it out from `ordered`, `global`'s entries it holds in
/// replay order, of the install at `root`, its scope database's steps by position from the first, and `global`'s log
/// up to the freeze an admin signed among them (`log_for`): the steps' once its log is frozen (`Truth::read`),
/// replay's before.
pub fn who(steps: &[Option<Given>], root: Option<Hash>, ordered: &[Entry], log: Option<&Log>, now: u64) -> Result<ScopeDb> {
    let (view, cuts) = view(root, ordered, now)?;
    Ok(Truth::read(steps, root, &view, &cuts, log, now)?.map_or(view, |t| t.db))
}

/// Whether `given`, step `at` of `global`'s database before its freeze, is a freeze an admin's own device signed, as
/// `view` says (who's who from everything the reader holds, once it read the log to its end; `cuts` its forks): the
/// freeze it says. One that isn't (a node's, a member's, a removed device's, a demoted admin's) is nothing, passed
/// over. One that is binds: it checks out (`anchor`) or the reader stops there. A device of theirs cut by a fork is
/// still theirs here: forks count only among the entries the freeze holds.
pub fn claimed(given: Option<&Given>, at: u64, view: &ScopeDb, cuts: &Cuts) -> Option<Freeze> {
    let step = &given.filter(|g| g.step.content.kind == FREEZE)?.step;
    let freeze = Freeze::of(step, at).ok()?;
    let forked = cuts.contains_key(&(step.author(), "global".to_string()));
    let theirs = view.client(&step.author()).is_some_and(|c| c.user == step.user()) || (forked && view.machine_ever(&step.author()).is_some_and(|m| m.user == step.user() && m.is(members::Kind::Client)));
    (theirs && view.may(step.user(), "change-settings", scopes::INSTALL)).then_some(freeze)
}

/// Of `steps`, `global`'s database's by position from the first, the end of `global`'s log the first freeze an admin
/// signed names (`claimed`): what a reader reads the log to (`log_to`) before it takes the steps.
pub fn wants_log(steps: &[Option<Given>], view: &ScopeDb, cuts: &Cuts) -> Option<u64> {
    steps.iter().enumerate().find_map(|(i, g)| claimed(g.as_ref(), i as u64 + 1, view, cuts)).map(|f| f.global())
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
    ensure!(version(&copy)? <= switch::READS, Newer);
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

/// The freeze `given` is, at position `at`, as checked here against `log`, `global`'s log up to the end it names
/// (`log_to`), of the install whose genesis is `root`: its copy comes to the hash it names; an admin's own device
/// signed it, as those entries say; and they add up to what the copy says (`says`). → the database it anchors, this
/// release's form of its copy. Everyone replays exactly those entries, forks among them only, so later entries,
/// forks or releases change nothing.
pub fn anchor(given: &Given, at: u64, root: Option<Hash>, log: &Log, now: u64) -> Result<ScopeDb> {
    let step = &given.step;
    let freeze = Freeze::of(step, at)?;
    ensure!(log.to == freeze.global(), "global's log was read to {}, not to the freeze's {}", log.to, freeze.global());
    let copy = connection(given.copy.as_deref().context("a freeze carries its copy")?)?;
    ensure!(crate::db::state_hash(&copy)? == freeze.hash, "a freeze's copy isn't what it says");
    ensure!(version(&copy)? <= switch::READS, Newer);
    let r = replay_all(root, &log.entries);
    let own = super::scope::build(&r.members, &Scope::Global, &r.state)?;
    carry(&copy, &own)?;
    let own = ScopeDb::open(own, now)?;
    ensure!(own.client(&step.author()).is_some_and(|c| c.user == step.user()) && own.may(step.user(), "change-settings", scopes::INSTALL), "only an admin's own device freezes the log");
    ensure!(says(&copy, own.conn())?, "who's who at {YOURS}'s freeze isn't what its admin's device said it was");
    normalize(copy, &Scope::Global, now)
}

/// The freeze a device of an admin's writes, at `logs`' ends (every scope's log but `global`'s, by its last step in
/// the bucket) and `log`, `global`'s log to its end, of the install at `root`: its content, and its copy.
pub fn freeze(root: Option<Hash>, log: &Log, mut logs: BTreeMap<String, u64>, now: u64) -> Result<(Content, Vec<u8>)> {
    logs.insert("global".into(), log.to);
    let r = replay_all(root, &log.entries);
    let mut db = ScopeDb::build(&r.members, &Scope::Global, &r.state, now)?;
    db.conn().pragma_update(None, "user_version", switch::READS)?;
    let freeze = Freeze { at: 0, hash: crate::db::state_hash(db.conn())?, logs };
    Ok((freeze.content(), db.bytes()?))
}

/// A scope's database before its first step (not `global`'s): what its log's entries up to the freeze's position there
/// (`entries`, read from its steps up to `Freeze::logs`') add up to with `global`'s at the freeze (`global`, its log up
/// to the freeze's end of it, of the install at `root`). Every reader works it out itself.
pub fn scope_anchor(freeze: &Freeze, root: Option<Hash>, global: &Log, scope: &Scope, entries: &[Entry], now: u64) -> Result<ScopeDb> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    ensure!(global.to == freeze.global(), "global's log was read to {}, not to the freeze's {}", global.to, freeze.global());
    let r = replay_all(root, &global.entries);
    let ordered = crate::entry::causal(entries.to_vec(), &r.ordered.iter().map(|e| (e.hash(), e.depth)).collect());
    let (_, state) = members::replay_from(&r.known, &r.member_entries, &ordered, ScopeState::default(), |_, _, _| {});
    ScopeDb::open(super::scope::build(&Members::default(), scope, &state)?, now)
}

/// `global`'s scope database as a reader has it once its log is frozen: the freeze, the last position applied (or
/// passed over), and the database there.
pub struct Truth {
    pub freeze: Freeze,
    pub at: u64,
    pub db: ScopeDb,
}

impl Truth {
    /// From the steps a reader has, by position from the first (none for one that will never be a step), with `view`
    /// and `cuts`, who's who before the freeze from everything it holds and its forks, to say which freeze an admin
    /// signed (`claimed`), and `log`, `global`'s log to the end that freeze names (`wants_log`): none before it.
    /// Fails there if it doesn't check out, or `log` isn't given.
    pub fn read(steps: &[Option<Given>], root: Option<Hash>, view: &ScopeDb, cuts: &Cuts, log: Option<&Log>, now: u64) -> Result<Option<Truth>> {
        let mut truth: Option<Truth> = None;
        for (i, given) in steps.iter().enumerate() {
            let at = i as u64 + 1;
            match &mut truth {
                Some(t) => drop(t.take(at, given.as_ref().map(|g| &g.step))?),
                None => truth = Truth::first(at, given.as_ref(), root, view, cuts, log, now)?,
            }
        }
        Ok(truth)
    }

    /// `read`, for a service endpoint, which holds no keys and takes its steps from its own device's node, which
    /// checked the freeze: the first an admin's own device signed (`claimed`) is taken as its copy says, unchecked
    /// against the log, which it can't read.
    pub fn read_given(steps: &[Option<Given>], view: &ScopeDb, cuts: &Cuts, now: u64) -> Result<Option<Truth>> {
        let mut truth: Option<Truth> = None;
        for (i, given) in steps.iter().enumerate() {
            let at = i as u64 + 1;
            match (&mut truth, claimed(given.as_ref(), at, view, cuts), given) {
                (Some(t), _, _) => drop(t.take(at, given.as_ref().map(|g| &g.step))?),
                (None, Some(freeze), Some(g)) => {
                    let copy = connection(g.copy.as_deref().context("a freeze carries its copy")?)?;
                    ensure!(crate::db::state_hash(&copy)? == freeze.hash, "a freeze's copy isn't what it says");
                    truth = Some(Truth { freeze, at, db: normalize(copy, &Scope::Global, now)? });
                }
                _ => {}
            }
        }
        Ok(truth)
    }

    /// Step `at` of `global`'s database, before its freeze: the freeze, if an admin's own device signed it as `view`
    /// says (`claimed`), checked against `log`, `global`'s log to the end it names (`anchor`). Any other step is
    /// nothing. A freeze an admin signed that doesn't check out stops the reader here, as a newer release's does
    /// (`Newer`): it's never passed over for another, which a reader that it did check out for would never take.
    pub fn first(at: u64, given: Option<&Given>, root: Option<Hash>, view: &ScopeDb, cuts: &Cuts, log: Option<&Log>, now: u64) -> Result<Option<Truth>> {
        let (Some(freeze), Some(g)) = (claimed(given, at, view, cuts), given) else { return Ok(None) };
        let log = log.filter(|l| l.to == freeze.global()).with_context(|| format!("who's who waits for global's log to step {}, where its freeze says it ends", freeze.global()))?;
        let db = anchor(g, at, root, log, now).with_context(|| format!("{YOURS}'s freeze at step {at}, which an admin's device signed, doesn't check out: this device reads who's who no further"))?;
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

    /// Who's who as `welcome` says it: from `freeze`, the step at its freeze's position, and `start`, the copy at
    /// `Welcome::copy` (its bytes, opened), or none to start from the freeze's own, then `after`, the steps after that
    /// one up to the Welcome's, each taken as any reader takes it. Fails unless it comes to the Welcome's hash, in this
    /// release's form: a copy or step a node or another device swapped is never used.
    pub fn welcomed(welcome: &Welcome, freeze: &Given, start: Option<&[u8]>, after: &[Option<Given>], now: u64) -> Result<Truth> {
        let f = Freeze::of(&freeze.step, welcome.freeze)?;
        ensure!(u64::from(freeze.step.seq()) == welcome.freeze, "that isn't the freeze it was welcomed with");
        let (at, copy) = match (welcome.copy(), start) {
            (Some(at), Some(bytes)) => (at, connection(bytes)?),
            (_, None) => {
                let copy = connection(freeze.copy.as_deref().context("a freeze carries its copy")?)?;
                ensure!(crate::db::state_hash(&copy)? == f.hash, "a freeze's copy isn't what it says");
                (welcome.freeze, copy)
            }
            (None, Some(_)) => bail!("no copy comes before the freeze's"),
        };
        ensure!(after.len() as u64 == welcome.at - at, "not every step up to where it was welcomed is here");
        let mut t = Truth { freeze: f, at, db: normalize(copy, &Scope::Global, now)? };
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
        Ok(match open(keys, &Scope::Global, at, &o.bytes) {
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
    ensure!(version(db.conn())? <= switch::READS, Newer);
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

/// The kind of step that moves a database on to a newer release's types, roles, rules and tables
/// (`permissions::DEFAULTS`, `scope::SCHEMA`): `{ reads, types, roles, rules }`, `reads` that release's
/// `switch::READS`, which the database keeps as its version. An admin's device writes one once every device reads it
/// (`switch::gate`), and only above the database's version: a release never puts back what a newer one said, and two
/// on different releases never write turn about. Readers never run their own release's: each database keeps the rows
/// its freeze and steps said. One that reads less stops at it (`Newer`): update it.
pub const DEFAULTS: &str = "defaults";

/// A database's version: the `switch::READS` of the release whose types, roles, rules and tables it has.
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
    data["reads"] = json!(switch::READS);
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
/// the database had; the install's own roles stay.
fn defaults(db: &ScopeDb, who: &ScopeDb, scope: &Scope, step: &Entry) -> Result<Applied> {
    let reads = u32::try_from(step.content.data["reads"].as_u64().context("a release's roles name its version")?)?;
    ensure!(reads <= switch::READS, Newer);
    ensure!(reads > version(db.conn())?, "a release's roles only ever move on");
    ensure!(who.client(&step.author()).is_some_and(|c| c.user == step.user()), "only a person's own device writes for them");
    ensure!(who.may(step.user(), "change-roles", scopes::INSTALL), "only the install's admins say the release's roles");
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
                // Someone new, with their first device: given a role in the install by someone who may give it.
                (None, None) => db.may_give(w, &user, super::scope::role(role("role")?), scopes::INSTALL),
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
            to == was || (db.may_give(w, &user, super::scope::role(to), scopes::INSTALL) && db.may_take(w, &user, super::scope::role(was), scopes::INSTALL))
        }
        (members::APP, "remove") => match text("user") {
            Ok(user) => db.user(&user).is_some_and(|u| db.may_take(w, &user, super::scope::role(u.role), scopes::INSTALL)),
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
        (members::APP, "group") => may("make-groups", scopes::INSTALL),
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
        (members::APP, "setting") => may("change-settings", scopes::INSTALL),
        (members::APP, "recovery") => false,
        (members::APP, "app") => {
            let name = text("name")?;
            let owner = Principal::parse(&text("owner")?)?;
            let to = scopes::owner_object(&owner);
            match db.app(&name) {
                None => {
                    let holders = match &owner {
                        Principal::User(u) => scopes::person(u),
                        Principal::Group(g) => format!("{}#admin", scopes::group(g)),
                        Principal::Admins => format!("{}#admin", scopes::INSTALL),
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
        (members::APP, "service") => may("switch", &scopes::device(&unhex(&text("device")?)?)),
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

    /// `global`'s entries, each written after every one before it, as they'd be in its log's steps, one a step.
    #[derive(Default)]
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

        /// Its log up to step `to`.
        fn log(&self, to: usize) -> Log {
            Log { to: to as u64, entries: self.entries[..to].to_vec() }
        }

        /// Who's who as a reader that holds every entry so far finds it, before the freeze: its view, and its forks.
        fn view(&self) -> (ScopeDb, Cuts) {
            let ordered = crate::entry::causal(self.entries.clone(), &Default::default());
            view(Some(self.entries[0].hash()), &ordered, 0).unwrap()
        }
    }

    /// A freeze `key` signs as `user` at step `at` of `global`'s database, of `log`: as an admin's device writes one.
    fn freeze_of(key: &SigningKey, user: &str, at: u64, root: Hash, log: &Log) -> Given {
        let (content, copy) = freeze(Some(root), log, BTreeMap::new(), 0).unwrap();
        Given { step: sign(key, &Scope::Global, at, user, 1, 0, content).unwrap(), copy: Some(copy) }
    }

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";

    /// Andre's Mac starts the install and lists Bob's laptop, Bob an admin too.
    fn two_admins() -> (History, SigningKey, SigningKey) {
        use crate::keys::Endpoint;
        use crate::members::{Kind, device, genesis};
        let (mac, laptop) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]));
        let mut h = History::default();
        h.write(&mac, ANDRE, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)));
        h.write(&mac, ANDRE, device(&Endpoint::of(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Admin))));
        (h, mac, laptop)
    }

    /// An admin demoted (or a device taken out) can't freeze the log to before it was, to be an admin again: its freeze
    /// is checked against everything the reader holds, so it's nothing, and the next an admin signs is the freeze.
    #[test]
    fn a_demoted_admin_s_freeze_makes_no_one_an_admin_again() {
        let (mut h, mac, laptop) = two_admins();
        let root = h.entries[0].hash();
        let before = h.entries.len();
        h.write(&mac, ANDRE, members::role(BOB, Role::Member));
        h.write(&mac, ANDRE, members::setting("name", Some("Ours")));
        let (view, cuts) = h.view();
        // Bob's laptop freezes the log where Bob was still an admin.
        let bobs = freeze_of(&laptop, BOB, 1, root, &h.log(before));
        assert!(claimed(Some(&bobs), 1, &view, &cuts).is_none(), "a demoted admin's freeze is nothing");
        let andres = freeze_of(&mac, ANDRE, 2, root, &h.log(h.entries.len()));
        let truth = Truth::read(&[Some(bobs.clone()), Some(andres)], Some(root), &view, &cuts, Some(&h.log(h.entries.len())), 0).unwrap().unwrap();
        assert_eq!((truth.at, truth.db.user(BOB).map(|u| u.role), truth.db.setting("name")), (2, Some(Role::Member), Some("Ours".into())), "Andre's freeze is the one");
        // Nor does one naming the log's end count while its signer isn't an admin.
        let bobs_late = freeze_of(&laptop, BOB, 1, root, &h.log(h.entries.len()));
        assert!(claimed(Some(&bobs_late), 1, &view, &cuts).is_none());
    }

    /// A fork, or anything else written after the freeze, changes nothing about it: every reader replays exactly the
    /// log's steps up to where it says the log ends, forks among those only, so a reader that reads it later anchors
    /// the same who's who as one that read it at once, never passing over it for another.
    #[test]
    fn a_fork_after_the_freeze_changes_no_reader_s_freeze() {
        let (mut h, mac, laptop) = two_admins();
        let root = h.entries[0].hash();
        h.write(&laptop, BOB, members::setting("name", Some("Ours")));
        let at = h.entries.len();
        let freeze = freeze_of(&mac, ANDRE, 1, root, &h.log(at));
        let (view, cuts) = h.view();
        let soon = Truth::read(&[Some(freeze.clone())], Some(root), &view, &cuts, Some(&h.log(at)), 0).unwrap().unwrap();
        // Then Bob's laptop, and Andre's Mac, restored from older backups, each sign another entry at a seq they
        // signed before: forks, in the log after the freeze's end.
        for (key, user) in [(&laptop, BOB), (&mac, ANDRE)] {
            let me = key.verifying_key().to_bytes();
            let mine: Vec<Entry> = h.entries[..at].iter().filter(|e| e.author() == me).cloned().collect();
            let last = mine.last().unwrap();
            let head = mine.len().checked_sub(2).map(|i| mine[i].head());
            let again = Entry::create(key, Ext { log: "global".into(), time: 1, user: user.into(), epoch: 1, previous: last.previous().to_vec() }, head, members::setting("name", Some("Forked")));
            assert_eq!(again.seq(), last.seq());
            h.entries.push(again);
        }
        let (later, later_cuts) = h.view();
        assert_eq!(later_cuts.len(), 2, "the later reader holds both forks");
        let late = Truth::read(&[Some(freeze)], Some(root), &later, &later_cuts, Some(&h.log(at)), 0).unwrap().unwrap();
        let hash = |t: &Truth| crate::db::state_hash(t.db.conn()).unwrap();
        assert_eq!(hash(&late), hash(&soon), "the same who's who, whenever it's read");
    }

    /// A freeze an admin's device signed that doesn't check out stops its reader there: never passed over for the next,
    /// which a reader it did check out for would never take.
    #[test]
    fn a_freeze_an_admin_signed_that_doesn_t_check_out_stops_its_reader() {
        let (h, mac, _) = two_admins();
        let root = h.entries[0].hash();
        let mut forged = freeze_of(&mac, ANDRE, 1, root, &h.log(h.entries.len()));
        let mut db = connection(forged.copy.as_deref().unwrap()).unwrap();
        db.execute_batch("UPDATE devices SET name = 'mine' WHERE name = 'laptop'").unwrap();
        let mut data = forged.step.content.data.clone();
        data["hash"] = json!(crate::db::state_hash(&db).unwrap());
        forged = Given { step: sign(&mac, &Scope::Global, 1, ANDRE, 1, 0, Content::new(members::APP, FREEZE, data)).unwrap(), copy: Some(db.serialize("main").unwrap().to_vec()) };
        let (view, cuts) = h.view();
        let next = freeze_of(&mac, ANDRE, 2, root, &h.log(h.entries.len()));
        assert!(Truth::read(&[Some(forged.clone()), Some(next)], Some(root), &view, &cuts, Some(&h.log(h.entries.len())), 0).is_err(), "a copy that isn't what the log says stops it");
        assert!(Truth::read(&[Some(freeze_of(&mac, ANDRE, 1, root, &h.log(h.entries.len())))], Some(root), &view, &cuts, None, 0).is_err(), "as does one whose log it hasn't read");
    }

    /// A newest hint is anyone's to write: one far above the last step jams no reader, which reads on to the steps'
    /// own end; only a step missing before a later one that's there stops it.
    #[test]
    fn a_forged_newest_hint_jams_no_reader() {
        use crate::bucket::Bucket;
        let bucket = crate::bucket::Memory::default();
        let prefix = prefix("global");
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            bucket.create(&crate::db::newest_name(&prefix), br#"{"at": 1000000000, "copy": 0, "epoch": 1}"#).await.unwrap();
            assert!(read(&bucket, &crate::db::Keys::new(), &Scope::Global, 0).await.unwrap().is_empty(), "nothing written, nothing read, and no failure");
            bucket.create(&crate::db::step_name(&prefix, 2), b"x").await.unwrap();
            assert!(missing(&bucket, &prefix, 1).await.is_err(), "but a step gone before one that's there stops it");
        });
    }

    /// A release's types, roles and rules move a database on only to a newer release's, by an admin's own device, and a
    /// reader of an older one stops there rather than read on as if it didn't; and once the switch is on, each `READS`
    /// is held to the tables, types, roles and rules it reads, so a release that changes them reads one more.
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
        let newer = apply(&db, None, &Scope::Global, &step(1, switch::READS + 1)).err().unwrap();
        assert!(newer.is::<Newer>(), "a newer release's stops this reader");
        let db = ScopeDb::open(apply(&db, None, &Scope::Global, &step(1, switch::READS)).unwrap().db, 0).unwrap();
        assert_eq!(version(db.conn()).unwrap(), switch::READS);
        assert!(apply(&db, None, &Scope::Global, &step(2, switch::READS)).is_err(), "the same release's again changes nothing");
        // What each `READS` reads, once it's switched: add `(READS, hash)` when a release changes them, and read one more.
        const PINNED: &[(u32, &str)] = &[];
        let mut h = blake3::Hasher::new();
        h.update(super::super::scope::SCHEMA.as_bytes());
        h.update(permissions::SCHEMA.as_bytes());
        h.update(rules_of(db.conn()).unwrap().to_string().as_bytes());
        let hash = h.finalize().to_hex().to_string();
        if switch::SWITCH {
            assert!(PINNED.contains(&(switch::READS, hash.as_str())), "the scope database's tables, types, roles or rules changed: read one more in switch::READS, and pin ({}, \"{hash}\")", switch::READS + 1);
        }
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
