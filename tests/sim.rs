//! Many servers writing one app's database in one bucket over bad links: every request is slow by a random amount, so
//! they arrive out of order, and some fail, before or after the bucket did what they asked. Every seed must end with
//! every server reading the same database, holding every write it was told landed, once. A failing seed is a bug.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use one::bucket::{Bucket, Object};
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

#[test]
fn many_servers_write_one_database_over_bad_links() {
    for seed in 0..SEEDS {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().unwrap();
        runtime.block_on(tokio::task::LocalSet::new().run_until(run(seed)));
    }
}
