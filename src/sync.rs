//! Sync: what two servers say to each other, and which scope goes where. See SPEC.md, "Sync".
//! The protocol has no clock and no network of its own; `node.rs` carries it over iroh and
//! `tests/sim.rs` over a simulated network.

use anyhow::{Result, bail, ensure};
use p2panda_core::{Hash, SigningKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::entry::{Content, Entry, Ext, Key, Scope};
use crate::members::{self, Members};
use crate::store::{Frontier, Store};

pub const ALPN: &[u8] = b"one/sync/1";
const MAX_MESSAGE: usize = 16 << 20;
/// Missing entries go in batches this big, so no message nears MAX_MESSAGE.
const BATCH: usize = 200;
/// Entries timed further ahead than this are refused: a wrong clock mustn't drag everyone's forward.
pub const FUTURE: u64 = 3_600_000;

pub type Wire = (Vec<u8>, Vec<u8>);

/// A driver serving on the sender: its name, the users it serves, and what it does (its operations), which callers
/// list until apps wrap drivers.
pub type Driver = (String, Vec<String>, Value);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Msg {
    Hello { frontier: Frontier, drivers: Vec<Driver> },
    Entries { entries: Vec<Wire> },
}

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
    /// The server whose pairing code this one joined with: it may sync before this one sees itself admitted.
    pub paired_by: Option<Key>,
    /// Whether an entry arrived that sorts before the newest of `member_entries`, so the members need replaying.
    stale: bool,
}

/// What handling a message did.
#[derive(Default)]
pub struct Handled {
    /// Entries new to this server.
    pub new: Vec<Entry>,
    /// What to send back.
    pub replies: Vec<Msg>,
    /// Whether the members changed.
    pub members: bool,
    /// Scopes this server stopped holding, and deleted.
    pub dropped: Vec<String>,
    /// Whether this server now holds a scope it didn't: it should say hello to every peer, so they send it.
    pub gained: bool,
    /// For each peer that now holds scopes this server has entries in and it didn't before: those scopes, whose
    /// entries it lacks this server should send.
    pub peers_gained: Vec<(Key, Vec<String>)>,
}

impl Replica {
    pub fn new(key: SigningKey, store: Store, root: Option<Hash>, paired_by: Option<Key>) -> Result<Replica> {
        let id = *key.verifying_key().as_bytes();
        let mut member_entries: Vec<Entry> = store.scope("global")?.into_iter().filter(members::is_member_entry).collect();
        member_entries.sort_by_key(Entry::order);
        let members = Members::replay(root, &member_entries);
        Ok(Replica { key, id, store, root, members, member_entries, paired_by, stale: false })
    }

    /// Whether this server holds `scope`. Until it sees itself admitted it holds only `global`.
    pub fn holds(&self, scope: &Scope) -> bool {
        *scope == Scope::Global || self.members.holds(&self.id, scope)
    }

    fn holds_log(&self, log: &str) -> bool {
        Scope::parse(log).is_ok_and(|s| self.holds(&s))
    }

    /// Whether `peer` holds the scope of `log`, as far as this server knows.
    pub fn peer_holds(&self, peer: &Key, log: &str) -> bool {
        Scope::parse(log).is_ok_and(|s| s == Scope::Global || self.members.holds(peer, &s))
    }

    /// Whether this server syncs with `peer`: a current server, or, while joining, the one it joined with.
    pub fn may_sync(&self, peer: &Key) -> bool {
        self.members.server(peer).is_some() || (self.members.server(&self.id).is_none() && self.paired_by == Some(*peer))
    }

    pub fn hello(&self, drivers: Vec<Driver>) -> Result<Msg> {
        Ok(Msg::Hello { frontier: self.store.frontier()?, drivers })
    }

    /// The message that passes `entries` on to `peer`: those in scopes it holds.
    pub fn for_peer(&self, peer: &Key, entries: &[Entry]) -> Option<Msg> {
        let wanted: Vec<Entry> = entries.iter().filter(|e| self.peer_holds(peer, e.log())).cloned().collect();
        (!wanted.is_empty()).then(|| entries_msg(&wanted))
    }

    /// Every entry `peer` lacks, given its frontier, in batches.
    pub fn missing_for(&self, peer: &Key, frontier: &Frontier) -> Result<Vec<Msg>> {
        self.missing_in(peer, frontier, |_| true)
    }

    /// Every entry `peer` lacks in the scopes `wanted` names, in batches.
    pub fn missing_in(&self, peer: &Key, frontier: &Frontier, wanted: impl Fn(&str) -> bool) -> Result<Vec<Msg>> {
        let missing = self.store.missing(frontier, |log| wanted(log) && self.peer_holds(peer, log))?;
        Ok(missing.chunks(BATCH).map(entries_msg).collect())
    }

