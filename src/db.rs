//! App databases: an app's data in a scope is one SQLite database, the truth, kept in the bucket as steps at numbered
//! positions, each sealed under one of its scope's epochs (`src/keys.rs`) and naming it: the changeset of one write
//! (SQLite's session extension), or one of the app's schema changes. Reading it is its newest copy and the steps after;
//! writing it is a step created only if its position is free, so when two devices write at once, one wins and the
//! other catches up and runs its write again.
//!
//! ```text
//! <prefix>/newest            {"at": n, "copy": m, "epoch": e}, replaced while unchanged: a hint, so readers needn't
//!                            list; e is the epoch copy m is sealed under, the oldest a reader starting there needs
//! <prefix>/<n>               step n, from 1
//! <prefix>/<n>.sig           who wrote step n, and on a copy's step what the database came to (`Sig`), sealed as
//!                            the step is
//! <prefix>/copy/<n>          the whole database after step n, under the epoch of the steps it holds
//! ```
//!
//! A step's signature is its own object, which older readers never look for, and is written just after the step, so a
//! step can be there without one for a moment, or for good (an older writer's, or one whose writer stopped between the
//! two). A reader checks each one there is (`Checks`): the writer's endpoint key signed the step's sealed bytes at its
//! position, and, on a step a copy follows, the only one that says it (hashing every row on every write would make each
//! write as slow as the database is big), that the database it read comes to that state hash (`state_hash`). A copy
//! needs no signer: it's good if it comes to the hash signed at the step it's at. In phase 1 a step, and a copy, count
//! whatever their signatures say, as they do on older readers, since anyone with the key can write one beside an older
//! writer's step; phase 2 decides what a missing or wrong one does. From phase 2 (`SIGNED_STEPS`) a writer signs a
//! step in it (`SIGNED`), one write, so no step is ever without one: readers from R1 on read both kinds, and older ones
//! pass over the new kind, which is why it waits for the gate.
//!
//! What no reader needs goes from the bucket (`tidy`): copies before the newest hint's, once that has been there a
//! day, since readers only ever start from the hint's; and, once phase 2 switches it on (`DELETE_COVERED_STEPS`), the
//! steps before it, kept `KEEP_STEPS_FOR` first. A reader that doesn't find the next step while the newest hint says a
//! later one is written (`gap`), as when it read the start long ago, or opened with no copy to start from, finds them
//! deleted: it goes on from the hint's copy, or with none to start from says so (`Gone`), rather than read what's
//! left as the whole database, or write its next step where a deleted one was. Steps aren't merged in levels: a reader
//! reads at most `COPY_EVERY` steps after a copy, so merging would save little, for a second kind of object to write,
//! check and delete.
//!
//! Along a database's positions epochs never go down: a step under an epoch lower than the step before it, or that
//! doesn't open with the key of the epoch it names, is skipped by every reader, its position taken, and the next write
//! takes the following one. A writer writes under the newest epoch it was given, at once; the next copy is under it.
//! A step under an epoch above every key a reader was given stops it (`Ahead`): its caller looks for that epoch's
//! entry, which is written before anything is sealed under it, and if there's none, the step is junk, passed over.
//! The newest hint is only a hint, and a copy is used only if it opens and its epoch is at least its step's: otherwise
//! a reader reads the steps from the start.
//!
//! No clock, disk or network: the bucket is all it reads and writes, so it runs wherever SQLite does. A database's key
//! in each epoch comes from its app's key in that epoch (`app_key`, `kernel::database_key`).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_char, c_int, c_void};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::Connection;
use rusqlite::types::{Value as Sql, ValueRef};
use serde_json::{Map, Value, json};

use crate::bucket::Bucket;

/// A copy is written once this many steps follow the last one.
pub const COPY_EVERY: u64 = 100;
/// How many times a write runs again after losing a race before it gives up.
const TRIES: usize = 50;

/// Whether `tidy` deletes the steps the newest copy covers. Off until phase 2's gate, when every device runs R1: an
/// older reader whose copy doesn't hold reads every step from the first, and would read what's left as the whole
/// database. The one place to switch it on.
pub const DELETE_COVERED_STEPS: bool = false;

/// Whether a writer signs a step in it (`SIGNED`), one create-only write, rather than beside it (`<n>.sig`), a second
/// that may never come. Off until phase 2's gate, when every device runs R1, which reads both: an older reader passes
/// over a kind of step it doesn't know, and would lose what's written in one. The one place to switch it on.
pub const SIGNED_STEPS: bool = false;

/// How long an app database's steps are kept before `tidy` may delete them, by when the bucket wrote them. How long
/// app data's history is kept isn't decided yet: until it is, as long as the scope database's, the plan's default.
pub const KEEP_STEPS_FOR: u64 = 7 * 365 * DAY;

/// How long the newest hint's copy has been in the bucket before `tidy` deletes the copies before it: a reader that read
/// an older hint a moment ago still finds the copy it named.
const KEEP_OLDER_COPIES_FOR: u64 = DAY;

const DAY: u64 = 24 * 60 * 60 * 1000;

/// What a reader found of the signatures beside the steps it applied: those that held, those with none (an older
/// writer's, or one it couldn't fetch), and those that didn't hold (signed wrong, or the database didn't come to the
/// state hash signed).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Checks {
    pub signed: u64,
    pub unsigned: u64,
    pub wrong: u64,
}

/// What stopped a reader: steps it hadn't read were deleted (`tidy`), and the newest hint's copy, at `copy`, which it
/// would go on from instead, isn't there or doesn't open for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gone {
    pub copy: u64,
}

impl std::fmt::Display for Gone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "steps it hadn't read were deleted, and its copy at step {}, which holds them, isn't there or doesn't open with the keys it was given", self.copy)
    }
}

impl std::error::Error for Gone {}

/// A database's keys, by the epoch each opens.
pub type Keys = BTreeMap<u32, [u8; 32]>;

/// What a read or write stopped at: a copy or step sealed under an epoch whose key it wasn't given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    pub epoch: u32,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "it needs the key of epoch {}, which it wasn't given", self.epoch)
    }
}

impl std::error::Error for Missing {}

/// What a read stopped at: step `at`, under an epoch above every key it was given, which is either a new epoch's (its
/// entry says so) or junk (none does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ahead {
    pub epoch: u32,
    pub at: u64,
}

impl std::fmt::Display for Ahead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "step {} is under epoch {}, newer than any key it was given", self.at, self.epoch)
    }
}

impl std::error::Error for Ahead {}

/// The most epochs a scope looks for sealed to a device that knows none of its entries yet.
pub const EPOCHS_MAX: u32 = 10_000;

/// Where a reader starts: the copy the newest hint names, as (its position, its epoch, its bytes), if it holds: it's
/// there, its epoch is one `keys` reaches, at least that of the step at its position, and it opens. Otherwise none, and
/// the reader reads every step. Fails with `Missing` for a copy under an epoch below the newest it was given but not given,
/// and with `Ahead` at the copy's step when both are under an epoch above every key it was given, unless that step is
/// one of `junk`: a reader that reads every step could only stop at one such step, after reading all those before.
pub async fn start(bucket: &impl Bucket, prefix: &str, keys: &Keys, junk: &BTreeSet<u64>) -> Result<Option<(u64, u32, Vec<u8>)>> {
    let copy = match bucket.get(&newest_name(prefix)).await? {
        Some(o) => newest(&o.bytes).1,
        None => 0,
    };
    start_at(bucket, prefix, keys, junk, copy).await
}

