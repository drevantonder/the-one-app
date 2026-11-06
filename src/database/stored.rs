//! A database as the bucket keeps it, an app's or the scope database. An app's data in a scope is one SQLite database, the truth, kept in the bucket as steps at numbered
//! positions, each sealed under one of its scope's epochs (`src/keys.rs`) and naming it: the changeset of one write
//! (SQLite's session extension), or one of the app's schema changes. Reading it is its newest copy and the steps after;
//! writing it is a step created only if its position is free, so when two devices write at once, one wins and the
//! other catches up and runs its write again.
//!
//! ```text
//! <prefix>/newest            {"at": n, "copy": m, "epoch": e}, replaced while unchanged: a hint, so readers needn't
//!                            list; e is the epoch copy m is sealed under, the oldest a reader starting there needs
//! <prefix>/<n>               step n, from 1
//! <prefix>/<n>.sig           an older writer's signature beside step n (`Sig`): read, never written
//! <prefix>/copy/<n>          the whole database after step n, under the epoch of the steps it holds
//! <prefix>/tidied            there once `tidy` has deleted steps, or may have: never deleted but by an admin's device
//! ```
//!
//! A step is signed in it (`SIGNED`): its writer's endpoint key signs its sealed bytes at its position, and, on a step
//! a copy follows, the state hash of the database after it (`state_hash`: hashing every row on every write would make
//! each write as slow as the database is big). The first step signed in it says its own position, and every step after
//! says the same (`Sig::from`), so the database records where its steps are signed in them from (`signed_from`), a
//! copy's step saying it for readers that start there. Before that position a step counts whatever its signature says:
//! older steps have none, or one beside them (`<n>.sig`, `Sig::check`), which is read and never written. From it,
//! `refused` and `copy_holds` are the one check every device makes, readers, `tidy`, backups and writers alike: a step
//! counts only if it's signed in it, its signature holds and says that position, and the database it reads comes to
//! the state hash it signs; a copy only if it comes to its step's hash. A step that doesn't is passed over by every
//! reader, its position taken. A key-holding writer is all an app database asks: a removed device gets no key to the
//! next epoch, and its steps under an older one are passed over.
//!
//! What no reader needs goes from the bucket (`tidy`): copies before the newest that holds, once that has been there a
//! day; and an app database's steps before it, once each has been there `KEEP_STEPS_FOR`, saying so first (`tidied`).
//! A scope database's steps, who's who and the record of who did what, are never deleted (`keeps_every_step`). Until
//! `tidied` is there a missing step is the end, and a reader never looks for it or lists copies. A reader that doesn't find the next step where steps were deleted (`gap`), as when it read the start long
//! ago, or opened with no copy to start from, goes on from the newest copy above it that holds, or with none says so
//! (`Gone`), rather than read what's left as the whole database, or write its next step where a deleted one was. The
//! bucket decides that, never the newest hint, which any device can write, even one without the key: a reader starts
//! from the hint's copy only if it holds, and from the newest that does otherwise, or, before the switch, from the
//! first step. Steps aren't merged in levels: a reader reads at most `COPY_EVERY` steps after a copy, so merging would
//! save little, for a second kind of object to write, check and delete.
//!
//! Along a database's positions epochs never go down: a step under an epoch lower than the step before it, or that
//! doesn't open with the key of the epoch it names, is skipped by every reader, its position taken, and the next write
//! takes the following one. A writer writes under the newest epoch it was given, at once; the next copy is under it.
//! A step under an epoch above every key a reader was given stops it (`Ahead`): its caller looks for that epoch's
//! entry, which is written before anything is sealed under it, and if there's none, the step is junk, passed over.
//! A copy is used only if it opens and its epoch is at least its step's.
//!
//! No clock, disk or network: the bucket is all it reads and writes, so it runs wherever SQLite does. A database's key
//! in each epoch comes from its app's key in that epoch (`keys::database_keys`).

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

/// How long an app database's steps are kept, by when the bucket wrote them, before `tidy` deletes those a copy covers:
/// how far back its history goes. A device that reads less than `database::READS` reads every step from the first
/// when its copy doesn't hold, and is told to update.
pub const KEEP_STEPS_FOR: u64 = 90 * DAY;

/// Whether every step of the database at `prefix` is kept forever: a scope database's (`database::steps`), who's who
/// and the record of who did what. `tidy` deletes none of its steps, whoever asks.
pub fn keeps_every_step(prefix: &str) -> bool {
    let mut parts = prefix.rsplit('/');
    parts.next() == Some(crate::database::steps::NAME) && parts.next() == Some(crate::database::scopes::APP)
}

/// How long the newest copy that holds has been in the bucket before `tidy` deletes the copies before it: a reader that
/// read an older hint a moment ago still finds the copy it named.
const KEEP_OLDER_COPIES_FOR: u64 = DAY;

const DAY: u64 = 24 * 60 * 60 * 1000;

/// What a reader found of the signatures beside the steps it applied: those that held, those with none (an older
/// writer's, or one it couldn't fetch), and those that didn't hold (signed wrong, or the database didn't come to the
/// state hash signed); and the steps phase 2 refused (`refused`), passed over.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Checks {
    pub signed: u64,
    pub unsigned: u64,
    pub wrong: u64,
    pub refused: u64,
}

/// What stopped a reader: steps it hadn't read were deleted (`tidy`), and the newest copy that would hold them, at
/// `copy`, doesn't hold (`copy_holds`) or doesn't open with the keys it was given; no newer one does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gone {
    pub copy: u64,
}

impl std::fmt::Display for Gone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "steps it hadn't read were deleted, and its copy at step {}, which holds them, isn't one it can use with the keys it was given", self.copy)
    }
}

impl std::error::Error for Gone {}

/// What a write or a schema change stopped at: the database changed under every try, so nothing was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contended(pub String);

impl std::fmt::Display for Contended {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Contended {}

/// What an object that wasn't answered for was left as: it may have been written, so look before trying again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MayHaveBeen;

impl std::fmt::Display for MayHaveBeen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the bucket didn't answer whether it was written, so it may have been: look before trying again")
    }
}

impl std::error::Error for MayHaveBeen {}

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
    let Some(sealed) = bucket.get(&copy_name(prefix, copy)).await? else { return Ok(None) };
    let step = bucket.get(&step_name(prefix, copy)).await?;
    Ok(open_copy(prefix, keys, copy, &sealed.bytes, step.as_ref().map(|s| s.bytes.as_slice()))?.map(|(epoch, bytes)| (copy, epoch, bytes)))
}

/// The copy at `copy`, sealed as `sealed`, beside its step sealed as `step`, opened as `start_at` takes it: its epoch and
/// bytes.
fn open_copy(prefix: &str, keys: &Keys, copy: u64, sealed: &[u8], step: Option<&[u8]>) -> Result<Option<(u32, Vec<u8>)>> {
    let name = copy_name(prefix, copy);
    // Epoch 0 is never given: what's under it is anyone's junk, even a device's without the key.
    let Some(epoch) = crate::keys::epoch_of(sealed).ok().filter(|e| *e > 0) else { return Ok(None) };
    let step = step.and_then(|s| crate::keys::epoch_of(s).ok());
    let given = keys.keys().next_back().copied().unwrap_or(0);
    if let Some(step) = step.filter(|s| *s > given && *s <= epoch) {
        bail!(Ahead { epoch: step, at: copy });
    }
    if epoch > given || step.is_none_or(|s| s > epoch) {
        tracing::debug!("{name} isn't a copy to start from");
        return Ok(None);
    }
    let key = keys.get(&epoch).ok_or(Missing { epoch })?;
    match crate::keys::open(key, &name, sealed) {
        Ok(bytes) => Ok(Some((epoch, bytes))),
        Err(err) => {
            tracing::debug!("{name}: {err:#}: not a copy to start from");
            Ok(None)
        }
    }
}

