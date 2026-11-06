//! What one device holds and knows: `global`'s scope database as its steps leave it, from the freeze the network's
//! recovery key signed (`database::steps`), which is what everything asks. No clock or I/O: `node.rs` reads and writes
//! the bucket, and says the time.

use anyhow::{Context, Result, ensure};
use ed25519_dalek::SigningKey;

use crate::database::scopes::ScopeDb;
use crate::database::steps::{self, Freeze, Given, Truth};
use crate::database::content::Content;
use crate::database::steps::Step;
use crate::keys::{Hash, Key};
use crate::database::scopes::Scope;

/// What one device holds and knows.
pub struct Replica {
    pub key: SigningKey,
    pub id: Key,
    /// The network's root, its id, once this device created or joined it: who's who is its first freeze
    /// (`Truth::first`).
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
    pub fn new(key: SigningKey, root: Option<Hash>, now: u64) -> Replica {
        let id = key.verifying_key().to_bytes();
        Replica { key, id, root, db: nobody(now), frozen: None, looked: 0 }
    }

    /// Step `at` of `global`'s scope database before this device has read its freeze, the next it looks at: if it's
    /// the network's first freeze (`Truth::first`), who's who from here. One a newer release wrote fails, so this
    /// device looks at it again and never past it.
    pub fn take_first(&mut self, at: u64, given: Option<&Given>, now: u64) -> Result<Handled> {
        ensure!(self.frozen.is_none() && at == self.looked + 1, "step {at} isn't the next");
        let mut handled = Handled::default();
        let truth = Truth::first(at, given, self.root, now)?;
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
        Ok(Some(steps::Welcome { freeze: freeze.at, at: *at, hash: crate::database::stored::state_hash(self.db.conn())? }))
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
    pub fn take_step(&mut self, at: u64, step: Option<&Step>, now: u64) -> Result<Handled> {
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
    pub fn make_step(&self, key: &SigningKey, user: &str, content: Content, now: u64) -> Result<Step> {
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

    /// What changed from `was`, who's who before, to `db`: scopes this device no longer holds; whether it holds one it
    /// didn't; and people who may no longer use a scope they could.
    fn changed(&mut self, was: &ScopeDb, handled: &mut Handled) -> Result<()> {
        let (had, has) = (was.run_by(&self.id), self.db.run_by(&self.id));
        handled.dropped.extend(had.iter().filter(|s| **s != Scope::Global && !has.contains(s)).map(Scope::to_string));
        handled.gained |= has.iter().any(|s| !had.contains(s));
        let after = self.db.roles();
        for ((user, scope), (_, invited)) in was.roles() {
            if !invited && !after.get(&(user.clone(), scope)).is_some_and(|(_, invited)| !invited) && !handled.lost.contains(&user) {
                handled.lost.push(user);
            }
        }
        Ok(())
    }
}

/// `global`'s scope database of nobody, which a device answers as until it has read its freeze.
fn nobody(now: u64) -> ScopeDb {
    ScopeDb::empty(&Scope::Global, now).expect("an empty who's who builds")
}
