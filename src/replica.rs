//! What one device holds and knows: `global`'s scope database as its steps leave it, from the freeze the network's
//! recovery key signed (`database::steps`), which is what everything asks; and the entries of each frozen log of the
//! scopes it holds, as the network's bucket has them, read only. No clock or I/O: `node.rs` reads and writes the
//! bucket, and says the time.

use anyhow::{Context, Result, ensure};
use ed25519_dalek::SigningKey;

use crate::database::scopes::ScopeDb;
use crate::database::steps::{self, Freeze, Given, Truth};
use crate::entry::{Content, Entry, Hash, Key, Scope};
use crate::members::Members;
use crate::store::Store;

/// What one device holds and knows.
pub struct Replica {
    pub key: SigningKey,
    pub id: Key,
    /// Each frozen log's entries this device holds, read only.
    pub store: Store,
    /// The network's genesis, once this device created or joined it.
    pub root: Option<Hash>,
    /// `global`'s scope database: one of nobody until this device has read its freeze.
    db: ScopeDb,
    /// Once this device has read the freeze: it, and the last of the scope database's steps applied to `db` (or
    /// passed over).
    pub frozen: Option<(Freeze, u64)>,
    /// Before then, the last of the scope database's steps this device looked at that wasn't its freeze.
    pub looked: u64,
}

/// What reading the bucket changed here.
#[derive(Default)]
pub struct Handled {
    /// Entries of the frozen logs new to this device.
    pub new: Vec<Entry>,
    /// Whether who's who changed.
    pub members: bool,
    /// Scopes this device stopped holding, and deleted.
    pub dropped: Vec<String>,
    /// Whether this device now holds a scope it didn't, whose steps it should read.
    pub gained: bool,
    /// People who may no longer use a scope they could, whose sessions this device forgets.
    pub lost: Vec<String>,
}

impl Replica {
    pub fn new(key: SigningKey, store: Store, root: Option<Hash>, now: u64) -> Result<Replica> {
        let id = key.verifying_key().to_bytes();
        Ok(Replica { key, id, store, root, db: nobody(now), frozen: None, looked: 0 })
    }

    /// The network's recovery key, as its genesis among `global`'s entries here names it: only the first freeze it
    /// signed is who's who.
    pub fn recovery(&self) -> Option<Key> {
        steps::recovery_of(self.root?, &self.store.scope("global").ok()?)
    }

    /// Step `at` of `global`'s scope database before this device has read its freeze, the next it looks at: if it's
    /// the freeze the network's recovery key signed, its copy checked against its hash, who's who from here. One that
    /// doesn't check out fails, so this device looks at it again and never past it.
    pub fn take_first(&mut self, at: u64, given: Option<&Given>, now: u64) -> Result<Handled> {
        ensure!(self.frozen.is_none() && at == self.looked + 1, "step {at} isn't the next");
        let mut handled = Handled::default();
        let recovery = self.recovery().context("this device hasn't read its network's genesis yet")?;
        let truth = Truth::first(at, given, recovery, now)?;
        self.looked = at;
        if let Some(t) = truth {
            self.frozen = Some((t.freeze, at));
            let was = std::mem::replace(&mut self.db, t.db);
            handled.members = true;
            self.changed(&was, &mut handled)?;
        }
        Ok(handled)
    }

    /// Where this device is in `global`'s scope database, to keep and start again from (`restart`).
    pub fn keep(&mut self) -> Result<Option<serde_json::Value>> {
        let Some((freeze, at)) = self.frozen.clone() else { return Ok(None) };
        let mut t = Truth { freeze, at, db: std::mem::replace(&mut self.db, nobody(0)) };
        let kept = t.keep();
        self.db = t.db;
        kept.map(Some)
    }

    /// Where this device has read `global`'s scope database to, for a device it lets in to start from.
    pub fn welcome(&self) -> Result<Option<steps::Welcome>> {
        let Some((freeze, at)) = &self.frozen else { return Ok(None) };
        Ok(Some(steps::Welcome { freeze: freeze.at, at: *at, hash: crate::db::state_hash(self.db.conn())? }))
    }

    /// Starts again from what `keep` kept, checked against its hash (`steps::Truth::kept`), as this device started.
    pub fn restart(&mut self, kept: &serde_json::Value, now: u64) -> Result<()> {
        let t = Truth::kept(kept, now)?;
        self.frozen = Some((t.freeze, t.at));
        self.db = t.db;
        Ok(())
    }

    /// `global`'s scope database as it is, to copy (`steps::written`).
    pub fn db_mut(&mut self) -> &mut ScopeDb {
        &mut self.db
    }