/// Whether the copy at `at` holds for a reader with `keys`, as `holding` checks it, from its objects as sealed in the
/// bucket: the copy, its step, and the signature beside that, if any. For a caller that reads the bucket itself
/// (backups).
pub fn sealed_copy_holds(prefix: &str, keys: &Keys, at: u64, copy: &[u8], step: Option<&[u8]>, beside: Option<&[u8]>) -> bool {
    let Ok(Some((_, bytes))) = open_copy(prefix, keys, at, copy, step) else { return false };
    let name = step_name(prefix, at);
    let Some((step, key)) = step.and_then(|s| Some((s, keys.get(&crate::keys::epoch_of(s).ok()?)?))) else { return false };
    let Ok(opened) = crate::keys::open(key, &name, step) else { return false };
    let signed = Sig::unwrap(&name, &opened).or_else(|| beside.map(|b| Sig::check(key, &name, step, b)));
    copy_holds(at, &bytes, &opened, signed.as_ref()).holds
}

/// The keys a node uses, of those a caller handed it, for a database of a scope whose current epoch is `current`, as
/// the network's signed `epoch` entries say: none above it (one a member made up would leave the database where nobody
/// else can follow), and `current`'s among them, the one it writes under. A key with the right number but the wrong
/// bytes only fails to open what's under its epoch, so a database opened with handed keys is shared only with calls
/// handed the same keys.
pub fn handed(mut keys: Keys, current: u32) -> std::result::Result<Keys, Missing> {
    keys.retain(|n, _| *n <= current);
    if keys.contains_key(&current) { Ok(keys) } else { Err(Missing { epoch: current }) }
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
    /// Copies it found are junk (`holding`).
    junk_copies: BTreeSet<u64>,
    /// Whose endpoint key signs the steps it writes: none, and it writes them unsigned, as older writers do.
    signer: Option<ed25519_dalek::SigningKey>,
    /// Whether it signs a step in it (`SIGNED`), rather than beside it (`<n>.sig`), as writers before phase 2 did: a test
    /// of what they wrote is the only one that writes beside.
    #[cfg(test)]
    in_step: bool,
    /// Where the database's steps are signed in them from, as what it read says (`Sig::from`).
    signed_from: Option<u64>,
    /// What it found of the signatures beside the steps it applied.
    checks: Checks,
    /// When (`Bucket::now`) the bucket last had no step after `at`, as far as this database knows: it read to its end,
    /// or wrote it. 0 if never, or on a runtime with no clock.
    current: u64,
    /// Whether a write leaves moving the newest hint on to its caller (`take_hint`), who may do it after going on.
    hint_later: bool,
    /// The newest hint a write left for its caller: (last step, newest copy, its epoch).
    hint_due: Option<(u64, u64, u32)>,
}

/// How long after the bucket had no step after a database's last one it can't have deleted the next: steps are deleted
/// only once they've been there `KEEP_STEPS_FOR`, so a step missing now that wasn't a moment ago was never written.
const CURRENT_FOR: u64 = DAY;
/// How long after a database was read to the bucket's end a write needn't look for newer steps again first: one it
/// missed is found when the write that was to follow it finds its position taken, and runs again.
const WRITES_TRUST_FOR: u64 = 5_000;

/// A copy a reader may start from (`copy_holds`): its position and epoch, its bytes, and what its step's signature says.
struct Held {
    at: u64,
    epoch: u32,
    bytes: Vec<u8>,
    signed: Option<Result<Sig>>,
    check: CopyCheck,
}