    /// Writes an entry to this server's log for `scope`, timed after everything it has seen.
    pub fn append(&mut self, scope: &Scope, user: &str, by: Option<Key>, content: Content, now: u64) -> Result<(Entry, Handled)> {
        ensure!(self.holds(scope), "this server doesn't hold {scope}");
        let log = scope.to_string();
        let head = self.store.own_head(&self.id, &log)?;
        let held = self.store.last(&self.id, &log)?.map(|e| e.seq());
        // A server that holds a scope again writes there only once it has its own log back, so no seq is written twice.
        ensure!(head.map(|h| h.0) == held, "this server is still getting its own entries in {scope} back");
        let time = now.max(self.store.max_time()? + 1);
        let entry = Entry::create(&self.key, Ext { log, time, user: user.into(), by }, head, content);
        self.store.insert_own(&entry, head)?;
        let mut handled = Handled::default();
        if members::is_member_entry(&entry) {
            let before = self.members.clone();
            self.add_member_entry(entry.clone());
            self.refresh(&before, &mut handled)?;
        }
        handled.new.push(entry.clone());
        Ok((entry, handled))
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
        for server in m.servers().filter(|s| s.key != me) {
            let gained: Vec<String> = logs
                .iter()
                .filter(|l| self.holds_log(l))
                .filter(|l| Scope::parse(l).is_ok_and(|s| m.holds(&server.key, &s) && !before.holds(&server.key, &s)))
                .cloned()
                .collect();
            if !gained.is_empty() {
                match handled.peers_gained.iter_mut().find(|(k, _)| *k == server.key) {
                    Some((_, logs)) => logs.extend(gained),
                    None => handled.peers_gained.push((server.key, gained)),
                }
            }
        }
        Ok(())
    }

    /// Handles one message from `peer` at time `now`.
    pub fn handle(&mut self, peer: &Key, msg: Msg, now: u64) -> Result<Handled> {
        if !self.may_sync(peer) {
            bail!("{} isn't a server of this install", crate::entry::short(peer));
        }
        let mut handled = Handled::default();
        match msg {
            Msg::Hello { frontier, .. } => handled.replies = self.missing_for(peer, &frontier)?,
            Msg::Entries { entries } => {
                let mut decoded: Vec<Entry> = entries
                    .iter()
                    // Checking a signature costs more than looking: skip entries this server has.
                    .filter(|(h, b)| !Entry::decode_trusted(h, b).is_ok_and(|e| self.store.has(&e.author(), e.log(), e.seq()).unwrap_or(false)))
                    .filter_map(|(h, b)| match Entry::decode(h, b) {
                        Ok(e) if e.time() <= now.saturating_add(FUTURE) => Some(e),
                        Ok(_) => {
                            tracing::debug!("dropped an entry from the future");
                            None
                        }
                        Err(err) => {
                            tracing::debug!("dropped entry: {err:#}");
                            None
                        }
                    })
                    .collect();
                // `global` first, since it decides which other scopes this server holds.
                decoded.sort_by_key(|e| e.log() != "global");
                let (mut gap, mut before) = (false, None);
                for entry in decoded {
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
                        Err(_) if self.store.lacks_predecessor(&entry)? => gap = true,
                        Err(err) => tracing::debug!("dropped entry: {err:#}"),
                    }
                }
                if let Some(before) = &before {
                    if !handled.members {
                        self.refresh(before, &mut handled)?;
                    }
                    handled.new.retain(|e| self.holds_log(e.log()));
                }
                // An entry arrived before the one it follows: say what this server has, so the sender fills the gap.
                if gap {
                    handled.replies.push(self.hello(Vec::new())?);
                }
            }
        }
        Ok(handled)
    }
}

fn entries_msg(entries: &[Entry]) -> Msg {
    Msg::Entries { entries: entries.iter().map(Entry::encode).collect() }
}

pub async fn write<W: AsyncWrite + Unpin>(w: &mut W, msg: &Msg) -> Result<()> {
    let mut bytes = Vec::new();
    ciborium::into_writer(msg, &mut bytes)?;
    w.write_u32(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read<R: AsyncRead + Unpin>(r: &mut R) -> Result<Msg> {
    let len = r.read_u32().await? as usize;
    ensure!(len <= MAX_MESSAGE, "message too large");
    let mut bytes = vec![0; len];
    r.read_exact(&mut bytes).await?;
    Ok(ciborium::from_reader(bytes.as_slice())?)
}
