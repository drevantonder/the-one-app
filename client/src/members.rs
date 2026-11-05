//! Who's who as the install's own signed entries say it, checked on the device with the nodes' own Rust (`one`'s
//! entry checks and member replay), never as a node says it: a node holds no keys, and a hacked one could list a seal
//! key of its own as someone's device, or itself as where their things run, to be sealed or handed keys. What a device
//! seals, and to whom it hands keys, it takes only from here.
//!
//! A device keeps `global`'s entries it has checked and the install's root, the genesis it saw when it joined, and
//! refuses entries of an install with another; then it asks a node only for entries it doesn't have
//! (`GET /v1/entries/global?have=`, `have`).
//!
//! What it asks of them it asks `global`'s scope database, built from that replay, and `may` over it
//! (`one::database::scopes::ScopeDb`, as nodes ask theirs): native apps directly, browsers as JSON (`view`). Replay
//! stays the truth, and this only its answers.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, bail, ensure};
use serde_json::{Map, Value, json};

pub use one::database::scopes::ScopeDb;
use one::database::scopes::{INSTALL, device, object, person};
use one::entry::{Entry, Hash, Key, Scope, causal, hex, pack, unpack};
use one::members::{Cuts, Kind, Members, Role, ScopeState, is_member_entry};

/// `global`'s entries a device holds, checked, and the install they're of; and once its log is frozen, its scope
/// database's steps (`one::database::steps`), each checked as from anyone, by position from the first: none for one
/// that isn't a step, which every reader passes over.
pub struct Global {
    root: Option<Hash>,
    entries: BTreeMap<Hash, Entry>,
    steps: Vec<Option<Entry>>,
}

impl Global {
    /// For the install whose genesis is `root`, or, for a device that's only now joined, whichever one genesis the
    /// first entries it's given hold.
    pub fn new(root: Option<Hash>) -> Global {
        Global { root, entries: BTreeMap::new(), steps: Vec::new() }
    }

    /// Adds steps as a node gives them (`GET /v1/steps/global`, or as `steps` kept them): `{ steps: [{ at, step }] }`,
    /// each step base64url (`steps::encode`) or none, its signature checked. Those after the next it lacks wait.
    pub fn add_steps(&mut self, given: &Value) -> Result<()> {
        for s in given["steps"].as_array().into_iter().flatten() {
            let Some(at) = s["at"].as_u64() else { continue };
            if at != self.steps.len() as u64 + 1 {
                continue;
            }
            let bytes = s["step"].as_str().and_then(|b| data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).ok());
            self.steps.push(one::database::steps::given(&Scope::Global, at, bytes.as_deref()));
        }
        Ok(())
    }

    /// Adds steps read from the bucket (`steps::read`), the next after those held.
    pub fn add_read(&mut self, read: Vec<Option<Entry>>) {
        self.steps.extend(read);
    }

    /// The steps held, as `add_steps` takes them: to keep.
    pub fn steps(&self) -> Value {
        let list: Vec<Value> = self.steps.iter().enumerate().map(|(i, s)| json!({ "at": i + 1, "step": s.as_ref().map(|s| data_encoding::BASE64URL_NOPAD.encode(&one::database::steps::encode(s))) })).collect();
        json!({ "steps": list })
    }

    /// The last step held: what to ask a node for those after (`?from=`).
    pub fn at(&self) -> u64 {
        self.steps.len() as u64
    }

    /// Who's who: once the log is frozen, the scope database its freeze anchors (built here from the entries it
    /// follows, and refused if that isn't what the freeze says), and the steps after, each applied only if it counts
    /// there; before, built from replay's members.
    pub fn who(&self) -> Result<ScopeDb> {
        match self.frozen()? {
            Some(frozen) => one::database::steps::truth(&frozen, &self.steps, now()),
            None => who(&self.members()?),
        }
    }

    /// The log's freeze, if one counts among the entries held (`members::frozen`).
    pub fn frozen(&self) -> Result<Option<one::members::Frozen>> {
        let Some(root) = self.root else { bail!("no entries of your One App yet") };
        let ordered = causal(self.entries.values().cloned().collect(), &HashMap::new());
        Ok(one::members::frozen(Some(root), &ordered, &self.forks()))
    }

    /// Each device that signed two entries at one seq, cut from the first.
    fn forks(&self) -> Cuts {
        let mut seen: HashMap<(Key, u32), Hash> = HashMap::new();
        let mut forks = Cuts::new();
        for e in self.entries.values() {
            if seen.insert((e.author(), e.seq()), e.hash()).is_some_and(|h| h != e.hash()) {
                let at = forks.entry((e.author(), "global".into())).or_insert(u32::MAX);
                *at = (*at).min(e.seq());
            }
        }
        forks
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
        let forks = self.forks();
        let ordered: Vec<Entry> = causal(self.entries.values().cloned().collect(), &HashMap::new()).into_iter().filter(is_member_entry).collect();
        let m = Members::replay(Some(root), &ordered, &forks);
        ensure!(!m.users.is_empty(), "your One App's first entry isn't among those");
        Ok(m)
    }
}

