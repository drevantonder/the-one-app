//! Who's who as the install's own signed entries say it, checked on the device with the nodes' own Rust (`one`'s
//! entry checks and member replay), never as a node says it: a node holds no keys, and a hacked one could list a seal
//! key of its own as someone's device, or itself as where their things run, to be sealed or handed keys. What a device
//! seals, and to whom it hands keys, it takes only from here.
//!
//! A device keeps `global`'s entries it has checked and the install's root, the genesis it saw when it joined, and
//! refuses entries of an install with another; then it asks a node only for entries it doesn't have
//! (`GET /v1/entries/global?have=`, `have`).
//!
//! What it asks of them it asks `Who`: `global`'s scope database, built from that replay (`one::database::scope`), and
//! `one::permissions::may` over it, the same on every client: native apps ask it directly, browsers and the MCP
//! Worker's devices as JSON (`Who::view`). Replay stays the truth, and this only its answers.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use one::database::scope::{self, device, object, person};
use one::entry::{Entry, Hash, Key, Scope, causal, hex, pack, unpack};
use one::keys::unhex;
use one::members::{Cuts, Kind, Members, ScopeState, is_member_entry};
use one::permissions::{self, INSTALL};

/// `global`'s entries a device holds, checked, and the install they're of.
pub struct Global {
    root: Option<Hash>,
    entries: BTreeMap<Hash, Entry>,
}

impl Global {
    /// For the install whose genesis is `root`, or, for a device that's only now joined, whichever one genesis the
    /// first entries it's given hold.
    pub fn new(root: Option<Hash>) -> Global {
        Global { root, entries: BTreeMap::new() }
    }

    /// The install's genesis, once known.
    pub fn root(&self) -> Option<Hash> {
        self.root
    }

    /// Adds a pack's entries (`entry::pack`), each checked as from anyone: signature, body and content.
    pub fn add(&mut self, packed: &[u8]) -> Result<()> {
        self.add_entries(unpack(packed)?)
    }

    /// Adds entries already checked as from anyone (`Entry::decode`): refused, all of them, if one isn't of `global`,
    /// or starts another install than this one.
    pub fn add_entries(&mut self, entries: Vec<Entry>) -> Result<()> {
        for e in &entries {
            ensure!(e.scope() == Scope::Global, "a node gave an entry of {} as one of everyone's", e.log());
        }
        let geneses: Vec<Hash> = entries.iter().chain(self.entries.values()).filter(|e| e.content.app == one::members::APP && e.content.kind == "genesis").map(Entry::hash).collect();
        let root = match self.root {
            Some(root) => root,
            None => match geneses.as_slice() {
                [only, rest @ ..] if rest.iter().all(|g| g == only) => *only,
                [] => bail!("those aren't any install's entries: none starts it"),
                _ => bail!("those entries start more than one install"),
            },
        };
        ensure!(geneses.iter().all(|g| *g == root), "those entries are another install's, not your One App's");
        self.root = Some(root);
        for e in entries {
            self.entries.entry(e.hash()).or_insert(e);
        }
        Ok(())
    }

    /// Every entry held, packed, to keep.
    pub fn pack(&self) -> Vec<u8> {
        pack(&self.entries.values().collect::<Vec<_>>())
    }

    /// What to ask a node for only what's new: `<key>.<seq>,…`, the next seq of each author's log held.
    pub fn have(&self) -> String {
        let mut next: BTreeMap<Key, u32> = BTreeMap::new();
        for e in self.entries.values() {
            let n = next.entry(e.author()).or_default();
            *n = (*n).max(e.seq() + 1);
        }
        next.iter().map(|(k, n)| format!("{}.{n}", hex(k))).collect::<Vec<_>>().join(",")
    }