impl Db {
    /// Reads a database as its newest steps leave it, with its keys by epoch: empty if it has none. It signs the steps
    /// it writes in them, refuses from where they're signed in them, and finds steps `tidy` deleted. Fails with `Missing`
    /// at a copy or step under an epoch whose key it wasn't given, and with `Ahead` at a step under an epoch above them
    /// all.
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
            junk_copies: BTreeSet::new(),
            signer: None,
            #[cfg(test)]
            in_step: true,
            signed_from: None,
            checks: Checks::default(),
            current: 0,
            hint_later: false,
            hint_due: None,
        };
        // From the hint's copy, if it holds; or, once steps may be deleted and it names one, the newest copy that does;
        // or from the first step, where `gap` finds any deleted.
        let hinted = hinted_copy(bucket, prefix).await?;
        match holding(bucket, prefix, &db.keys, &db.junk, &mut db.junk_copies, 0, hinted, hinted > 0).await?.0 {
            Some(held) => db.start_from(held)?,
            None => confine(&mut db.conn)?,
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

    /// Where the database's steps are signed in them from, as far as it has read.
    pub fn signed_from(&self) -> Option<u64> {
        self.signed_from
    }

    /// What it found of the signatures beside the steps it applied.
    pub fn checks(&self) -> Checks {
        self.checks
    }

    /// Starts from a copy that holds. Its step's signature is counted, and whether the copy comes to the state hash it
    /// signs; before steps are signed in them, a copy counts whatever that says, as it does on older readers, since a
    /// signature anyone with the key could write beside an older writer's step mustn't send every reader back to the
    /// first step.
    fn start_from(&mut self, held: Held) -> Result<()> {
        let Held { at, epoch, bytes, signed, check } = held;
        self.count(at, signed);
        if check.hash == Some(false) {
            self.wrong_hash(at);
        }
        let mut conn = Connection::open_in_memory()?;
        conn.deserialize_read_exact("main", &bytes[..], bytes.len(), false)?;
        // After the copy, which SQLite reads in by attaching it.
        confine(&mut conn)?;
        self.conn = conn;
        (self.at, self.copy, self.epoch, self.copy_epoch, self.signed_from) = (at, at, epoch, epoch, check.from);
        self.used = BTreeSet::from([epoch]);
        Ok(())
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
            let at = self.at + 1;
            let name = self.step(at);
            let Some(o) = bucket.get(&name).await? else {
                if self.gap(bucket).await? {
                    continue;
                }
                self.current = bucket.now();
                break;
            };
            if self.junk.contains(&at) {
                self.at += 1;
                continue;
            }
            if let Some((epoch, step)) = self.unseal(at, &name, &o.bytes)? {
                // One it can't fetch stops it, to be read again, rather than count as none.
                let signed = signature(bucket, &self.prefix, &self.keys[&epoch], &name, &o.bytes, &step).await?;
                if self.take(at, &step, signed)? {
                    self.epoch = epoch;
                    self.used.insert(epoch);
                }
            }
            self.at += 1;
        }
        Ok(self.at > from)
    }

    /// Applies step `at`, opened as `step`, with `signed` the signature in or beside it, unless phase 2 refuses it
    /// (`refused`, and, from where steps are signed in them, a state hash it signs that what it reads doesn't come to):
    /// whether it counts.
    fn take(&mut self, at: u64, step: &[u8], signed: Option<Result<Sig>>) -> Result<bool> {
        let from = signed_from(self.signed_from, at, step, signed.as_ref());
        let checked = from.is_some_and(|f| at >= f);
        if refused(from, at, step, signed.as_ref()) {
            self.checks.refused += 1;
            tracing::warn!("{}: step {at} isn't signed as every step from {} must be: passed over", self.prefix, from.unwrap_or(0));
            return Ok(false);
        }
        let hash = self.count(at, signed);
        let Some(hash) = hash else {
            self.conn.execute_batch("savepoint take")?;
            let took = self.apply(step);
            self.conn.execute_batch(if took.is_ok() { "release take" } else { "rollback to take; release take" })?;
            took?;
            self.signed_from = from;
            return Ok(true);
        };
        self.conn.execute_batch("savepoint take")?;
        let came = self.apply(step).and_then(|()| state_hash(&self.conn));
        let held = came.as_ref().is_ok_and(|h| *h == hash);
        if checked && !held {
            self.conn.execute_batch("rollback to take; release take")?;
            came?;
            self.wrong_hash(at);
            self.checks.refused += 1;
            tracing::warn!("{}: step {at} doesn't come to the state its signer signed: passed over", self.prefix);
            return Ok(false);
        }
        self.conn.execute_batch("release take")?;
        came?;
        if !held {
            self.wrong_hash(at);
        }
        self.signed_from = from;
        Ok(true)
    }

    /// When the step after the last it read isn't there: whether steps it hasn't read were deleted, as when it read
    /// the start long ago, or opened with no copy to start from. Only `tidy` deletes them, and it
    /// says so first (`tidied`), so until then a missing step is the end, whatever is in the bucket; after, the reader
    /// goes on from the newest copy above it that holds (true), or, with one there it can't use and none newer that
    /// holds, fails with `Gone`, rather than take what it has for the whole database, and write its next step where one
    /// was. Copies that don't open with its keys are anyone's junk, passed over. A bucket it can't list (a client's,
    /// which only reads) has only the newest hint's.
    async fn gap(&mut self, bucket: &impl Bucket) -> Result<bool> {
        let next = self.at + 1;
        // Read to the end of what the bucket had a moment ago: a step that's missing now was never there to be deleted.
        if self.current > 0 && bucket.now().saturating_sub(self.current) < CURRENT_FOR {
            return Ok(false);
        }
        if bucket.get(&tidied_name(&self.prefix)).await?.is_none() {
            return Ok(false);
        }
        // Written since it looked: it reads on.
        if bucket.get(&self.step(next)).await?.is_some() {
            return Ok(true);
        }
        let hinted = hinted_copy(bucket, &self.prefix).await?;
        match holding(bucket, &self.prefix, &self.keys, &self.junk, &mut self.junk_copies, self.at, hinted, true).await? {
            (Some(held), _) => {
                tracing::info!("{}: the steps after {} were deleted: read on from the copy at {}", self.prefix, self.at, held.at);
                self.start_from(held)?;
                Ok(true)
            }
            (None, Some(copy)) => bail!(Gone { copy }),
            (None, None) => Ok(false),
        }
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
        // Epoch 0 is never given: anyone's junk.
        let Some(epoch) = crate::keys::epoch_of(sealed).ok().filter(|e| *e > 0) else { return Ok(None) };
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
            let from = self.signs_from(self.at + 1)?;
            self.conn.execute_batch("savepoint migrate")?;
            let tried = self.apply(&step).and_then(|()| self.hash_due(self.at + 1));
            self.conn.execute_batch("rollback to migrate; release migrate")?;
            let hash = tried?;
            let (epoch, key) = self.newest_key()?;
            let name = self.step(self.at + 1);
            let written = signed(self.signer.as_ref().filter(|_| from.is_some()), &name, &step, hash.as_deref(), from);
            let sealed = crate::keys::seal(epoch, &key, &name, &written);
            if bucket.create(&name, &sealed).await? {
                self.apply(&step)?;
                self.wrote(epoch, from);
                #[cfg(test)]
                self.sign(bucket, &name, epoch, &key, &sealed, hash.as_deref(), from.is_none()).await;
                self.written(bucket, &written).await;
            }
        }
        return Err(Contended(format!("{} changed too often to change its schema", self.prefix)).into())
    }

    /// The schema changes this database has had.
    pub fn version(&self) -> Result<usize> {
        Ok(self.conn.query_row("pragma user_version", [], |r| r.get::<_, i64>(0))? as usize)
    }

    /// Runs a write in one transaction, which is the step written after this device's newest, under the newest epoch
    /// it was given: when another device wrote that first, it catches up and runs `f` again, so `f` must do only
    /// database work.
    pub async fn write<R>(&mut self, bucket: &impl Bucket, mut f: impl FnMut(&Connection) -> Result<R>) -> Result<R> {
        for tries in 0..TRIES {
            // Not asking the bucket first, when what it was asked a moment ago is all there was: a step written since
            // takes this one's position, which it finds out below, and then reads on.
            let looked = self.current > 0 && bucket.now().saturating_sub(self.current) < WRITES_TRUST_FOR;
            if tries > 0 || !looked {
                self.catch_up(bucket).await?;
            }
            let (epoch, key) = self.newest_key()?;
            let due = self.signer.is_some() && self.copy_due(self.at + 1);
            let from = self.signs_from(self.at + 1)?;
            let open = Open::begin(&mut self.conn)?;
            open.execute_batch("savepoint write")?;
            let (result, changes) = changes(&open, &mut f)?;
            if changes.is_empty() {
                open.execute_batch("commit")?;
                return Ok(result);
            }
            let inner = [b"c".as_slice(), &changes].concat();
            let name = step_name(&self.prefix, self.at + 1);
            let hash = if due { Some(state_hash(&open)?) } else { None };
            let step = signed(self.signer.as_ref().filter(|_| from.is_some()), &name, &inner, hash.as_deref(), from);
            let sealed = crate::keys::seal(epoch, &key, &name, &step);
            if created(bucket, &name, &sealed).await? {
                open.execute_batch("commit")?;
                drop(open);
                self.wrote(epoch, from);
                self.current = bucket.now();
                #[cfg(test)]
                self.sign(bucket, &name, epoch, &key, &sealed, hash.as_deref(), from.is_none()).await;
                self.written(bucket, &step).await;
                return Ok(result);
            }
        }
        return Err(Contended(format!("{} changed too often to write to", self.prefix)).into())
    }

    /// After its own step under `epoch` was created at the next position, signed in it from `from` if it says.
    fn wrote(&mut self, epoch: u32, from: Option<u64>) {
        self.at += 1;
        self.epoch = epoch;
        self.used.insert(epoch);
        self.signed_from = self.signed_from.or(from);
    }

    fn in_step(&self) -> bool {
        #[cfg(test)]
        return self.in_step;
        #[cfg(not(test))]
        true
    }

    /// The position its step at `at` says steps are signed in them from, if it signs it in it: the database's, once it
    /// has one, which it can't write without a key to sign with; or, signing in steps itself first, its own.
    fn signs_from(&self, at: u64) -> Result<Option<u64>> {
        Ok(match (self.signed_from, &self.signer) {
            (Some(from), Some(_)) => Some(from),
            (Some(from), None) => bail!("{}'s steps are signed in them from step {from}, and this device has no key to sign with", self.prefix),
            (None, Some(_)) if self.in_step() => Some(at),
            (None, _) => None,
        })
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
    /// it has a signer and signs `beside` the step. Tried once, since the step is written whatever happens to it: a
    /// reader takes a step with none as an older writer's.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    async fn sign(&mut self, bucket: &impl Bucket, name: &str, epoch: u32, key: &[u8; 32], sealed: &[u8], hash: Option<&str>, beside: bool) {
        let Some(signer) = self.signer.as_ref().filter(|_| beside) else { return };
        let sig = format!("{name}.sig");
        match bucket.create(&sig, &Sig::seal(signer, epoch, key, name, sealed, hash)).await {
            Ok(true) => {}
            Ok(false) => tracing::warn!("{sig} was there before its step's writer signed it"),
            Err(err) => tracing::debug!("{sig}: {err:#}"),
        }
    }

    /// After writing a step, opened as `step`: a copy, if steps have piled up since the last, and the newest hint moved
    /// on. Both only save readers work, so a link that fails them fails nothing.
    async fn written(&mut self, bucket: &impl Bucket, step: &[u8]) {
        if let Err(err) = self.hint(bucket, step).await {
            tracing::debug!("{}: {err:#}", self.prefix);
        }
    }

    /// A copy after its own step, opened as `step`, if one is due and holds as every reader checks it (`copy_holds`), and
    /// the newest hint moved on, naming the copy only if it's the one written there: someone else's at that position,
    /// junk perhaps, is never named, and the next write tries again.
    async fn hint(&mut self, bucket: &impl Bucket, step: &[u8]) -> Result<()> {
        if self.at >= self.copy + COPY_EVERY
            && let Some(key) = self.keys.get(&self.epoch)
        {
            let name = copy_name(&self.prefix, self.at);
            let bytes = self.conn.serialize("main")?.to_vec();
            let signed = Sig::unwrap(&step_name(&self.prefix, self.at), step);
            if copy_holds(self.at, &bytes, step, signed.as_ref()).holds {
                let sealed = crate::keys::seal(self.epoch, key, &name, &bytes);
                let ours = match bucket.create(&name, &sealed).await {
                    Ok(true) => true,
                    // Its own, if the answer to it was lost.
                    _ => bucket.get(&name).await.ok().flatten().is_some_and(|o| o.bytes == sealed),
                };
                if ours {
                    (self.copy, self.copy_epoch) = (self.at, self.epoch);
                }
            }
        }
        if self.hint_later {
            self.hint_due = Some((self.at, self.copy, self.copy_epoch));
            return Ok(());
        }
        hint(bucket, &self.prefix, self.at, self.copy, self.copy_epoch).await
    }

    /// Whether the writes that follow leave moving the newest hint on to their caller (`take_hint`), rather than do it
    /// before they return.
    pub fn leave_hint(&mut self, later: bool) {
        self.hint_later = later;
    }

    /// The newest hint the writes since the last call left: its database's prefix, last step, newest copy and that
    /// copy's epoch, for `hint`.
    pub fn take_hint(&mut self) -> Option<(String, u64, u64, u32)> {
        self.hint_due.take().map(|(at, copy, epoch)| (self.prefix.clone(), at, copy, epoch))
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
/// that holds (`holding`: opens with `keys`, passing over `junk`, and `copy_holds`), once that one has been there
/// `KEEP_OLDER_COPIES_FOR`, and but for a database that keeps every step (`keeps_every_step`), the steps before it and
/// their signatures, each once it's been there `KEEP_STEPS_FOR`, if it comes to the state hash signed at its step, and
/// once it has said so (`tidied`). → how many objects went.
pub async fn tidy(bucket: &impl Bucket, prefix: &str, keys: &Keys, junk: &BTreeSet<u64>, now: u64) -> Result<usize> {
    let hinted = hinted_copy(bucket, prefix).await?;
    let Some(held) = holding(bucket, prefix, keys, junk, &mut BTreeSet::new(), 0, hinted, true).await?.0 else { return Ok(0) };
    // One whose signature is wrong: nothing older goes for it.
    if held.check.hash == Some(false) || matches!(held.signed, Some(Err(_))) {
        return Ok(0);
    }
    let copy = held.at;
    let copies = bucket.objects(&copies_folder(prefix)).await?;
    let newest = copies.iter().find(|(n, _)| *n == position_text(copy)).map(|(_, written)| *written);
    if newest.is_none_or(|w| w + KEEP_OLDER_COPIES_FOR > now) {
        return Ok(0);
    }
    let mut gone = 0;
    for (n, _) in copies.iter().filter(|(n, _)| n.parse::<u64>().is_ok_and(|n| n < copy)) {
        bucket.delete(&format!("{}/{n}", copies_folder(prefix))).await?;
        gone += 1;
    }
    if keeps_every_step(prefix) || held.check.hash != Some(true) {
        return Ok(gone);
    }
    let old: Vec<String> = bucket.objects(prefix).await?.into_iter().map(|(n, written)| (format!("{prefix}/{n}"), written)).filter(|(full, written)| position(full).is_some_and(|(_, at)| at < copy) && written + KEEP_STEPS_FOR <= now).map(|(full, _)| full).collect();
    if old.is_empty() {
        return Ok(gone);
    }
    // Said before any goes, so a reader that finds a step missing looks for the copy that holds it.
    bucket.create(&tidied_name(prefix), b"{}").await?;
    for full in old {
        bucket.delete(&full).await?;
        gone += 1;
    }
    Ok(gone)
}

/// The copy the newest hint at `prefix` names: 0 for none.
async fn hinted_copy(bucket: &impl Bucket, prefix: &str) -> Result<u64> {
    Ok(bucket.get(&newest_name(prefix)).await?.map_or(0, |o| newest(&o.bytes).1))
}

/// The newest copy above `above` that holds, `switched` as phase 2's switch is: the newest hint's, `hinted`, first, then,
/// if it doesn't and `list`, the others the bucket has, newest first. → it, or none, and the newest of those above it a
/// reader can't use that aren't junk (they open with a key it was given and don't hold). Junk is anyone's to write, as
/// the hint is, so it's passed over, and kept in `junk_copies`, since a copy, written once, never comes to hold; and so
/// is a copy that doesn't open with the keys it was given, which never stops a reader. A bucket that isn't listed from
/// here (a client's, which only reads) has only the hint's.
#[allow(clippy::too_many_arguments)]
async fn holding(bucket: &impl Bucket, prefix: &str, keys: &Keys, junk: &BTreeSet<u64>, junk_copies: &mut BTreeSet<u64>, above: u64, hinted: u64, list: bool) -> Result<(Option<Held>, Option<u64>)> {
    let mut unusable = None;
    for listed in [false, true] {
        let copies = match (listed, list) {
            (false, _) => vec![hinted],
            (true, false) => break,
            (true, true) => match bucket.objects(&copies_folder(prefix)).await {
                Ok(listed) => {
                    let mut listed: Vec<u64> = listed.iter().filter_map(|(n, _)| n.parse().ok()).filter(|n| *n != hinted).collect();
                    listed.sort_unstable_by(|a, b| b.cmp(a));
                    listed
                }
                Err(err) if unanswered(&err) => return Err(err),
                Err(err) => {
                    tracing::debug!("{prefix}'s copies: {err:#}");
                    break;
                }
            },
        };
        for copy in copies {
            if copy <= above || junk_copies.contains(&copy) {
                continue;
            }
            match check_copy(bucket, prefix, keys, junk, copy).await {
                Ok(Checked::Held(held)) if held.check.holds => return Ok((Some(held), unusable)),
                Ok(Checked::Held(_)) => unusable = unusable.max(Some(copy)),
                Ok(Checked::Junk) => drop(junk_copies.insert(copy)),
                Ok(Checked::Absent) => {}
                Err(err) if err.is::<Missing>() => tracing::debug!("{prefix}'s copy at {copy}: {err:#}: passed over"),
                Err(err) => return Err(err),
            }
        }
    }
    Ok((None, unusable))
}

/// What's at a copy's position.
enum Checked {
    /// A copy that opens, with what phase 2 says of it.
    Held(Held),
    /// Something that never will.
    Junk,
    /// Nothing yet.
    Absent,
}

/// The copy at `copy`, if it's one to start from (`open_copy`), with what its step's signature says of it
/// (`copy_holds`). A signature it can't fetch stops it, rather than count as none.
async fn check_copy(bucket: &impl Bucket, prefix: &str, keys: &Keys, junk: &BTreeSet<u64>, copy: u64) -> Result<Checked> {
    if copy == 0 || junk.contains(&copy) {
        return Ok(Checked::Junk);
    }
    let Some(sealed) = bucket.get(&copy_name(prefix, copy)).await? else { return Ok(Checked::Absent) };
    let step = bucket.get(&step_name(prefix, copy)).await?;
    let Some((epoch, bytes)) = open_copy(prefix, keys, copy, &sealed.bytes, step.as_ref().map(|s| s.bytes.as_slice()))? else { return Ok(Checked::Junk) };
    let Some((step, signed)) = opened_at(bucket, prefix, keys, copy).await? else { return Ok(Checked::Junk) };
    let check = copy_holds(copy, &bytes, &step, signed.as_ref());
    Ok(Checked::Held(Held { at: copy, epoch, bytes, signed, check }))
}

/// What phase 2 says of a copy (`copy_holds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyCheck {
    /// Whether a reader may start from it.
    pub holds: bool,
    /// Where its database's steps are signed in them from, as its step says.
    pub from: Option<u64>,
    /// Whether it comes to the state hash signed at its step, if one is.
    pub hash: Option<bool>,
}

