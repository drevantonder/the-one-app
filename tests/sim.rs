//! Many devices writing one app's database in one bucket over bad links: every request is slow by a random amount, so
//! they arrive out of order, and some fail, before or after the bucket did what they asked, or time out while still on
//! their way and land later; now and then the bucket is out of reach for up to a minute and a half, or refuses to write
//! for as long. Some writes are given up on part way, as a call that runs out of time is, and what they sent goes on.
//! Every seed must end with every device reading the same database, holding every write it was told landed, once, none
//! it was told failed, and nothing one attempt wrote twice; and a write says it may have been made only when the bucket
//! left one of its requests unanswered. A failing seed is a bug. `ONE_SIM_SEEDS=<from>..<to>` sweeps more.
//!
//! Then the same with each device keeping a copy on its disk, sent some of the others' steps as they're written (some
//! of them forged: other bytes at that step, or at the next), keeping each only once the bucket confirms it and
//! otherwise fetching from the bucket into its copy, and reading from its copy alone in between: what it reads may be
//! behind, but never anything no device read at the same step, nothing the bucket doesn't hold is kept, and the copy
//! opens nothing without the key.
//!
//! Each device signs its steps, and every signature a reader finds beside a step must hold.
//!
//! In odd seeds the scope's epoch changes midway: each device is handed epoch 2 when it first meets a step under it,
//! and one device never is, writing under epoch 1 after it all the same: what it writes after is passed over, and
//! nothing the others wrote is lost.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use one::bucket::{Bucket, Copied, Failed, Object, Trouble};
use one::db::{Db, Keys, Missing, execute};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::json;

const DEVICES: usize = 8;
const WRITES: usize = 30;
/// Seeds every `cargo test` runs: enough to catch what fails often, few enough that the many runs on one machine stay
/// quick. What fails rarely, a sweep of `ONE_SIM_SEEDS` finds.
const SEEDS: u64 = 16;

/// One bucket, reached over links that delay every request and fail some.
#[derive(Clone)]
struct Link {
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    rng: Arc<Mutex<StdRng>>,
    /// Requests on their way, which go on when whoever made them stops waiting, as a device's do on the thread that
    /// makes them.
    going: Arc<std::sync::atomic::AtomicUsize>,
    /// Whether it fails nothing, to read what the bucket holds once every device is done.
    steady: bool,
    /// Until when the bucket is out of reach.
    down: Arc<Mutex<tokio::time::Instant>>,
    /// Until when it refuses to write, as it does a key that may not, or URLs the vault won't sign.
    refusing: Arc<Mutex<tokio::time::Instant>>,
    /// The attempts at writing that were told they failed, and not that they may have landed.
    failed: Arc<Mutex<Vec<i64>>>,
}