    /// The members, replayed as a node replays them: in causal order, each entry counting only as `one::members`
    /// says, a device that signed two entries at one seq cut from there and removed. Refused if they aren't the
    /// install's, or its genesis isn't among them.
    pub fn members(&self) -> Result<Members> {
        let Some(root) = self.root else { bail!("no entries of your One App yet") };
        let mut seen: HashMap<(Key, u32), Hash> = HashMap::new();
        let mut forks = Cuts::new();
        for e in self.entries.values() {
            if seen.insert((e.author(), e.seq()), e.hash()).is_some_and(|h| h != e.hash()) {
                let at = forks.entry((e.author(), "global".into())).or_insert(u32::MAX);
                *at = (*at).min(e.seq());
            }
        }
        let ordered: Vec<Entry> = causal(self.entries.values().cloned().collect(), &HashMap::new()).into_iter().filter(is_member_entry).collect();
        let m = Members::replay(Some(root), &ordered, &forks);
        ensure!(!m.users.is_empty(), "your One App's first entry isn't among those");
        Ok(m)
    }
}

/// A device of the install, as it's listed: its key, the key things are sealed to, whose it is, its name, what it does,
/// and for a passkey its credential, and the device it's on.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub key: Key,
    pub seal: Key,
    pub user: String,
    pub name: String,
    pub kinds: Vec<Kind>,
    pub passkey: Option<String>,
    pub device: Option<Key>,
}

/// Who's who, as `global`'s scope database built from a checked replay says it, and who may do what asked of `may`
/// over it, now: whom a device seals each scope's keys to, which nodes it hands them, which epochs it takes, and what
/// its person may approve and remove.
pub struct Who {
    db: Connection,
    now: u64,
}

/// A person's id, from how the scope database names them.
fn id(principal: &str) -> String {
    principal.strip_prefix("person:").unwrap_or(principal).to_string()
}

fn key(hex: &str) -> Result<Key> {
    unhex(hex)
}