    /// The position of `global`'s scope database this device has applied its steps to.
    pub fn at(&self) -> Option<u64> {
        self.frozen.as_ref().map(|(_, at)| *at)
    }

    /// A step of `global`'s scope database another device wrote, at the position after the last this device applied:
    /// applied if it counts, passed over if not; either way its position is taken.
    pub fn take_step(&mut self, at: u64, step: Option<&Entry>, now: u64) -> Result<Handled> {
        let (freeze, last) = self.frozen.clone().context("this device hasn't read who's who yet")?;
        ensure!(at == last + 1, "step {at} isn't the next");
        let mut handled = Handled::default();
        let applied = step.map(|s| steps::apply(&self.db, None, &Scope::Global, s));
        // A newer release's: this device reads no further until it's updated.
        if let Some(Err(err)) = &applied
            && err.is::<steps::Newer>()
        {
            anyhow::bail!("{err}");
        }
        self.frozen = Some((freeze, at));
        match applied {
            Some(Ok(a)) => {
                let was = std::mem::replace(&mut self.db, ScopeDb::open(a.db, now)?);
                self.changed(&was, &mut handled)?;
                handled.members = true;
            }
            Some(Err(err)) => tracing::info!("global's step {at} doesn't count: {err:#}"),
            None => tracing::debug!("global's step {at} isn't one: passed over"),
        }
        Ok(handled)
    }

    /// A step of `global`'s database signed by `key` (this device's, or the recovery key) as `user`, for the position
    /// after the last it applied, once it would count there.
    pub fn make_step(&self, key: &SigningKey, user: &str, content: Content, now: u64) -> Result<Entry> {
        let at = self.at().context("this device hasn't read who's who yet")? + 1;
        let step = steps::sign(key, &Scope::Global, at, user, self.db.epoch(&Scope::Global), now, content)?;
        steps::apply(&self.db, None, &Scope::Global, &step)?;
        Ok(step)
    }

    /// `global`'s scope database: who's who, and who may do what.
    pub fn db(&self) -> &ScopeDb {
        &self.db
    }

    /// `global`'s epochs' hashes by number: which key opens which of its steps.
    pub fn global_epochs(&self) -> std::collections::BTreeMap<u32, String> {
        self.db.epoch_hashes(&Scope::Global)
    }

    /// The newest epoch of a scope made: a step under a newer one is junk.
    pub fn epoch_made(&self, scope: &Scope) -> u32 {
        self.db.epoch(scope)
    }

    /// Whether this device holds `scope`: `global`, and each scope whose calls it runs. Until it sees itself listed it
    /// holds only `global`.
    pub fn holds(&self, scope: &Scope) -> bool {
        *scope == Scope::Global || self.db().runs_for(&self.id, scope)
    }

    fn holds_log(&self, log: &str) -> bool {
        Scope::parse(log).is_ok_and(|s| self.holds(&s))
    }

    /// What changed from `was`, who's who before, to `db`: scopes this device no longer holds, and deletes; whether it
    /// holds one it didn't; and people who may no longer use a scope they could.
    fn changed(&mut self, was: &ScopeDb, handled: &mut Handled) -> Result<()> {
        for log in &self.store.logs()? {
            if !self.holds_log(log) {
                self.store.delete(log)?;
                handled.dropped.push(log.clone());
            }
        }
        let had = was.run_by(&self.id);
        handled.gained |= self.db.run_by(&self.id).iter().any(|s| !had.contains(s));
        let after = self.db.roles();
        for ((user, scope), (_, invited)) in was.roles() {
            if !invited && !after.get(&(user.clone(), scope)).is_some_and(|(_, invited)| !invited) && !handled.lost.contains(&user) {
                handled.lost.push(user);
            }
        }
        Ok(())
    }

    /// Takes a frozen log's entries as the bucket gives them: each checked (signature, body, chain) and kept once
    /// everything it follows is here, in scopes this device holds. They change nothing.
    pub fn receive(&mut self, entries: Vec<Entry>) -> Result<Handled> {
        let mut handled = Handled::default();
        let entries = entries.into_iter().filter(|e| self.holds_log(e.log())).collect();
        let (taken, _) = self.store.take(entries)?;
        handled.new = taken;
        Ok(handled)
    }
}

/// `global`'s scope database of nobody, which a device answers as until it has read its freeze.
fn nobody(now: u64) -> ScopeDb {
    ScopeDb::build(&Members::default(), &Scope::Global, &Default::default(), now).expect("an empty who's who builds")
}