/// `start`, from the copy at `copy`, which the newest hint names.
async fn start_at(bucket: &impl Bucket, prefix: &str, keys: &Keys, junk: &BTreeSet<u64>, copy: u64) -> Result<Option<(u64, u32, Vec<u8>)>> {
    if copy == 0 || junk.contains(&copy) {
        return Ok(None);
    }
    let name = copy_name(prefix, copy);
    let Some(sealed) = bucket.get(&name).await? else { return Ok(None) };
    let Ok(epoch) = crate::keys::epoch_of(&sealed.bytes) else { return Ok(None) };
    let step = bucket.get(&step_name(prefix, copy)).await?.and_then(|s| crate::keys::epoch_of(&s.bytes).ok());
    let given = keys.keys().next_back().copied().unwrap_or(0);
    if let Some(step) = step.filter(|s| *s > given && *s <= epoch) {
        bail!(Ahead { epoch: step, at: copy });
    }
    if epoch > given || step.is_none_or(|s| s > epoch) {
        tracing::debug!("{name} isn't a copy to start from: reading every step");
        return Ok(None);
    }
    let key = keys.get(&epoch).ok_or(Missing { epoch })?;
    match crate::keys::open(key, &name, &sealed.bytes) {
        Ok(bytes) => Ok(Some((copy, epoch, bytes))),
        Err(err) => {
            tracing::debug!("{name}: {err:#}: reading every step");
            Ok(None)
        }
    }
}

/// The keys a node uses, of those a caller handed it, for a database of a scope whose current epoch is `current`, as
/// the install's signed `epoch` entries say: none above it (one a member made up would leave the database where nobody
/// else can follow), and `current`'s among them, the one it writes under. A key with the right number but the wrong
/// bytes only fails to open what's under its epoch, so a database opened with handed keys is shared only with calls
/// handed the same keys.
pub fn handed(mut keys: Keys, current: u32) -> std::result::Result<Keys, Missing> {
    keys.retain(|n, _| *n <= current);
    if keys.contains_key(&current) { Ok(keys) } else { Err(Missing { epoch: current }) }
}

/// An app's key in epoch `n` of a scope, from its copy in the bucket (`keys::app_key_name`), sealed under that epoch's
/// key. The first to need it makes it, at random, unless `make` is false (an epoch nothing was written under yet,
/// where a reader has nothing to open): whichever is written first is the key.
pub async fn app_key(bucket: &impl Bucket, epoch_key: &[u8; 32], scope: &str, app: &str, n: u32, make: bool) -> Result<Option<[u8; 32]>> {
    let name = crate::keys::app_key_name(scope, app, n);
    if let Some(o) = bucket.get(&name).await? {
        return crate::kernel::open_app_key(epoch_key, app, &o.bytes).map(Some);
    }
    if !make {
        return Ok(None);
    }
    let key = rand::random();
    if bucket.create(&name, &crate::kernel::seal_app_key(epoch_key, app, &key)).await? {
        return Ok(Some(key));
    }
    let made = bucket.get(&name).await?.context("an app's key went as it was made")?;
    crate::kernel::open_app_key(epoch_key, app, &made.bytes).map(Some)
}

/// One SQLite database of an app's, at a position in its steps.
pub struct Db {
    prefix: String,
    keys: Keys,
    conn: Connection,
    /// The last step applied (or passed over).
    at: u64,
    /// The newest copy this device knows of.
    copy: u64,
    /// The epoch of the last step applied, or of the copy it started from: 0 before either.
    epoch: u32,
    /// The epoch of the copy it started from, as the newest hint says it.
    copy_epoch: u32,
    /// The epochs of what it read: the copy it started from and the steps it applied.
    used: BTreeSet<u32>,
    /// Positions its caller found hold junk under an epoch no entry made (`Ahead`), passed over.
    junk: BTreeSet<u64>,
    /// Whose endpoint key signs the steps it writes: none, and it writes them unsigned, as older writers do.
    signer: Option<ed25519_dalek::SigningKey>,
    /// Whether it signs a step in it (`SIGNED`), rather than beside it: `SIGNED_STEPS`.
    in_step: bool,
    /// What it found of the signatures beside the steps it applied.
    checks: Checks,
}

impl Db {
    /// Reads a database as its newest steps leave it, with its keys by epoch: empty if it has none. Fails with `Missing` at a copy or step under an epoch whose key it wasn't given, and with
    /// `Ahead` at a step under an epoch above them all.
    pub async fn open(bucket: &impl Bucket, prefix: &str, keys: Keys) -> Result<Db> {
        Db::open_passing(bucket, prefix, keys, BTreeSet::new()).await
    }

