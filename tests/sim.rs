//! Many servers writing one app's database in one bucket over bad links: every request is slow by a random amount, so
//! they arrive out of order, and some fail, before or after the bucket did what they asked, or time out while still on
//! their way and land later. Some writes are given up on part way, as a call that runs out of time is, and what they
//! sent goes on. Every seed must end with every server reading the same database, holding every write it was told
//! landed, once, and nothing one attempt wrote twice. A failing seed is a bug. `ONE_SIM_SEEDS=<from>..<to>` sweeps more.
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
    /// Requests on their way, which go on when whoever made them stops waiting, as a server's do on the thread that
    /// makes them.
    going: Arc<std::sync::atomic::AtomicUsize>,
    /// Whether it fails nothing, to read what the bucket holds once every server is done.
    steady: bool,
}

impl Link {
    fn new(seed: u64) -> Link {
        Link { objects: Default::default(), rng: Arc::new(Mutex::new(StdRng::seed_from_u64(seed))), going: Default::default(), steady: false }
    }

    /// The same bucket over a link that fails nothing: a new server's read of it, which on a server's own copy goes
    /// on from what it read before when the link fails it.
    fn steady(&self) -> Link {
        Link { steady: true, ..self.clone() }
    }

    /// Sends a request: it waits on the way there, may fail before it reaches the bucket, may fail after, and its
    /// answer waits on the way back. Some time out while they're still on their way, and reach the bucket later.
    async fn send<T: 'static>(&self, does: impl FnOnce(&mut BTreeMap<String, Vec<u8>>) -> T + 'static) -> Result<T> {
        let (there, before, after, back, late) = {
            let mut rng = self.rng.lock().unwrap();
            let fails = if self.steady { 0.0 } else { 1.0 };
            (rng.gen_range(0..100), rng.gen_bool(0.05 * fails), rng.gen_bool(0.05 * fails), rng.gen_range(0..100), rng.gen_bool(0.03 * fails))
        };
        let (objects, going) = (self.objects.clone(), self.going.clone());
        going.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let request = tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(there)).await;
            let done = (!before).then(|| does(&mut objects.lock().unwrap()));
            tokio::time::sleep(Duration::from_millis(back)).await;
            going.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            match done {
                None => bail!("the link dropped it"),
                Some(_) if after => bail!("the link dropped the answer"),
                Some(done) => Ok(done),
            }
        });
        if late {
            tokio::time::sleep(Duration::from_millis(there / 2)).await;
            bail!("the link dropped it, or it's late");
        }
        request.await.unwrap()
    }

    /// Waits until no request is on its way.
    async fn settle(&self) {
        while self.going.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// How long a write is waited for before it's given up on, as a call that runs out of time or whose caller goes
    /// away is: mostly as long as it takes.
    fn patience(&self) -> Duration {
        let mut rng = self.rng.lock().unwrap();
        if rng.gen_bool(0.2) { Duration::from_millis(rng.gen_range(0..400)) } else { Duration::from_secs(3600) }
    }

    /// Writes, retried as `retry` does, each try a new attempt, given up on when the link's patience runs out: what it
    /// wrote may still land.
    async fn write<B: Bucket>(&self, db: &mut Db, bucket: &B, id: u64, server: usize) {
        retry(async || {
            let attempt = self.rng.lock().unwrap().r#gen::<i64>();
            tokio::time::timeout(self.patience(), db.write(bucket, write(id, server, attempt))).await.unwrap_or_else(|_| bail!("given up on"))
        })
        .await
    }
}

fn tag(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

impl Bucket for Link {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        let name = name.to_string();
        self.send(move |objects| objects.get(&name).map(|b| Object { bytes: b.clone(), tag: tag(b) })).await
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        let (name, bytes) = (name.to_string(), bytes.to_vec());
        self.send(move |objects| !objects.contains_key(&name) && objects.insert(name, bytes).is_none()).await
    }

    async fn replace(&self, name: &str, bytes: &[u8], tag_was: Option<&str>) -> Result<bool> {
        let (name, bytes, tag_was) = (name.to_string(), bytes.to_vec(), tag_was.map(str::to_string));
        self.send(move |objects| {
            let ok = objects.get(&name).map(|b| tag(b)) == tag_was;
            if ok {
                objects.insert(name, bytes);
            }
            ok
        })
        .await
    }
}

const SCHEMA: &str = "create table writes (id integer primary key, server integer not null); create table count (one integer primary key, n integer not null); insert into count values (1, 0); create table attempts (attempt integer primary key)";

