//! Many servers writing one app's database in one bucket over bad links: every request is slow by a random amount, so
//! they arrive out of order, and some fail, before or after the bucket did what they asked. Every seed must end with
//! every server reading the same database, holding every write it was told landed, once. A failing seed is a bug.
//!
//! Then the same with each server keeping a copy on its disk, sent some of the others' steps as they're written, and
//! reading from its copy alone in between: what it reads may be behind, but never anything no server read at the same
//! step, and the copy opens nothing without the key. A hacked server that sends some of them steps of its own making,
//! sealed with the key, at positions their copies lack, is caught: each copy ends matching the bucket.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use one::bucket::{Bucket, Copied, Object};
use one::db::{Db, execute};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::json;

const SERVERS: usize = 8;
const WRITES: usize = 30;
const SEEDS: u64 = 40;

/// One bucket, reached over links that delay every request and fail some.
#[derive(Clone)]
struct Link {
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    rng: Arc<Mutex<StdRng>>,
}

impl Link {
    /// Waits as the link does, then says whether the request fails: before it reaches the bucket, or after.
    async fn travel(&self) -> (bool, bool) {
        let (wait, before, after) = {
            let mut rng = self.rng.lock().unwrap();
            (rng.gen_range(0..200), rng.gen_bool(0.05), rng.gen_bool(0.05))
        };
        tokio::time::sleep(Duration::from_millis(wait)).await;
        (before, after)
    }
}

fn tag(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

impl Bucket for Link {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        let (before, after) = self.travel().await;
        if before || after {
            bail!("the link dropped it");
        }
        Ok(self.objects.lock().unwrap().get(name).map(|b| Object { bytes: b.clone(), tag: tag(b) }))
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        let (before, after) = self.travel().await;
        if before {
            bail!("the link dropped it");
        }
        let created = {
            let mut objects = self.objects.lock().unwrap();
            !objects.contains_key(name) && objects.insert(name.into(), bytes.to_vec()).is_none()
        };
        if after {
            bail!("the link dropped the answer");
        }
        Ok(created)
    }

    async fn replace(&self, name: &str, bytes: &[u8], tag_was: Option<&str>) -> Result<bool> {
        let (before, after) = self.travel().await;
        if before {
            bail!("the link dropped it");
        }
        let replaced = {
            let mut objects = self.objects.lock().unwrap();
            let ok = objects.get(name).map(|b| tag(b)).as_deref() == tag_was;
            if ok {
                objects.insert(name.into(), bytes.to_vec());
            }
            ok
        };
        if after {
            bail!("the link dropped the answer");
        }
        Ok(replaced)
    }
}

const SCHEMA: &str = "create table writes (id integer primary key, server integer not null); create table count (one integer primary key, n integer not null); insert into count values (1, 0)";

/// One write: a row of its own, and the count moved by it, in one transaction. Written again after a failure it
/// changes nothing, so a write the link lost the answer to can be tried again.
fn write(id: u64, server: usize) -> impl FnMut(&rusqlite::Connection) -> Result<()> {
    move |c| {
        execute(c, "insert or ignore into writes values (?, ?)", &[json!(id), json!(server)])?;
        execute(c, "update count set n = n + changes()", &[])?;
        Ok(())
    }
}

