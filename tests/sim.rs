//! The network simulator: servers of several users, with groups, ACLs, trust and privileged servers switching on and
//! off, over a hostile network, all from one seed. Messages are delayed, reordered, duplicated and dropped; links go
//! down and come back. Once the network heals, each server must hold exactly the scopes it should, with every entry
//! the others have there, and servers holding the same entries must make the same views.
//! Run more seeds with `SIM_SEEDS=10000 cargo test --release --test sim`.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use one::entry::{Content, Entry, Key, Scope};
use one::members::{self, Members, Principal, Role, node};
use one::store::{Frontier, Store};
use one::sync::{Handled, Msg, Replica};
use p2panda_core::SigningKey;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde_json::json;

struct Server {
    r: Replica,
    user: String,
    clock: u64,
}

struct Net {
    queue: VecDeque<(usize, usize, Msg)>,
    up: Vec<Vec<bool>>,
    /// The frontier each server last heard from each other: (to, from).
    frontiers: HashMap<(usize, usize), Frontier>,
}

fn id(rng: &mut StdRng) -> String {
    format!("{:016x}", rng.r#gen::<u64>())
}

impl Server {
    fn id(&self) -> Key {
        self.r.id
    }

    fn write(&mut self, now: u64, scope: &Scope, user: &str, content: Content) -> Option<Handled> {
        self.clock = self.clock.max(now);
        self.r.append(scope, user, None, content, self.clock).ok().map(|(_, h)| h)
    }
}

fn run(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let n = rng.gen_range(2..7);
    let users: Vec<String> = (0..rng.gen_range(1..=n)).map(|_| id(&mut rng)).collect();
    let keys: Vec<SigningKey> = (0..n).map(|_| SigningKey::from(rng.r#gen::<[u8; 32]>())).collect();
    // Server 0 is the first admin's; each other is one user's, the first admin's own included.
    let owner_of: Vec<usize> = (0..n).map(|i| if i == 0 { 0 } else { rng.gen_range(0..users.len()) }).collect();

    let mut first = Replica::new(keys[0].clone(), Store::memory().unwrap(), None, None).unwrap();
    let genesis = Entry::create(
        &keys[0],
        one::entry::Ext { log: "global".into(), time: 1, user: users[0].clone(), by: None },
        None,
        members::genesis("Sim", &users[0], "u0", "s0", &[9; 32]),
    );
    let root = Some(genesis.hash());
    first.root = root;
    first.store.insert_own(&genesis, None).unwrap();
    let mut servers: Vec<Server> = Vec::new();
    servers.push(Server { r: Replica::new(keys[0].clone(), first.store, root, None).unwrap(), user: users[0].clone(), clock: 1 });
    let mut added = BTreeSet::from([users[0].clone()]);
    for i in 1..n {
        let user = users[owner_of[i]].clone();
        let new = added.insert(user.clone()).then(|| (format!("u{}", owner_of[i]), if rng.gen_ratio(1, 3) { Role::Admin } else { Role::Member }));
        let key = *keys[i].verifying_key().as_bytes();
        let admit = members::admit(&key, &user, &format!("s{i}"), true, new.as_ref().map(|(name, role)| (name.as_str(), *role)));
        let admin = users[0].clone();
        servers[0].write(2, &Scope::Global, &admin, admit).unwrap();
    }
    let everything = servers[0].r.store.ordered().unwrap();
    for i in 1..n {
        let r = Replica::new(keys[i].clone(), Store::memory().unwrap(), root, None).unwrap();
        servers.push(Server { r, user: users[owner_of[i]].clone(), clock: 2 });
        for e in &everything {
            servers[i].r.store.insert(e).unwrap();
        }
        servers[i].r = Replica::new(keys[i].clone(), std::mem::replace(&mut servers[i].r.store, Store::memory().unwrap()), root, None).unwrap();
    }

    let mut net = Net { queue: VecDeque::new(), up: vec![vec![true; n]; n], frontiers: HashMap::new() };
    let mut groups: Vec<String> = Vec::new();
    let steps = rng.gen_range(200..1200);
    for step in 0..steps {
        let now = 10 + step as u64 * rng.gen_range(0..3);
        match rng.gen_range(0..100) {
            0..25 => {
                // An app entry, as a call would write it: as a user who trusts the server, in a scope they may use.
                let i = rng.gen_range(0..n);
                let m = &servers[i].r.members;
                let me = servers[i].id();
                let as_user: Vec<&String> = users.iter().filter(|u| m.serves(&me, u)).collect();
                let Some(user) = as_user.choose(&mut rng).map(|u| u.to_string()) else { continue };
                let scopes: Vec<Scope> = m.scopes().into_iter().filter(|s| m.may_use(&user, s) && servers[i].r.holds(s)).collect();
                let Some(scope) = scopes.choose(&mut rng).cloned() else { continue };
                let skew = rng.gen_range(0..50);
                let content = Content::new("board", "post", json!({ "text": step }));
                if let Some(h) = servers[i].write(now.saturating_sub(skew), &scope, &user, content) {
                    after(&mut net, &servers, i, None, h);
                }
            }
            25..30 => {
                let (a, b) = (rng.gen_range(0..n), rng.gen_range(0..n));
                let up = !net.up[a][b];
                net.up[a][b] = up;
                net.up[b][a] = up;
            }
            30..38 => {
                let i = rng.gen_range(0..n);
                for j in 0..n {
                    if j != i && net.up[i][j] {
                        net.queue.push_back((i, j, servers[i].r.hello(Vec::new()).unwrap()));
                    }
                }
            }
            38..48 => {
                let i = rng.gen_range(0..n);
                let content = member_change(&mut rng, &servers, i, &users, &mut groups);
                let user = servers[i].user.clone();
                if let Some(h) = servers[i].write(now, &Scope::Global, &user, content) {
                    after(&mut net, &servers, i, None, h);
                }
            }
            _ => deliver(&mut rng, &mut net, &mut servers),
        }
    }

    // Heal: every link up, hellos until nothing new moves.
    net.up = vec![vec![true; n]; n];
    for round in 0.. {
        assert!(round < 30, "seed {seed}: sync never settled");
        let before: u64 = servers.iter().map(|s| s.r.store.count().unwrap()).sum();
        for i in 0..n {
            for j in 0..n {
                if i != j {
                    net.queue.push_back((i, j, servers[i].r.hello(Vec::new()).unwrap()));
                }
            }
        }
        while !net.queue.is_empty() {
            deliver(&mut rng, &mut net, &mut servers);
        }
        let after: u64 = servers.iter().map(|s| s.r.store.count().unwrap()).sum();
        if round > 2 && after == before {
            break;
        }
    }

    check(seed, &servers);
}

/// A random change to the members, as server `i`'s user would make it: often one that won't count.
fn member_change(rng: &mut StdRng, servers: &[Server], i: usize, users: &[String], groups: &mut Vec<String>) -> Content {
    let s = &servers[i];
    let m = &s.r.members;
    let other = &servers[rng.gen_range(0..servers.len())];
    let someone = users.choose(rng).unwrap().clone();
    let hex = one::entry::hex(&other.id());
    match rng.gen_range(0..10) {
        0 => {
            let group = id(rng);
            groups.push(group.clone());
            node("group", json!({ "group": group, "name": "g" }))
        }
        1 => match groups.choose(rng) {
            Some(g) => node("invite", json!({ "group": g, "user": someone, "role": if rng.gen_ratio(1, 3) { "admin" } else { "member" } })),
            None => node("group", json!({ "group": id(rng), "name": "g" })),
        },
        2 => {
            let invited: Vec<&String> = m.groups.values().filter(|g| g.invited.contains_key(&s.user)).map(|g| &g.id).collect();
            match invited.choose(rng) {
                Some(g) => node("accept", json!({ "group": g })),
                None => members::trust(&other.id(), true),
            }
        }
        3 => match groups.choose(rng) {
            Some(g) => node("member", json!({ "group": g, "user": if rng.gen_ratio(1, 2) { &s.user } else { &someone }, "role": null })),
            None => members::trust(&other.id(), false),
        },
        // Share a server with another user, who may then trust it.
        4 => {
            let me = Principal::User(s.user.clone());
            members::acl(&s.id(), std::slice::from_ref(&me), &[me.clone(), Principal::User(someone)])
        }
        5 => members::trust(&other.id(), rng.gen_ratio(2, 3)),
        // An admin makes a server privileged, or gives it back to its user.
        6 => {
            let owner = Principal::User(other.user.clone());
            match rng.gen_ratio(1, 2) {
                true => members::acl(&other.id(), &[Principal::Install], &[Principal::Install]),
                false => members::acl(&other.id(), std::slice::from_ref(&owner), std::slice::from_ref(&owner)),
            }
        }
        // A group owns a server.
        7 => match groups.choose(rng) {
            Some(g) => members::acl(&s.id(), &[Principal::Group(g.clone())], &[Principal::Group(g.clone()), Principal::User(s.user.clone())]),
            None => members::trust(&other.id(), true),
        },
        8 => node("role", json!({ "user": someone, "role": if rng.gen_ratio(1, 2) { "admin" } else { "member" } })),
        _ => node("rename", json!({ "key": hex, "name": format!("renamed {}", rng.gen_range(0..9)) })),
    }
}

/// Passes on what a server just wrote or received: to every peer that holds it, and, if the members changed, a
/// hello to every peer if it now holds more, and to each peer that now holds more what it lacks there.
fn after(net: &mut Net, servers: &[Server], from: usize, except: Option<usize>, h: Handled) {
    for to in 0..servers.len() {
        if to == from || !net.up[from][to] {
            continue;
        }
        if Some(to) != except {
            if let Some(msg) = servers[from].r.for_peer(&servers[to].id(), &h.new) {
                net.queue.push_back((from, to, msg));
            }
        }
        if h.gained {
            net.queue.push_back((from, to, servers[from].r.hello(Vec::new()).unwrap()));
        }
        if let Some((_, logs)) = h.peers_gained.iter().find(|(k, _)| *k == servers[to].id()) {
            let frontier = net.frontiers.get(&(from, to)).cloned().unwrap_or_default();
            for msg in servers[from].r.missing_in(&servers[to].id(), &frontier, |l| logs.iter().any(|g| g == l)).unwrap() {
                net.queue.push_back((from, to, msg));
            }
        }
    }
}

fn deliver(rng: &mut StdRng, net: &mut Net, servers: &mut [Server]) {
    if net.queue.is_empty() {
        return;
    }
    let pick = rng.gen_range(0..net.queue.len().min(4));
    let (from, to, msg) = net.queue.remove(pick).unwrap();
    if !net.up[from][to] || rng.gen_ratio(1, 20) {
        return; // the link is down, or the message was lost
    }
    if rng.gen_ratio(1, 20) {
        net.queue.push_back((from, to, msg.clone())); // duplicated
    }
    if let Msg::Hello { frontier, .. } = &msg {
        net.frontiers.insert((to, from), frontier.clone());
    }
    let peer = servers[from].id();
    let Ok(mut h) = servers[to].r.handle(&peer, msg, u64::MAX / 2) else { return };
    for reply in h.replies.drain(..) {
        net.queue.push_back((to, from, reply));
    }
    after(net, servers, to, Some(from), h);
}

/// What each scope's view is on a server: the ids of its entries that count, in replay order.
fn views(s: &Server) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for log in s.r.store.logs().unwrap() {
        let mut ids = Vec::new();
        members::replay(s.r.root, &s.r.member_entries, &s.r.store.scope(&log).unwrap(), |e, _, _| ids.push(e.id()));
        out.insert(log, ids);
    }
    out
}

fn summary(m: &Members) -> String {
    let users: Vec<_> = m.users.values().map(|u| (&u.id, &u.name, u.role, u.removed)).collect();
    let machines: Vec<_> = m.machines.values().map(|s| (s.key, &s.user, &s.name, s.removed)).collect();
    let groups: Vec<_> = m.groups.values().map(|g| (&g.id, &g.members, &g.invited)).collect();
    let acls: Vec<_> = m.acls.iter().map(|(k, a)| (k, format!("{:?} {:?}", a.owners, a.users))).collect();
    format!("{users:?} {machines:?} {groups:?} {acls:?} {:?}", m.trusted)
}

fn check(seed: u64, servers: &[Server]) {
    let current: Vec<&Server> = servers.iter().filter(|s| s.r.members.server(&s.id()).is_some()).collect();
    let reference = &current[0].r;
    let global: BTreeSet<_> = reference.store.scope("global").unwrap().iter().map(|e| e.hash()).collect();
    for s in &current {
        let theirs: BTreeSet<_> = s.r.store.scope("global").unwrap().iter().map(|e| e.hash()).collect();
        assert_eq!(theirs, global, "seed {seed}: servers hold different global entries");
        assert_eq!(summary(&s.r.members), summary(&reference.members), "seed {seed}: servers disagree on members");
    }
    let m = &reference.members;
    // Every entry of each scope, on any server that should hold it.
    let mut entries: BTreeMap<String, BTreeSet<_>> = BTreeMap::new();
    for s in &current {
        for log in s.r.store.logs().unwrap() {
            assert!(m.holds(&s.id(), &Scope::parse(&log).unwrap()), "seed {seed}: a server holds {log}, which it shouldn't");
            entries.entry(log.clone()).or_default().extend(s.r.store.scope(&log).unwrap().iter().map(|e| e.hash()));
        }
    }
    let views: Vec<_> = current.iter().map(|s| views(s)).collect();
    for (log, all) in &entries {
        let scope = Scope::parse(log).unwrap();
        let mut view: Option<&Vec<String>> = None;
        for (s, v) in current.iter().zip(&views) {
            if !m.holds(&s.id(), &scope) {
                continue;
            }
            let theirs: BTreeSet<_> = s.r.store.scope(log).unwrap().iter().map(|e| e.hash()).collect();
            assert_eq!(&theirs, all, "seed {seed}: a server that holds {log} lacks some of its entries");
            let mine = v.get(log).unwrap();
            assert!(view.is_none_or(|v| v == mine), "seed {seed}: servers holding the same entries of {log} made different views");
            view = Some(mine);
        }
    }
}

#[test]
fn converges() {
    let seeds: u64 = std::env::var("SIM_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(if cfg!(debug_assertions) { 40 } else { 400 });
    let start: u64 = std::env::var("SIM_START").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    for seed in start..start + seeds {
        run(seed);
    }
}
