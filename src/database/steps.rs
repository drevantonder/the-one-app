//! A scope database's steps: the database is the truth, and every change to it is a step at the next free position of
//! its own in the bucket, which the bucket keeps in one order.
//!
//! A step is one change, signed by the device that made it as its person: the same content an entry of the log carried
//! (`content`'s kinds), for one position, under `entry::STEP_DOMAIN`, so it's never taken for an
//! entry nor an entry for it. Its position is signed in it (its `seq`), so a step can't be written twice, or anywhere
//! but where its writer meant it: one that loses its position to another is signed again for the next. A node only
//! carries a client's step to the bucket: it can hold one back, but change none.
//!
//! `global`'s database starts at its first step that is a freeze the network's recovery key signed (`FREEZE`,
//! `Truth::first`), the key its genesis names (`recovery_of`), the first entry of `global`'s log, whose hash every
//! device of the network pins: a network made since starts with one (`first_freeze`), and the two made before were
//! frozen by their recovery key once, their logs read only from then on. The freeze carries the database there, its
//! first copy, which counts only if it comes to the hash the freeze names. A freeze the recovery key signed that doesn't
//! check out stops the reader there, as a newer release's step does, never passed over for the next; anything else
//! shaped like a freeze is nothing. A device let in starts where the device that let it in had read to (its `Welcome`),
//! which pins who's who by its hash.
//!
//! Every other scope's database starts, before its first step, from its start (`START`, step 0): for a scope whose
//! frozen log the freeze names, what that log's entries added up to there, written once by a device holding its keys;
//! for any other, nothing.
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
//! counted one changes is its kind's rule's (`rules`), applied to a fresh copy of the database (`scope::fresh`).
//!
//! Its steps are sealed with the key the frozen log had in each epoch (`KEY_NAME`, `seal`), under their own names. A
//! reader opens them itself: a node passes them on sealed. The frozen logs are read no more, but for the genesis, the
//! first entry of `global`'s, whose hash every device pins and which names the recovery key (`genesis_name`).
//!
//! ```text
//! <prefix>/0                  a scope's start, but `global`'s, with its copy
//! <prefix>/<n>               step n, from 1: one signed step, its header, signature and body, and a freeze's copy
//! <prefix>/copy/<n>          the database after step n, every `stored::COPY_EVERY` steps
//! <prefix>/newest            the newest hint, as a database's
//! ```

use crate::config::YOURS;
use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::SigningKey;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::entry::{Content, Entry, Ext, Hash, Key, Scope, unhex};
use crate::database::scopes::{Principal, Role};
use crate::permissions;

use super::scopes::{self, ScopeDb};
use super::READS;
use super::scope::RULE;

/// The node's database of a scope that its steps are: its `scope`.
pub const NAME: &str = "scope";