    /// `open`, passing over the steps at `junk`.
    pub async fn open_passing(bucket: &impl Bucket, prefix: &str, keys: Keys, junk: BTreeSet<u64>) -> Result<Db> {
        let mut db = Db {
            prefix: prefix.into(),
            keys,
            conn: Connection::open_in_memory()?,
            at: 0,
            copy: 0,
            epoch: 0,
            copy_epoch: 0,
            used: BTreeSet::new(),
            junk,
            signer: None,
            in_step: SIGNED_STEPS,
            checks: Checks::default(),
        };
        let hinted = match bucket.get(&newest_name(prefix)).await? {
            Some(o) => newest(&o.bytes).1,
            None => 0,
        };
        if !db.start_from(bucket, hinted).await? {
            confine(&mut db.conn)?;
        }
        // A bucket that stops answering partway: it goes on from what it read while that gets it further, rather than
        // failing and its caller reading it all again, which over a bad link may never get to the end.
        let mut stuck = 0;
        loop {
            let from = db.at;
            match db.catch_up(bucket).await {
                Ok(_) => return Ok(db),
                Err(err) if unanswered(&err) && stuck < 3 => {
                    stuck = if db.at > from { 0 } else { stuck + 1 };
                    bucket.wait(100).await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Signs the steps it writes from now on with this endpoint key.
    pub fn sign_as(&mut self, key: ed25519_dalek::SigningKey) {
        self.signer = Some(key);
    }

    /// What it found of the signatures beside the steps it applied.
    pub fn checks(&self) -> Checks {
        self.checks
    }

    /// Starts from the copy at `copy`, if there's one to start from (`start_at`): whether there was. Its step's
    /// signature is checked, and whether the copy comes to the state hash it signs, but in phase 1 a copy counts whatever
    /// that says, as it does on older readers: a signature anyone with the key could write beside an older writer's step
    /// mustn't send every reader back to the first step.
    async fn start_from(&mut self, bucket: &impl Bucket, copy: u64) -> Result<bool> {
        let Some((copy, epoch, bytes)) = start_at(bucket, &self.prefix, &self.keys, &self.junk, copy).await? else { return Ok(false) };
        // One it can't fetch is as good as none.
        let signed = signature_at(bucket, &self.prefix, &self.keys, copy).await.unwrap_or(None);
        if let Some(hash) = self.count(copy, signed)
            && state_hash(&copied(&bytes)?)? != hash
        {
            self.wrong_hash(copy);
        }
        // PHASE 2: here a copy whose step's signature doesn't hold, or that doesn't come to its hash, isn't started from.
        let mut conn = Connection::open_in_memory()?;
        conn.deserialize_read_exact("main", &bytes[..], bytes.len(), false)?;
        // After the copy, which SQLite reads in by attaching it.
        confine(&mut conn)?;
        self.conn = conn;
        (self.at, self.copy, self.epoch, self.copy_epoch) = (copy, copy, epoch, epoch);
        self.used = BTreeSet::from([epoch]);
        Ok(true)
    }

    /// Passes over step `at`, under an epoch no entry made, as every reader does.
    pub fn pass_over(&mut self, at: u64) {
        self.junk.insert(at);
    }

    pub fn at(&self) -> u64 {
        self.at
    }

    /// The epoch of the last step it applied, or the copy it started from: 0 before either.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// The keys it was opened with, by epoch.
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// Takes keys of epochs it wasn't given, as a device does when it's handed a scope's new epoch: what it holds
    /// stays as it was read.
    pub fn add_keys(&mut self, keys: &Keys) {
        for (epoch, key) in keys {
            self.keys.entry(*epoch).or_insert(*key);
        }
    }

    /// Whether `keys` open everything it read, and are the same as its own for every epoch both have: so a call given
    /// them may use it as it is.
    pub fn opens_for(&self, keys: &Keys) -> bool {
        self.used.iter().all(|e| keys.contains_key(e)) && keys.iter().all(|(e, k)| self.keys.get(e).is_none_or(|mine| mine == k))
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Applies the steps written since this device last read: whether there were any.
    pub async fn catch_up(&mut self, bucket: &impl Bucket) -> Result<bool> {
        let from = self.at;
        loop {
            let name = self.step(self.at + 1);
            let Some(o) = bucket.get(&name).await? else {
                if self.gap(bucket).await? {
                    continue;
                }
                break;
            };
            if self.junk.contains(&(self.at + 1)) {
                self.at += 1;
                continue;
            }
            if let Some((epoch, step)) = self.unseal(self.at + 1, &name, &o.bytes)? {
                // One it can't fetch is as good as none: in phase 1 a step counts whatever its signature says.
                // PHASE 2: here a step with no signature, or one that doesn't hold, stops counting.
                let signed = signature(bucket, &self.prefix, &self.keys[&epoch], &name, &o.bytes, &step).await.unwrap_or(None);
                let hash = self.count(self.at + 1, signed);
                self.apply(&step)?;
                self.epoch = epoch;
                self.used.insert(epoch);
                if let Some(hash) = hash
                    && state_hash(&self.conn)? != hash
                {
                    self.wrong_hash(self.at + 1);
                }
            }
            self.at += 1;
        }
        Ok(self.at > from)
    }

    /// When the step after the last it read isn't there: whether steps it hasn't read were deleted (`tidy`), as when it
    /// read the start long ago, or its copy here has steps the bucket no longer has. The newest hint says a later step
    /// is written; then the reader goes on from the hint's copy (true), or, with none to start from, fails with `Gone`,
    /// rather than take what it has for the whole database, and write its next step where one was.
    async fn gap(&mut self, bucket: &impl Bucket) -> Result<bool> {
        let next = self.at + 1;
        let Some(hint) = bucket.get(&newest_name(&self.prefix)).await? else { return Ok(false) };
        let (at, copy, _) = newest(&hint.bytes);
        if at < next {
            return Ok(false);
        }
        // Written since it looked: it reads on.
        if bucket.get(&self.step(next)).await?.is_some() {
            return Ok(true);
        }
        // A hint that says a step is written that isn't is someone's junk.
        if bucket.get(&self.step(at)).await?.is_none() {
            return Ok(false);
        }
        if copy > self.at && self.start_from(bucket, copy).await? {
            tracing::info!("{}: the steps after {} were deleted: read on from the copy at {copy}", self.prefix, next - 1);
            return Ok(true);
        }
        bail!(Gone { copy })
    }

    /// Counts what a step's signature, at `at`, says: → the state hash it signs, if it holds and says one.
    fn count(&mut self, at: u64, signed: Option<Result<Sig>>) -> Option<String> {
        match signed {
            None => {
                self.checks.unsigned += 1;
                None
            }
            Some(Ok(sig)) => {
                self.checks.signed += 1;
                sig.hash
            }
            Some(Err(err)) => {
                self.checks.wrong += 1;
                tracing::warn!("{}: step {at}'s signature: {err:#}", self.prefix);
                None
            }
        }
    }

    /// After what it read to step `at` didn't come to the state hash that step's signature says.
    fn wrong_hash(&mut self, at: u64) {
        self.checks.signed -= 1;
        self.checks.wrong += 1;
        tracing::warn!("{}: what it read to step {at} isn't what that step's writer signed it comes to", self.prefix);
    }

    /// Step `at`, opened: None for one every reader passes over, under an epoch lower than the step before it or that
    /// doesn't open with the key of the epoch it names.
    fn unseal(&self, at: u64, name: &str, sealed: &[u8]) -> Result<Option<(u32, Vec<u8>)>> {
        let Ok(epoch) = crate::keys::epoch_of(sealed) else { return Ok(None) };
        if epoch < self.epoch {
            tracing::debug!("{name} is under epoch {epoch}, after {}: passed over", self.epoch);
            return Ok(None);
        }
        if self.keys.keys().next_back().is_none_or(|newest| epoch > *newest) {
            bail!(Ahead { epoch, at });
        }
        let key = self.keys.get(&epoch).ok_or(Missing { epoch })?;
        match crate::keys::open(key, name, sealed) {
            Ok(step) => Ok(Some((epoch, step))),
            Err(err) => {
                tracing::debug!("{name}: {err:#}: passed over");
                Ok(None)
            }
        }
    }

    /// The newest epoch it was given, and its key: what it writes under.
    fn newest_key(&self) -> Result<(u32, [u8; 32])> {
        let (epoch, key) = self.keys.last_key_value().with_context(|| format!("{} was opened with no keys to write with", self.prefix))?;
        ensure!(*epoch >= self.epoch, Missing { epoch: self.epoch });
        Ok((*epoch, *key))
    }

    /// Brings the database's schema to the app's newest: each of `migrations` it lacks, in order, as a step.
    pub async fn migrate(&mut self, bucket: &impl Bucket, migrations: &[String]) -> Result<()> {
        for _ in 0..TRIES {
            self.catch_up(bucket).await?;
            let version = self.version()?;
            let Some(sql) = migrations.get(version) else { return Ok(()) };
            let step = [b"s".as_slice(), &serde_json::to_vec(&json!({ "version": version + 1, "sql": sql }))?].concat();
            // Tried here first, so a schema that fails is never written, and for what it comes to, if that's signed.
            self.conn.execute_batch("savepoint migrate")?;
            let tried = self.apply(&step).and_then(|()| self.hash_due(self.at + 1));
            self.conn.execute_batch("rollback to migrate; release migrate")?;
            let hash = tried?;
            let (epoch, key) = self.newest_key()?;
            let name = self.step(self.at + 1);
            let written = signed(self.signer.as_ref().filter(|_| self.in_step), &name, &step, hash.as_deref());
            let sealed = crate::keys::seal(epoch, &key, &name, &written);
            if bucket.create(&name, &sealed).await? {
                self.apply(&step)?;
                self.wrote(epoch);
                self.sign(bucket, &name, epoch, &key, &sealed, hash.as_deref()).await;
                self.written(bucket).await;
            }
        }
        bail!("{} changed too often to change its schema", self.prefix)
    }

    /// The schema changes this database has had.
    pub fn version(&self) -> Result<usize> {
        Ok(self.conn.query_row("pragma user_version", [], |r| r.get::<_, i64>(0))? as usize)
    }

    /// Runs a write in one transaction, which is the step written after this device's newest, under the newest epoch
    /// it was given: when another device wrote that first, it catches up and runs `f` again, so `f` must do only
    /// database work.
    pub async fn write<R>(&mut self, bucket: &impl Bucket, mut f: impl FnMut(&Connection) -> Result<R>) -> Result<R> {
        for _ in 0..TRIES {
            self.catch_up(bucket).await?;
            let (epoch, key) = self.newest_key()?;
            let due = self.signer.is_some() && self.copy_due(self.at + 1);
            let open = Open::begin(&mut self.conn)?;
            let (result, changes) = changes(&open, &mut f)?;
            if changes.is_empty() {
                open.execute_batch("commit")?;
                return Ok(result);
            }
            let name = step_name(&self.prefix, self.at + 1);
            let hash = if due { Some(state_hash(&open)?) } else { None };
            let step = signed(self.signer.as_ref().filter(|_| self.in_step), &name, &[b"c".as_slice(), &changes].concat(), hash.as_deref());
            let sealed = crate::keys::seal(epoch, &key, &name, &step);
            if created(bucket, &name, &sealed).await? {
                open.execute_batch("commit")?;
                drop(open);
                self.wrote(epoch);
                self.sign(bucket, &name, epoch, &key, &sealed, hash.as_deref()).await;
                self.written(bucket).await;
                return Ok(result);
            }
        }
        bail!("{} changed too often to write to", self.prefix)
    }

    /// After its own step under `epoch` was created at the next position.
    fn wrote(&mut self, epoch: u32) {
        self.at += 1;
        self.epoch = epoch;
        self.used.insert(epoch);
    }

    /// Reads with a statement that only reads: its rows, each column by name.
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
        query(&self.conn, sql, params)
    }

    /// The database's own connection, for reads that need more than `query`.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    fn step(&self, n: u64) -> String {
        step_name(&self.prefix, n)
    }

    /// Whether the step it writes at `at` is followed by a copy (`hint`): only that one's signature says the state hash,
    /// since only a copy is checked by it (`start_from`, `tidy`), and hashing every row on every write would make each
    /// write as slow as the database is big.
    fn copy_due(&self, at: u64) -> bool {
        at >= self.copy + COPY_EVERY
    }

    /// The state hash its step at `at` signs, if it's a copy's and it signs its steps: of the database as it is now.
    fn hash_due(&self, at: u64) -> Result<Option<String>> {
        Ok(match self.signer.is_some() && self.copy_due(at) {
            true => Some(state_hash(&self.conn)?),
            false => None,
        })
    }

    /// After writing step `name`, sealed under `epoch`'s `key` as `sealed`: its signature beside it, saying `hash`, if
    /// it has a signer and doesn't sign in the step. Tried once, since the step is written whatever happens to it: a
    /// reader takes a step with none as an older writer's.
    async fn sign(&mut self, bucket: &impl Bucket, name: &str, epoch: u32, key: &[u8; 32], sealed: &[u8], hash: Option<&str>) {
        let Some(signer) = self.signer.as_ref().filter(|_| !self.in_step) else { return };
        let sig = format!("{name}.sig");
        match bucket.create(&sig, &Sig::seal(signer, epoch, key, name, sealed, hash)).await {
            Ok(true) => {}
            Ok(false) => tracing::warn!("{sig} was there before its step's writer signed it"),
            Err(err) => tracing::debug!("{sig}: {err:#}"),
        }
    }

    /// After writing a step: a copy, if steps have piled up since the last, and the newest hint moved on. Both only
    /// save readers work, so a link that fails them fails nothing.
    async fn written(&mut self, bucket: &impl Bucket) {
        if let Err(err) = self.hint(bucket).await {
            tracing::debug!("{}: {err:#}", self.prefix);
        }
    }

    async fn hint(&mut self, bucket: &impl Bucket) -> Result<()> {
        if self.at >= self.copy + COPY_EVERY
            && let Some(key) = self.keys.get(&self.epoch)
        {
            let name = copy_name(&self.prefix, self.at);
            let bytes = self.conn.serialize("main")?.to_vec();
            bucket.create(&name, &crate::keys::seal(self.epoch, key, &name, &bytes)).await?;
            (self.copy, self.copy_epoch) = (self.at, self.epoch);
        }
        hint(bucket, &self.prefix, self.at, self.copy, self.copy_epoch).await
    }

    /// Applies a step: a changeset, or a schema change this database is due.
    fn apply(&self, step: &[u8]) -> Result<()> {
        match step.split_first() {
            Some((b'c', changes)) => apply_changes(&self.conn, changes),
            Some((b's', change)) => {
                let change: Value = serde_json::from_slice(change)?;
                let version = change["version"].as_u64().context("a schema change has its version")? as usize;
                if self.version()? + 1 == version {
                    self.conn.execute_batch(&format!("{}\n;pragma user_version = {version}", change["sql"].as_str().context("a schema change has its SQL")?))?;
                }
                Ok(())
            }
            // One signed in it: the step it signs.
            Some((&SIGNED, _)) => match split_signed(step) {
                Some((_, inner)) if inner.first() != Some(&SIGNED) => self.apply(inner),
                _ => Ok(()),
            },
            // A kind of step a newer device writes.
            _ => Ok(()),
        }
    }
}

/// Deletes from the bucket, at `now` (ms), what no reader of the database at `prefix` needs: copies before the newest
/// hint's, once that copy has been there `KEEP_OLDER_COPIES_FOR`, and with `steps` (`DELETE_COVERED_STEPS`), the steps
/// before it and their signatures, each once it's been there `KEEP_STEPS_FOR`. Only once the hint's copy has been
/// there that long and opens with `keys` (passing over `junk`), and comes to the state hash signed at its step if
/// that has one; the steps go only if it has. → how many objects went.
pub async fn tidy(bucket: &impl Bucket, prefix: &str, keys: &Keys, junk: &BTreeSet<u64>, now: u64, steps: bool) -> Result<usize> {
    let Some((copy, _, bytes)) = start(bucket, prefix, keys, junk).await? else { return Ok(0) };
    let signed = match signature_at(bucket, prefix, keys, copy).await? {
        Some(Ok(Sig { hash: Some(hash), .. })) if state_hash(&copied(&bytes)?)? == hash => true,
        None | Some(Ok(Sig { hash: None, .. })) => false,
        _ => return Ok(0),
    };
    let copies = bucket.objects(&format!("{}/copy", prefix)).await?;
    let newest = copies.iter().find(|(n, _)| *n == format!("{copy:012}")).map(|(_, written)| *written);
    if newest.is_none_or(|w| w + KEEP_OLDER_COPIES_FOR > now) {
        return Ok(0);
    }
    let mut gone = 0;
    for (n, _) in copies.iter().filter(|(n, _)| n.parse::<u64>().is_ok_and(|n| n < copy)) {
        bucket.delete(&format!("{}/copy/{n}", prefix)).await?;
        gone += 1;
    }
    if !(steps && signed) {
        return Ok(gone);
    }
    for (n, written) in bucket.objects(prefix).await? {
        let full = format!("{}/{n}", prefix);
        if position(&full).is_some_and(|(_, at)| at < copy) && written + KEEP_STEPS_FOR <= now {
            bucket.delete(&full).await?;
            gone += 1;
        }
    }
    Ok(gone)
}

/// How long an object whose answer was lost is sent again for, waiting longer each time, before it may have been
/// written: a call's own time runs out sooner.
const ANSWERED_WITHIN_MS: u64 = 60_000;

/// Creates an object that only this device would write with these bytes, such as a write's step: whether they're
/// there now, rather than another's. When the answer is lost, the same is sent again until the bucket says, rather than
/// asked what's there, since the first may still be on its way and land after a read that found nothing: a write that
/// took that for a lost race would run again on top of itself, and one told it failed would be there after all. One the
/// bucket refused, or that never left, isn't there, unless one sent before is.
pub async fn created(bucket: &impl Bucket, name: &str, bytes: &[u8]) -> Result<bool> {
    let may_have = "the bucket didn't answer whether it was written, so it may have been: look before trying again";
    let (mut sent, mut wait, mut waited) = (false, 100, 0);
    loop {
        match bucket.create(name, bytes).await {
            Ok(true) => return Ok(true),
            Ok(false) if !sent => return Ok(false),
            // One sent before, whose answer was lost, may be what's there.
            Ok(false) => match bucket.get(name).await {
                Ok(o) => return Ok(o.with_context(|| format!("{name} went once written"))?.bytes == bytes),
                Err(err) if !unanswered(&err) => return Err(err.context(may_have)),
                Err(_) => {}
            },
            Err(err) if !unanswered(&err) => return Err(if sent { err.context(may_have) } else { err }),
            Err(_) => sent = true,
        }
        if waited >= ANSWERED_WITHIN_MS {
            bail!("{may_have}");
        }
        bucket.wait(wait).await;
        waited += wait;
        wait = (wait * 2).min(5_000);
    }
}

/// Whether a request's answer never came, so it may have done what it asked.
fn unanswered(err: &anyhow::Error) -> bool {
    crate::bucket::Trouble::of(err) == Some(crate::bucket::Trouble::Unreachable)
}

/// A database's objects in the bucket, by its prefix: its newest hint, step `n`, and its copy after step `n`.
pub fn newest_name(prefix: &str) -> String {
    format!("{prefix}/newest")
}

pub fn step_name(prefix: &str, n: u64) -> String {
    format!("{prefix}/{n:012}")
}

pub fn copy_name(prefix: &str, n: u64) -> String {
    format!("{prefix}/copy/{n:012}")
}

/// The signature beside step `n`.
pub fn sig_name(prefix: &str, n: u64) -> String {
    format!("{}.sig", step_name(prefix, n))
}

/// Whether the steps at `prefix` are a database's, with signatures beside them, rather than a scope's log's
/// (`log::prefix`), whose entries are signed by their authors already.
pub fn signed_beside(prefix: &str) -> bool {
    !prefix.ends_with(&format!("/{}/{}", crate::members::APP, crate::log::NAME))
}

/// A database's prefix and position, from the name of one of its steps or the signature beside one.
pub fn position(name: &str) -> Option<(&str, u64)> {
    let (prefix, last) = name.rsplit_once('/')?;
    let n = last.strip_suffix(".sig").unwrap_or(last).parse::<u64>().ok().filter(|n| *n > 0)?;
    [step_name(prefix, n), sig_name(prefix, n)].contains(&name.to_string()).then_some((prefix, n))
}

/// A step's signature: its writer's endpoint key (`by`), and, on a step a copy follows, the state hash of the database
/// after it (`hash`). Beside the step (`<prefix>/<n>.sig`), it's `{"by", "hash"?, "sig"}`, sealed under the step's
/// epoch and bound to its own name, since who wrote what is as much the scope's as what they wrote: `by` hex, and `sig`
/// (base64url) the writer's signature, under `one/step`, over the canonical `{"hash"?, "name", "step"}`, the step's name
/// (its database and position) and the BLAKE3 of its sealed bytes (hex). In the step (`SIGNED`), the same JSON comes
/// before the step it signs, and `sig` is under `one/signed-step`, with `step` the BLAKE3 of the step it signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sig {
    pub by: [u8; 32],
    pub hash: Option<String>,
}

impl Sig {
    const BESIDE: &str = "one/step";
    const IN: &str = "one/signed-step";

    fn signed(name: &str, what: &[u8], hash: Option<&str>) -> Value {
        let mut signed = json!({ "name": name, "step": blake3::hash(what).to_hex().as_str() });
        if let Some(hash) = hash {
            signed["hash"] = json!(hash);
        }
        signed
    }

    /// `{"by", "hash"?, "sig"}`: `signer`'s signature, under `domain`, of `what` at `name`, saying `hash`.
    fn make(signer: &ed25519_dalek::SigningKey, domain: &str, name: &str, what: &[u8], hash: Option<&str>) -> Vec<u8> {
        let sig = crate::keys::sign(signer, domain, &Self::signed(name, what, hash));
        let mut made = json!({ "by": crate::entry::hex(signer.verifying_key().as_bytes()), "sig": data_encoding::BASE64URL_NOPAD.encode(&sig) });
        if let Some(hash) = hash {
            made["hash"] = json!(hash);
        }
        crate::keys::canonical(&made)
    }

    /// Reads `{"by", "hash"?, "sig"}` and checks it signs `what` at `name` under `domain`.
    fn read(bytes: &[u8], domain: &str, name: &str, what: &[u8]) -> Result<Sig> {
        let v: Value = serde_json::from_slice(bytes).context("a step's signature is JSON")?;
        let by: [u8; 32] = crate::keys::unhex(v["by"].as_str().context("a step's signature says who signed it")?)?;
        let hash = match &v["hash"] {
            Value::Null => None,
            hash => Some(hash.as_str().context("a step's state hash is text")?.to_string()),
        };
        let signature = data_encoding::BASE64URL_NOPAD.decode(v["sig"].as_str().context("a step's signature has its signature")?.as_bytes())?;
        crate::keys::verify(&by, domain, &Self::signed(name, what, hash.as_deref()), &signature).with_context(|| format!("{name}'s signer"))?;
        Ok(Sig { by, hash })
    }

    /// Step `name`'s signature beside it, by `signer`, saying `hash`, sealed under `epoch`'s `key`.
    pub fn seal(signer: &ed25519_dalek::SigningKey, epoch: u32, key: &[u8; 32], name: &str, sealed: &[u8], hash: Option<&str>) -> Vec<u8> {
        crate::keys::seal(epoch, key, &format!("{name}.sig"), &Self::make(signer, Self::BESIDE, name, sealed, hash))
    }

    /// Opens step `name`'s signature beside it with the key the step opened with, and checks it signs the step's
    /// `sealed` bytes there.
    pub fn check(key: &[u8; 32], name: &str, sealed: &[u8], sig: &[u8]) -> Result<Sig> {
        Self::read(&crate::keys::open(key, &format!("{name}.sig"), sig)?, Self::BESIDE, name, sealed)
    }

    /// `step`, to be written at `name`, with `signer`'s signature in it, saying `hash`.
    pub fn wrap(signer: &ed25519_dalek::SigningKey, name: &str, step: &[u8], hash: Option<&str>) -> Vec<u8> {
        let sig = Self::make(signer, Self::IN, name, step, hash);
        [&[SIGNED][..], &(sig.len() as u32).to_be_bytes(), &sig, step].concat()
    }

    /// The signature in step `name`, opened as `step`, if it's one signed in it.
    pub fn unwrap(name: &str, step: &[u8]) -> Option<Result<Sig>> {
        if step.first() != Some(&SIGNED) {
            return None;
        }
        Some(match split_signed(step) {
            Some((sig, inner)) => Self::read(sig, Self::IN, name, inner),
            None => Err(anyhow::anyhow!("{name} is a signed step that doesn't say where its signature ends")),
        })
    }
}

/// A step signed in it: `v`, its signature's length (4 bytes, big-endian), its signature (`Sig`), then the step it
/// signs, a changeset (`c`) or a schema change (`s`).
pub const SIGNED: u8 = b'v';

/// A step signed in it, opened: its signature's bytes, and the step it signs.
fn split_signed(step: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = step.strip_prefix(&[SIGNED])?;
    let len = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
    let rest = &rest[4..];
    (rest.len() >= len).then(|| rest.split_at(len))
}

/// `step`, to be written at `name`: with `signer`'s signature in it, saying `hash`, if there's one (`SIGNED_STEPS`).
fn signed(signer: Option<&ed25519_dalek::SigningKey>, name: &str, step: &[u8], hash: Option<&str>) -> Vec<u8> {
    match signer {
        Some(signer) => Sig::wrap(signer, name, step, hash),
        None => step.to_vec(),
    }
}

/// The signature in or beside step `name`, sealed as `sealed` under `key` and opened as `step`: none if it has none.
async fn signature(bucket: &impl Bucket, prefix: &str, key: &[u8; 32], name: &str, sealed: &[u8], step: &[u8]) -> Result<Option<Result<Sig>>> {
    if let Some(sig) = Sig::unwrap(name, step) {
        return Ok(Some(sig));
    }
    let Some((_, at)) = position(name) else { return Ok(None) };
    Ok(bucket.get(&sig_name(prefix, at)).await?.map(|s| Sig::check(key, name, sealed, &s.bytes)))
}

/// The signature in or beside the step at `at`: none if it has none, or isn't there, or doesn't open with `keys`.
async fn signature_at(bucket: &impl Bucket, prefix: &str, keys: &Keys, at: u64) -> Result<Option<Result<Sig>>> {
    let name = step_name(prefix, at);
    let Some(o) = bucket.get(&name).await? else { return Ok(None) };
    let Some(key) = crate::keys::epoch_of(&o.bytes).ok().and_then(|e| keys.get(&e)) else { return Ok(None) };
    let Ok(step) = crate::keys::open(key, &name, &o.bytes) else { return Ok(None) };
    signature(bucket, prefix, key, &name, &o.bytes, &step).await
}

/// A database's state, the same on every device that holds the same rows, whatever order its steps came in and however
/// SQLite laid out its pages: BLAKE3, hex, over `one/state`, its schema version, then each table by name but SQLite's
/// own and virtual ones (whose rows are their shadow tables'): its name, its columns' names, and its rows in primary
/// key order, each value with its type. A table without a primary key is its rows by rowid, `rowid` its first column:
/// its steps name its rows by rowid (`changes`), so two devices whose rowids differ hold different databases. A row's bytes are
/// `r`, then each value: `n` for null, `i` and 8 bytes big-endian, `f` and the 8 bytes of its IEEE 754 bits, or `s`
/// (text) or `b` (blob) with a 4-byte big-endian length and its bytes. A table's name is written as text is but with
/// `t`, and each column's name as text.
pub fn state_hash(conn: &Connection) -> Result<String> {
    fn bytes(h: &mut blake3::Hasher, kind: u8, b: &[u8]) {
        h.update(&[kind]);
        h.update(&(b.len() as u32).to_be_bytes());
        h.update(b);
    }
    let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    let mut h = blake3::Hasher::new();
    h.update(b"one/state\0");
    h.update(&conn.query_row("pragma user_version", [], |r| r.get::<_, i64>(0))?.to_be_bytes());
    let tables: Vec<String> = conn
        .prepare("select name from sqlite_schema where type = 'table' and name not like 'sqlite\\_%' escape '\\' and sql not like 'create virtual table%' order by name")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for table in tables {
        let mut keys: Vec<(i64, String)> = conn.prepare("select pk, name from pragma_table_info(?1) where pk > 0")?.query_map([&table], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        keys.sort();
        let sql = match keys.is_empty() {
            true => format!("select rowid, * from {} order by rowid", quote(&table)),
            false => format!("select * from {} order by {}", quote(&table), keys.iter().map(|(_, k)| quote(k)).collect::<Vec<_>>().join(", ")),
        };
        let mut stmt = conn.prepare(&sql)?;
        bytes(&mut h, b't', table.as_bytes());
        for name in stmt.column_names() {
            bytes(&mut h, b's', name.as_bytes());
        }
        let n = stmt.column_count();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            h.update(b"r");
            for i in 0..n {
                match row.get_ref(i)? {
                    ValueRef::Null => drop(h.update(b"n")),
                    ValueRef::Integer(v) => drop(h.update(b"i").update(&v.to_be_bytes())),
                    ValueRef::Real(v) => drop(h.update(b"f").update(&v.to_bits().to_be_bytes())),
                    ValueRef::Text(t) => bytes(&mut h, b's', t),
                    ValueRef::Blob(b) => bytes(&mut h, b'b', b),
                }
            }
        }
    }
    Ok(h.finalize().to_hex().to_string())
}

/// A copy's bytes as a database of its own, to read.
fn copied(bytes: &[u8]) -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    conn.deserialize_read_exact("main", bytes, bytes.len(), true)?;
    Ok(conn)
}

/// Moves the newest hint at `prefix` on to say at least step `at` and copy `copy` (sealed under `epoch`), replacing it
/// only while unchanged.
pub async fn hint(bucket: &impl Bucket, prefix: &str, at: u64, copy: u64, epoch: u32) -> Result<()> {
    let name = newest_name(prefix);
    for _ in 0..TRIES {
        let current = bucket.get(&name).await?;
        let mut said = current.as_ref().map(|o| newest(&o.bytes));
        // One that says a step is written that isn't is someone's junk, not raised on.
        if let Some((was_at, _, _)) = said
            && was_at > at
            && bucket.get(&step_name(prefix, was_at)).await?.is_none()
        {
            said = Some((0, 0, 0));
        }
        let (was_at, was_copy, was_epoch) = said.unwrap_or((0, 0, 0));
        let epoch = if copy >= was_copy { epoch } else { was_epoch };
        let (at, copy) = (at.max(was_at), copy.max(was_copy));
        if current.as_ref().map(|o| newest(&o.bytes)) == Some((at, copy, epoch)) {
            return Ok(());
        }
        if bucket.replace(&name, &serde_json::to_vec(&json!({ "at": at, "copy": copy, "epoch": epoch }))?, current.as_ref().map(|o| o.tag.as_str())).await? {
            return Ok(());
        }
    }
    Ok(())
}

/// What a database's newest hint says: (its last step, its newest copy, the epoch that copy is sealed under). One
/// that isn't a hint says nothing.
pub fn newest(bytes: &[u8]) -> (u64, u64, u32) {
    let v: Value = serde_json::from_slice(bytes).unwrap_or_default();
    (v["at"].as_u64().unwrap_or(0), v["copy"].as_u64().unwrap_or(0), u32::try_from(v["epoch"].as_u64().unwrap_or(0)).unwrap_or(0))
}

/// Keeps an app's SQL inside its own database: every statement it gives (queries, writes and schema changes, which run on
/// every device that reads the database) runs here, and SQLite lets SQL open other files (`attach`, `vacuum into`),
/// load code (`load_extension`), and turn off what keeps a database whole (pragmas). So no database can be attached,
/// which `vacuum into` needs too, and only pragmas that read the schema or set what an app's own data needs are allowed.
fn confine(conn: &mut Connection) -> Result<()> {
    use rusqlite::hooks::{AuthAction, Authorization};
    use rusqlite::limits::Limit;
    const PRAGMAS: &[&str] = &[
        "user_version", "application_id", "foreign_keys", "defer_foreign_keys", "recursive_triggers", "table_info",
        "table_xinfo", "table_list", "index_list", "index_info", "index_xinfo", "foreign_key_list", "foreign_key_check",
        "integrity_check", "quick_check", "optimize", "data_version", "page_count", "freelist_count",
    ];
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    conn.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| match ctx.action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
        AuthAction::Pragma { pragma_name, .. } if !PRAGMAS.contains(&pragma_name.to_ascii_lowercase().as_str()) => Authorization::Deny,
        AuthAction::Function { function_name } if ["load_extension", "fts3_tokenizer"].contains(&function_name.to_ascii_lowercase().as_str()) => {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }))?;
    Ok(())
}

