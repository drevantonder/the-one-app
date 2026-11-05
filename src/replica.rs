//! What one device holds and knows: the entries of each scope it holds, as the install's bucket has them, the
//! members `global`'s add up to, and `global`'s scope database built from them, which is what everything else asks.
//! No clock or I/O: `node.rs` reads and writes the bucket, and says the time.

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use ed25519_dalek::SigningKey;

use crate::entry::{Content, Entry, Ext, Hash, Key, Scope};
use crate::database::scopes::ScopeDb;
use crate::members::{self, Members};
use crate::store::Store;

/// What one device holds and knows: its store, and the members its `global` adds up to.
pub struct Replica {
    pub key: SigningKey,
    pub id: Key,
    pub store: Store,
    /// The install's genesis, once this device created or joined it.
    pub root: Option<Hash>,
    pub members: Members,
    /// `global`'s scope database, built again whenever the members change: what everything but replay asks. While the
    /// last build failed, one of nobody (`built`), so nothing is sealed or handed to anyone the members no longer list.
    /// Reading `global` asks replay instead (`global_epochs`), since that's how it builds again.
    db: ScopeDb,
    /// The last that built, while `db` is that one of nobody: none since this device started, if it never built.
    built: Option<ScopeDb>,
    /// Makes the next builds fail, for a test.
    #[cfg(test)]
    pub fail: bool,
    /// `global`'s node entries, in replay order.
    pub member_entries: Vec<Entry>,
    /// Whether an entry arrived that sorts before the newest of `member_entries`, so the members need replaying.
    stale: bool,
    /// Once `global`'s log is frozen (`database::switch`): its freeze, and the last of the scope database's steps
    /// applied to `db` (or passed over). From then on `db` is the truth, and entries change nothing.
    pub frozen: Option<(crate::database::steps::Freeze, u64)>,
    /// Before it's frozen, the last of the scope database's steps this device looked at that wasn't its freeze.
    pub looked: u64,
    /// Whether one of those says an admin's device is freezing the log (`steps::FREEZING`).
    pub freezing: bool,
}

/// What taking new entries did.
#[derive(Default)]
pub struct Handled {
    /// Entries new to this device.
    pub new: Vec<Entry>,
    /// Whether the members changed.
    pub members: bool,
    /// Scopes this device stopped holding, and deleted.
    pub dropped: Vec<String>,
    /// Whether this device now holds a scope it didn't, whose entries it should read.
    pub gained: bool,
    /// People who may no longer use a scope they could, whose sessions this device forgets.
    pub lost: Vec<String>,
    /// Whether where devices' entries stop counting changed (a removal or a fork), so every scope is replayed again.
    pub recount: bool,
}

