//! What one server holds and knows: the entries of each scope it holds, as the install's bucket has them, and the
//! members `global`'s add up to. No clock or I/O: `node.rs` reads and writes the bucket.

use anyhow::{Result, ensure};
use p2panda_core::{Hash, SigningKey};

use crate::entry::{Content, Entry, Ext, Key, Scope};
use crate::members::{self, Members};
use crate::store::Store;

/// Entries timed further ahead than this are refused: a wrong clock mustn't drag everyone's forward.
pub const FUTURE: u64 = 3_600_000;

/// What one server holds and knows: its store, and the members its `global` adds up to.
pub struct Replica {
    pub key: SigningKey,
    pub id: Key,
    pub store: Store,
    /// The install's genesis, once this server created or joined it.
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
    /// Entries new to this server.
    pub new: Vec<Entry>,
    /// Whether the members changed.
    pub members: bool,
    /// Scopes this server stopped holding, and deleted.
    pub dropped: Vec<String>,
    /// Whether this server now holds a scope it didn't, whose entries it should read.
    pub gained: bool,
    /// People who may no longer use a scope they could, whose sessions this server forgets.
    pub lost: Vec<String>,
}

impl Replica {
    pub fn new(key: SigningKey, store: Store, root: Option<Hash>) -> Result<Replica> {
        let id = *key.verifying_key().as_bytes();
        let mut member_entries: Vec<Entry> = store.scope("global")?.into_iter().filter(members::is_member_entry).collect();
        member_entries.sort_by_key(Entry::order);
        let members = Members::replay(root, &member_entries);
        Ok(Replica { key, id, store, root, members, member_entries, stale: false })
    }

    /// Whether this server holds `scope`. Until it sees itself admitted it holds only `global`.
    pub fn holds(&self, scope: &Scope) -> bool {
        *scope == Scope::Global || self.members.holds(&self.id, scope)
    }

    fn holds_log(&self, log: &str) -> bool {
        Scope::parse(log).is_ok_and(|s| self.holds(&s))
    }

    /// Makes an entry for this server's log of `scope`, timed after everything it has seen, to write to the bucket.
    pub fn make(&self, scope: &Scope, user: &str, by: Option<Key>, content: Content, now: u64) -> Result<Entry> {
        ensure!(self.holds(scope), "this device doesn't hold {scope}");
        let log = scope.to_string();
        let head = self.store.own_head(&self.id, &log)?;
        let held = self.store.last(&self.id, &log)?.map(|e| e.seq());
        // A server that holds a scope again writes there only once it has its own log back, so no seq is written twice.
        ensure!(head.map(|h| h.0) == held, "this device is still getting its own entries in {scope} back");
        let time = now.max(self.store.max_time()? + 1);
        Ok(Entry::create(&self.key, Ext { log, time, user: user.into(), by }, head, content))
    }

    /// Keeps an entry this server made, once the bucket has it.
    pub fn add_own(&mut self, entry: &Entry) -> Result<Handled> {
        let head = self.store.own_head(&self.id, entry.log())?;
        self.store.insert_own(entry, head)?;
        let mut handled = Handled::default();
        if members::is_member_entry(entry) {
            let before = self.members.clone();
            self.add_member_entry(entry.clone());
            self.refresh(&before, &mut handled)?;
        }
        handled.new.push(entry.clone());
        Ok(handled)
    }

    /// Adds one of `global`'s node entries: replayed onto the members if it's the newest, else replayed with all.
    /// Whether one of `global`'s node entries would count if this server wrote it now.
    pub fn would_count(&self, user: &str, by: Option<Key>, content: Content, now: u64) -> Result<bool> {
        let head = self.store.own_head(&self.id, "global")?;
        let time = now.max(self.store.max_time()? + 1);
        let entry = Entry::create(&self.key, Ext { log: "global".into(), time, user: user.into(), by }, head, content);
        Ok(self.members.clone().step(&entry))
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

    /// Replays the members again, deletes every scope this server no longer holds, and says what changed since `before`.
    pub fn refresh(&mut self, before: &Members, handled: &mut Handled) -> Result<()> {
        if std::mem::take(&mut self.stale) {
            self.members = Members::replay(self.root, &self.member_entries);
        }
        handled.members = true;
        let logs = self.store.logs()?;
        for log in &logs {
            if !self.holds_log(log) {
                self.store.delete(log)?;
                handled.dropped.push(log.clone());
            }
        }
        let (me, m) = (self.id, &self.members);
        handled.gained |= m.scopes().iter().any(|s| m.holds(&me, s) && !before.holds(&me, s));
        let after = m.roles();
        for ((user, scope), (_, invited)) in before.roles() {
            if !invited && !after.get(&(user.clone(), scope)).is_some_and(|(_, invited)| !invited) && !handled.lost.contains(&user) {
                handled.lost.push(user);
            }
        }
        Ok(())
    }

    /// Takes entries as the bucket gives them, at time `now`: each checked (signature, body, chain), in scopes this
    /// server holds; `global`'s first, since they decide which other scopes it holds.
    pub fn receive(&mut self, mut entries: Vec<Entry>, now: u64) -> Result<Handled> {
        let mut handled = Handled::default();
        entries.retain(|e| e.time() <= now.saturating_add(FUTURE));
        entries.sort_by_key(|e| e.log() != "global");
        let mut before = None;
        for entry in entries {
            if entry.log() != "global" && !handled.members {
                // Replay the members once, after the batch's `global` entries and before the rest.
                if let Some(before) = &before {
                    self.refresh(before, &mut handled)?;
                }
            }
            if !self.holds_log(entry.log()) {
                continue;
            }
            match self.store.insert(&entry) {
                Ok(true) => {
                    if members::is_member_entry(&entry) {
                        before.get_or_insert_with(|| self.members.clone());
                        self.add_member_entry(entry.clone());
                    }
                    handled.new.push(entry);
                }
                Ok(false) => {}
                Err(err) => tracing::debug!("dropped entry: {err:#}"),
            }
        }
        if let Some(before) = &before {
            if !handled.members {
                self.refresh(before, &mut handled)?;
            }
            handled.new.retain(|e| self.holds_log(e.log()));
        }
        Ok(handled)
    }
}