/// Reads with a statement that only reads.
pub fn query(conn: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
    let mut stmt = conn.prepare(sql)?;
    ensure!(stmt.readonly(), "a query only reads");
    run(&mut stmt, params)
}

/// Runs a call's statements on a scope's database, in the order it ran them: each (sql, params, check), where a
/// check's first value must be true, else the write fails with its message.
pub fn statements(conn: &Connection, list: &[(String, Vec<Value>, Option<String>)]) -> Result<()> {
    for (sql, params, check) in list {
        let rows = execute(conn, sql, params)?;
        if let Some(message) = check {
            let held = rows.first().and_then(|r| r.values().next()).is_some_and(|v| v.as_i64().is_some_and(|n| n != 0) || v == true);
            ensure!(held, "{message}");
        }
    }
    Ok(())
}

/// Runs a statement that may write: its rows, if it gives any (`returning`).
pub fn execute(conn: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
    run(&mut conn.prepare(sql)?, params)
}

fn run(stmt: &mut rusqlite::Statement, params: &[Value]) -> Result<Vec<Map<String, Value>>> {
    ensure!(stmt.parameter_count() == params.len(), "the statement takes {} values, given {}", stmt.parameter_count(), params.len());
    for (i, p) in params.iter().enumerate() {
        stmt.raw_bind_parameter(i + 1, to_sql(p))?;
    }
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
    let mut rows = stmt.raw_query();
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut m = Map::new();
        for (i, name) in names.iter().enumerate() {
            m.insert(name.clone(), from_sql(row.get_ref(i)?));
        }
        out.push(m);
    }
    Ok(out)
}