impl Who {
    /// From replay's members: `global`'s scope database, as every reader builds it.
    pub fn of(m: &Members) -> Result<Who> {
        let now = n0_future::time::SystemTime::now().duration_since(n0_future::time::SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        Ok(Who { db: scope::build(m, &Scope::Global, &ScopeState::default())?, now })
    }

    fn may(&self, principal: &str, permission: &str, object: &str) -> Result<bool> {
        permissions::may(&self.db, principal, permission, object, self.now)
    }

    /// The install's genesis, hex, and its name.
    pub fn install(&self) -> Result<(Option<String>, String)> {
        Ok(self.db.query_row("SELECT root, name FROM install", [], |r| Ok((r.get(0)?, r.get(1)?)))?)
    }

    /// The install's recovery key, which each epoch is sealed to besides devices.
    pub fn recovery(&self) -> Result<Option<Key>> {
        let k: Option<String> = self.db.query_row("SELECT key FROM recovery WHERE set_at IS NOT NULL ORDER BY n DESC LIMIT 1", [], |r| r.get(0)).optional()?;
        k.as_deref().map(key).transpose()
    }

    /// Each device listed now, its person still in, in the order of their keys; or the one of `only`.
    fn listed(&self, only: Option<&Key>) -> Result<Vec<Listed>> {
        let mut q = self.db.prepare(
            "SELECT d.key, d.seal, d.principal, d.name, d.client, d.compute, d.service, d.passkey, d.device FROM devices d
             JOIN principals p ON p.id = d.principal WHERE d.removed = 0 AND p.removed = 0 AND (?1 IS NULL OR d.key = ?1) ORDER BY d.key",
        )?;
        let rows = q.query_map([only.map(hex)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get(3)?, r.get::<_, bool>(4)?, r.get::<_, bool>(5)?, r.get::<_, Option<String>>(6)?, r.get(7)?, r.get::<_, Option<String>>(8)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (k, seal, user, name, client, compute, service, passkey, on) = row?;
            let kinds = [(client, Kind::Client), (compute, Kind::Compute), (service.is_some(), Kind::Service)].into_iter().filter(|(is, _)| *is).map(|(_, k)| k).collect();
            out.push(Listed { key: key(&k)?, seal: key(&seal)?, user: id(&user), name, kinds, passkey, device: on.as_deref().map(key).transpose()? });
        }
        Ok(out)
    }

    /// Each device listed now.
    pub fn devices(&self) -> Result<Vec<Listed>> {
        self.listed(None)
    }

    /// A device listed now.
    pub fn device(&self, key: &Key) -> Result<Option<Listed>> {
        Ok(self.listed(Some(key))?.pop())
    }

    /// The serverless node's endpoint, as an admin's `serverless` setting names it.
    pub fn serverless(&self) -> Result<Option<Key>> {
        let k: Option<String> = self.db.query_row("SELECT value FROM settings WHERE name = 'serverless'", [], |r| r.get(0)).optional()?;
        Ok(k.and_then(|k| key(&k).ok()))
    }

    /// The nodes that run calls, each with the key what's handed it is sealed to: the devices listed as compute, and the
    /// serverless node, whose seal key is its key's twin.
    pub fn nodes(&self) -> Result<Vec<(Key, Key)>> {
        let mut out: Vec<(Key, Key)> = self.devices()?.into_iter().filter(|d| d.kinds.contains(&Kind::Compute)).map(|d| (d.key, d.seal)).collect();
        if let Some(k) = self.serverless()?
            && let Ok(seal) = one::kernel::twin_public(&k)
        {
            out.push((k, seal));
        }
        Ok(out)
    }

    /// The people in the install.
    pub fn people(&self) -> Result<Vec<String>> {
        let mut q = self.db.prepare("SELECT id FROM principals WHERE kind = 'person' AND removed = 0 ORDER BY id")?;
        Ok(q.query_map([], |r| r.get::<_, String>(0))?.map(|p| p.map(|p| id(&p))).collect::<rusqlite::Result<_>>()?)
    }

    /// Every scope there is: `global`, each group's, and each person's (those taken out too: removing deletes nothing).
    pub fn scopes(&self) -> Result<Vec<Scope>> {
        let mut q = self.db.prepare("SELECT kind, id FROM principals ORDER BY kind = 'person', id")?;
        let mut out = vec![Scope::Global];
        for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (kind, p) = row?;
            out.push(match kind.as_str() {
                "group" => Scope::Group(p.strip_prefix("group:").unwrap_or(&p).to_string()),
                _ => Scope::User(id(&p)),
            });
        }
        Ok(out)
    }

    /// Whether `user` opens a scope: everyone's, their own, and their groups'.
    pub fn uses(&self, user: &str, scope: &Scope) -> Result<bool> {
        self.may(&person(user), "use", &object(scope))
    }

    /// `user`'s role in a scope, as the role they hold on it: in the install for `global`, in the group, and `admin` of
    /// their own.
    pub fn role_in(&self, user: &str, scope: &Scope) -> Result<Option<&'static str>> {
        let mut q = self.db.prepare("SELECT relation FROM relations WHERE subject = ?1 AND object = ?2 AND (until IS NULL OR until > ?3)")?;
        let held = q.query_map(params![person(user), object(scope), self.now as i64], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(match () {
            _ if held.iter().any(|r| r == "admin" || r == "owner") => Some("admin"),
            _ if held.iter().any(|r| r == "member") => Some("member"),
            _ => None,
        })
    }

    /// Whether `user` starts a scope's new epochs: its admins (everyone of their own).
    pub fn starts(&self, user: &str, scope: &Scope) -> Result<bool> {
        Ok(self.role_in(user, scope)? == Some("admin"))
    }

    /// What a scope's epochs are sealed to: the client devices of the people who open it, nothing that only runs calls.
    pub fn openers(&self, scope: &Scope) -> Result<Vec<(Key, Key)>> {
        let mut uses = HashMap::new();
        let mut out = Vec::new();
        for d in self.devices()?.into_iter().filter(|d| d.kinds.contains(&Kind::Client)) {
            let opens = match uses.get(&d.user) {
                Some(o) => *o,
                None => *uses.entry(d.user.clone()).or_insert(self.uses(&d.user, scope)?),
            };
            if opens {
                out.push((d.key, d.seal));
            }
        }
        Ok(out)
    }

    /// Whether a scope's calls run on `node`, so it may be handed the scope's keys.
    pub fn runs_for(&self, node: &Key, scope: &Scope) -> Result<bool> {
        self.may(&device(node), "run", &object(scope))
    }

    /// Whether `node` runs any of `user`'s calls: some scope they open.
    pub fn serves(&self, node: &Key, user: &str) -> Result<bool> {
        for s in self.scopes()? {
            if self.uses(user, &s)? && self.runs_for(node, &s)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Where `user`'s own things run besides their own machines, as their own device chose, if it did: nowhere else is
    /// a choice too.
    pub fn runs(&self, user: &str) -> Result<Option<Vec<Key>>> {
        if self.db.query_row("SELECT 1 FROM runs_chosen WHERE principal = ?", [person(user)], |_| Ok(())).optional()?.is_none() {
            return Ok(None);
        }
        let mut q = self.db.prepare("SELECT device FROM runs WHERE principal = ? ORDER BY device")?;
        let on = q.query_map([person(user)], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(on.iter().map(|k| key(k)).collect::<Result<_>>()?))
    }

    /// Where `user`'s own things run besides their own machines until they choose (none once they have): the machines
    /// of others' that run everyone's.
    pub fn runs_until_chosen(&self, user: &str) -> Result<Vec<Key>> {
        if self.runs(user)?.is_some() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for d in self.devices()?.into_iter().filter(|d| d.kinds.contains(&Kind::Compute) && d.user != user) {
            if self.runs_for(&d.key, &Scope::Global)? {
                out.push(d.key);
            }
        }
        Ok(out)
    }

    /// A scope's newest epoch: 0 while it has none.
    pub fn epoch(&self, scope: &Scope) -> Result<u32> {
        Ok(self.db.query_row("SELECT coalesce(max(epoch), 0) FROM epochs WHERE scope = ?", [scope.to_string()], |r| r.get(0))?)
    }

    /// The hash of a scope's epoch `n`, which its key must have to be used.
    pub fn epoch_hash(&self, scope: &Scope, n: u32) -> Result<Option<String>> {
        Ok(self.db.query_row("SELECT key_hash FROM epochs WHERE scope = ? AND epoch = ?", params![scope.to_string(), n], |r| r.get(0)).optional()?)
    }

    /// Whether `user` lets in a new device of `of`'s: their own.
    pub fn approves(&self, user: &str, of: &str) -> Result<bool> {
        self.may(&person(user), "add-devices", &person(of))
    }

    /// Whether `user` takes a device out: their own, or as an admin, anyone's.
    pub fn removes(&self, user: &str, key: &Key) -> Result<bool> {
        self.may(&person(user), "remove", &device(key))
    }

    /// Whether `user` invites someone new into the install, and lets their first device in.
    pub fn invites(&self, user: &str) -> Result<bool> {
        self.may(&person(user), "invite", INSTALL)
    }

    /// Whether `user` takes someone out of the install.
    pub fn removes_people(&self, user: &str) -> Result<bool> {
        self.may(&person(user), "remove", INSTALL)
    }

    /// Its answers as JSON, for a browser to keep and look up (`kernel/web/device.js`, `net-worker.js`):
    ///
    /// - `root`, `name`, `recovery` (hex, what each epoch is sealed to besides devices)
    /// - `devices`: each listed now, `{ key, seal, user, name, roles, passkey, device }`, and the serverless node as
    ///   `{ key, seal, roles: ["compute"] }`
    /// - `openers`: each scope's `{ key, seal }` its epochs are sealed to
    /// - `runs_for`: each node's scopes whose calls it runs: the only nodes handed a scope's keys
    /// - `epochs`: each scope's newest epoch `n` and every epoch's hash
    /// - `people`: each person's `{ uses, starts }` (scopes they open, and whose new epochs they start), `nodes` (those
    ///   that run their calls), `runs_until_chosen`, `approves` (whose new devices they let in), `invites`,
    ///   `removes_people` and `removes` (the devices they may take out)
    pub fn view(&self) -> Result<Value> {
        let (root, name) = self.install()?;
        let scopes = self.scopes()?;
        let listed = self.devices()?;
        let mut devices: Vec<Value> = listed.iter().map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal), "user": d.user, "name": d.name, "roles": d.kinds, "passkey": d.passkey, "device": d.device.as_ref().map(hex) })).collect();
        if let Some(k) = self.serverless()?
            && let Ok(seal) = one::kernel::twin_public(&k)
        {
            devices.push(json!({ "key": hex(&k), "seal": hex(&seal), "roles": ["compute"] }));
        }
        let people = self.people()?;
        let mut uses: BTreeMap<&str, Vec<&Scope>> = BTreeMap::new();
        for u in &people {
            let mine = uses.entry(u.as_str()).or_default();
            for s in &scopes {
                if self.uses(u, s)? {
                    mine.push(s);
                }
            }
        }
        // A scope someone opens, with the client devices of everyone who does.
        let mut openers = Map::new();
        for s in &scopes {
            if uses.values().any(|on| on.contains(&s)) {
                let to: Vec<Value> = listed.iter().filter(|d| d.kinds.contains(&Kind::Client) && uses.get(d.user.as_str()).is_some_and(|on| on.contains(&s))).map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal) })).collect();
                openers.insert(s.to_string(), json!(to));
            }
        }
        let nodes = self.nodes()?;
        let mut runs_for = Map::new();
        for (k, _) in &nodes {
            let mut on = Vec::new();
            for s in &scopes {
                if self.runs_for(k, s)? {
                    on.push(s.to_string());
                }
            }
            runs_for.insert(hex(k), json!(on));
        }
        let mut epochs = Map::new();
        let mut q = self.db.prepare("SELECT scope, epoch, key_hash FROM epochs ORDER BY scope, epoch")?;
        for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, r.get::<_, String>(2)?)))? {
            let (scope, n, hash) = row?;
            let e = epochs.entry(scope).or_insert_with(|| json!({ "n": 0, "hashes": {} }));
            e["n"] = json!(n.max(e["n"].as_u64().unwrap_or(0) as u32));
            e["hashes"][n.to_string()] = json!(hash);
        }
        let mut answers = Map::new();
        for u in &people {
            let mut starts = Vec::new();
            for s in &scopes {
                if self.starts(u, s)? {
                    starts.push(s.to_string());
                }
            }
            let mut serves = Vec::new();
            for (k, _) in &nodes {
                if self.serves(k, u)? {
                    serves.push(hex(k));
                }
            }
            let mut approves = Vec::new();
            for of in &people {
                if self.approves(u, of)? {
                    approves.push(of);
                }
            }
            let mut removes = Vec::new();
            for d in &listed {
                if self.removes(u, &d.key)? {
                    removes.push(hex(&d.key));
                }
            }
            answers.insert(
                u.clone(),
                json!({
                    "uses": uses[u.as_str()].iter().map(|s| s.to_string()).collect::<Vec<_>>(), "starts": starts, "nodes": serves,
                    "runs_until_chosen": self.runs_until_chosen(u)?.iter().map(hex).collect::<Vec<_>>(),
                    "approves": approves, "invites": self.invites(u)?, "removes_people": self.removes_people(u)?, "removes": removes,
                }),
            );
        }
        Ok(json!({
            "root": root, "name": name, "recovery": self.recovery()?.as_ref().map(hex),
            "devices": devices, "openers": openers, "runs_for": runs_for, "epochs": epochs, "people": answers,
        }))
    }
}