/// Where a scope's database's steps are in the bucket.
pub fn prefix(scope: &str) -> String {
    format!("{}/{}/{}", crate::bucket::folder(scope), scopes::APP, NAME)
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
pub fn open(keys: &crate::database::stored::Keys, made: u32, scope: &Scope, at: u64, sealed: &[u8]) -> Opened {
    let name = crate::database::stored::step_name(&prefix(&scope.to_string()), at);
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
pub async fn read(bucket: &impl crate::bucket::Bucket, keys: &crate::database::stored::Keys, scope: &Scope, from: u64) -> Result<Read> {
    let prefix = prefix(&scope.to_string());
    let mut out = Read::default();
    loop {
        let at = from + out.steps.len() as u64 + 1;
        let Some(o) = bucket.get(&crate::database::stored::step_name(&prefix, at)).await? else {
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
pub async fn missing(bucket: &impl crate::bucket::Bucket, keys: &crate::database::stored::Keys, scope: &Scope, at: u64) -> Result<()> {
    let later = match bucket.get(&crate::database::stored::step_name(&prefix(&scope.to_string()), at + 1)).await? {
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
    /// The state hash its copy comes to, as it is (`stored::state_hash`).
    pub hash: String,
    /// The last step of every scope's log that counts, by scope: `global`'s among them.
    pub logs: BTreeMap<String, u64>,
}

impl Freeze {
    /// The freeze `step` at position `at` says.
    pub fn of(step: &Entry, at: u64) -> Result<Freeze> {
        ensure!(step.content.app == scopes::APP && step.content.kind == FREEZE && step.scope() == Scope::Global, "not a freeze");
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
        Content::new(scopes::APP, FREEZE, json!({ "hash": self.hash, "logs": self.logs }))
    }

    /// The last step of `global`'s log it holds.
    pub fn global(&self) -> u64 {
        self.logs.get("global").copied().unwrap_or(0)
    }
}

/// The name the steps' key is derived under in each epoch (`cloudflare::database_key`): the frozen log's, whose key
/// they took over.
pub const KEY_NAME: &str = "log";

/// Seals a step or copy of a scope's database as `name` under the newest epoch `keys` has.
pub fn seal(keys: &crate::database::stored::Keys, name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let (epoch, key) = keys.last_key_value().with_context(|| format!("no key to write {name} with"))?;
    Ok(crate::keys::seal(*epoch, key, name, bytes))
}

/// Where the network's genesis is: the first step of `global`'s frozen log, a pack of entries under its first epoch,
/// read only. Every device pins its hash.
pub fn genesis_name() -> String {
    crate::database::stored::step_name(&format!("{}/{}/{KEY_NAME}", crate::bucket::folder("global"), scopes::APP), 1)
}

/// The entries in `sealed`, the first step of `global`'s frozen log as the bucket holds it (`genesis_name`), opened
/// with `keys`, `global`'s by epoch: the genesis among them (`recovery_of`).
pub fn genesis_entries(keys: &crate::database::stored::Keys, sealed: &[u8]) -> Result<Vec<Entry>> {
    let key = keys.get(&crate::keys::epoch_of(sealed)?).context("the genesis is under a key this device wasn't given")?;
    crate::entry::unpack(&crate::keys::open(key, &genesis_name(), sealed)?)
}

/// The entries of `global`'s frozen log's first step, the genesis among them, read from the bucket with `keys`.
pub async fn genesis(bucket: &impl crate::bucket::Bucket, keys: &crate::database::stored::Keys) -> Result<Vec<Entry>> {
    let sealed = bucket.get(&genesis_name()).await?.with_context(|| format!("{YOURS}'s genesis isn't in its bucket"))?;
    genesis_entries(keys, &sealed.bytes)
}

/// The network's recovery key, as its genesis names it: the entry whose hash is `root` among `entries`, those of
/// `global`'s log's first step. Only the first freeze it signed is who's who (`Truth::first`).
pub fn recovery_of(root: Hash, entries: &[Entry]) -> Option<Key> {
    let genesis = entries.iter().find(|e| e.hash() == root && e.content.app == scopes::APP && e.content.kind == "genesis")?;
    genesis.content.data["recovery"].as_str().and_then(|k| unhex(k).ok())
}

/// The network's genesis for a reader that pins none of its own: the one genesis among `entries`, none while there
/// are two; or `pinned`, whatever genesis anyone writes.
pub fn genesis_of(pinned: Option<Hash>, entries: &[Entry]) -> Option<Hash> {
    if pinned.is_some() {
        return pinned;
    }
    let mut found = entries.iter().filter(|e| e.content.app == scopes::APP && e.content.kind == "genesis").map(Entry::hash);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// The network's genesis (`root`, or for a reader that pins none, `genesis_of`) and the recovery key it names, read
/// from the bucket with `keys`, `global`'s by epoch (`genesis_entries`).
pub async fn recovery_key(bucket: &impl crate::bucket::Bucket, keys: &crate::database::stored::Keys, root: Option<Hash>) -> Result<(Hash, Key)> {
    let entries = genesis(bucket, keys).await?;
    let root = genesis_of(root, &entries).with_context(|| format!("{YOURS}'s genesis isn't the first of its log's entries"))?;
    let key = recovery_of(root, &entries).with_context(|| format!("{YOURS}'s genesis isn't the first of its log's entries"))?;
    Ok((root, key))
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
    ScopeDb::open(super::scope::fresh(&copy, scope)?, now)
}

/// The freeze a new network starts with, at step 1 of `global`'s database, signed by its recovery key as `user`: who's
/// who as `genesis`, its log's first entry, says it, with its log's one step. → the step, and its copy.
pub fn first_freeze(recovery: &SigningKey, genesis: &Entry, user: &str, now: u64) -> Result<Given> {
    let mut db = first(genesis, now)?;
    let freeze = Freeze { at: 1, hash: crate::database::stored::state_hash(db.conn())?, logs: [("global".to_string(), 1)].into() };
    let step = sign(recovery, &Scope::Global, 1, user, 1, now, freeze.content())?;
    Ok(Given { step, copy: Some(db.bytes()?) })
}

/// `global`'s database as `genesis`, its log's first entry, makes it: the network's first person, device, epoch and
/// recovery key, with this release's types, roles and rules.
pub fn first(genesis: &Entry, now: u64) -> Result<ScopeDb> {
    let db = super::scope::empty(&Scope::Global)?;
    db.pragma_update(None, "user_version", READS)?;
    super::rules::genesis(&db, genesis)?;
    super::scope::derive(&db)?;
    ScopeDb::open(db, now)
}

/// The kind of step that starts a scope's database (not `global`'s) whose frozen log the freeze names, at position 0:
/// `{ hash }`, with its copy, what that log's entries added up to at the freeze. Written once, by a device that holds
/// the scope's keys (it's sealed under them, as its steps are).
pub const START: &str = "start";

/// Where a scope's database's start is: position 0, before its first step.
pub fn start_name(scope: &Scope) -> String {
    crate::database::stored::step_name(&prefix(&scope.to_string()), 0)
}

/// `scope`'s start, step 0 of its database, signed by `key` as `user`, the scope at `epoch`: `db`, what its frozen log's
/// entries added up to at the freeze.
pub fn start(key: &SigningKey, scope: &Scope, user: &str, epoch: u32, now: u64, db: &mut ScopeDb) -> Result<Given> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    let hash = crate::database::stored::state_hash(db.conn())?;
    let step = sign(key, scope, 0, user, epoch, now, Content::new(scopes::APP, START, json!({ "hash": hash })))?;
    Ok(Given { step, copy: Some(db.bytes()?) })
}

/// A scope's database before its first step (not `global`'s), as `global`'s database `global` says, the freeze
/// `freeze` among it: one with nothing in it, unless the freeze names the scope's frozen log, when it's the copy its
/// start carries (`START`, step 0, `given`), which counts only if its copy comes to the hash it names and its signer is
/// a device the network ever listed: it opened with the scope's keys, and was written once, by this release, so no
/// later removal takes it back. Without one that counts, the scope can't be read yet.
pub fn started(freeze: &Freeze, global: &ScopeDb, scope: &Scope, given: Option<&Given>, now: u64) -> Result<ScopeDb> {
    ensure!(*scope != Scope::Global, "global's database starts from its freeze");
    if freeze.logs.get(&scope.to_string()).is_none_or(|n| *n == 0) {
        return ScopeDb::empty(scope, now);
    }
    let given = given.with_context(|| format!("{scope}'s database has no start yet: one of its devices writes it"))?;
    let step = &given.step;
    ensure!(step.content.app == scopes::APP && step.content.kind == START && step.seq() == 0 && step.scope() == *scope, "{scope}'s step 0 isn't its start");
    ensure!(global.machine_ever(&step.author()).is_some(), "{scope}'s start isn't from one of the network's devices");
    let copy = connection(given.copy.as_deref().context("a start carries its copy")?)?;
    ensure!(Some(crate::database::stored::state_hash(&copy)?.as_str()) == step.content.data["hash"].as_str(), "{scope}'s start's copy isn't what it says");
    normalize(copy, scope, now)
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
        ensure!(crate::database::stored::state_hash(&copy)? == freeze.hash, "a freeze's copy isn't what it says");
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
        Ok(json!({ "freeze": self.freeze.content().data, "freeze_at": self.freeze.at, "at": self.at, "hash": crate::database::stored::state_hash(self.db.conn())?, "db": data_encoding::BASE64.encode(&bytes) }))
    }

    /// What `keep` kept, checked against the hash it kept with it, in this release's form.
    pub fn kept(kept: &Value, now: u64) -> Result<Truth> {
        let bytes = data_encoding::BASE64.decode(kept["db"].as_str().context("no database kept")?.as_bytes())?;
        let conn = connection(&bytes)?;
        ensure!(Some(crate::database::stored::state_hash(&conn)?.as_str()) == kept["hash"].as_str(), "who's who this device kept isn't what it was");
        let freeze = Freeze::from_data(&kept["freeze"], kept["freeze_at"].as_u64().context("no freeze kept")?)?;
        Ok(Truth { freeze, at: kept["at"].as_u64().context("no position kept")?, db: normalize(conn, &Scope::Global, now)? })
    }
}

/// After its writer wrote step `at` of a scope's database: a copy of the database it leaves (`db`'s bytes), sealed
/// as its steps are, at every `COPY_EVERY`th step; and the newest hint moved on, as a database's (`stored::hint`). Only
/// saves readers work, so a link that fails it fails nothing. The hint is anyone's to write, so nothing here trusts it.
pub async fn written(bucket: &impl crate::bucket::Bucket, keys: &crate::database::stored::Keys, scope: &Scope, at: u64, bytes: Vec<u8>) -> Result<()> {
    let prefix = prefix(&scope.to_string());
    let epoch = *keys.keys().next_back().context("no key")?;
    if at % crate::database::stored::COPY_EVERY == 0 {
        let name = crate::database::stored::copy_name(&prefix, at);
        bucket.create(&name, &seal(keys, &name, &bytes)?).await?;
    }
    crate::database::stored::hint(bucket, &prefix, at, at - at % crate::database::stored::COPY_EVERY, epoch).await
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

    /// The copy a new device starts from: the newest at or before `at` the bucket keeps (one every `stored::COPY_EVERY`
    /// steps), or none, for the freeze's own.
    pub fn copy(&self) -> Option<u64> {
        let c = self.at - self.at % crate::database::stored::COPY_EVERY;
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
        Ok(Welcome { freeze: self.freeze.at, at: self.at, hash: crate::database::stored::state_hash(self.db.conn())? })
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
        ensure!(crate::database::stored::state_hash(t.db.conn())? == welcome.hash, "who's who from that copy isn't what the device that let this one in said");
        Ok(t)
    }
}

/// Who's who as `welcome` says it, read from the bucket with `keys`, `global`'s log's keys by epoch (`Truth::welcomed`):
/// from the copy before the Welcome's step, or, if that one isn't there or doesn't come to its hash, from its freeze's.
pub async fn welcomed(bucket: &impl crate::bucket::Bucket, keys: &crate::database::stored::Keys, welcome: &Welcome, now: u64) -> Result<Truth> {
    async fn step(bucket: &impl crate::bucket::Bucket, keys: &crate::database::stored::Keys, at: u64) -> Result<Option<Given>> {
        let o = bucket.get(&crate::database::stored::step_name(&prefix("global"), at)).await?.with_context(|| format!("global's step {at} isn't there"))?;
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
        let name = crate::database::stored::copy_name(&prefix, c);
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
    if step.content.app == scopes::APP && step.content.kind == DEFAULTS {
        return defaults(db, who, scope, step);
    }
    ensure!(version(db.conn())? <= READS, Newer);
    ensure!(step.epoch() <= who.epoch(scope), "it names an epoch its scope hasn't reached");
    allowed(who, scope, step)?;
    super::rules::signed(who, scope, step)?;
    let out = super::scope::fresh(db.conn(), scope)?;
    super::rules::apply(who, &out, scope, step)?;
    if *scope == Scope::Global {
        super::scope::derive(&out)?;
    }
    named(&out, step)?;
    Ok(Applied { db: out })
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
    Ok(Content::new(scopes::APP, DEFAULTS, data))
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


/// A `DEFAULTS` step: by an admin's device, the release's types, roles and rules as it says them, in place of those
/// the database had; the network's own roles stay.
fn defaults(db: &ScopeDb, who: &ScopeDb, scope: &Scope, step: &Entry) -> Result<Applied> {
    // Whose it is first: one that doesn't count is passed over, whatever release it names.
    ensure!(who.client(&step.author()).is_some_and(|c| c.user == step.user()), "only a person's own device writes for them");
    ensure!(who.may(step.user(), "change-roles", scopes::NETWORK), "only the network's admins say the release's roles");
    let reads = u32::try_from(step.content.data["reads"].as_u64().context("a release's roles name its version")?)?;
    ensure!(reads <= READS, Newer);
    ensure!(reads > version(db.conn())?, "a release's roles only ever move on");
    let out = super::scope::fresh(db.conn(), scope)?;
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

/// What a counted step names that no other table keeps: its blobs, and a release.
fn named(db: &Connection, step: &Entry) -> Result<()> {
    let pinned = step.content.app == scopes::APP && step.content.kind != "keep";
    for blob in &step.content.blobs {
        db.execute("INSERT INTO blobs VALUES (?1, ?2) ON CONFLICT (blob) DO UPDATE SET pinned = max(pinned, ?2)", rusqlite::params![blob, pinned])?;
    }
    if step.content.app == scopes::APP && scopes::RELEASES.contains(&step.content.kind.as_str()) {
        db.execute(
            "INSERT INTO releases VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![step.seq(), step.content.kind, serde_json::to_string(&step.content)?, crate::entry::hex(&step.author()), i64::try_from(step.time()).unwrap_or(i64::MAX)],
        )?;
    }
    Ok(())
}

/// Rule 3: whether `step`'s signer, as its person, holds the permission its change needs in `db` (`global`'s), before
/// it. Its kind's rule is checked too, as it's applied (`rules::apply`): this is the engine's.
pub fn allowed(db: &ScopeDb, scope: &Scope, step: &Entry) -> Result<()> {
    let data = &step.content.data;
    let w = step.user();
    let text = |k: &str| data[k].as_str().map(str::to_string).with_context(|| format!("no {k}"));
    let may = |verb: &str, object: &str| db.may(w, verb, object);
    let role = |k: &str| -> Result<Role> { Ok(serde_json::from_value(data[k].clone())?) };
    let ok = match (step.content.app.as_str(), step.content.kind.as_str()) {
        // Rule 1: the recovery key lets a device in, and hands over to the next; nothing else.
        _ if db.recovery() == Some(step.author()) => matches!(step.content.kind.as_str(), "device" | "recovery") && *scope == Scope::Global,
        (scopes::APP, "genesis") => false,
        (scopes::APP, "device") => {
            let user = text("user")?;
            let key = unhex(&text("key")?)?;
            let kinds: Vec<scopes::Kind> = serde_json::from_value(data["roles"].clone()).unwrap_or_default();
            let theirs = kinds.iter().any(|k| matches!(k, scopes::Kind::Client | scopes::Kind::Service));
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
        (scopes::APP, "role") => {
            let user = text("user")?;
            let was = db.user(&user).context("no such person")?.role;
            let to = role("role")?;
            to == was || (db.may_give(w, &user, super::scope::role(to), scopes::NETWORK) && db.may_take(w, &user, super::scope::role(was), scopes::NETWORK))
        }
        (scopes::APP, "remove") => match text("user") {
            Ok(user) => db.user(&user).is_some_and(|u| db.may_take(w, &user, super::scope::role(u.role), scopes::NETWORK)),
            Err(_) => may("remove", &scopes::device(&unhex(&text("key")?)?)),
        },
        // A person's own say over their devices: turning one down before it's listed, and forgetting what one of
        // theirs signed in with (its rule checks it's theirs).
        (scopes::APP, "refuse" | "forget-sign-in") => may("add-devices", &scopes::person(w)),
        (scopes::APP, "epoch") => {
            let s = Scope::parse(&text("scope")?)?;
            may("remove", &scopes::object(&s))
        }
        (scopes::APP, "runs") => may("choose-devices", &scopes::person(w)),
        (scopes::APP, scopes::APP_SCHEDULES) | (scopes::APP, "vault") => may("choose-devices", &scopes::object(&Scope::parse(&text("scope")?)?)),
        (scopes::APP, "group") => may("make-groups", scopes::NETWORK),
        (scopes::APP, "invite") => db.may_give(w, &text("user")?, super::scope::role(role("role")?), &scopes::group(&text("group")?)),
        // Taking up an invite: the role the invite gave, which was checked as it was given.
        (scopes::APP, "accept") => db.roles().get(&(w.to_string(), Scope::Group(text("group")?))).is_some_and(|(_, invited)| *invited),
        (scopes::APP, "member") => {
            let (g, user) = (text("group")?, text("user")?);
            let object = scopes::group(&g);
            let was = db.group_role(&user, &g).or_else(|| db.group(&g).and_then(|grp| grp.invited.get(&user).copied()));
            let to: Option<Role> = serde_json::from_value(data["role"].clone())?;
            let took = was.is_none_or(|r| Some(r) == to || db.may_take(w, &user, super::scope::role(r), &object));
            took && to.is_none_or(|r| Some(r) == was || db.may_give(w, &user, super::scope::role(r), &object))
        }
        (scopes::APP, "rename") => {
            if let Ok(key) = text("key") {
                may("rename", &scopes::device(&unhex(&key)?))
            } else if let Ok(user) = text("user") {
                may("rename", &scopes::person(&user))
            } else {
                may("rename", &scopes::group(&text("group")?))
            }
        }
        (scopes::APP, "setting") => may("change-settings", scopes::NETWORK),
        // Only while there's no recovery key (the one there was forked `global`): an admin names the next.
        (scopes::APP, "recovery") => db.recovery().is_none() && may("change-settings", scopes::NETWORK),
        (scopes::APP, "app") => {
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
        (scopes::APP, "deploy") => may("deploy", &scopes::app(&text("app")?)),
        (scopes::APP, "service") => {
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
        (scopes::APP, "install" | "uninstall" | "schedule" | "vault-request") => may("add-apps", &scopes::object(scope)) || runs_it(db, scope, step),
        (scopes::APP, "keep" | "forget") => may("use", &scopes::object(scope)) || runs_it(db, scope, step),
        // A release, which every node checks is signed with the release key before it takes it.
        (scopes::APP, kind) if scopes::RELEASES.contains(&kind) => true,
        (scopes::APP, "arrange") => *scope == Scope::User(w.to_string()) && may("use", &scopes::object(scope)),
        // An app's own entries, and the release's: as their rules count them, by who signed them.
        (scopes::APP, _) => false,
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
    use crate::database::content;

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

    /// `global`'s database after `entries`, the first the genesis, each after it applied as a step if it counts.
    fn global_of(entries: &[Entry]) -> ScopeDb {
        let mut db = first(&entries[0], 0).unwrap();
        for e in &entries[1..] {
            if let Ok(a) = apply(&db, None, &Scope::Global, e) {
                db = ScopeDb::open(a.db, 0).unwrap();
            }
        }
        db
    }

    /// Andre's Mac starts the network, its recovery key kept offline, and lists Bob's laptop, Bob an admin too.
    fn two_admins() -> (History, SigningKey, SigningKey, SigningKey) {
        use crate::keys::Endpoint;
        use crate::database::content::{device, genesis};
        use crate::database::scopes::Kind;
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
        let step = sign(&mac, &Scope::Global, 2, ANDRE, 1, 0, content::setting("name", Some("Ours"))).unwrap();
        assert!(t.take(2, Some(&step)).unwrap());
        let w = t.welcome().unwrap();
        let after = [Some(Given { step, copy: None })];
        assert_eq!(crate::database::stored::state_hash(Truth::welcomed(&w, &theirs, None, &after, 0).unwrap().db.conn()).unwrap(), w.hash);
        assert!(Truth::welcomed(&Welcome { hash: "0".repeat(64), ..w.clone() }, &theirs, None, &after, 0).is_err(), "nor another hash");
        assert!(Truth::welcomed(&w, &theirs, None, &[None], 0).is_err(), "nor a step held back");
    }

    /// A scope the freeze names starts from its start, step 0, as it was written, by a device the network listed, its
    /// copy checked against its hash; without one it can't be read, and a scope the freeze doesn't name starts empty.
    #[test]
    fn a_scope_starts_from_its_start() {
        let (h, mac, _, _) = two_admins();
        let global = global_of(&h.entries);
        let scope = Scope::User(ANDRE.into());
        let conn = super::super::scope::empty(&scope).unwrap();
        conn.execute_batch(r#"INSERT INTO installs VALUES ('notes', 1, 'person:a000000000000001', 0); INSERT INTO pages VALUES ('user', '{"items":["notes"]}');"#).unwrap();
        let mut db = ScopeDb::open(conn, 0).unwrap();
        let freeze = Freeze { at: 2, hash: "0".repeat(64), logs: [("global".to_string(), 9), (scope.to_string(), 4)].into() };
        let given = start(&mac, &scope, ANDRE, 1, 0, &mut db).unwrap();
        let given = decode(&scope, 0, &encoded(&given)).unwrap();
        let hash = |d: &ScopeDb| crate::database::stored::state_hash(d.conn()).unwrap();
        assert_eq!(hash(&started(&freeze, &global, &scope, Some(&given), 0).unwrap()), hash(&db));
        assert!(started(&freeze, &global, &scope, None, 0).is_err(), "none yet: it waits for one");
        let stranger = start(&SigningKey::from_bytes(&[9; 32]), &scope, ANDRE, 1, 0, &mut db).unwrap();
        assert!(started(&freeze, &global, &scope, Some(&stranger), 0).is_err(), "nor one from outside the network");
        let mut swapped = given.clone();
        let mut c = connection(swapped.copy.as_deref().unwrap()).unwrap();
        c.execute_batch("PRAGMA user_version = 1").unwrap();
        swapped.copy = Some(c.serialize("main").unwrap().to_vec());
        assert!(started(&freeze, &global, &scope, Some(&swapped), 0).is_err(), "nor a copy that isn't what it says");
        let other = Scope::User(BOB.into());
        assert!(started(&freeze, &global, &other, None, 0).unwrap().state().installed.is_empty(), "a scope with no frozen log starts empty");
    }

    #[test]
    fn a_scope_s_reader_stops_at_a_newer_release_s_step() {
        let (h, mac, _, _) = two_admins();
        let global = global_of(&h.entries);
        let scope = Scope::User(ANDRE.into());
        let db = || ScopeDb::empty(&scope, 0).unwrap();
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
        let keys: crate::database::stored::Keys = [(1, [9; 32])].into();
        let (_, mac, _, _) = two_admins();
        let step = |at: u64| {
            let name = crate::database::stored::step_name(&prefix, at);
            seal(&keys, &name, &encode(&sign(&mac, &Scope::Global, at, ANDRE, 1, 0, content::setting("name", Some("x"))).unwrap(), None)).unwrap()
        };
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            bucket.create(&crate::database::stored::newest_name(&prefix), br#"{"at": 1000000000, "copy": 0, "epoch": 1}"#).await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0).await.unwrap().steps.is_empty(), "nothing written, nothing read, and no failure");
            bucket.create(&crate::database::stored::step_name(&prefix, 3), b"x").await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0).await.unwrap().steps.is_empty(), "nor junk after a position with nothing");
            bucket.create(&crate::database::stored::step_name(&prefix, 2), &step(2)).await.unwrap();
            assert!(read(&bucket, &keys, &Scope::Global, 0).await.is_err(), "but a step after one with nothing stops it: that one was written");
            // Junk at 1, under an epoch no one made.
            let junk = crate::database::stored::step_name(&prefix, 1);
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
        h.write(&mac, ANDRE, content::role(BOB, Role::Member));
        let db = global_of(&h.entries);
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
        use crate::database::content::genesis;
        use crate::database::scopes::Kind;
        const ANDRE: &str = "a000000000000001";
        let mac = SigningKey::from_bytes(&[1; 32]);
        let g = Entry::create(&mac, Ext { log: "global".into(), time: 0, user: ANDRE.into(), epoch: 0, previous: Vec::new() }, None, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)));
        // A database an older release made.
        let db = first(&g, 0).unwrap();
        db.conn().pragma_update(None, "user_version", READS - 1).unwrap();
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
        const PINNED: &[(u32, &str)] = &[(2, "1a26a7b14a1b8998d5edcf6b6d00db14a698c6109eb865f7bd8e7c86dcd295a4"), (3, "e614e4c705368711133be27dd44ff51c24ff89da7fe059001ad3990f0b454a67"), (4, "faa5b6936f41b2763b4a4a4533d82323bb60e068ba3822ed6bafbf4e2921fd6b"), (5, "faa5b6936f41b2763b4a4a4533d82323bb60e068ba3822ed6bafbf4e2921fd6b")];
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
        use crate::database::content::{device, genesis, group, node, remove_device, setting};
        use crate::database::scopes::Kind;
        const ANDRE: &str = "a000000000000001";
        const BOB: &str = "b000000000000002";
        let (mac, laptop, phone) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]), SigningKey::from_bytes(&[3; 32]));
        let g = Entry::create(&mac, Ext { log: "global".into(), time: 0, user: ANDRE.into(), epoch: 0, previous: Vec::new() }, None, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)));
        let mut db = first(&g, 0).unwrap();
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
        assert!(step(&mut db, &mac, ANDRE, 1, remove_device(&laptop.verifying_key().to_bytes())));
        assert!(!step(&mut db, &laptop, BOB, 1, node("rename", json!({ "user": BOB, "name": "Robert" }))), "a removed device's next step counts for nothing");
        assert!(db.group("c000000000000003").is_some() && db.setting("name").is_none());
        // Signed for one place, it's nowhere else; and an entry, signed the same way, is never a step.
        let s = sign(&mac, &Scope::Global, 9, ANDRE, 1, 0, setting("name", Some("x"))).unwrap();
        assert!(decode(&Scope::Global, 10, &encode(&s, None)).is_err() && decode(&Scope::User(ANDRE.into()), 9, &encode(&s, None)).is_err());
        assert!(decode(&Scope::Global, 0, &encode(&g, None)).is_err());
    }
}