fn to_sql(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(*b as i64),
        Value::Number(n) => n.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

fn from_sql(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => i.into(),
        ValueRef::Real(f) => f.into(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into(),
        ValueRef::Blob(b) => b.iter().map(|b| Value::from(*b)).collect(),
    }
}

// SQLite's session extension, which the bundled SQLite is built with (`.cargo/config.toml`).

#[repr(C)]
struct Session {
    _private: [u8; 0],
}

#[repr(C)]
struct Iter {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn sqlite3session_create(db: *mut rusqlite::ffi::sqlite3, name: *const c_char, out: *mut *mut Session) -> c_int;
    fn sqlite3session_object_config(s: *mut Session, op: c_int, arg: *mut c_void) -> c_int;
    fn sqlite3session_attach(s: *mut Session, table: *const c_char) -> c_int;
    fn sqlite3session_changeset(s: *mut Session, n: *mut c_int, out: *mut *mut c_void) -> c_int;
    fn sqlite3session_delete(s: *mut Session);
    fn sqlite3changeset_apply(
        db: *mut rusqlite::ffi::sqlite3,
        n: c_int,
        changes: *mut c_void,
        filter: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int>,
        conflict: Option<unsafe extern "C" fn(*mut c_void, c_int, *mut Iter) -> c_int>,
        ctx: *mut c_void,
    ) -> c_int;
}

const SESSION_OBJCONFIG_ROWID: c_int = 2;
const CHANGESET_DATA: c_int = 1;
const CHANGESET_CONFLICT: c_int = 3;
const CHANGESET_OMIT: c_int = 0;
const CHANGESET_REPLACE: c_int = 1;

/// Runs `f` with every change it makes recorded: → its result, and its changeset (empty if it changed nothing).
fn changes<R>(conn: &Connection, f: &mut impl FnMut(&Connection) -> Result<R>) -> Result<(R, Vec<u8>)> {
    let db = unsafe { conn.handle() };
    let mut s: *mut Session = std::ptr::null_mut();
    check(conn, unsafe { sqlite3session_create(db, c"main".as_ptr(), &mut s) })?;
    let made = (|| {
        // Tables without a primary key are recorded by rowid.
        let mut on: c_int = 1;
        check(conn, unsafe { sqlite3session_object_config(s, SESSION_OBJCONFIG_ROWID, (&raw mut on).cast()) })?;
        check(conn, unsafe { sqlite3session_attach(s, std::ptr::null()) })?;
        let result = f(conn)?;
        let (mut n, mut p): (c_int, *mut c_void) = (0, std::ptr::null_mut());
        check(conn, unsafe { sqlite3session_changeset(s, &mut n, &mut p) })?;
        let bytes = if p.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(p as *const u8, n as usize) }.to_vec() };
        unsafe { rusqlite::ffi::sqlite3_free(p) };
        Ok((result, bytes))
    })();
    unsafe { sqlite3session_delete(s) };
    made
}