impl Link {
    fn new(seed: u64) -> Link {
        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed)));
        let now = || Arc::new(Mutex::new(tokio::time::Instant::now()));
        Link { objects: Default::default(), rng, going: Default::default(), steady: false, down: now(), refusing: now(), failed: Default::default() }
    }

    /// The same bucket over a link that fails nothing: a new device's read of it, which on a device's own copy goes
    /// on from what it read before when the link fails it.
    fn steady(&self) -> Link {
        Link { steady: true, ..self.clone() }
    }

    /// Sends a request: it waits on the way there, may fail before it reaches the bucket, may fail after, and its
    /// answer waits on the way back. Some time out while they're still on their way, and reach the bucket later, often
    /// as the bucket goes out of reach, which nothing else reaches until it's back.
    async fn send<T: 'static>(&self, does: impl FnOnce(&mut BTreeMap<String, Vec<u8>>) -> T + 'static) -> Result<T> {
        let (there, before, after, back, late, outage) = {
            let mut rng = self.rng.lock().unwrap();
            let fails = if self.steady { 0.0 } else { 1.0 };
            let (there, before, after, back, late) = (rng.gen_range(0..100), rng.gen_bool(0.05 * fails), rng.gen_bool(0.05 * fails), rng.gen_range(0..100), rng.gen_bool(0.03 * fails));
            let outage = rng.gen_bool(if late { 0.05 } else { 0.0001 } * fails).then(|| rng.gen_range(1_000..90_000));
            (there, before, after, back, late, outage)
        };
        if let Some(ms) = outage {
            let mut down = self.down.lock().unwrap();
            let now = tokio::time::Instant::now();
            if *down <= now {
                *down = now + Duration::from_millis(ms);
            }
        }
        let (objects, going, down, steady) = (self.objects.clone(), self.going.clone(), self.down.clone(), self.steady);
        going.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let request = tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(there)).await;
            let reached = !before && (late || steady || tokio::time::Instant::now() >= *down.lock().unwrap());
            let done = reached.then(|| does(&mut objects.lock().unwrap()));
            tokio::time::sleep(Duration::from_millis(back)).await;
            going.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            match done {
                None => Err(dropped("the link dropped it")),
                Some(_) if after => Err(dropped("the link dropped the answer")),
                Some(done) => Ok(done),
            }
        });
        if late {
            tokio::time::sleep(Duration::from_millis(there / 2)).await;
            return Err(dropped("the link dropped it, or it's late"));
        }
        request.await.unwrap()
    }

    /// Checks that no attempt told it failed was written after all.
    fn none_failed_landed(&self, db: &Db, seed: u64) {
        let landed: Vec<i64> = db.query("select attempt from attempts", &[]).unwrap().iter().map(|r| r["attempt"].as_i64().unwrap()).collect();
        for attempt in self.failed.lock().unwrap().iter() {
            assert!(!landed.contains(attempt), "seed {seed}: a write told it failed was written");
        }
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
    /// wrote may still land. Fails only with an epoch it wasn't given.
    async fn write<B: Bucket>(&self, db: &mut Db, bucket: &B, id: u64, device: usize) -> Result<(), Missing> {
        retry(async || {
            let attempt = self.rng.lock().unwrap().r#gen::<i64>();
            let (wrote, unanswered) = UNANSWERED
                .scope(Default::default(), async {
                    let wrote = tokio::time::timeout(self.patience(), db.write(bucket, write(id, device, attempt))).await.unwrap_or_else(|_| bail!("given up on"));
                    (wrote, UNANSWERED.with(|u| u.get()))
                })
                .await;
            if let Err(e) = &wrote {
                assert!(unanswered || !format!("{e:#}").contains("may have been"), "a write said it may have been made, but the bucket answered all it sent: {e:#}");
            }
            if wrote.as_ref().is_err_and(|e| !format!("{e:#}").contains("given up on") && !format!("{e:#}").contains("may have been")) {
                self.failed.lock().unwrap().push(attempt);
            }
            match wrote {
                Err(e) if e.is::<Missing>() => Ok(Err(*e.downcast_ref::<Missing>().unwrap())),
                // A step under an epoch newer than it was given: its node finds that epoch's entry, and hands it the key.
                Err(e) if e.is::<one::db::Ahead>() => Ok(Err(Missing { epoch: e.downcast_ref::<one::db::Ahead>().unwrap().epoch })),
                other => other.map(Ok),
            }
        })
        .await
    }
}

tokio::task_local! {
    /// Whether a create the attempt running sent went unanswered.
    static UNANSWERED: std::cell::Cell<bool>;
}

/// A request that failed on the link, with no answer from the bucket: it may have done what it asked.
fn dropped(what: &str) -> anyhow::Error {
    Failed(Trouble::Unreachable, what.into()).into()
}

