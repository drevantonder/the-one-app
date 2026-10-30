//! The network simulator: many devices, one space, a hostile network, all from one seed.
//! Messages are delayed, reordered, duplicated and dropped; links go down and come back.
//! Once the network heals, every device must hold the same entries and the same members.
//! Run more seeds with `SIM_SEEDS=10000 cargo test --release --test sim`.

use std::collections::{BTreeSet, VecDeque};

use one::entry::{Content, Entry, Key};
use one::space::{self, Members, Role};
use one::store::Store;
use one::sync::{self, Msg};
use p2panda_core::SigningKey;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::json;

struct Device {
    key: SigningKey,
    store: Store,
    clock: u64,
    written: Vec<[u8; 32]>,
}

impl Device {
    fn id(&self) -> Key {
        *self.key.verifying_key().as_bytes()
    }

    fn write(&mut self, now: u64, content: Content) -> Entry {
        self.clock = self.clock.max(now).max(self.store.max_time().unwrap() + 1);
        let prev = self.store.last(&self.id(), one::entry::SPACE_LOG).unwrap();
        let entry = Entry::create(&self.key, self.store.space(), self.clock, prev.as_ref(), content);
        assert!(self.store.insert(&entry).unwrap());
        self.written.push(*entry.hash().as_bytes());
        entry
    }
}

struct Net {
    queue: VecDeque<(usize, usize, Msg)>,
    up: Vec<Vec<bool>>,
}

fn run(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let n = rng.gen_range(2..7);
    let root = SigningKey::from(rng.r#gen::<[u8; 32]>());
    let space: Key = *root.verifying_key().as_bytes();
    let mut devices: Vec<Device> = (0..n)
        .map(|_| Device { key: SigningKey::from(rng.r#gen::<[u8; 32]>()), store: Store::memory(space).unwrap(), clock: 0, written: Vec::new() })
        .collect();

    let first = devices[0].id();
    devices[0].write(1, space::genesis(&root, &first, "Sim", "p0", "d0"));
    for i in 1..n {
        let id = devices[i].id();
        devices[0].write(2, space::admit(&id, &format!("p{i}"), &format!("d{i}"), Role::Member));
    }

    let mut net = Net { queue: VecDeque::new(), up: vec![vec![true; n]; n] };
    let steps = rng.gen_range(200..1500);
    for step in 0..steps {
        let now = 10 + step as u64 * rng.gen_range(0..3);
        match rng.gen_range(0..100) {
            0..25 => {
                let i = rng.gen_range(0..n);
                let skew = rng.gen_range(0..50);
                let entry = devices[i].write(now.saturating_sub(skew), Content { app: "board".into(), kind: "post".into(), data: json!({ "text": step }) });
                broadcast(&mut net, &devices, i, None, &[entry]);
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
                        net.queue.push_back((i, j, sync::hello(&devices[i].store).unwrap()));
                    }
                }
            }
            _ => deliver(&mut rng, &mut net, &mut devices),
        }
    }

    // Heal: every link up, hellos until nothing new moves.
    net.up = vec![vec![true; n]; n];
    for _ in 0..4 {
        for i in 0..n {
            for j in 0..n {
                if i != j {
                    net.queue.push_back((i, j, sync::hello(&devices[i].store).unwrap()));
                }
            }
        }
        while !net.queue.is_empty() {
            deliver(&mut rng, &mut net, &mut devices);
        }
    }

    let everything: BTreeSet<[u8; 32]> = devices.iter().flat_map(|d| d.written.iter().copied()).collect();
    let reference: Vec<[u8; 32]> = devices[0].store.ordered().unwrap().iter().map(|e| *e.hash().as_bytes()).collect();
    assert_eq!(reference.iter().copied().collect::<BTreeSet<_>>(), everything, "seed {seed}: an entry was lost");
    let members = Members::fold(&space, &devices[0].store.ordered().unwrap());
    assert_eq!(members.current_devices().count(), n, "seed {seed}: membership is incomplete");
    for d in &devices[1..] {
        let order: Vec<[u8; 32]> = d.store.ordered().unwrap().iter().map(|e| *e.hash().as_bytes()).collect();
        assert_eq!(order, reference, "seed {seed}: devices disagree on fold order");
        let theirs = Members::fold(&space, &d.store.ordered().unwrap());
        assert_eq!(format!("{:?}", theirs.devices.keys().collect::<Vec<_>>()), format!("{:?}", members.devices.keys().collect::<Vec<_>>()), "seed {seed}: devices disagree on members");
    }
}

fn broadcast(net: &mut Net, devices: &[Device], from: usize, except: Option<usize>, entries: &[Entry]) {
    for to in 0..devices.len() {
        if to != from && Some(to) != except && net.up[from][to] {
            net.queue.push_back((from, to, sync::entries(devices[from].store.space(), entries)));
        }
    }
}

fn deliver(rng: &mut StdRng, net: &mut Net, devices: &mut [Device]) {
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
    let (new, replies) = sync::handle(&devices[to].store, msg, u64::MAX / 2).unwrap();
    for reply in replies {
        net.queue.push_back((to, from, reply));
    }
    if !new.is_empty() {
        broadcast(net, devices, to, Some(from), &new);
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
