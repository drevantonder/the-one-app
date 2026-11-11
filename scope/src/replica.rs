//! What one device holds and knows: `global`'s scope database as its steps leave it, from the freeze the network's
//! recovery key signed (`database::steps`), which is what everything asks. No clock or I/O: the app runtime (`app-runtime/src/network.rs`) reads and writes
//! the bucket, and says the time.

use anyhow::{Context, Result, ensure};
use ed25519_dalek::SigningKey;

use crate::content::Content;
use crate::scopes::{Scope, ScopeDb};
use crate::steps::{self, Freeze, Given, Reader, Sealed, Step, Truth};
use ::key::{Hash, Key};

pub struct Replica {
    pub key: SigningKey,
    pub id: Key,
    /// `global`'s scope database as this device read it, of the network's root, once this device created or joined it.
    reader: Reader,
    /// Who's who until this device has read the freeze: nobody.
    nobody: ScopeDb,
    /// Whether what it started from lacks steps it hasn't yet tried to read again (`steps::read_again`).
    pub read_again: bool,
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
    /// Users who may no longer use a scope they could, whose sessions this device forgets.
    pub lost: Vec<String>,
}

impl Replica {
    pub fn new(key: SigningKey, root: Option<Hash>, now: u64) -> Replica {
        let id = key.verifying_key().to_bytes();
        Replica { key, id, reader: Reader::new(root), nobody: nobody(now), read_again: false }
    }

    /// The network's root, its id.
    pub fn root(&self) -> Option<Hash> {
        self.reader.root()
    }

    pub fn freeze(&self) -> Option<&Freeze> {
        self.reader.truth().map(|t| &t.freeze)
    }

    /// Who's who, once this device has read the freeze.
    pub fn truth(&self) -> Option<&Truth> {
        self.reader.truth()
    }

    /// The last of `global`'s steps this device took, or looked at before its freeze.
    pub fn looked(&self) -> u64 {
        self.reader.at()
    }

    /// Takes `sealed`'s steps from the next on (`steps::Reader::add`). → what that changed here, also when it then
    /// failed, and the epoch it stopped at, if it did.
    pub fn add(&mut self, sealed: &Sealed, keys: &database::stored::Keys, now: u64) -> (Handled, Result<Option<u32>>) {
        let mut handled = Handled::default();
        let id = self.id;
        let waits = self.reader.add(sealed, keys, None, now, |_, _, changed| {
            if let Some((was, db)) = changed {
                changed_from(&id, &was, db, &mut handled)?;
            }
            Ok(())
        });
        (handled, waits)
    }

    /// Where this device is in `global`'s scope database, to keep and start again from (`restart`).
    pub fn keep(&mut self) -> Result<Option<serde_json::Value>> {
        self.reader.truth_mut().map(Truth::keep).transpose()
    }

    /// Where this device has read `global`'s scope database to, for a device it lets in to start from.
    pub fn welcome(&self) -> Result<Option<steps::Welcome>> {
        self.reader.truth().map(Truth::welcome).transpose()
    }

    /// Starts again from what `keep` kept, checked against its hash (`steps::Truth::kept`), as this device started.
    pub fn restart(&mut self, kept: &serde_json::Value, now: u64) -> Result<()> {
        self.reader.resume(Truth::kept(kept, now)?)?;
        self.read_again = !self.reader.whole();
        Ok(())
    }

    /// Who's who as `fresh` read it again from the bucket, in place of what it had. → what that changed here.
    pub fn replace(&mut self, fresh: Reader) -> Result<Handled> {
        let mut handled = Handled::default();
        if let (Some(was), Some(now)) = (self.reader.truth(), fresh.truth()) {
            changed_from(&self.id, &was.db, &now.db, &mut handled)?;
        }
        self.reader = fresh;
        self.read_again = false;
        Ok(handled)
    }

    /// `global`'s scope database as it is, to copy (`steps::written`).
    pub fn db_mut(&mut self) -> &mut ScopeDb {
        match self.reader.truth_mut() {
            Some(t) => &mut t.db,
            None => &mut self.nobody,
        }
    }

    /// The position of `global`'s scope database this device has applied its steps to.
    pub fn at(&self) -> Option<u64> {
        self.reader.truth().map(|t| t.at)
    }

    /// A step of `global`'s scope database this device wrote, at the position after the last it applied: applied if it
    /// counts, passed over if not; either way its position is taken.
    pub fn take_step(&mut self, at: u64, step: &Step, now: u64) -> Result<Handled> {
        ensure!(self.at().context("this device hasn't read who's who yet")? + 1 == at, "step {at} isn't the next");
        let mut handled = Handled::default();
        let given = Given { step: step.clone(), copy: None };
        if let Some((was, db)) = self.reader.take(Some(&given), now)? {
            changed_from(&self.id, &was, db, &mut handled)?;
        }
        Ok(handled)
    }

    /// A step of `global`'s database signed by `key` (this device's, or the recovery key) as `user`, for the position
    /// after the last it applied, once it would count there.
    pub fn make_step(&self, key: &SigningKey, user: &str, content: Content, now: u64) -> Result<Step> {
        let at = self.at().context("this device hasn't read who's who yet")? + 1;
        let step = steps::sign(key, &Scope::Global, at, user, self.db().epoch(&Scope::Global)?, now, content)?;
        steps::apply(self.db(), None, &Scope::Global, &step)?;
        Ok(step)
    }

    /// `global`'s scope database: who's who, and who may do what.
    pub fn db(&self) -> &ScopeDb {
        self.reader.truth().map_or(&self.nobody, |t| &t.db)
    }

    /// Whether this device holds `scope`: `global`, and each scope whose calls it runs. Until it sees itself listed it
    /// holds only `global`.
    pub fn holds(&self, scope: &Scope) -> bool {
        *scope == Scope::Global || self.db().runs_for(&self.id, scope)
    }
}

/// What changed for the device `id` from `was`, who's who before, to `db`: scopes it no longer holds; whether it holds
/// one it didn't; and users who may no longer use a scope they could.
fn changed_from(id: &Key, was: &ScopeDb, db: &ScopeDb, handled: &mut Handled) -> Result<()> {
    handled.members = true;
    let (had, has) = (was.run_by(id)?, db.run_by(id)?);
    handled.dropped.extend(had.iter().filter(|s| **s != Scope::Global && !has.contains(s)).map(Scope::to_string));
    handled.gained |= has.iter().any(|s| !had.contains(s));
    let after = db.roles()?;
    for ((user, scope), (_, invited)) in was.roles()? {
        if !invited && !after.get(&(user.clone(), scope)).is_some_and(|(_, invited)| !invited) && !handled.lost.contains(&user) {
            handled.lost.push(user);
        }
    }
    Ok(())
}

/// `global`'s scope database of nobody, which a device answers as until it has read its freeze.
fn nobody(now: u64) -> ScopeDb {
    ScopeDb::empty(&Scope::Global, now).expect("an empty who's who builds")
}