/// Phase 2's one check of a copy at `at`, as its bytes, beside its step opened as `step` with `signed` the signature in
/// or beside it, the same on every device and for every use: readers, `tidy`, backups, and the writer of the copy. A
/// copy holds only if it's a database; one whose step is signed in it only if that signature holds; and, once steps
/// are signed in them, only if it comes to the state hash signed. Before, it holds whatever that says.
pub fn copy_holds(at: u64, copy: &[u8], step: &[u8], signed: Option<&Result<Sig>>) -> CopyCheck {
    let inside = step.first() == Some(&SIGNED);
    let from = match signed {
        Some(Ok(sig)) if inside => sig.from.filter(|f| *f <= at),
        _ => None,
    };
    let database = copied(copy).ok();
    let hash = match signed {
        Some(Ok(Sig { hash: Some(hash), .. })) => Some(database.as_ref().is_some_and(|c| state_hash(c).is_ok_and(|h| h == *hash))),
        _ => None,
    };
    let holds = database.is_some() && !(inside && !matches!(signed, Some(Ok(_)))) && (from.is_none() || hash == Some(true));
    CopyCheck { holds, from, hash }
}

/// Where a database's steps are signed in them from, after the step at `at`, opened as `step` with `signed` the
/// signature in or beside it, given `before`, where the steps before said: the first signed in it that says a position
/// (`Sig::from`), its own on the first a writer signs in it, marks it, as a copy's step does (`copy_holds`).
pub fn signed_from(before: Option<u64>, at: u64, step: &[u8], signed: Option<&Result<Sig>>) -> Option<u64> {
    before.or(match signed {
        Some(Ok(sig)) if step.first() == Some(&SIGNED) => sig.from.filter(|f| *f <= at),
        _ => None,
    })
}