/// Retries what the link fails, as a server does when a person tries again.
async fn retry<T>(mut f: impl AsyncFnMut() -> Result<T>) -> T {
    loop {
        if let Ok(v) = f().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn state(db: &Db) -> (Vec<(u64, u64)>, u64) {
    let rows = db.query("select id, server from writes order by id", &[]).unwrap().iter().map(|r| (r["id"].as_u64().unwrap(), r["server"].as_u64().unwrap())).collect();
    let n = db.query("select n from count", &[]).unwrap()[0]["n"].as_u64().unwrap();
    (rows, n)
}

async fn run(seed: u64) {
    let link = Link { objects: Default::default(), rng: Arc::new(Mutex::new(StdRng::seed_from_u64(seed))) };
    let key = one::kernel::database_key(&[seed as u8; 32], "main");
    let mut servers = Vec::new();
    for server in 0..SERVERS {
        let link = link.clone();
        servers.push(tokio::task::spawn_local(async move {
            let mut db = retry(async || Db::open(&link, "global/sim/main", key).await).await;
            retry(async || db.migrate(&link, &[SCHEMA.to_string()]).await).await;
            for i in 0..WRITES {
                let id = (server * WRITES + i) as u64;
                retry(async || db.write(&link, write(id, server)).await).await;
            }
            retry(async || db.catch_up(&link).await).await;
            db
        }));
    }
    let mut dbs = Vec::new();
    for s in servers {
        dbs.push(s.await.unwrap());
    }
    // Each reads to the end, as a new server does from nothing.
    for db in &mut dbs {
        retry(async || db.catch_up(&link).await).await;
    }
    let fresh = retry(async || Db::open(&link, "global/sim/main", key).await).await;
    let (rows, n) = state(&fresh);
    assert_eq!(rows.len(), SERVERS * WRITES, "seed {seed}: every write landed once");
    assert_eq!(n, rows.len() as u64, "seed {seed}: the count moved once for each write, however often the link lost an answer");
    for (i, db) in dbs.iter().enumerate() {
        assert_eq!(state(db), (rows.clone(), n), "seed {seed}: server {i} reads what a new server does");
    }
    assert!(link.objects.lock().unwrap().keys().any(|k| k.contains("/copy/")), "seed {seed}: copies were written as steps piled up");
}

/// Runs each seed on a paused clock of its own, every seed at once: each is its own run, so the order doesn't matter.
fn seeds<F: Future<Output = ()>>(seeds: u64, run: fn(u64) -> F) {
    std::thread::scope(|s| {
        for seed in 0..seeds {
            s.spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().unwrap();
                runtime.block_on(tokio::task::LocalSet::new().run_until(run(seed)));
            });
        }
    });
}

#[test]
fn many_servers_write_one_database_over_bad_links() {
    seeds(SEEDS, run);
}

const COPYING: usize = 4;

/// What servers read at each step, which every read at that step must match.
type Seen = Arc<Mutex<BTreeMap<u64, (Vec<(u64, u64)>, u64)>>>;

fn saw(seen: &Seen, db: &Db, seed: u64) {
    let read = state(db);
    let mut seen = seen.lock().unwrap();
    let first = seen.entry(db.at()).or_insert_with(|| read.clone());
    assert_eq!(*first, read, "seed {seed}: two reads at step {} differ", db.at());
}