/// `global`'s scope database, built from replay's members, as of now.
pub fn who(m: &Members) -> Result<ScopeDb> {
    ScopeDb::build(m, &Scope::Global, &ScopeState::default(), now())
}

fn now() -> u64 {
    n0_future::time::SystemTime::now().duration_since(n0_future::time::SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// What a browser seals, hands and signs by, as JSON for it to keep and look up (`kernel/web/device.js`,
/// `net-worker.js`), each answer worked out here:
///
/// - `root`, `name`, `recovery` (hex, what each epoch is sealed to besides devices)
/// - `devices`: each listed now, `{ key, seal, user, name, roles, passkey, device }`, and the serverless node as
///   `{ key, seal, roles: ["compute"] }`
/// - `removed`: the keys taken out, or turned down before they were listed, which are never sealed anything again
/// - `sign_in_clients`: the OIDC clients the install's people sign in with, its setting (`sign_in_proof::CLIENTS`)
/// - `openers`: each scope's `{ key, seal }` its epochs are sealed to
/// - `runs_for`: each node's scopes whose calls it runs: the only nodes handed a scope's keys
/// - `epochs`: each scope's newest epoch `n` and every epoch's hash
/// - `people`: each person's `uses` and `starts` (scopes they open, and whose new epochs they start), `nodes` (those
///   that run their calls), `runs_until_chosen`, `approves` (whose new devices they let in), `invites` (someone new),
///   `removes_people`, `removes` (the devices they may take out), and `keeps`: the apps they keep running on a device
///   still listed, `{ endpoint, scope, app, until, keys }` (each epoch's key hash), the only ones their browser renews
/// - `roles` and `runs`, as an older release's tabs read them (each scope's people's roles, and the machines each
///   person who chose runs their things on): kept for those tabs, which share this record; phase 3 removes them
pub fn view(m: &Members) -> Result<Value> {
    view_of(&who(m)?)
}

/// `view`, of who's who as it is (`Global::who`).
pub fn view_of(db: &ScopeDb) -> Result<Value> {
    let scopes = db.scopes();
    let listed = db.machines().into_iter().filter(|d| !d.removed).collect::<Vec<_>>();
    let mut devices: Vec<Value> = listed.iter().map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal), "user": d.user, "name": d.name, "roles": d.roles, "passkey": d.passkey, "device": d.device.as_ref().map(hex), "since": d.since, "place": d.place })).collect();
    let nodes = db.nodes();
    if let Some(k) = db.serverless()
        && let Some((_, seal)) = nodes.iter().find(|(n, _)| *n == k)
    {
        devices.push(json!({ "key": hex(&k), "seal": hex(seal), "roles": ["compute"] }));
    }
    let people: Vec<String> = db.users().into_iter().filter(|u| !u.removed).map(|u| u.id).collect();
    let uses = |u: &str| scopes.iter().filter(|s| db.may(u, "use", &object(s))).collect::<Vec<_>>();
    let uses: BTreeMap<&str, Vec<&Scope>> = people.iter().map(|u| (u.as_str(), uses(u))).collect();
    // A scope someone opens, with the client devices of everyone who does.
    let mut openers = Map::new();
    for s in scopes.iter().filter(|s| uses.values().any(|on| on.contains(s))) {
        let to: Vec<Value> = listed.iter().filter(|d| d.is(Kind::Client) && uses.get(d.user.as_str()).is_some_and(|on| on.contains(&s))).map(|d| json!({ "key": hex(&d.key), "seal": hex(&d.seal) })).collect();
        openers.insert(s.to_string(), json!(to));
    }
    let runs_for: Map<String, Value> = nodes.iter().map(|(k, _)| (hex(k), json!(scopes.iter().filter(|s| db.runs_for(k, s)).map(Scope::to_string).collect::<Vec<_>>()))).collect();
    let mut epochs = Map::new();
    for (s, n) in db.epochs() {
        epochs.insert(s.to_string(), json!({ "n": n, "hashes": db.epoch_hashes(&s).into_iter().map(|(n, h)| (n.to_string(), json!(h))).collect::<Map<_, _>>() }));
    }
    let keeps = db.keeps();
    let mut answers = Map::new();
    for u in &people {
        let keeps: Vec<Value> = keeps
            .iter()
            .filter(|((k, s, _), g)| g.by == *u && db.machine(k).is_some() && db.may(u, "keep", &object(s)))
            .map(|((k, s, a), g)| json!({ "endpoint": hex(k), "scope": s.to_string(), "app": a, "until": g.until, "keys": g.hashes }))
            .collect();
        answers.insert(
            u.clone(),
            json!({
                "uses": uses[u.as_str()].iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                "starts": scopes.iter().filter(|s| db.role_in(u, s) == Some(Role::Admin)).map(Scope::to_string).collect::<Vec<_>>(),
                "nodes": nodes.iter().filter(|(k, _)| db.serves(k, u)).map(|(k, _)| hex(k)).collect::<Vec<_>>(),
                "runs_until_chosen": db.runs_until_chosen(u).iter().map(hex).collect::<Vec<_>>(),
                "approves": people.iter().filter(|of| db.may(u, "add-devices", &person(of))).collect::<Vec<_>>(),
                "invites": db.may(u, "invite", INSTALL),
                "removes_people": db.may(u, "remove", INSTALL),
                "removes": listed.iter().filter(|d| db.may(u, "remove", &device(&d.key))).map(|d| hex(&d.key)).collect::<Vec<_>>(),
                "keeps": keeps,
                "sign_ins": db.sign_ins(u),
            }),
        );
    }
    // For an older release's tabs, which read these and stop working without them. Phase 3 removes them.
    let mut roles = Map::new();
    for s in &scopes {
        let of: Map<String, Value> = people.iter().filter_map(|u| Some((u.clone(), json!(db.role_in(u, s)?)))).collect();
        if !of.is_empty() {
            roles.insert(s.to_string(), json!(of));
        }
    }
    let runs: Map<String, Value> = people.iter().filter_map(|u| Some((u.clone(), json!(db.runs(u)?.iter().map(hex).collect::<Vec<_>>())))).collect();
    Ok(json!({
        "root": db.root().map(|r| r.to_hex()), "name": db.name(), "recovery": db.recovery().as_ref().map(hex),
        "devices": devices, "openers": openers,
        "removed": db.machines().iter().filter(|d| d.removed).map(|d| hex(&d.key)).collect::<Vec<_>>(),
        "sign_in_clients": db.setting(one::sign_in_proof::CLIENTS), "runs_for": runs_for, "epochs": epochs, "people": answers,
        "roles": roles, "runs": runs,
    }))
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

    /// What a browser seals to, hands keys and approves by is what replay says, of everyone, every scope and every
    /// device: an admin and a member, one taken out, a group, a device taken out, a serverless node, someone who chose
    /// their things run nowhere else and someone who never chose. (`ScopeDb`'s own reads are checked in `one`.)
    #[test]
    fn a_browsers_answers_are_replays() {
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
        let stranger = key(8);
        i.write(&mac, ANDRE, members::refuse(&at(&stranger), "MacBook Pro"));
        let mut g = Global::new(None);
        g.add_entries(i.entries.clone()).unwrap();
        let m = g.members().unwrap();
        assert!(m.user(BOB).is_some() && m.removed_user(CAROL) && m.machine(&phone.verifying_key().to_bytes()).is_none(), "the install is as written");
        let nodes_: Vec<Key> = m.computes().map(|c| c.key).chain(m.serverless()).collect();

        // What a browser looks up is what replay says.
        let k = view(&m).unwrap();
        let names = |v: &Value| v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
        let people: Vec<&str> = m.users.values().filter(|u| !u.removed).map(|u| u.id.as_str()).collect();
        assert_eq!(k["people"].as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(), people, "no one taken out");
        for u in people {
            let a = &k["people"][u];
            let scopes = |f: &dyn Fn(&Scope) -> bool| m.scopes().iter().filter(|s| f(s)).map(Scope::to_string).collect::<Vec<_>>();
            assert_eq!(names(&a["uses"]), scopes(&|s| m.may_use(u, s)), "{u} opens");
            assert_eq!(names(&a["starts"]), scopes(&|s| m.role_in(u, s) == Some(Role::Admin)), "{u} starts epochs of");
            assert_eq!(names(&a["nodes"]), nodes_.iter().filter(|n| m.serves(n, u)).map(hex).collect::<Vec<_>>(), "{u}'s nodes");
            assert_eq!(names(&a["approves"]), vec![u.to_string()], "{u} lets in their own devices");
            let removes: Vec<String> = m.machines.keys().filter(|k| m.machine(k).is_some() && m.looks_after(u, k)).map(hex).collect();
            assert_eq!(names(&a["removes"]), removes, "{u} takes out");
            assert_eq!(a["invites"], json!(m.is_admin(u)), "{u} invites");
            assert_eq!(a["removes_people"], json!(m.is_admin(u)), "{u} takes people out");
        }
        let devices: Vec<String> = k["devices"].as_array().unwrap().iter().map(|d| d["key"].as_str().unwrap().to_string()).collect();
        assert_eq!(devices, m.machines.keys().filter(|k| m.machine(k).is_some()).chain(m.serverless().as_ref()).map(hex).collect::<Vec<_>>());
        // Never sealed anything again: a device taken out, one turned down before it was listed, and a removed person's.
        let mut removed = names(&k["removed"]);
        removed.sort();
        let mut out: Vec<String> = [&phone, &carols, &stranger].iter().map(|d| hex(&d.verifying_key().to_bytes())).collect();
        out.sort();
        assert_eq!(removed, out);
        for s in &m.scopes() {
            assert_eq!(k["epochs"][s.to_string()]["n"].as_u64().unwrap_or(0) as u32, m.epoch(s), "{s}'s epoch");
            for n in 1..=m.epoch(s) {
                assert_eq!(k["epochs"][s.to_string()]["hashes"][n.to_string()].as_str(), m.epoch_hash(s, n), "{s}'s epoch {n}");
            }
        }
    }

    /// A browser renews only the keeps its own replay holds: a grant a node lists, or signs for someone else's scope,
    /// to have their app's keys sealed to it, is never among them.
    #[test]
    fn only_replayed_keeps_are_renewed() {
        let key = |n: u8| SigningKey::from_bytes(&[n; 32]);
        let (mac, laptop, bobs_box, hacked) = (key(1), key(2), key(3), key(4));
        let at = |k: &SigningKey| Endpoint::of(k);
        let pk = |k: &SigningKey| k.verifying_key().to_bytes();
        let (bobs, soon) = (Scope::User(BOB.into()), u64::MAX / 2);
        let keys = BTreeMap::from([(1, "b".repeat(64))]);
        let mut i = Install::default();
        i.write(&mac, ANDRE, members::genesis("Home", ANDRE, "Andre", &at(&mac), "mac", &[Kind::Client], &[9; 32], &"e".repeat(64)));
        i.write(&mac, ANDRE, members::device(&at(&hacked), ANDRE, "box", &[Kind::Compute], None));
        i.write(&mac, ANDRE, members::device(&at(&laptop), BOB, "laptop", &[Kind::Client], Some(("Bob", Role::Member))));
        i.write(&laptop, BOB, members::device(&at(&bobs_box), BOB, "bob's box", &[Kind::Compute], None));
        i.write(&laptop, BOB, members::grant(&pk(&bobs_box), &bobs, "notes", soon, &keys));
        // The hacked node grants itself Bob's notes, as Bob would.
        i.write(&hacked, BOB, members::grant(&pk(&hacked), &bobs, "notes", soon, &keys));
        let mut g = Global::new(None);
        g.add_entries(i.entries).unwrap();
        let k = view(&g.members().unwrap()).unwrap();
        let endpoints = |u: &str| k["people"][u]["keeps"].as_array().unwrap().iter().map(|g| g["endpoint"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(endpoints(BOB), vec![hex(&pk(&bobs_box))], "Bob renews only the grant he signed");
        assert!(endpoints(ANDRE).is_empty(), "nor does anyone else renew one");
    }
}