/// One write: a row of its own, and the count moved by it, in one transaction. Written again after a failure it
/// changes nothing, so a write the link lost the answer to can be tried again; but each attempt lands at most once, or
/// its own row clashes.
fn write(id: u64, server: usize, attempt: i64) -> impl FnMut(&rusqlite::Connection) -> Result<()> {
    move |c| {
        execute(c, "insert into attempts values (?)", &[json!(attempt)])?;
        execute(c, "insert or ignore into writes values (?, ?)", &[json!(id), json!(server)])?;
        execute(c, "update count set n = n + changes()", &[])?;
        Ok(())
    }
}

/// Retries what the link fails, as a server does when a person tries again, until a link this bad can't be to blame.
async fn retry<T>(mut f: impl AsyncFnMut() -> Result<T>) -> T {
    let mut last = None;
    for _ in 0..1000 {
        match f().await {
            Ok(v) => return v,
            Err(err) if !format!("{err:#}").contains("the link dropped") && !format!("{err:#}").contains("given up on") && !format!("{err:#}").contains("changed too often") => panic!("it failed, not the link: {err:#}"),
            Err(err) => last = Some(err),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("a thousand tries failed, the last with {:#}", last.unwrap())
}

fn state(db: &Db) -> (Vec<(u64, u64)>, u64) {
    let rows = db.query("select id, server from writes order by id", &[]).unwrap().iter().map(|r| (r["id"].as_u64().unwrap(), r["server"].as_u64().unwrap())).collect();
    let n = db.query("select n from count", &[]).unwrap()[0]["n"].as_u64().unwrap();
    (rows, n)
}

async fn run(seed: u64) {
    let link = Link::new(seed);
    let key = one::kernel::database_key(&[seed as u8; 32], "main");
    let mut servers = Vec::new();
    for server in 0..SERVERS {
        let link = link.clone();
        servers.push(tokio::task::spawn_local(async move {
            let mut db = retry(async || Db::open(&link, "global/sim/main", key).await).await;
            retry(async || db.migrate(&link, &[SCHEMA.to_string()]).await).await;
            for i in 0..WRITES {
                let id = (server * WRITES + i) as u64;
                link.write(&mut db, &link, id, server).await;
            }
            retry(async || db.catch_up(&link).await).await;
            db
        }));
    }
    let mut dbs = Vec::new();
    for s in servers {
        dbs.push(s.await.unwrap());
    }
    link.settle().await;
    // Each reads to the end, as a new server does from nothing.
    for db in &mut dbs {
        retry(async || db.catch_up(&link).await).await;
    }
    let fresh = Db::open(&link.steady(), "global/sim/main", key).await.unwrap();
    let (rows, n) = state(&fresh);
    assert_eq!(rows.len(), SERVERS * WRITES, "seed {seed}: every write landed once");
    assert_eq!(n, rows.len() as u64, "seed {seed}: the count moved once for each write, however often the link lost an answer");
    for (i, db) in dbs.iter().enumerate() {
        assert_eq!(state(db), (rows.clone(), n), "seed {seed}: server {i} reads what a new server does");
    }
    assert!(link.objects.lock().unwrap().keys().any(|k| k.contains("/copy/")), "seed {seed}: copies were written as steps piled up");
}

/// Runs each seed on a paused clock of its own, as many at once as there are cores: each is its own run, so the order
/// doesn't matter. `ONE_SIM_SEEDS=<from>..<to>` runs those seeds instead, and `ONE_SIM_THREADS` how many at once.
fn seeds<F: Future<Output = ()>>(count: u64, run: fn(u64) -> F) {
    let range = match std::env::var("ONE_SIM_SEEDS") {
        Ok(r) => {
            let (from, to) = r.split_once("..").expect("ONE_SIM_SEEDS is <from>..<to>");
            from.parse().unwrap()..to.parse().unwrap()
        }
        Err(_) => 0..count,
    };
    let threads = std::env::var("ONE_SIM_THREADS").map(|t| t.parse().unwrap()).unwrap_or_else(|_| std::thread::available_parallelism().map_or(4, |n| n.get()));
    let next = std::sync::atomic::AtomicU64::new(range.start);
    let failed = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let seed = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if seed >= range.end {
                    break;
                }
                let runtime = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().unwrap();
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.block_on(tokio::task::LocalSet::new().run_until(run(seed))))).is_err() {
                    failed.lock().unwrap().push(seed);
                }
            });
        }
    });
    let mut failed = failed.into_inner().unwrap();
    failed.sort();
    assert!(failed.is_empty(), "seeds {failed:?} failed");
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
    let link = Link::new(seed);
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
                link.write(&mut db, here, id, server).await;
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
    link.settle().await;
    let fresh = Db::open(&link.steady(), "global/sim/main", key).await.unwrap();
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