/// A write's transaction, rolled back when it's dropped uncommitted: on an error, and when the write's future is
/// dropped while it waits on the bucket (a call that ran out of time, or whose caller went away), which would otherwise
/// leave it open, its changes seen by reads, and every later write failing to begin. It holds the connection mutably,
/// as only that is `Send` across the bucket's await.
struct Open<'a>(&'a mut Connection);

impl<'a> Open<'a> {
    fn begin(conn: &'a mut Connection) -> Result<Self> {
        conn.execute_batch("begin")?;
        Ok(Open(conn))
    }
}

impl std::ops::Deref for Open<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.0
    }
}

impl Drop for Open<'_> {
    fn drop(&mut self) {
        if !self.0.is_autocommit() {
            let _ = self.0.execute_batch("rollback");
        }
    }
}

/// Applies a changeset written from the state this database is in, so nothing should clash; if something does, the
/// change wins where it can, the same way on every device.
fn apply_changes(conn: &Connection, changes: &[u8]) -> Result<()> {
    unsafe extern "C" fn conflict(_: *mut c_void, kind: c_int, _: *mut Iter) -> c_int {
        match kind {
            CHANGESET_DATA | CHANGESET_CONFLICT => CHANGESET_REPLACE,
            _ => CHANGESET_OMIT,
        }
    }
    let mut bytes = changes.to_vec();
    let db = unsafe { conn.handle() };
    check(conn, unsafe { sqlite3changeset_apply(db, bytes.len() as c_int, bytes.as_mut_ptr().cast(), None, Some(conflict), std::ptr::null_mut()) })
}