/// What a device takes from the members to seal and hand keys by, as JSON: `Who::view`.
pub fn view(m: &Members) -> Result<Value> {
    Who::of(m)?.view()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use one::entry::{Content, Ext, Head};
    use one::keys::Endpoint;
    use one::members::{self, Role};

    const ANDRE: &str = "a000000000000001";
    const BOB: &str = "b000000000000002";
    const CAROL: &str = "c000000000000003";
    const GROUP: &str = "d000000000000004";

    /// Signs entries of `global`, each after every entry before it.
    #[derive(Default)]
    struct Install {
        entries: Vec<Entry>,
        heads: HashMap<Key, Head>,
    }

    impl Install {
        fn write(&mut self, key: &SigningKey, user: &str, content: Content) {
            let me = key.verifying_key().to_bytes();
            let previous = self.entries.iter().filter(|e| e.author() != me).map(Entry::hash).collect();
            let e = Entry::create(key, Ext { log: "global".into(), time: 0, user: user.into(), epoch: 0, previous }, self.heads.get(&me).copied(), content);
            self.heads.insert(me, e.head());
            self.entries.push(e);
        }
    }

    /// Whom `Who` says seals, hands and approves what is whom replay says, of everyone, every scope and every device:
    /// an admin and a member, one taken out, a group, a device taken out, a serverless node, someone who chose their
    /// things run nowhere else and someone who never chose.
    #[test]
    fn who_answers_as_replay_does() {
        let key = |n: u8| SigningKey::from_bytes(&[n; 32]);
        let (mac, phone, boxed, laptop, bobs_box, carols, serverless) = (key(1), key(2), key(3), key(4), key(5), key(6), key(7));
        let at = |k: &SigningKey| Endpoint::of(k);
        let hash = |c: char| c.to_string().repeat(64);
        let mut i = Install::default();
        i.write(&mac, ANDRE, members::genesis("Home", ANDRE, "Andre", &at(&mac), "mac", &[Kind::Client], &[9; 32], &hash('e')));
        i.write(&mac, ANDRE, members::device(&at(&phone), ANDRE, "phone", &[Kind::Client], None));
        i.write(&mac, ANDRE, members::device(&at(&boxed), ANDRE, "box", &[Kind::Compute], None));
        i.write(&mac, ANDRE, members::device(&at(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Member))));
        i.write(&mac, ANDRE, members::device(&at(&carols), CAROL, "phone", &[Kind::Client], Some(("Carol", Role::Member))));
        i.write(&mac, ANDRE, members::setting("serverless", Some(&hex(&serverless.verifying_key().to_bytes()))));
        i.write(&laptop, BOB, members::device(&at(&bobs_box), BOB, "bob's box", &[Kind::Compute], None));
        i.write(&laptop, BOB, members::runs(&[]));
        i.write(&laptop, BOB, members::epoch(&Scope::User(BOB.into()), 1, &[laptop.verifying_key().to_bytes()], &hash('b')));
        i.write(&laptop, BOB, members::group(GROUP, "Band"));
        i.write(&laptop, BOB, members::invite(GROUP, ANDRE, Role::Member));
        i.write(&mac, ANDRE, members::accept(GROUP));
        i.write(&laptop, BOB, members::epoch(&Scope::Group(GROUP.into()), 1, &[laptop.verifying_key().to_bytes(), mac.verifying_key().to_bytes()], &hash('d')));
        i.write(&mac, ANDRE, members::remove_device(&phone.verifying_key().to_bytes(), None));
        i.write(&mac, ANDRE, members::remove_user(CAROL, &BTreeMap::from([(carols.verifying_key().to_bytes(), None)])));
        let mut g = Global::new(None);
        g.add_entries(i.entries.clone()).unwrap();
        let m = g.members().unwrap();
        assert!(m.user(BOB).is_some() && m.removed_user(CAROL) && m.machine(&phone.verifying_key().to_bytes()).is_none(), "the install is as written");
        let who = Who::of(&m).unwrap();

        assert_eq!(who.people().unwrap(), m.users.values().filter(|u| !u.removed).map(|u| u.id.clone()).collect::<Vec<_>>());
        assert_eq!(who.scopes().unwrap(), m.scopes());
        assert_eq!(who.recovery().unwrap(), m.recovery);
        assert_eq!(who.serverless().unwrap(), m.serverless());
        let listed: Vec<Listed> = m
            .machines
            .values()
            .filter(|d| m.machine(&d.key).is_some())
            .map(|d| Listed { key: d.key, seal: d.seal, user: d.user.clone(), name: d.name.clone(), kinds: d.roles.iter().copied().collect(), passkey: d.passkey.clone(), device: d.device })
            .collect();
        assert_eq!(who.devices().unwrap(), listed);
        let nodes: Vec<Key> = m.computes().map(|c| c.key).chain(m.serverless()).collect();
        assert_eq!(who.nodes().unwrap().iter().map(|n| n.0).collect::<Vec<_>>(), nodes);
        for s in &m.scopes() {
            let openers: Vec<(Key, Key)> = m.clients().filter(|c| m.may_use(&c.user, s)).map(|c| (c.key, c.seal)).collect();
            assert_eq!(who.openers(s).unwrap(), openers, "{s}'s openers");
            assert_eq!(who.epoch(s).unwrap(), m.epoch(s), "{s}'s epoch");
            for n in 0..=m.epoch(s) + 1 {
                assert_eq!(who.epoch_hash(s, n).unwrap().as_deref(), m.epoch_hash(s, n), "{s}'s epoch {n}");
            }
            for node in &nodes {
                assert_eq!(who.runs_for(node, s).unwrap(), m.runs_for(node, s), "{} runs {s}", hex(node));
            }
        }
        for u in [ANDRE, BOB, CAROL] {
            for s in &m.scopes() {
                assert_eq!(who.uses(u, s).unwrap(), m.may_use(u, s), "{u} opens {s}");
                assert_eq!(who.starts(u, s).unwrap(), m.role_in(u, s) == Some(Role::Admin), "{u} starts {s}'s epochs");
            }
            for node in &nodes {
                assert_eq!(who.serves(node, u).unwrap(), m.serves(node, u), "{} runs {u}'s", hex(node));
            }
            assert_eq!(who.runs(u).unwrap(), m.runs.get(u).map(|on| on.iter().copied().collect()), "where {u}'s run");
            let unchosen: Vec<Key> = if m.runs.contains_key(u) { Vec::new() } else { m.computes().filter(|c| c.user != u && m.runs_for(&c.key, &Scope::Global)).map(|c| c.key).collect() };
            assert_eq!(who.runs_until_chosen(u).unwrap(), unchosen, "where {u}'s run until they choose");
            for of in [ANDRE, BOB, CAROL] {
                assert_eq!(who.approves(u, of).unwrap(), u == of && m.user(u).is_some(), "{u} lets in {of}'s devices");
            }
            for k in m.machines.keys() {
                assert_eq!(who.removes(u, k).unwrap(), m.looks_after(u, k), "{u} takes out {}", hex(k));
            }
            assert_eq!(who.invites(u).unwrap(), m.is_admin(u), "{u} invites");
            assert_eq!(who.removes_people(u).unwrap(), m.is_admin(u), "{u} takes people out");
        }
        assert_eq!(who.runs(BOB).unwrap(), Some(Vec::new()), "nowhere else is a choice");
        assert!(who.runs_until_chosen(ANDRE).unwrap().is_empty() && who.runs(ANDRE).unwrap().is_none(), "Andre's things run on his own box");
        assert_eq!(who.runs_until_chosen(CAROL).unwrap(), vec![boxed.verifying_key().to_bytes()], "everyone's machine, until they choose");

        let k = who.view().unwrap();
        assert_eq!(k["people"][BOB]["uses"], json!(["global", format!("group:{GROUP}"), format!("user:{BOB}")]));
        assert_eq!(k["people"][ANDRE]["starts"], json!(["global", format!("user:{ANDRE}")]));
        assert!(k["people"].get(CAROL).is_none(), "no one taken out");
    }
}