/// Phase 2's one check of a step, the same on every device: whether the step at `at`, opened as `step` with `signed`
/// the signature in or beside it, is refused, in a database whose steps are signed in them from `from` (`signed_from`).
/// Before that position nothing is: older steps, and those signed beside them whose signature never landed, have none
/// for good. From it, a step counts only if it's signed in it, and its signature holds and says that position. Its
/// state hash is checked as it's applied.
pub fn refused(from: Option<u64>, at: u64, step: &[u8], signed: Option<&Result<Sig>>) -> bool {
    let Some(from) = from.filter(|f| at >= *f) else { return false };
    !(step.first() == Some(&SIGNED) && matches!(signed, Some(Ok(sig)) if sig.from == Some(from)))
}

/// What says a database's steps may have been deleted (`tidy`): created before the first goes.
pub fn tidied_name(prefix: &str) -> String {
    format!("{prefix}/{}", crate::protocol::TIDIED)
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
    let (mut sent, mut wait, mut waited) = (false, 100, 0);
    loop {
        match bucket.create(name, bytes).await {
            Ok(true) => return Ok(true),
            Ok(false) if !sent => return Ok(false),
            // One sent before, whose answer was lost, may be what's there.
            Ok(false) => match bucket.get(name).await {
                Ok(o) => return Ok(o.with_context(|| format!("{name} went once written"))?.bytes == bytes),
                Err(err) if !unanswered(&err) => return Err(err.context(MayHaveBeen)),
                Err(_) => {}
            },
            Err(err) if !unanswered(&err) => return Err(if sent { err.context(MayHaveBeen) } else { err }),
            Err(_) => sent = true,
        }
        if waited >= ANSWERED_WITHIN_MS {
            return Err(MayHaveBeen.into());
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
    format!("{prefix}/{}", crate::protocol::NEWEST)
}

/// A step's position as its name says it.
fn position_text(n: u64) -> String {
    format!("{n:0width$}", width = crate::protocol::STEP_DIGITS)
}

pub fn step_name(prefix: &str, n: u64) -> String {
    format!("{prefix}/{}", position_text(n))
}

/// The folder a database's copies are in.
pub fn copies_folder(prefix: &str) -> String {
    format!("{prefix}/{}", crate::protocol::COPY)
}

pub fn copy_name(prefix: &str, n: u64) -> String {
    format!("{}/{}", copies_folder(prefix), position_text(n))
}

/// The signature beside step `n`.
pub fn sig_name(prefix: &str, n: u64) -> String {
    format!("{}.sig", step_name(prefix, n))
}

/// Whether the steps at `prefix` may have signatures beside them, as an app database's older steps do, rather than the
/// scope database's (`database::steps`), whose steps are signed in them.
pub fn signed_beside(prefix: &str) -> bool {
    !keeps_every_step(prefix)
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
/// before the step it signs, with `from` too, and `sig` is under `one/signed-step`, over `{"from"?, "hash"?, "name",
/// "step"}`, with `step` the BLAKE3 of the step it signs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sig {
    pub by: [u8; 32],
    pub hash: Option<String>,
    /// In a step signed in it, where its database's steps are signed in them from: its own position on the first.
    pub from: Option<u64>,
}

impl Sig {
    const BESIDE: &str = "one/step";
    const IN: &str = "one/signed-step";

    fn signed(name: &str, what: &[u8], hash: Option<&str>, from: Option<u64>) -> Value {
        let mut signed = json!({ "name": name, "step": blake3::hash(what).to_hex().as_str() });
        if let Some(hash) = hash {
            signed["hash"] = json!(hash);
        }
        if let Some(from) = from {
            signed["from"] = json!(from);
        }
        signed
    }

    /// `{"by", "from"?, "hash"?, "sig"}`: `signer`'s signature, under `domain`, of `what` at `name`, saying `hash` and
    /// `from`.
    fn make(signer: &ed25519_dalek::SigningKey, domain: &str, name: &str, what: &[u8], hash: Option<&str>, from: Option<u64>) -> Vec<u8> {
        let sig = crate::keys::sign(signer, domain, &Self::signed(name, what, hash, from));
        let mut made = json!({ "by": crate::keys::hex(signer.verifying_key().as_bytes()), "sig": data_encoding::BASE64URL_NOPAD.encode(&sig) });
        if let Some(hash) = hash {
            made["hash"] = json!(hash);
        }
        if let Some(from) = from {
            made["from"] = json!(from);
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
        let from = match &v["from"] {
            Value::Null => None,
            from => Some(from.as_u64().context("where steps are signed from is a position")?),
        };
        let signature = data_encoding::BASE64URL_NOPAD.decode(v["sig"].as_str().context("a step's signature has its signature")?.as_bytes())?;
        crate::keys::verify(&by, domain, &Self::signed(name, what, hash.as_deref(), from), &signature).with_context(|| format!("{name}'s signer"))?;
        Ok(Sig { by, hash, from })
    }

    /// Step `name`'s signature beside it, by `signer`, saying `hash`, sealed under `epoch`'s `key`.
    #[cfg(test)]
    pub fn seal(signer: &ed25519_dalek::SigningKey, epoch: u32, key: &[u8; 32], name: &str, sealed: &[u8], hash: Option<&str>) -> Vec<u8> {
        crate::keys::seal(epoch, key, &format!("{name}.sig"), &Self::make(signer, Self::BESIDE, name, sealed, hash, None))
    }

    /// Opens step `name`'s signature beside it with the key the step opened with, and checks it signs the step's
    /// `sealed` bytes there.
    pub fn check(key: &[u8; 32], name: &str, sealed: &[u8], sig: &[u8]) -> Result<Sig> {
        Self::read(&crate::keys::open(key, &format!("{name}.sig"), sig)?, Self::BESIDE, name, sealed)
    }

    /// `step`, to be written at `name`, with `signer`'s signature in it, saying `hash`, and that steps are signed in
    /// them from `from`.
    pub fn wrap(signer: &ed25519_dalek::SigningKey, name: &str, step: &[u8], hash: Option<&str>, from: Option<u64>) -> Vec<u8> {
        let sig = Self::make(signer, Self::IN, name, step, hash, from);
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

/// `step`, to be written at `name`: with `signer`'s signature in it, saying `hash` and `from`, if there's one.
fn signed(signer: Option<&ed25519_dalek::SigningKey>, name: &str, step: &[u8], hash: Option<&str>, from: Option<u64>) -> Vec<u8> {
    match signer {
        Some(signer) => Sig::wrap(signer, name, step, hash, from),
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

/// The step at `at`, opened, and the signature in or beside it: none if it isn't there, or doesn't open with `keys`.
async fn opened_at(bucket: &impl Bucket, prefix: &str, keys: &Keys, at: u64) -> Result<Option<(Vec<u8>, Option<Result<Sig>>)>> {
    let name = step_name(prefix, at);
    let Some(o) = bucket.get(&name).await? else { return Ok(None) };
    let Some(key) = crate::keys::epoch_of(&o.bytes).ok().and_then(|e| keys.get(&e)) else { return Ok(None) };
    let Ok(step) = crate::keys::open(key, &name, &o.bytes) else { return Ok(None) };
    let signed = signature(bucket, prefix, key, &name, &o.bytes, &step).await?;
    Ok(Some((step, signed)))
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

/// A copy's bytes as a database of its own, to read: if they're one.
fn copied(bytes: &[u8]) -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    conn.deserialize_read_exact("main", bytes, bytes.len(), true)?;
    conn.query_row("select count(*) from sqlite_schema", [], |_| Ok(()))?;
    Ok(conn)
}

/// Moves the newest hint at `prefix` on to say at least step `at` and copy `copy` (sealed under `epoch`), replacing it
/// only while unchanged.
pub async fn hint(bucket: &impl Bucket, prefix: &str, at: u64, copy: u64, epoch: u32) -> Result<()> {
    let name = newest_name(prefix);
    for _ in 0..TRIES {
        let current = bucket.get(&name).await?;
        let mut said = current.as_ref().map(|o| newest(&o.bytes));
        // One that says a step is written that isn't is someone's junk, not raised on; its copy never goes down, since
        // a reader goes on from the newest that holds whichever it names, and one lowered may have been deleted.
        if let Some((was_at, was_copy, was_epoch)) = said
            && was_at > at
            && bucket.get(&step_name(prefix, was_at)).await?.is_none()
        {
            said = Some((0, was_copy, was_epoch));
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

    impl Db {
        /// As a writer before phase 2 wrote: its signature beside its steps.
        fn older(mut self) -> Db {
            self.in_step = false;
            self
        }
    }

    /// A device that finds steps `tidy` deleted, writing beside its steps as writers before phase 2 did.
    async fn switched(bucket: &impl Bucket, key: &Keys) -> Result<Db> {
        let mut db = Db::open(bucket, "p", key.clone()).await?;
        db.in_step = false;
        Ok(db)
    }

    #[test]
    fn what_the_bucket_keeps_is_sealed_to_its_name() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
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
            assert_eq!(notes(&Db::open(&bucket, "p", key).await.unwrap().older()), ["a secret note", "another"]);
        });
    }

    #[test]
    fn a_step_says_who_wrote_it_and_what_the_database_came_to() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let writer = ed25519_dalek::SigningKey::from_bytes(&rand::random());
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            a.sign_as(writer.clone());
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..3 {
                a.write(&bucket, |c| execute(c, "insert into counts values (?, ?)", &[json!(format!("c{n}")), json!(n)]).map(|_| ())).await.unwrap();
                note(&mut a, &bucket, "a note").await;
            }
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            assert_eq!(read.checks(), Checks { signed: 7, ..Checks::default() });
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
            forged["by"] = json!(crate::keys::hex(forger.verifying_key().as_bytes()));
            objects.insert(sig_name("p", 4), crate::keys::seal(1, &key[&1], &sig_name("p", 4), &serde_json::to_vec(&forged).unwrap()));
            // A step with none is an older writer's.
            objects.remove(&sig_name("p", 5));
            drop(objects);
            assert_eq!(Db::open(&bucket, "p", key.clone()).await.unwrap().older().checks(), Checks { signed: 4, unsigned: 1, wrong: 2, refused: 0 });
            // The last step's says what the database comes to, which a reader checks: here, it doesn't.
            let mut objects = bucket.objects.lock().unwrap();
            let name = step_name("p", 7);
            let step = objects[&name].clone();
            objects.insert(sig_name("p", 7), Sig::seal(&writer, 1, &key[&1], &name, &step, Some(&"0".repeat(64))));
            drop(objects);
            assert_eq!(Db::open(&bucket, "p", key).await.unwrap().older().checks().wrong, 3);
        });
    }

    #[test]
    fn the_state_hash_is_the_rows_whichever_way_they_came() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
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
            let copied = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            assert!(copied.copy > 0, "a copy was written");
            assert_eq!(state_hash(copied.connection()).unwrap(), mine);
            bucket.objects.lock().unwrap().retain(|n, _| !n.contains("/copy/"));
            let stepped = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
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
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..2 * COPY_EVERY + 5 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            let all = notes(&a);
            let count = |what: &str| bucket.objects.lock().unwrap().keys().filter(|n| n.contains(what)).count();
            assert_eq!(count("/copy/"), 2);
            let now = 10 * KEEP_STEPS_FOR;
            // Not yet: the steps it covers are kept `KEEP_STEPS_FOR`, though the copies before the newest go.
            let steps = count(".sig");
            assert_eq!(tidy(&bucket, "p", &key, &BTreeSet::new(), KEEP_STEPS_FOR - 1).await.unwrap(), 1);
            assert_eq!((count("/copy/"), count(".sig")), (1, steps));
            assert_eq!(notes(&Db::open(&bucket, "p", key.clone()).await.unwrap().older()), all);
            // Then the steps it covers go, but the one at its position, which says what it comes to.
            let (_, copy, _) = newest(&bucket.objects.lock().unwrap()["p/newest"]);
            assert_eq!(tidy(&bucket, "p", &key, &BTreeSet::new(), now).await.unwrap(), 2 * (copy as usize - 1));
            assert!(bucket.objects.lock().unwrap().contains_key(&step_name("p", copy)) && !bucket.objects.lock().unwrap().contains_key(&step_name("p", 1)));
            let read = switched(&bucket, &key).await.unwrap();
            assert_eq!(notes(&read), all);
            assert_eq!(read.checks().wrong, 0);
            // Not with a copy that isn't what its step's signer says it comes to.
            let mut b = switched(&bucket, &key).await.unwrap();
            b.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            for n in 0..COPY_EVERY {
                note(&mut b, &bucket, &format!("more {n}")).await;
            }
            let (_, newer, _) = newest(&bucket.objects.lock().unwrap()["p/newest"]);
            let wrong = crate::keys::seal(1, &key[&1], &copy_name("p", newer), &bucket.objects.lock().unwrap().get(&copy_name("p", copy)).map(|c| crate::keys::open(&key[&1], &copy_name("p", copy), c).unwrap()).unwrap());
            bucket.objects.lock().unwrap().insert(copy_name("p", newer), wrong);
            assert_eq!(tidy(&bucket, "p", &key, &BTreeSet::new(), now).await.unwrap(), 0);
            // A reader starts from it all the same, in phase 1, and counts it: a signature beside a step is anyone's to
            // write who has the key.
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            assert_eq!((read.at(), read.checks().wrong), (b.at(), 1));
            // Without it, a reader starts from the copy before, and reads every step since.
            bucket.objects.lock().unwrap().remove(&copy_name("p", newer));
            let read = switched(&bucket, &key).await.unwrap();
            assert_eq!((read.copy, notes(&read)), (copy, notes(&b)));
            // With no copy that holds, and the steps before it gone, a reader says so rather than reading what's left.
            bucket.objects.lock().unwrap().insert(copy_name("p", copy), crate::keys::seal(1, &key[&1], &copy_name("p", copy), b"not the database"));
            let err = switched(&bucket, &key).await.err().expect("it can't read");
            assert_eq!(err.downcast_ref::<Gone>(), Some(&Gone { copy }), "{err:#}");
        });
    }

    #[test]
    fn tidying_never_deletes_a_scope_database_s_steps() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let prefix = crate::database::steps::prefix("global");
            assert!(keeps_every_step(&prefix) && !keeps_every_step("global/notes/main") && !keeps_every_step("global/node/main"));
            let mut a = Db::open(&bucket, &prefix, key.clone()).await.unwrap().older();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..2 * COPY_EVERY + 5 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            let of_steps = |n: &String| position(n).is_some_and(|(p, _)| p == prefix);
            let steps = bucket.objects.lock().unwrap().keys().filter(|n| of_steps(n)).count();
            // The copy before the newest goes, and nothing else.
            assert_eq!(tidy(&bucket, &prefix, &key, &BTreeSet::new(), u64::MAX / 2).await.unwrap(), 1);
            let objects = bucket.objects.lock().unwrap().clone();
            assert_eq!(objects.keys().filter(|n| of_steps(n)).count(), steps);
            assert!(objects.contains_key(&step_name(&prefix, 1)) && !objects.contains_key(&tidied_name(&prefix)));
        });
    }

    #[test]
    fn a_reader_reads_steps_signed_in_them_and_beside_them() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let (beside, inside) = (ed25519_dalek::SigningKey::from_bytes(&rand::random()), ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            a.sign_as(beside.clone());
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            // A writer of phase 2's, signing in the step, beside one of today's.
            let mut b = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            b.sign_as(inside.clone());
            (a.in_step, b.in_step) = (false, true);
            for n in 0..COPY_EVERY + 10 {
                let db = if n % 2 == 0 { &mut a } else { &mut b };
                note(db, &bucket, &format!("note {n}")).await;
            }
            a.catch_up(&bucket).await.unwrap();
            let all = notes(&b);
            assert_eq!(notes(&a), all);
            let steps = b.at();
            let sigs = bucket.objects.lock().unwrap().keys().filter(|n| n.ends_with(".sig")).count() as u64;
            assert_eq!(sigs, 2, "from the first step signed in it, every writer signs in them, and none beside");
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
            assert_eq!((by(2).by, by(3).by, by(4).by), (beside.verifying_key().to_bytes(), inside.verifying_key().to_bytes(), beside.verifying_key().to_bytes()));
            assert_eq!((by(2).from, by(3).from, by(4).from, a.signed_from()), (None, Some(3), Some(3), Some(3)));
            let (_, copy, _) = newest(&bucket.objects.lock().unwrap()["p/newest"]);
            assert!(copy > 0 && by(copy).hash.is_some());
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            assert_eq!((notes(&read), read.checks().wrong), (all.clone(), 0));
            bucket.objects.lock().unwrap().retain(|n, _| !n.contains("/copy/"));
            let read = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            assert_eq!((notes(&read), read.checks()), (all.clone(), Checks { signed: steps, ..Checks::default() }));
            // One whose signature doesn't hold is counted, and in phase 1 read all the same.
            let forger = ed25519_dalek::SigningKey::from_bytes(&rand::random());
            let name = step_name("p", 3);
            let step = crate::keys::open(&key[&1], &name, &bucket.objects.lock().unwrap()[&name]).unwrap();
            let (_, inner) = split_signed(&step).unwrap();
            let mut forged = Sig::wrap(&forger, &name, inner, None, None);
            let at = forged.len() - inner.len() - 1;
            forged[at] ^= 1;
            bucket.objects.lock().unwrap().insert(name.clone(), crate::keys::seal(1, &key[&1], &name, &forged));
            let read = Db::open(&bucket, "p", key).await.unwrap().older();
            assert_eq!((notes(&read), read.checks().wrong), (all, 1));
        });
    }

    #[test]
    fn deleted_steps_are_never_read_as_the_whole_database_nor_written_over() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = switched(&bucket, &key).await.unwrap();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..50 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            // A device that read this far, and was away while the rest was written, and the steps it hasn't read deleted.
            let mut away = switched(&bucket, &key).await.unwrap();
            for n in 50..204 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            assert_eq!(a.at(), 205);
            let mut all = notes(&a);
            assert!(tidy(&bucket, "p", &key, &BTreeSet::new(), 10 * KEEP_STEPS_FOR).await.unwrap() > 0);
            assert!(!bucket.objects.lock().unwrap().contains_key(&step_name("p", 1)));
            // Its write goes after every step, from the newest copy, and every reader reads it.
            note(&mut away, &bucket, "back").await;
            assert_eq!(away.at(), 206);
            all.push("back".into());
            assert_eq!(notes(&away), all);
            assert_eq!(notes(&switched(&bucket, &key).await.unwrap()), all);
            // A reader whose hint names a copy that isn't there, with the steps before the newest gone, reads it from the
            // newest copy that holds, rather than reading it as empty; with none that holds, it says so.
            bucket.objects.lock().unwrap().insert(newest_name("p"), serde_json::to_vec(&json!({ "at": 206, "copy": 101, "epoch": 1 })).unwrap());
            assert_eq!(notes(&switched(&bucket, &key).await.unwrap()), all);
            let copy = away.copy;
            let mut objects = bucket.objects.lock().unwrap();
            let held = objects[&copy_name("p", copy)].clone();
            objects.insert(copy_name("p", copy), crate::keys::seal(1, &key[&1], &copy_name("p", copy), b"not the database"));
            drop(objects);
            assert!(switched(&bucket, &key).await.is_err(), "it can't read");
            // Nor does a hint that says nothing was written make it start a database where one was.
            bucket.objects.lock().unwrap().insert(copy_name("p", copy), held);
            bucket.objects.lock().unwrap().insert(newest_name("p"), serde_json::to_vec(&json!({ "at": 0, "copy": 0, "epoch": 0 })).unwrap());
            assert_eq!(notes(&switched(&bucket, &key).await.unwrap()), all);
        });
    }

    /// A device that read a database to its end a moment ago writes without reading again first, and one that wrote
    /// meanwhile loses nothing for it: its write finds its position taken, reads on and goes after.
    #[test]
    fn a_write_that_skips_looking_loses_no_one_else_s_write() {
        rt().block_on(async {
            let bucket = Memory::default();
            bucket.clock.store(1_000, std::sync::atomic::Ordering::Relaxed);
            let key = keys(&[(1, rand::random())]);
            let mut a = switched(&bucket, &key).await.unwrap();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            let mut b = switched(&bucket, &key).await.unwrap();
            b.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            note(&mut b, &bucket, "from b").await;
            note(&mut a, &bucket, "from a").await;
            note(&mut b, &bucket, "again from b").await;
            for db in [&mut a, &mut b] {
                db.catch_up(&bucket).await.unwrap();
            }
            let all = vec!["from b".to_string(), "from a".into(), "again from b".into()];
            assert_eq!((notes(&a), notes(&b)), (all.clone(), all));
            assert_eq!(a.at(), b.at());
        });
    }

    /// A newest hint is anyone's to write, even a device's without the key, as is junk at a copy's position: neither
    /// loses a write nor stops a reader that a good copy would start.
    #[test]
    fn a_forged_hint_or_junk_copy_loses_no_write_and_strands_no_reader() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = switched(&bucket, &key).await.unwrap();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..2 * COPY_EVERY + 5 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            assert!(tidy(&bucket, "p", &key, &BTreeSet::new(), 10 * KEEP_STEPS_FOR).await.unwrap() > 0);
            let hint = |at: u64, copy: u64| bucket.objects.lock().unwrap().insert(newest_name("p"), serde_json::to_vec(&json!({ "at": at, "copy": copy })).unwrap());
            let mut all = notes(&a);
            // A hint past every step, naming no copy: a new reader reads it all, and its writes land after the last.
            hint(999_999_999, 0);
            let mut b = switched(&bucket, &key).await.unwrap();
            assert_eq!(notes(&b), all);
            note(&mut b, &bucket, "after").await;
            note(&mut b, &bucket, "and after").await;
            assert_eq!(b.at(), a.at() + 2);
            all.extend(["after".to_string(), "and after".to_string()]);
            a.catch_up(&bucket).await.unwrap();
            assert_eq!((notes(&a), notes(&switched(&bucket, &key).await.unwrap())), (all.clone(), all.clone()));
            // Junk where the next copy goes, from a device without the key: the writer's hint never names it.
            let next = newest(&bucket.objects.lock().unwrap()[&newest_name("p")]).1 + COPY_EVERY;
            let junk = crate::keys::seal(1, &rand::random(), &copy_name("p", next), b"junk");
            bucket.objects.lock().unwrap().insert(copy_name("p", next), junk.clone());
            while a.at() <= next {
                note(&mut a, &bucket, "more").await;
                all.push("more".into());
            }
            let (_, named, _) = newest(&bucket.objects.lock().unwrap()[&newest_name("p")]);
            assert!(named > next, "the hint names its own copy, after the junk, not the junk: {named}");
            // Lowered to the junk, or to a copy that's gone: a reader reads from the newest that holds, and tidying
            // goes on from it.
            for copy in [next, 1, 999_999_999] {
                hint(a.at(), copy);
                assert_eq!(notes(&switched(&bucket, &key).await.unwrap()), all, "a hint naming copy {copy}");
            }
            assert!(tidy(&bucket, "p", &key, &BTreeSet::new(), 10 * KEEP_STEPS_FOR).await.unwrap() > 0);
            assert!(!bucket.objects.lock().unwrap().contains_key(&step_name("p", next - 1)));
            assert_eq!(bucket.objects.lock().unwrap().get(&copy_name("p", next)), None, "junk below the copy that holds goes with the copies before it");
            // The writer's hint keeps the copy it names: never lowered, even from a hint that says junk.
            note(&mut a, &bucket, "last").await;
            all.push("last".into());
            assert_eq!(newest(&bucket.objects.lock().unwrap()[&newest_name("p")]).1, 999_999_999);
            assert_eq!(notes(&switched(&bucket, &key).await.unwrap()), all);
        });
    }

    /// From where a database's steps are signed in them, a step that isn't, or whose signature doesn't hold or doesn't
    /// say where they're signed from, is passed over by every reader, from the first step or from a copy, and a writer
    /// that can't sign doesn't write.
    #[test]
    fn from_where_steps_are_signed_every_reader_refuses_the_same_steps() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            note(&mut a, &bucket, "before").await;
            a.in_step = true;
            note(&mut a, &bucket, "signed").await;
            assert_eq!(a.signed_from(), Some(3));
            // Another device's steps, as one that ignores that would write them: unsigned, signed beside, signed in them
            // saying another position, and signed wrong.
            let other = ed25519_dalek::SigningKey::from_bytes(&rand::random());
            let inner = |n: u64| {
                let name = step_name("p", n);
                let opened = crate::keys::open(&key[&1], &name, &bucket.objects.lock().unwrap()[&name]).unwrap();
                split_signed(&opened).map_or(opened.clone(), |(_, inner)| inner.to_vec())
            };
            let plain = inner(2);
            let put = |n: u64, step: Vec<u8>| bucket.objects.lock().unwrap().insert(step_name("p", n), crate::keys::seal(1, &key[&1], &step_name("p", n), &step));
            put(4, plain.clone());
            put(5, plain.clone());
            let beside = Sig::seal(&other, 1, &key[&1], &step_name("p", 5), &bucket.objects.lock().unwrap()[&step_name("p", 5)], None);
            bucket.objects.lock().unwrap().insert(sig_name("p", 5), beside);
            put(6, Sig::wrap(&other, &step_name("p", 6), &plain, None, Some(6)));
            let mut wrong = Sig::wrap(&other, &step_name("p", 7), &plain, None, Some(3));
            let flip = wrong.len() - plain.len() - 1;
            wrong[flip] ^= 1;
            put(7, wrong);
            // Each is passed over, and the next write goes after them.
            for n in 0..COPY_EVERY {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            let all = notes(&a);
            assert_eq!(all.iter().filter(|n| *n == "before").count(), 1);
            assert!(a.checks().refused >= 4, "{:?}", a.checks());
            let from_copy = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert!(from_copy.copy > 7 && from_copy.signed_from() == Some(3));
            bucket.objects.lock().unwrap().retain(|n, _| !n.contains("/copy/"));
            let from_start = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert_eq!((notes(&from_copy), notes(&from_start)), (all.clone(), all.clone()));
            assert_eq!((from_start.checks().refused, from_start.signed_from()), (a.checks().refused, Some(3)));
            // A writer with no key to sign with doesn't write.
            let mut unsigned = Db::open(&bucket, "p", key.clone()).await.unwrap();
            assert!(unsigned.write(&bucket, |c| execute(c, "insert into notes values ('x')", &[]).map(|_| ())).await.is_err());
        });
    }

    /// A bucket that notes the objects and folders asked of it.
    struct Counted<'a> {
        bucket: &'a Memory,
        asked: std::sync::Mutex<Vec<String>>,
    }

    impl Bucket for Counted<'_> {
        async fn get(&self, name: &str) -> Result<Option<crate::bucket::Object>> {
            self.asked.lock().unwrap().push(name.into());
            self.bucket.get(name).await
        }
        async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
            self.bucket.create(name, bytes).await
        }
        async fn replace(&self, name: &str, bytes: &[u8], tag: Option<&str>) -> Result<bool> {
            self.bucket.replace(name, bytes, tag).await
        }
        async fn wait(&self, ms: u64) {
            self.bucket.wait(ms).await
        }
        async fn objects(&self, folder: &str) -> Result<Vec<(String, u64)>> {
            self.asked.lock().unwrap().push(format!("{folder}/"));
            self.bucket.objects(folder).await
        }
    }

    /// A device of the network without the key can write `tidied`, and steps and copies far ahead under an epoch
    /// nobody was given: a copy that doesn't open stops nobody.
    #[test]
    fn junk_from_a_device_without_the_key_stops_no_reader() {
        rt().block_on(async {
            let bucket = Memory::default();
            let key = keys(&[(1, rand::random()), (3, rand::random())]);
            let mut a = Db::open(&bucket, "p", key.clone()).await.unwrap().older();
            a.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            a.migrate(&bucket, &[SCHEMA.into()]).await.unwrap();
            for n in 0..COPY_EVERY + 5 {
                note(&mut a, &bucket, &format!("note {n}")).await;
            }
            let mut all = notes(&a);
            let far = 1_000_000;
            {
                let mut objects = bucket.objects.lock().unwrap();
                objects.insert(tidied_name("p"), b"{}".to_vec());
                // Under epoch 0, never given, and under epoch 2, which this reader wasn't.
                for (at, epoch) in [(far, 0), (far + 1, 2)] {
                    for name in [step_name("p", at), copy_name("p", at)] {
                        objects.insert(name.clone(), crate::keys::seal(epoch, &rand::random(), &name, b"junk"));
                    }
                }
            }
            let counted = Counted { bucket: &bucket, asked: Default::default() };
            let mut b = Db::open(&counted, "p", key.clone()).await.unwrap();
            assert_eq!(notes(&b), all);
            note(&mut b, &counted, "after").await;
            all.push("after".into());
            a.catch_up(&counted).await.unwrap();
            assert_eq!((notes(&a), notes(&Db::open(&counted, "p", key.clone()).await.unwrap())), (all.clone(), all.clone()));
            // It finds the junk copies, and passes over them.
            assert_eq!(notes(&switched(&bucket, &key).await.unwrap()), all);
            bucket.objects.lock().unwrap().insert(newest_name("p"), serde_json::to_vec(&json!({ "at": far + 1, "copy": 0 })).unwrap());
            assert_eq!(notes(&switched(&bucket, &key).await.unwrap()), all);
        });
    }
}