fn tag(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

impl Bucket for Link {
    async fn get(&self, name: &str) -> Result<Option<Object>> {
        let name = name.to_string();
        self.send(move |objects| objects.get(&name).map(|b| Object { bytes: b.clone(), tag: tag(b) })).await
    }

    async fn wait(&self, ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<bool> {
        let now = tokio::time::Instant::now();
        {
            let mut rng = self.rng.lock().unwrap();
            let mut refusing = self.refusing.lock().unwrap();
            if !self.steady && *refusing <= now && rng.gen_bool(0.002) {
                *refusing = now + Duration::from_millis(rng.gen_range(1_000..90_000));
            }
            if now < *refusing {
                return Err(Failed(Trouble::Refused, "won't let this device write".into()).into());
            }
        }
        let (name, bytes) = (name.to_string(), bytes.to_vec());
        let created = self.send(move |objects| !objects.contains_key(&name) && objects.insert(name, bytes).is_none()).await;
        if created.is_err() {
            let _ = UNANSWERED.try_with(|u| u.set(true));
        }
        created
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

const SCHEMA: &str = "create table writes (id integer primary key, device integer not null); create table count (one integer primary key, n integer not null); insert into count values (1, 0); create table attempts (attempt integer primary key)";

/// One write: a row of its own, and the count moved by it, in one transaction. Written again after a failure it
/// changes nothing, so a write the link lost the answer to can be tried again; but each attempt lands at most once, or
/// its own row clashes.
fn write(id: u64, device: usize, attempt: i64) -> impl FnMut(&rusqlite::Connection) -> Result<()> {
    move |c| {
        execute(c, "insert into attempts values (?)", &[json!(attempt)])?;
        execute(c, "insert or ignore into writes values (?, ?)", &[json!(id), json!(device)])?;
        execute(c, "update count set n = n + changes()", &[])?;
        Ok(())
    }
}

/// Retries what the link fails, as a device does when a person tries again, until a link this bad can't be to blame.
async fn retry<T>(mut f: impl AsyncFnMut() -> Result<T>) -> T {
    let mut last = None;
    for _ in 0..10_000 {
        match f().await {
            Ok(v) => return v,
            Err(err) if !format!("{err:#}").contains("the link dropped") && !format!("{err:#}").contains("given up on") && !format!("{err:#}").contains("changed too often") && !format!("{err:#}").contains("may have been") && !format!("{err:#}").contains("won't let") => panic!("it failed, not the link: {err:#}"),
            Err(err) => last = Some(err),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("ten thousand tries failed, the last with {:#}", last.unwrap())
}

/// Checks that every signature a reader found beside a step held, however the link mangled the writes: each is written
/// only once its step is known to be there, with the state its writer read. A link may lose one, but not all.
fn signed(link: &Link, db: &Db, seed: u64) {
    let checks = db.checks();
    assert_eq!(checks.wrong, 0, "seed {seed}: a signature beside a step didn't hold: {checks:?}");
    assert!(link.objects.lock().unwrap().keys().any(|n| n.ends_with(".sig")), "seed {seed}: no step was signed");
}

fn state(db: &Db) -> (Vec<(u64, u64)>, u64) {
    let rows = db.query("select id, device from writes order by id", &[]).unwrap().iter().map(|r| (r["id"].as_u64().unwrap(), r["device"].as_u64().unwrap())).collect();
    let n = db.query("select n from count", &[]).unwrap()[0]["n"].as_u64().unwrap();
    (rows, n)
}

/// A database's keys in epoch 1, and in epochs 1 and 2.
fn keys(seed: u64) -> (Keys, Keys) {
    let (one, two) = (one::kernel::database_key(&[seed as u8; 32], "main"), one::kernel::database_key(&[!(seed as u8); 32], "main"));
    (Keys::from([(1, one)]), Keys::from([(1, one), (2, two)]))
}

/// Opens the database and brings it to the schema as a device does: handed `newer`, epoch 2, when it meets a step under
/// it, as its node is once it finds that epoch's entry; never handed it, it doesn't open.
async fn open(link: &Link, older: &Keys, newer: Option<&Keys>) -> Option<Db> {
    let ahead = |e: &anyhow::Error| e.is::<one::db::Ahead>();
    let mut keys = older.clone();
    let mut db = loop {
        match retry(async || match Db::open(link, "global/sim/main", keys.clone()).await {
            Err(e) if ahead(&e) && keys.len() == 1 => Ok(None),
            other => other.map(Some),
        })
        .await
        {
            Some(db) => break db,
            None => keys = newer?.clone(),
        }
    };
    db.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
    while !retry(async || match db.migrate(link, &[SCHEMA.to_string()]).await {
        Err(e) if ahead(&e) && db.keys().len() == 1 => Ok(false),
        other => other.map(|()| true),
    })
    .await
    {
        db.add_keys(newer?);
    }
    Some(db)
}

/// Writes `id` as `device`, from epoch 2 on once it's past `from` or meets a step under it, as a device is handed a
/// scope's new epoch.
async fn write_handed(link: &Link, db: &mut Db, id: u64, device: usize, from: bool, newer: &Keys) {
    if from {
        db.add_keys(newer);
    }
    if link.write(db, link, id, device).await.is_err() {
        db.add_keys(newer);
        link.write(db, link, id, device).await.expect("given every epoch");
    }
}

/// A step under epoch 1, written by a device that wasn't handed epoch 2, at the first free position: its own step at
/// `at` again, so applied twice it would move the count twice.
async fn stale_step(link: &Link, key: &[u8; 32], at: u64) {
    let name = one::db::step_name("global/sim/main", at);
    let step = one::keys::open(key, &name, &link.objects.lock().unwrap()[&name]).unwrap();
    let mut objects = link.objects.lock().unwrap();
    let free = (at + 1..).find(|n| !objects.contains_key(&one::db::step_name("global/sim/main", *n))).unwrap();
    let name = one::db::step_name("global/sim/main", free);
    objects.insert(name.clone(), one::keys::seal(1, key, &name, &step));
}

async fn run(seed: u64) {
    let link = Link::new(seed);
    let changes = seed % 2 == 1;
    let (older, newer) = keys(seed);
    let mut devices = Vec::new();
    for device in 0..DEVICES {
        let (link, older, newer) = (link.clone(), older.clone(), newer.clone());
        devices.push(tokio::task::spawn_local(async move {
            let mut db = open(&link, &older, Some(&newer)).await.expect("handed every epoch");
            for i in 0..WRITES {
                let id = (device * WRITES + i) as u64;
                write_handed(&link, &mut db, id, device, changes && i >= WRITES / 2, &newer).await;
            }
            retry(async || db.catch_up(&link).await).await;
            db
        }));
    }
    // One never handed epoch 2: it writes until it meets a step under it, then under epoch 1 all the same.
    let stale = changes.then(|| {
        let (link, older) = (link.clone(), older.clone());
        tokio::task::spawn_local(async move {
            let mut db = open(&link, &older, None).await?;
            let mut own = None;
            for i in 0..WRITES {
                let at = db.at();
                if link.write(&mut db, &link, (DEVICES * WRITES + i) as u64, DEVICES).await.is_err() {
                    break;
                }
                own = own.or((db.at() > at).then(|| db.at()));
            }
            own
        })
    });
    let mut dbs = Vec::new();
    for s in devices {
        dbs.push(s.await.unwrap());
    }
    let mut extra = 0;
    if let Some(stale) = stale {
        let own = stale.await.unwrap();
        link.settle().await;
        if let Some(at) = own {
            stale_step(&link, &older[&1], at).await;
            // And one more after it, from a device that was.
            write_handed(&link, &mut dbs[0], u64::MAX >> 1, 0, true, &newer).await;
            extra = 1;
        }
    }
    link.settle().await;
    // Each reads to the end, as a new device does from nothing.
    for db in &mut dbs {
        retry(async || db.catch_up(&link).await).await;
    }
    let fresh = Db::open(&link.steady(), "global/sim/main", if changes { newer } else { older }).await.unwrap();
    link.none_failed_landed(&fresh, seed);
    signed(&link, &fresh, seed);
    let (rows, n) = state(&fresh);
    assert_eq!(rows.iter().filter(|r| r.1 < DEVICES as u64).count(), DEVICES * WRITES + extra, "seed {seed}: every write landed once");
    assert_eq!(n, rows.len() as u64, "seed {seed}: the count moved once for each write, however often the link lost an answer");
    for (i, db) in dbs.iter().enumerate() {
        assert_eq!(state(db), (rows.clone(), n), "seed {seed}: device {i} reads what a new device does");
        assert_eq!(db.checks().wrong, 0, "seed {seed}: device {i} read a signature that didn't hold");
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
fn many_devices_write_one_database_over_bad_links() {
    seeds(SEEDS, run);
}

const COPYING: usize = 4;

/// What devices read at each step, which every read at that step must match.
type Seen = Arc<Mutex<BTreeMap<u64, (Vec<(u64, u64)>, u64)>>>;

fn saw(seen: &Seen, db: &Db, seed: u64) {
    let read = state(db);
    let mut seen = seen.lock().unwrap();
    let first = seen.entry(db.at()).or_insert_with(|| read.clone());
    assert_eq!(*first, read, "seed {seed}: two reads at step {} differ", db.at());
}

async fn run_copied(seed: u64) {
    let link = Link::new(seed);
    let key = keys(seed).0;
    let root = crate::temp(&format!("sim-{seed}"));
    let copies: Vec<Copied<Link>> = (0..COPYING).map(|s| Copied::new(link.clone(), root.join(s.to_string()))).collect();
    let seen = Seen::default();
    let mut devices = Vec::new();
    for device in 0..COPYING {
        let (link, copies, seen, key) = (link.clone(), copies.clone(), seen.clone(), key.clone());
        devices.push(tokio::task::spawn_local(async move {
            let here = &copies[device];
            let mut db = retry(async || Db::open(here, "global/sim/main", key.clone()).await).await;
            db.sign_as(ed25519_dalek::SigningKey::from_bytes(&rand::random()));
            retry(async || db.migrate(here, &[SCHEMA.to_string()]).await).await;
            for i in 0..WRITES {
                let id = (device * WRITES + i) as u64;
                let at = db.at();
                link.write(&mut db, here, id, device).await.expect("one epoch");
                // Its step reaches some of the others, some of them forged on the way, and each keeps what the bucket
                // confirms, fetching the rest from the bucket, if the link lets it.
                if db.at() > at {
                    let (name, next) = (one::db::step_name("global/sim/main", db.at()), one::db::step_name("global/sim/main", db.at() + 1));
                    let step = here.kept(&name).unwrap().expect("a step written is kept");
                    let first = &key[&1];
                    let plain = one::keys::open(first, &name, &step).unwrap();
                    for (_, copy) in copies.iter().enumerate().filter(|(o, _)| *o != device) {
                        let (sent, forged, ahead) = {
                            let mut rng = link.rng.lock().unwrap();
                            (rng.gen_bool(0.6), rng.gen_bool(0.2), rng.gen_bool(0.2))
                        };
                        if !sent {
                            continue;
                        }
                        let mut steps = vec![(name.clone(), if forged { one::keys::seal(1, first, &name, &plain) } else { step.clone() })];
                        if ahead {
                            steps.push((next.clone(), one::keys::seal(1, first, &next, &plain)));
                        }
                        if !copy.take_sent(&steps).await.is_ok_and(|took| took[0]) {
                            let _ = copy.fill("global/sim/main", 0).await;
                        }
                    }
                }
                // A read from its copy alone, as when the bucket is out of reach.
                db.catch_up(&here.here_only()).await.unwrap_or_else(|e| panic!("seed {seed}: device {device}'s copy failed it: {e:#}"));
                saw(&seen, &db, seed);
            }
            db
        }));
    }
    let mut dbs = Vec::new();
    for s in devices {
        dbs.push(s.await.unwrap());
    }
    link.settle().await;
    let fresh = Db::open(&link.steady(), "global/sim/main", key.clone()).await.unwrap();
    link.none_failed_landed(&fresh, seed);
    signed(&link, &fresh, seed);
    for (db, copy) in dbs.iter_mut().zip(&copies) {
        retry(async || db.catch_up(copy).await).await;
    }
    for (i, (db, copy)) in dbs.iter().zip(&copies).enumerate() {
        assert_eq!(state(db), state(&fresh), "seed {seed}: device {i} caught up with the bucket");
        assert_eq!(db.checks().wrong, 0, "seed {seed}: device {i} read a signature that didn't hold");
        // Its copy alone opens to a step some device read, with the key and only with it.
        let alone = Db::open(&copy.here_only(), "global/sim/main", key.clone()).await.unwrap_or_else(|e| panic!("seed {seed}: device {i}'s copy didn't open: {e:#}"));
        saw(&seen, &alone, seed);
        let other = Keys::from([(1, one::kernel::database_key(&[!(seed as u8); 32], "main"))]);
        assert!(Db::open(&copy.here_only(), "global/sim/main", other).await.is_ok_and(|db| db.version().unwrap() == 0), "seed {seed}: another key opened device {i}'s copy");
    }
    for (i, copy) in copies.iter().enumerate() {
        for name in walk(&root.join(i.to_string())).iter().filter_map(|f| f.strip_prefix(root.join(i.to_string())).ok()?.to_str().map(str::to_string)) {
            assert_eq!(copy.kept(&name).unwrap(), link.objects.lock().unwrap().get(&name).cloned(), "seed {seed}: device {i}'s {name} is the bucket's");
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
fn copies_on_devices_are_behind_never_wrong() {
    seeds(SEEDS / 2, run_copied);
}