impl Replica {
    pub fn new(key: SigningKey, store: Store, root: Option<Hash>, now: u64) -> Result<Replica> {
        let id = key.verifying_key().to_bytes();
        let member_entries: Vec<Entry> = store.scope("global")?.into_iter().filter(members::is_member_entry).collect();
        let members = Members::replay(root, &member_entries, &store.forks()?);
        // One that doesn't build answers nobody, and reads `global` until it does, as `refresh` does.
        let (db, built) = match ScopeDb::build(&members, &Scope::Global, &Default::default(), now) {
            Ok(db) => (db, None),
            Err(err) => {
                tracing::error!("{:#}", err.context("who's who didn't build, so this device answers nobody until it does"));
                (nobody(now), Some(nobody(now)))
            }
        };
        Ok(Replica { key, id, store, root, members, db, built, #[cfg(test)] fail: false, member_entries, stale: false, frozen: None, looked: 0, freezing: false })
    }

    /// The freeze a device of an admin's writes now (`steps::freeze`), at `logs`' ends, every other scope's log by its
    /// last step: from everything this device holds of `global`. → its content, and its copy.
    pub fn freeze_now(&self, logs: BTreeMap<String, u64>, now: u64) -> Result<(Content, Vec<u8>)> {
        crate::database::steps::freeze(self.root, &self.store.scope("global")?, &self.store.forks()?, logs, now)
    }

    /// Step `at` of `global`'s scope database before its log is frozen, the next this device looks at: if it's the
    /// freeze, and checks out against the entries this device holds (`steps::anchor`), the database it anchors is
    /// who's who from here. Fails while an entry it names isn't here, so it's looked at again once it is.
    pub fn take_first(&mut self, at: u64, given: Option<&crate::database::steps::Given>, now: u64) -> Result<Handled> {
        ensure!(self.frozen.is_none() && at == self.looked + 1, "step {at} isn't the next");
        let mut handled = Handled::default();
        let truth = crate::database::steps::Truth::first(at, given, self.root, &self.store.scope("global")?, &self.store.forks()?, now)?;
        self.looked = at;
        let admins = |s: &Entry| self.db.client(&s.author()).is_some_and(|c| c.user == s.user()) && self.db.may(s.user(), "change-settings", crate::database::scopes::INSTALL);
        self.freezing |= given.is_some_and(|g| g.step.content.kind == crate::database::steps::FREEZING && admins(&g.step));
        if let Some(t) = truth {
            tracing::info!("global's log is frozen at step {at}: the scope database is the truth");
            self.frozen = Some((t.freeze, at));
            let was = std::mem::replace(&mut self.db, t.db);
            self.built = None;
            handled.members = true;
            self.changed(&was, &mut handled)?;
        }
        Ok(handled)
    }

    /// Where this device is in `global`'s scope database once it's frozen, to keep and start again from (`restart`).
    pub fn keep(&mut self) -> Result<Option<serde_json::Value>> {
        let Some((freeze, at)) = self.frozen.clone() else { return Ok(None) };
        let mut t = crate::database::steps::Truth { freeze, at, db: std::mem::replace(&mut self.db, nobody(0)) };
        let kept = t.keep();
        self.db = t.db;
        kept.map(Some)
    }

    /// Starts again from what `keep` kept, checked against its hash (`steps::Truth::kept`), as this device started.
    pub fn restart(&mut self, kept: &serde_json::Value, now: u64) -> Result<()> {
        let t = crate::database::steps::Truth::kept(kept, now)?;
        self.frozen = Some((t.freeze, t.at));
        self.db = t.db;
        self.built = None;
        Ok(())
    }

    /// `global`'s scope database as it is, to copy (`steps::written`).
    pub fn db_mut(&mut self) -> &mut ScopeDb {
        &mut self.db
    }

    /// The position of `global`'s scope database this device has applied its steps to, once its log is frozen.
    pub fn at(&self) -> Option<u64> {
        self.frozen.as_ref().map(|(_, at)| *at)
    }

    /// A step of `global`'s scope database another device wrote, at the position after the last this device applied
    /// (`database::steps`): applied if it counts, passed over if not; either way its position is taken.
    pub fn take_step(&mut self, at: u64, step: Option<&Entry>, now: u64) -> Result<Handled> {
        let (freeze, last) = self.frozen.clone().context("global's log isn't frozen")?;
        ensure!(at == last + 1, "step {at} isn't the next");
        let mut handled = Handled::default();
        let applied = step.map(|s| crate::database::steps::apply(&self.db, None, &Scope::Global, s));
        // A newer release's: this device reads no further until it's updated.
        if let Some(Err(err)) = &applied
            && err.is::<crate::database::steps::Newer>()
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

    /// A step of `scope`'s database signed by this device as `user`, for the position after the last it applied, once
    /// it would count there: `global`'s only, until other scopes' databases are steps too.
    pub fn make_step(&self, scope: &Scope, user: &str, content: Content, now: u64) -> Result<Entry> {
        let at = self.at().context("global's log isn't frozen")? + 1;
        ensure!(*scope == Scope::Global, "only global's database is written as steps yet");
        let step = crate::database::steps::sign(&self.key, scope, at, user, self.db.epoch(scope), now, content)?;
        crate::database::steps::apply(&self.db, None, scope, &step)?;
        Ok(step)
    }

    /// `global`'s scope database: who's who, and who may do what.
    pub fn db(&self) -> &ScopeDb {
        &self.db
    }

    /// `global`'s epochs' hashes by number, as replay says: which key opens which of its steps, not who may have them,
    /// so they're asked even while who's who doesn't build, since reading `global` is how it builds again.
    pub fn global_epochs(&self) -> BTreeMap<u32, String> {
        if self.frozen.is_some() {
            return self.db.epoch_hashes(&Scope::Global);
        }
        self.members.epoch_hashes.iter().filter(|((s, _), _)| *s == Scope::Global).map(|((_, n), h)| (*n, h.clone())).collect()
    }

    /// The newest epoch of a scope an entry that counts made, as replay says: a step under a newer one is junk unless
    /// `global` says otherwise. Not who may have it, so asked even while who's who doesn't build, when every scope
    /// would have none and real steps would be passed over for good.
    pub fn epoch_made(&self, scope: &Scope) -> u32 {
        if self.frozen.is_some() {
            return self.db.epoch(scope);
        }
        self.members.epoch(scope)
    }

    /// Whether this device holds `scope`: `global`, and each scope whose calls it runs. Until it sees itself listed it
    /// holds only `global`.
    pub fn holds(&self, scope: &Scope) -> bool {
        *scope == Scope::Global || self.db().runs_for(&self.id, scope)
    }

    fn holds_log(&self, log: &str) -> bool {
        Scope::parse(log).is_ok_and(|s| self.holds(&s))
    }

    /// Makes an entry for this device's log of `scope`, after everything it holds there and in `global`, to write
    /// to the bucket.
    pub fn make(&self, scope: &Scope, user: &str, content: Content, now: u64) -> Result<Entry> {
        ensure!(self.holds(scope), "this device doesn't hold {scope}");
        let log = scope.to_string();
        let head = self.store.own_head(&self.id, &log)?;
        let held = self.store.last(&self.id, &log)?.map(|e| e.seq());
        // A device that holds a scope again writes there only once it has its own log back, so no seq is written twice.
        ensure!(head.map(|h| h.0) == held, "this device is still getting its own entries in {scope} back");
        let mut previous = self.store.heads(&log, Some(&self.id))?;
        if *scope != Scope::Global {
            previous.extend(self.store.heads("global", None)?);
        }
        let ext = Ext { log, time: now, user: user.into(), epoch: self.db().epoch(scope), previous };
        Ok(Entry::create(&self.key, ext, head, content))
    }

    /// Keeps an entry this device made, once the bucket has it.
    pub fn add_own(&mut self, entry: &Entry, now: u64) -> Result<Handled> {
        let head = self.store.own_head(&self.id, entry.log())?;
        let entry = self.store.insert_own(entry, head)?;
        let mut handled = Handled::default();
        if members::is_member_entry(&entry) {
            let before = self.members.cuts.clone();
            self.add_member_entry(entry.clone());
            self.refresh(&before, &mut handled, now)?;
        }
        handled.new.push(entry);
        Ok(handled)
    }

    /// Whether an entry would count, written now: why not, if it wouldn't. A member entry only when one of its
    /// user's own devices signs it.
    pub fn check(&self, entry: &Entry) -> Result<()> {
        let signer = self.db().machine(&entry.author()).map_or("that device".to_string(), |m| m.name);
        if members::is_member_entry(entry) {
            let recovery = self.members.recovery == Some(entry.author());
            ensure!(recovery || self.members.client(&entry.author()).is_some_and(|c| c.user == entry.user()), "{signer} isn't one of your devices, so it can't sign that for you: sign it on one of yours");
            self.members.clone().try_step(entry)?;
        } else if entry.content.kind != "genesis" {
            ensure!(self.members.counts(entry), "{signer} can't write in {} for you", entry.log());
        }
        Ok(())
    }

    /// Whether an entry another device signed can be added now: it's new, this device holds everything it follows, and
    /// it would count.
    pub fn check_signed(&self, entry: &Entry) -> Result<()> {
        ensure!(self.holds(&entry.scope()), "this device doesn't hold {}", entry.log());
        ensure!(!self.store.has(&entry.author(), entry.log(), entry.seq())?, "entry {} is here already", entry.id());
        let prev = match entry.seq() {
            0 => None,
            n => Some(self.store.get(&entry.author(), entry.log(), n - 1)?.context("it doesn't follow its device's last entry here")?.head()),
        };
        entry.follows(prev)?;
        for hash in entry.previous() {
            ensure!(self.store.has_hash(hash)?, "it follows an entry this device doesn't have");
        }
        self.check(entry)
    }

    fn add_member_entry(&mut self, entry: Entry) {
        let order = entry.order();
        let at = self.member_entries.partition_point(|e| e.order() < order);
        if at == self.member_entries.len() && !self.stale {
            self.members.step(&entry);
        } else {
            self.stale = true;
        }
        self.member_entries.insert(at, entry);
    }

    /// Replays the members again and builds `global`'s scope database from them, deletes every scope this device no
    /// longer holds, and says what changed since the members had the cuts `before`. If it doesn't build, the device
    /// answers nobody until it does, rather than as the members were.
    fn refresh(&mut self, before: &members::Cuts, handled: &mut Handled, now: u64) -> Result<()> {
        // Once the log is frozen its entries change nothing: an older device's, which it wrote unaware.
        if self.frozen.is_some() {
            return Ok(());
        }
        if std::mem::take(&mut self.stale) {
            self.members = Members::replay(self.root, &self.member_entries, &self.store.forks()?);
        }
        let built = ScopeDb::build(&self.members, &Scope::Global, &Default::default(), now);
        #[cfg(test)]
        let built = if self.fail { Err(anyhow::anyhow!("made to fail")) } else { built };
        let was = match built {
            Ok(db) => {
                let was = std::mem::replace(&mut self.db, db);
                self.built.take().unwrap_or(was)
            }
            Err(err) => {
                if self.built.is_none() {
                    self.built = Some(std::mem::replace(&mut self.db, nobody(now)));
                }
                return Err(err.context("who's who didn't build, so this device answers nobody until it does"));
            }
        };
        handled.members = true;
        handled.recount |= *before != self.members.cuts;
        self.changed(&was, handled)
    }

    /// What changed from `was`, who's who before, to `db`: scopes this device no longer holds, and deletes; whether it
    /// holds one it didn't; and people who may no longer use a scope they could.
    fn changed(&mut self, was: &ScopeDb, handled: &mut Handled) -> Result<()> {
        let logs = self.store.logs()?;
        for log in &logs {
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

    /// Takes entries as the bucket gives them: each checked (signature, body, chain) and kept once everything it
    /// follows is here, in scopes this device holds; `global`'s first, since they decide which other scopes it holds.
    /// While who's who doesn't build, taking `global`'s builds it again or fails, so no others are dropped meanwhile.
    pub fn receive(&mut self, entries: Vec<Entry>, now: u64) -> Result<Handled> {
        let mut handled = Handled::default();
        let (global, rest): (Vec<Entry>, Vec<Entry>) = entries.into_iter().partition(|e| e.log() == "global");
        self.took(global, &mut handled, now)?;
        let rest = rest.into_iter().filter(|e| self.holds_log(e.log())).collect();
        self.took(rest, &mut handled, now)?;
        handled.new.retain(|e| self.holds_log(e.log()));
        Ok(handled)
    }

    /// Keeps entries, with any that waited for them, and follows the members if they changed, or builds them again if
    /// they last didn't.
    fn took(&mut self, entries: Vec<Entry>, handled: &mut Handled, now: u64) -> Result<()> {
        let before = self.members.cuts.clone();
        let (taken, forked) = self.store.take(entries)?;
        // A fork stops its device's entries counting from there, wherever they sort: replayed again.
        self.stale |= forked;
        let mut changed = forked;
        for entry in taken {
            if members::is_member_entry(&entry) {
                self.add_member_entry(entry.clone());
                changed = true;
            }
            handled.new.push(entry);
        }
        if changed || self.built.is_some() {
            self.refresh(&before, handled, now)?;
        }
        Ok(())
    }
}

/// `global`'s scope database of nobody, which a device answers as while who's who doesn't build. An empty one always
/// builds: if it didn't, this device would stop rather than answer as the members were.
fn nobody(now: u64) -> ScopeDb {
    ScopeDb::build(&Members::default(), &Scope::Global, &Default::default(), now).expect("an empty who's who builds")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Endpoint;
    use crate::database::steps::{self, Given};
    use crate::members::{Kind, Role, device, genesis, setting};

    /// The log's freeze is a step, and exactly one counts: the first that an admin's own device signed and whose copy
    /// says what the entries it names add up to here, compared in this release's form, so a freezer whose release had
    /// other rules (as one that flips `INSTALL_ADMINS_MAKE_GROUP_ADMINS`) still anchors it, its rules kept. A copy that
    /// says otherwise, or a freeze naming entries not here yet, anchors nothing. Once frozen, an older device's
    /// entries change nothing, a step does, and who's who is kept and started again from, never the entries.
    #[test]
    fn one_freeze_anchors_who_s_who_and_the_log_changes_nothing_after() {
        const ANDRE: &str = "a000000000000001";
        let (mac, old) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]));
        let ext = |user: &str| Ext { log: "global".into(), time: 0, user: user.into(), epoch: 1, previous: Vec::new() };
        let g = Entry::create(&mac, Ext { epoch: 0, ..ext(ANDRE) }, None, genesis("Home", ANDRE, "Andre", &Endpoint::of(&mac), "mac", &[Kind::Client], &[7; 32], &"e".repeat(64)));
        let listed = Entry::create(&mac, ext(ANDRE), Some(g.head()), device(&Endpoint::of(&old), ANDRE, "old laptop", &[Kind::Client], None));
        let mut r = Replica::new(mac.clone(), Store::memory().unwrap(), Some(g.hash()), 0).unwrap();
        r.receive(vec![g.clone(), listed.clone()], 0).unwrap();
        // Signed with its copy changed as `edit` changes it, at `at`.
        let freeze = |at: u64, (content, copy): &(Content, Vec<u8>), edit: &str| {
            let mut db = steps::connection(copy).unwrap();
            db.execute_batch(edit).unwrap();
            let mut data = content.data.clone();
            data["hash"] = serde_json::json!(crate::db::state_hash(&db).unwrap());
            let step = steps::sign(&mac, &Scope::Global, at, ANDRE, 1, 0, Content::new(members::APP, steps::FREEZE, data)).unwrap();
            Given { step, copy: Some(db.serialize("main").unwrap().to_vec()) }
        };
        let now = r.freeze_now(Default::default(), 0).unwrap();
        assert!(r.take_first(1, Some(&freeze(1, &now, "UPDATE devices SET name = 'mine' WHERE name = 'old laptop'")), 0).is_ok() && r.at().is_none(), "a copy that isn't what the entries say anchors nothing");
        let mut ahead = now.clone();
        ahead.0.data["frontier"][crate::entry::hex(&mac.verifying_key().to_bytes())] = serde_json::json!(5);
        assert!(r.take_first(2, Some(&freeze(2, &ahead, "")), 0).is_err() && r.looked == 1, "nor one naming entries not here yet, which waits for them");
        r.take_first(2, None, 0).unwrap();
        let other_rules = "DELETE FROM relations WHERE rowid = (SELECT max(rowid) FROM relations WHERE substr(subject, 1, 1) IN ('#', '@'))";
        r.take_first(3, Some(&freeze(3, &now, other_rules)), 0).unwrap();
        assert_eq!(r.at(), Some(3), "frozen, at its freeze");
        let rules = steps::rules_of(r.db().conn()).unwrap();
        assert_eq!(rules, steps::rules_of(&{
            let db = steps::connection(&now.1).unwrap();
            db.execute_batch(other_rules).unwrap();
            db
        }).unwrap(), "with the freezer's rules, not this release's");
        assert!(r.take_step(4, Some(&freeze(4, &now, "").step), 0).is_ok() && r.at() == Some(4) && steps::rules_of(r.db().conn()).unwrap() == rules, "a second freeze counts for nothing");
        // The old laptop, still on a build that writes the log, makes Mal an admin: nothing changes.
        let late = Entry::create(&old, ext(ANDRE), None, device(&Endpoint::of(&SigningKey::from_bytes(&[3; 32])), "c000000000000003", "mal's", &[Kind::Client], Some(("Mal", Role::Admin))));
        r.receive(vec![late], 0).unwrap();
        assert!(r.db().user("c000000000000003").is_none(), "an entry after the freeze changes nothing");
        // A step does.
        let step = r.make_step(&Scope::Global, ANDRE, setting("greeting", Some("hi")), 0).unwrap();
        r.take_step(5, Some(&step), 0).unwrap();
        assert_eq!(r.db().setting("greeting").as_deref(), Some("hi"));
        assert!(r.take_step(5, Some(&step), 0).is_err(), "a position is taken once");
        // Kept, and started again from: the same who's who, its rules too; one changed since isn't used.
        let kept = r.keep().unwrap().unwrap();
        let mut again = Replica::new(mac.clone(), Store::memory().unwrap(), Some(g.hash()), 0).unwrap();
        again.restart(&kept, 0).unwrap();
        assert_eq!((again.at(), again.db().setting("greeting"), steps::rules_of(again.db().conn()).unwrap()), (Some(5), Some("hi".into()), rules));
        let mut changed = kept.clone();
        changed["hash"] = serde_json::json!("0".repeat(64));
        assert!(again.restart(&changed, 0).is_err());
    }
}
