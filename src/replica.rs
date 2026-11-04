//! What one device holds and knows: the entries of each scope it holds, as the install's bucket has them, and the
//! members `global`'s add up to. No clock or I/O: `node.rs` reads and writes the bucket.

use anyhow::{Context, Result, ensure};
use ed25519_dalek::SigningKey;

use crate::entry::{Content, Entry, Ext, Hash, Key, Scope};
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
    /// `global`'s node entries, in replay order.
    pub member_entries: Vec<Entry>,
    /// Whether an entry arrived that sorts before the newest of `member_entries`, so the members need replaying.
    stale: bool,
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
    pub fn new(key: SigningKey, store: Store, root: Option<Hash>) -> Result<Replica> {
        let id = key.verifying_key().to_bytes();
        let member_entries: Vec<Entry> = store.scope("global")?.into_iter().filter(members::is_member_entry).collect();
        let members = Members::replay(root, &member_entries, &store.forks()?);
        Ok(Replica { key, id, store, root, members, member_entries, stale: false })
    }

    /// Whether this device holds `scope`: `global`, and each scope whose calls it runs (`Members::runs_for`). Until it
    /// sees itself listed it holds only `global`.
    pub fn holds(&self, scope: &Scope) -> bool {
        *scope == Scope::Global || self.members.runs_for(&self.id, scope)
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
        let ext = Ext { log, time: now, user: user.into(), epoch: self.members.epoch(scope), previous };
        Ok(Entry::create(&self.key, ext, head, content))
    }

    /// Keeps an entry this device made, once the bucket has it.
    pub fn add_own(&mut self, entry: &Entry) -> Result<Handled> {
        let head = self.store.own_head(&self.id, entry.log())?;
        let entry = self.store.insert_own(entry, head)?;
        let mut handled = Handled::default();
        if members::is_member_entry(&entry) {
            let before = self.members.clone();
            self.add_member_entry(entry.clone());
            self.refresh(&before, &mut handled)?;
        }
        handled.new.push(entry);
        Ok(handled)
    }

    /// Whether an entry would count, written now: why not, if it wouldn't. A member entry only when one of its
    /// user's own devices signs it.
    pub fn check(&self, entry: &Entry) -> Result<()> {
        let signer = self.members.machine(&entry.author()).map_or("that device".to_string(), |m| m.name.clone());
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

    /// Replays the members again, deletes every scope this device no longer holds, and says what changed since `before`.
    pub fn refresh(&mut self, before: &Members, handled: &mut Handled) -> Result<()> {
        if std::mem::take(&mut self.stale) {
            self.members = Members::replay(self.root, &self.member_entries, &self.store.forks()?);
        }
        handled.members = true;
        handled.recount |= before.cuts != self.members.cuts;
        let logs = self.store.logs()?;
        for log in &logs {
            if !self.holds_log(log) {
                self.store.delete(log)?;
                handled.dropped.push(log.clone());
            }
        }
        let (me, m) = (self.id, &self.members);
        handled.gained |= m.scopes().iter().any(|s| m.runs_for(&me, s) && !before.runs_for(&me, s));
        let after = m.roles();
        for ((user, scope), (_, invited)) in before.roles() {
            if !invited && !after.get(&(user.clone(), scope)).is_some_and(|(_, invited)| !invited) && !handled.lost.contains(&user) {
                handled.lost.push(user);
            }
        }
        Ok(())
    }

    /// Takes entries as the bucket gives them: each checked (signature, body, chain) and kept once everything it
    /// follows is here, in scopes this device holds; `global`'s first, since they decide which other scopes it holds.
    pub fn receive(&mut self, mut entries: Vec<Entry>) -> Result<Handled> {
        let mut handled = Handled::default();
        entries.retain(|e| self.holds_log(e.log()));
        let (global, rest): (Vec<Entry>, Vec<Entry>) = entries.into_iter().partition(|e| e.log() == "global");
        self.took(global, &mut handled)?;
        let rest = rest.into_iter().filter(|e| self.holds_log(e.log())).collect();
        self.took(rest, &mut handled)?;
        handled.new.retain(|e| self.holds_log(e.log()));
        Ok(handled)
    }

    /// Keeps entries, with any that waited for them, and follows the members if they changed.
    fn took(&mut self, entries: Vec<Entry>, handled: &mut Handled) -> Result<()> {
        let before = self.members.clone();
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
        if changed {
            self.refresh(&before, handled)?;
        }
        Ok(())
    }
}