async fn run_copied(seed: u64) {
    let link = Link { objects: Default::default(), rng: Arc::new(Mutex::new(StdRng::seed_from_u64(seed))) };
    let key = one::kernel::database_key(&[seed as u8; 32], "main");
    let root = std::env::temp_dir().join(format!("one-sim-{}-{seed}", std::process::id()));
    let copies: Vec<Copied<Link>> = (0..COPYING).map(|s| Copied::new(link.clone(), root.join(s.to_string()))).collect();
    let seen = Seen::default();
    let mut servers = Vec::new();
    for server in 0..COPYING {
        let (link, copies, seen) = (link.clone(), copies.clone(), seen.clone());
        servers.push(tokio::task::spawn_local(async move {
            let here = &copies[server];
            let mut db = retry(async || Db::open(here, "global/sim/main", key).await).await;
            retry(async || db.migrate(here, &[SCHEMA.to_string()]).await).await;
            for i in 0..WRITES {
                let id = (server * WRITES + i) as u64;
                let at = db.at();
                retry(async || db.write(here, write(id, server)).await).await;
                // Its ping, carrying the step it wrote, reaches some of the others.
                let name = one::db::step_name("global/sim/main", db.at());
                if let Some(step) = here.kept(&name).unwrap().filter(|_| db.at() > at) {
                    for (other, copy) in copies.iter().enumerate().filter(|(o, _)| *o != server) {
                        if link.rng.lock().unwrap().gen_bool(0.6) {
                            copy.keep(&name, &step, &server.to_string()).unwrap_or_else(|e| panic!("seed {seed}: server {other} kept no step: {e:#}"));
                        }
                    }
                }
                // A read from its copy alone, as when the bucket is out of reach.
                db.catch_up(&here.here_only()).await.unwrap_or_else(|e| panic!("seed {seed}: server {server}'s copy failed it: {e:#}"));
                saw(&seen, &db, seed);
            }
            db
        }));
    }
    let mut dbs = Vec::new();
    for s in servers {
        dbs.push(s.await.unwrap());
    }
    let fresh = retry(async || Db::open(&link, "global/sim/main", key).await).await;
    // Server 0 is hacked: it sends the others, at each position their copies lack, a step of a kind that writes nothing.
    let mut forged = vec![Vec::new(); COPYING];
    for (i, copy) in copies.iter().enumerate().skip(1) {
        for n in 1..=fresh.at() {
            let name = one::db::step_name("global/sim/main", n);
            if copy.kept(&name).unwrap().is_none() {
                copy.keep(&name, &one::db::seal(&key, &name, b"x"), "0").unwrap();
                forged[i].push((name, "0".to_string()));
            }
        }
    }
    for (i, (db, copy)) in dbs.iter_mut().zip(&copies).enumerate() {
        // Read before the check: the forged steps are read as they are.
        retry(async || db.catch_up(copy).await).await;
        let mut wrong = Vec::new();
        while !walk(&root.join(i.to_string()).join(".sent")).is_empty() {
            wrong.extend(copy.check().await.unwrap());
        }
        // One the link failed is checked again later.
        wrong.sort();
        assert_eq!(wrong, forged[i], "seed {seed}: server {i} found each step server 0 forged, and only those");
        if !wrong.is_empty() {
            // As a node does: the database is read again, from its copy, now the bucket's.
            *db = retry(async || Db::open(copy, "global/sim/main", key).await).await;
        }
    }
    for (i, (db, copy)) in dbs.iter().zip(&copies).enumerate() {
        assert_eq!(state(db), state(&fresh), "seed {seed}: server {i} caught up with the bucket");
        // Its copy alone opens to a step some server read, with the key and only with it.
        let alone = Db::open(&copy.here_only(), "global/sim/main", key).await.unwrap_or_else(|e| panic!("seed {seed}: server {i}'s copy didn't open: {e:#}"));
        saw(&seen, &alone, seed);
        assert!(Db::open(&copy.here_only(), "global/sim/main", one::kernel::database_key(&[!(seed as u8); 32], "main")).await.is_err(), "seed {seed}: another key opened server {i}'s copy");
    }
    for (i, copy) in copies.iter().enumerate() {
        for name in walk(&root.join(i.to_string())).iter().filter_map(|f| f.strip_prefix(root.join(i.to_string())).ok()?.to_str().map(str::to_string)).filter(|n| !n.starts_with(".sent")) {
            assert_eq!(copy.kept(&name).unwrap(), link.objects.lock().unwrap().get(&name).cloned(), "seed {seed}: server {i}'s {name} is the bucket's");
        }
    }
    for file in walk(&root) {
        let bytes = std::fs::read(&file).unwrap();
        assert!(!bytes.windows(6).any(|w| w == b"writes"), "seed {seed}: {} isn't sealed", file.display());
    }
    std::fs::remove_dir_all(&root).unwrap();
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flat_map(|e| {
        let path = e.unwrap().path();
        if path.is_dir() { walk(&path) } else { vec![path] }
    }).collect()
}

#[test]
fn copies_on_servers_are_behind_never_wrong() {
    seeds(SEEDS / 4, run_copied);
}