fn check(conn: &Connection, rc: c_int) -> Result<()> {
    if rc == 0 {
        return Ok(());
    }
    let message = unsafe { std::ffi::CStr::from_ptr(rusqlite::ffi::sqlite3_errmsg(conn.handle())) }.to_string_lossy().to_string();
    bail!("SQLite said {rc}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::Memory;

    const SCHEMA: &str = "create table counts (name text primary key, n integer not null); create table notes (text text)";

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().build().unwrap()
    }

    fn keys(list: &[(u32, [u8; 32])]) -> Keys {
        list.iter().copied().collect()
    }

    async fn note(db: &mut Db, bucket: &impl Bucket, text: &str) {
        db.write(bucket, |c| execute(c, "insert into notes values (?)", &[json!(text)]).map(|_| ())).await.unwrap();
    }

    fn notes(db: &Db) -> Vec<String> {
        db.query("select text from notes order by rowid", &[]).unwrap().into_iter().map(|r| r["text"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn what_the_bucket_keeps_is_sealed_to_its_name() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap();
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            note(&mut a, &bucket, "a secret note").await;
            note(&mut a, &bucket, "another").await;
            for (name, bytes) in bucket.objects.lock().unwrap().iter() {
                assert!(name.ends_with("newest") || !bytes.windows(6).any(|w| w == b"secret" || w == b"create"), "{name} isn't sealed");
            }
            let other = Db::open(&bucket, "p", keys(&[(1, rand::random())])).await.unwrap();
            assert_eq!(other.version().unwrap(), 0, "another key opens nothing");
            // A step moved to another position doesn't open there.
            let mut objects = bucket.objects.lock().unwrap();
            let second = objects.get("p/000000000002").unwrap().clone();
            objects.insert("p/000000000004".into(), second);
            drop(objects);
            assert_eq!(notes(&Db::open(&bucket, "p", key).await.unwrap()), ["a secret note", "another"]);
        });
    }

    #[test]
    fn a_step_says_who_wrote_it_and_what_the_database_came_to() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let writer = ed25519_dalek::SigningKey::from_bytes(&rand::random());
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap();
            a.sign_as(writer.clone());
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..3 {
                a.write(&bucket, |c| execute(c, "insert into counts values (?, ?)", &[json!(format!("c{n}")), json!(n)]).map(|_| ())).await.unwrap();
                note(&mut a, &bucket, "a note").await;
            }
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert_eq!(read.checks(), Checks { signed: 7, unsigned: 0, wrong: 0 });
            let opened = |n: u64| {
                let objects = bucket.objects.lock().unwrap();
                Sig::check(&key[&1], &step_name("p", n), &objects[&step_name("p", n)], &objects[&sig_name("p", n)])
            };
            assert_eq!(opened(7).unwrap().by, writer.verifying_key().to_bytes());
            // Only a step a copy follows says what the database came to: no copy is due yet.
            assert_eq!(opened(7).unwrap().hash, None);
            // Moved beside another step, or signed by another key over these bytes, it doesn't hold.
            let mut objects = bucket.objects.lock().unwrap();
            let moved = objects[&sig_name("p", 2)].clone();
            objects.insert(sig_name("p", 3), moved);
            let forger = ed25519_dalek::SigningKey::from_bytes(&rand::random());
            let mut forged = serde_json::from_slice::<Value>(&crate::keys::open(&key[&1], &sig_name("p", 4), &objects[&sig_name("p", 4)]).unwrap()).unwrap();
            forged["by"] = json!(crate::entry::hex(forger.verifying_key().as_bytes()));
            objects.insert(sig_name("p", 4), crate::keys::seal(1, &key[&1], &sig_name("p", 4), &serde_json::to_vec(&forged).unwrap()));
            // A step with none is an older writer's.
            objects.remove(&sig_name("p", 5));
            drop(objects);
            assert_eq!(Db::open(&bucket, "p", key.clone()).await.unwrap().checks(), Checks { signed: 4, unsigned: 1, wrong: 2 });
            // The last step's says what the database comes to, which a reader checks: here, it doesn't.
            let mut objects = bucket.objects.lock().unwrap();
            let name = step_name("p", 7);
            let step = objects[&name].clone();
            objects.insert(sig_name("p", 7), Sig::seal(&writer, 1, &key[&1], &name, &step, Some(&"0".repeat(64))));
            drop(objects);
            assert_eq!(Db::open(&bucket, "p", key).await.unwrap().checks().wrong, 3);
        });
    }

    #[test]
    fn the_state_hash_is_the_rows_whichever_way_they_came() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..COPY_EVERY + 5 {
                let name = format!("c{}", n % 7);
                a.write(&bucket, |c| execute(c, "insert into counts values (?, 1) on conflict (name) do update set n = n + 1", &[json!(name)]).map(|_| ())).await.unwrap();
                if n % 3 == 0 {
                    note(&mut a, &bucket, &format!("note {n}")).await;
                }
                if n % 10 == 0 {
                    a.write(&bucket, |c| execute(c, "delete from counts where name = ?", &[json!(format!("c{}", n % 4))]).map(|_| ())).await.unwrap();
                }
            }
            let mine = state_hash(a.connection()).unwrap();
            // From the copy, from every step, and as it was written: the same.
            let copied = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert!(copied.copy > 0, "a copy was written");
            assert_eq!(state_hash(copied.connection()).unwrap(), mine);
            bucket.objects.lock().unwrap().retain(|n, _| !n.contains("/copy/"));
            let stepped = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert_eq!((stepped.copy, state_hash(stepped.connection()).unwrap()), (0, mine.clone()));
            a.write(&bucket, |c| execute(c, "update counts set n = n + 1", &[]).map(|_| ())).await.unwrap();
            assert_ne!(state_hash(a.connection()).unwrap(), mine);
            // A table without a primary key is its rows by rowid, which its steps name them by.
            let numbered = |rowid: i64| {
                let c = Connection::open_in_memory().unwrap();
                c.execute_batch(&format!("create table notes (text text); insert into notes (rowid, text) values ({rowid}, 'a')")).unwrap();
                state_hash(&c).unwrap()
            };
            assert_ne!(numbered(1), numbered(2));
        });
    }

    #[test]
    fn tidying_keeps_what_readers_need() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..2 * COPY_EVERY + 5 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            let all = notes(&a);
            let count = |what: &str| bucket.objects.lock().unwrap().keys().filter(|n| n.contains(what)).count();
            assert_eq!(count("/copy/"), 2);
            let now = 10 * KEEP_STEPS_FOR;
            // Until phase 2, only the copies before the newest go.
            let steps = count(".sig");
            assert_eq!(tidy(&bucket, "p", &key, &BTreeSet::new(), now, false).await.unwrap(), 1);
            assert_eq!((count("/copy/"), count(".sig")), (1, steps));
            assert_eq!(notes(&Db::open(&bucket, "p", key.clone()).await.unwrap()), all);
            // With the steps it covers, but the one at its position, which says what it comes to.
            let (_, copy, _) = newest(&bucket.objects.lock().unwrap()["p/newest"]);
            assert_eq!(tidy(&bucket, "p", &key, &BTreeSet::new(), now, true).await.unwrap(), 2 * (copy as usize - 1));
            assert!(bucket.objects.lock().unwrap().contains_key(&step_name("p", copy)) && !bucket.objects.lock().unwrap().contains_key(&step_name("p", 1)));
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert_eq!(notes(&read), all);
            assert_eq!(read.checks().wrong, 0);
            // Not with a copy that isn't what its step's signer says it comes to.
            let mut b = Db::open(&bucket, "p", key.clone()).await.unwrap();
            b.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            for n in 0..COPY_EVERY {
                note(&mut b, &bucket, &format!("more {n}")).await;
            }
            let (_, newer, _) = newest(&bucket.objects.lock().unwrap()["p/newest"]);
            let wrong = crate::keys::seal(1, &key[&1], &copy_name("p", newer), &bucket.objects.lock().unwrap().get(&copy_name("p", copy)).map(|c| crate::keys::open(&key[&1], &copy_name("p", copy), c).unwrap()).unwrap());
            bucket.objects.lock().unwrap().insert(copy_name("p", newer), wrong);
            assert_eq!(tidy(&bucket, "p", &key, &BTreeSet::new(), now, true).await.unwrap(), 0);
            // A reader starts from it all the same, in phase 1, and counts it: a signature beside a step is anyone's to
            // write who has the key.
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert_eq!((read.at(), read.checks().wrong), (b.at(), 1));
            // A reader with no copy to start from, and the steps before it gone, says so rather than reading what's left.
            bucket.objects.lock().unwrap().remove(&copy_name("p", newer));
            let err = Db::open(&bucket, "p", key).await.err().expect("it can't read");
            assert_eq!(err.downcast_ref::<Gone>(), Some(&Gone { copy: newer }));
        });
    }

    #[test]
    fn a_reader_reads_steps_signed_in_them_and_beside_them() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let (beside, inside) = (ed25519_dalek::SigningKey::from_bytes(&rand::random()), ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap();
            a.sign_as(beside.clone());
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            // A writer of phase 2's, signing in the step, beside one of today's.
            let mut b = Db::open(&bucket, "p", key.clone()).await.unwrap();
            b.sign_as(inside.clone());
            b.in_step = true;
            for n in 0..COPY_EVERY + 10 {
                let db = if n % 2 == 0 { &mut a } else { &mut b };
                note(db, &bucket, &format!("note {n}")).await;
            }
            a.catch_up(&bucket).await.unwrap();
            let all = notes(&b);
            assert_eq!(notes(&a), all);
            let steps = b.at();
            let sigs = bucket.objects.lock().unwrap().keys().filter(|n| n.ends_with(".sig")).count() as u64;
            assert_eq!(sigs, steps / 2 + 1, "a step signed in it has none beside it");
            // Each step's signer is who wrote it, and the copy's step says what it comes to, whichever kind wrote it.
            let by = |n: u64| {
                let objects = bucket.objects.lock().unwrap();
                let name = step_name("p", n);
                let step = crate::keys::open(&key[&1], &name, &objects[&name]).unwrap();
                match Sig::unwrap(&name, &step) {
                    Some(sig) => sig.unwrap(),
                    None => Sig::check(&key[&1], &name, &objects[&name], &objects[&sig_name("p", n)]).unwrap(),
                }
            };
            assert_eq!((by(2).by, by(3).by), (beside.verifying_key().to_bytes(), inside.verifying_key().to_bytes()));
            let (_, copy, _) = newest(&bucket.objects.lock().unwrap()["p/newest"]);
            assert!(copy > 0 && by(copy).hash.is_some());
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert_eq!((notes(&read), read.checks().wrong), (all.clone(), 0));
            bucket.objects.lock().unwrap().retain(|n, _| !n.contains("/copy/"));
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert_eq!((notes(&read), read.checks()), (all.clone(), Checks { signed: steps, unsigned: 0, wrong: 0 }));
            // One whose signature doesn't hold is counted, and in phase 1 read all the same.
            let forger = ed25519_dalek::SigningKey::from_bytes(&rand::random());
            let name = step_name("p", 3);
            let step = crate::keys::open(&key[&1], &name, &bucket.objects.lock().unwrap()[&name]).unwrap();
            let (_, inner) = split_signed(&step).unwrap();
            let mut forged = Sig::wrap(&forger, &name, inner, None);
            let at = forged.len() - inner.len() - 1;
            forged[at] ^= 1;
            bucket.objects.lock().unwrap().insert(name.clone(), crate::keys::seal(1, &key[&1], &name, &forged));
            let read = Db::open(&bucket, "p", key).await.unwrap();
            assert_eq!((notes(&read), read.checks().wrong), (all, 1));
        });
    }

    #[test]
    fn deleted_steps_are_never_read_as_the_whole_database_nor_written_over() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..50 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            // A device that read this far, and was away while the rest was written, and the steps it hasn't read deleted.
            let mut away = Db::open(&bucket, "p", key.clone()).await.unwrap();
            for n in 50..204 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            assert_eq!(a.at(), 205);
            let mut all = notes(&a);
            assert!(tidy(&bucket, "p", &key, &BTreeSet::new(), 10 * KEEP_STEPS_FOR, true).await.unwrap() > 0);
            assert!(!bucket.objects.lock().unwrap().contains_key(&step_name("p", 1)));
            // Its write goes after every step, from the newest copy, and every reader reads it.
            note(&mut away, &bucket, "back").await;
            assert_eq!(away.at(), 206);
            all.push("back".into());
            assert_eq!(notes(&away), all);
            assert_eq!(notes(&Db::open(&bucket, "p", key.clone()).await.unwrap()), all);
            // A reader whose hint names a copy that isn't there, with the steps before the newest gone, can't read it,
            // and says so, rather than reading it as empty.
            bucket.objects.lock().unwrap().insert(newest_name("p"), serde_json::to_vec(&json!({ "at": 206, "copy": 101, "epoch": 1 })).unwrap());
            let err = Db::open(&bucket, "p", key.clone()).await.err().expect("it can't read");
            assert_eq!(err.downcast_ref::<Gone>(), Some(&Gone { copy: 101 }));
        });
    }

}
