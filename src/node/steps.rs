//! `global`'s scope database (`database::steps`): reading the steps others wrote in the bucket, from the freeze the
//! network's recovery key signed, and writing this device's own and those its people's other devices sign. What this
//! device has read of each it keeps in its folder, checked against its hash when it starts again (`scope-kept/`).

use super::*;
use crate::database::steps;

impl Node {
    /// Whether this device has read who's who: `global`'s freeze, and the steps after it.
    pub fn frozen(&self) -> bool {
        self.with(|i| Ok(i.r.frozen.is_some())).unwrap_or(false)
    }

    /// Reads `global`'s steps in the bucket after the last this device applied: each applied if it counts, passed over
    /// if not. Before it has read the freeze, it looks at each for it. A step under an epoch above every key this
    /// device opens waits for that epoch's key; one above every epoch made is junk, passed over. → whether there were
    /// any.
    pub(super) async fn read_steps(&self) -> Result<bool> {
        let _write = self.0.writes.lock().await;
        self.read_steps_now().await
    }

    /// `read_steps`, while this device holds its writes.
    pub(super) async fn read_steps_now(&self) -> Result<bool> {
        let bucket = self.bucket()?;
        let prefix = steps::prefix("global");
        let from = self.with(|i| Ok(i.r.at().unwrap_or(i.r.looked)))?;
        let mut at = from;
        loop {
            let name = crate::db::step_name(&prefix, at + 1);
            let keys = match self.log_key(&Scope::Global).await {
                Ok(keys) => keys,
                // Not given `global`'s newest epoch yet, as between its own new epoch's step and its seals: it reads
                // with those it holds, and a step under the newest waits for it, as one under any epoch not handed yet.
                Err(err) if err.is::<NotGiven>() => self.log_key_of(&self.epochs_held(&Scope::Global).await?.0).await?,
                Err(err) => return Err(err),
            };
            let Some(o) = crate::bucket::Bucket::get(&bucket, &name).await? else {
                steps::missing(&bucket, &keys, &Scope::Global, at + 1).await?;
                break;
            };
            let given = match steps::open(&keys, u32::MAX, &Scope::Global, at + 1, &o.bytes) {
                steps::Opened::Step(given) => Some(given),
                steps::Opened::Junk => None,
                steps::Opened::Waits(epoch) if !self.frozen() || self.with(|i| Ok(i.r.epoch_made(&Scope::Global)))? >= epoch => {
                    // Made, but not handed to this device yet, or before it's read the freeze: it reads on once it is.
                    self.forget_epochs(&Scope::Global);
                    break;
                }
                steps::Opened::Waits(_) => None,
            };
            let handled = self.with(|i| match i.r.frozen {
                Some(_) => i.r.take_step(at + 1, given.as_ref().map(|g| &g.step), now_ms()),
                None => i.r.take_first(at + 1, given.as_ref(), now_ms()),
            })?;
            at += 1;
            self.after(handled).await?;
        }
        if at > from {
            self.keep_global();
        }
        Ok(at > from)
    }

    /// `global`'s frozen log to step `end`, where its freeze says it ends (`steps::log_to`), read only: kept while this
    /// device runs, for each other scope's database to start from.
    async fn global_log(&self, end: u64) -> Result<Arc<steps::Log>> {
        if let Some(log) = self.0.frozen_log.lock().unwrap().clone().filter(|l| l.last() >= end) {
            return Ok(log);
        }
        let root = self.with(|i| Ok(i.r.root))?;
        let log = Arc::new(steps::log_to(&self.bucket()?, &self.log_key(&Scope::Global).await?, root, end).await?);
        *self.0.frozen_log.lock().unwrap() = Some(log.clone());
        Ok(log)
    }

    /// Where this device read `global`'s scope database to, and the database there, kept in its folder: what it starts
    /// again from (`start_kept`), never the entries again.
    fn keep_global(&self) {
        match self.with(|i| i.r.keep()) {
            Ok(Some(kept)) => self.keep_scope_db(&Scope::Global, &kept),
            Ok(None) => {}
            Err(err) => tracing::warn!("keeping who's who: {err:#}"),
        }
    }

    fn kept_path(&self, scope: &Scope) -> PathBuf {
        self.network_dir().join("scope-kept").join(data_encoding::HEXLOWER.encode(scope.to_string().as_bytes()))
    }

    fn keep_scope_db(&self, scope: &Scope, kept: &Value) {
        let path = self.kept_path(scope);
        if let Err(err) = std::fs::create_dir_all(path.parent().expect("a folder")).map_err(anyhow::Error::from).and_then(|_| write_private(&path, kept.to_string().as_bytes()).map_err(Into::into)) {
            tracing::warn!("keeping {scope}'s database: {err:#}");
        }
    }

    fn kept_scope_db(&self, scope: &Scope) -> Option<Value> {
        serde_json::from_slice(&std::fs::read(self.kept_path(scope)).ok()?).ok()
    }

    /// As this device starts: `global`'s scope database and each other scope's as it kept them, each checked against
    /// the hash it kept with it. One that doesn't check out is read again from the bucket.
    pub(super) fn start_kept(&self) {
        let Some(kept) = self.kept_scope_db(&Scope::Global) else { return };
        if let Err(err) = self.with(|i| i.r.restart(&kept, now_ms())) {
            tracing::warn!("who's who this device kept: {err:#}: it's read again from the bucket");
            return;
        }
        let Ok(dir) = std::fs::read_dir(self.network_dir().join("scope-kept")) else { return };
        for file in dir.flatten() {
            let Some(scope) = data_encoding::HEXLOWER.decode(file.file_name().as_encoded_bytes()).ok().and_then(|b| String::from_utf8(b).ok()).and_then(|s| Scope::parse(&s).ok()) else { continue };
            if scope == Scope::Global {
                continue;
            }
            let Some(kept) = self.kept_scope_db(&scope) else { continue };
            let opened = (|| -> Result<(u64, ScopeDb)> {
                let bytes = data_encoding::BASE64.decode(kept["db"].as_str().context("no database")?.as_bytes())?;
                let conn = steps::connection(&bytes)?;
                ensure!(Some(crate::db::state_hash(&conn)?.as_str()) == kept["hash"].as_str(), "it isn't what it was");
                Ok((kept["at"].as_u64().context("no position")?, steps::normalize(conn, &scope, now_ms())?))
            })();
            match opened {
                Ok(db) => drop(self.with(|i| Ok(i.scope_dbs.insert(scope.clone(), db)))),
                Err(err) => tracing::warn!("{scope}'s database this device kept: {err:#}: it's read again from the bucket"),
            }
        }
    }

    /// Writes `content` to `global`'s scope database as a step this device signs as `user`, at the next position: when
    /// another device wrote there first, it reads that, and signs it again for the one after.
    pub(super) async fn write_step(&self, user: &str, content: Content) -> Result<Entry> {
        self.write_step_as(&self.0.key, user, content).await
    }

    /// `write_step`, signed by `key`: this device's, or the network's recovery key, which only ever signs on the machine
    /// it's kept on.
    pub(super) async fn write_step_as(&self, key: &SigningKey, user: &str, content: Content) -> Result<Entry> {
        let _write = self.own_writes().await?;
        for _ in 0..50 {
            self.read_steps_now().await?;
            let step = self.with(|i| i.r.make_step(key, user, content.clone(), now_ms()))?;
            if self.put_step(&step).await? {
                return Ok(step);
            }
        }
        bail!("who's who changed too often to write to")
    }

    /// Writes a step its device signed to the bucket at its position, once it would count there, and applies it:
    /// whether the position was free. Its blobs go first, so a device that reads the step finds them.
    async fn put_step(&self, step: &Entry) -> Result<bool> {
        let at = u64::from(step.seq());
        ensure!(self.with(|i| Ok(i.r.at()))? == Some(at - 1), "step {at} isn't the next here");
        for blob in &step.content.blobs {
            self.share_blob(&Scope::Global, blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
        }
        let keys = self.log_key(&Scope::Global).await?;
        let name = crate::db::step_name(&steps::prefix("global"), at);
        if !crate::bucket::Bucket::create(&self.bucket()?, &name, &crate::log::seal(&keys, &name, &steps::encode(step, None))?).await? {
            return Ok(false);
        }
        let handled = self.with(|i| i.r.take_step(at, Some(step), now_ms()))?;
        self.after(handled).await?;
        self.keep_global();
        // A copy now and then, and the newest hint: only saves readers work.
        let bytes = self.with(|i| i.r.db_mut().bytes())?;
        if let Err(err) = steps::written(&self.bucket()?, &keys, &Scope::Global, at, bytes).await {
            tracing::debug!("global's copy and hint: {err:#}");
        }
        self.ping(&Scope::Global, None);
        Ok(true)
    }

    /// Writes steps `caller`'s own device signed (a browser's, say), each once it's checked: it's for the next
    /// position here, written as them, and counts. A step for a position already taken fails, saying the next.
    pub async fn put_signed_steps(&self, caller: &Caller, list: Vec<Entry>) -> Result<()> {
        self.read_callers_logs(caller).await?;
        for step in list {
            ensure!(step.user() == caller.user, "step {} isn't written as you", step.seq());
            let _write = self.0.writes.lock().await;
            self.read_steps_now().await?;
            let next = self.with(|i| Ok(i.r.at()))?.context("who's who here isn't written as steps yet: update this device")? + 1;
            ensure!(u64::from(step.seq()) == next, "{} is taken: sign it again for {next}", step.seq());
            self.with(|i| steps::apply(i.r.db(), None, &Scope::Global, &step).map(drop))?;
            ensure!(self.put_step(&step).await?, "{} is taken: sign it again for {}", step.seq(), next + 1);
        }
        Ok(())
    }

    /// `global`'s steps after `from` as the bucket holds them, sealed, each `{ at, sealed }` (base64url), its freeze
    /// among them: a client device opens them with its own keys and applies them itself, never taking who's who, nor
    /// that a step isn't one, from a node. With `{ at }`, the last there, after which a writer signs its own. Fails
    /// before the log is frozen here, so a writer writes entries. For a device that starts from its `welcome`
    /// (`steps::Welcome`), those after the copy before its step instead, with that copy and the freeze's step, sealed:
    /// `{ welcome, freeze: { at, sealed }, start: { at, sealed } }`, without `start` if the copy isn't there.
    pub async fn global_steps(&self, caller: &Caller, from: u64, welcome: Option<steps::Welcome>) -> Result<Value> {
        self.read_steps().await?;
        let read = self.with(|i| {
            ensure!(i.r.db().may(&caller.user, "use", scopes::NETWORK), "you aren't one of this network's people");
            Ok(i.r.at())
        })?;
        let read = read.context("global's log isn't frozen")?;
        let bucket = self.bucket()?;
        let prefix = steps::prefix("global");
        let sealed = |at: u64, o: crate::bucket::Object| json!({ "at": at, "sealed": data_encoding::BASE64URL_NOPAD.encode(&o.bytes) });
        let mut out = json!({});
        let mut from = from;
        if let Some(w) = welcome.filter(|w| w.at <= read)
            && let Some(freeze) = crate::bucket::Bucket::get(&bucket, &crate::db::step_name(&prefix, w.freeze)).await?
        {
            out = json!({ "welcome": w.to_string(), "freeze": sealed(w.freeze, freeze) });
            from = w.freeze;
            if let Some(c) = w.copy()
                && let Some(copy) = crate::bucket::Bucket::get(&bucket, &crate::db::copy_name(&prefix, c)).await?
            {
                out["start"] = sealed(c, copy);
                from = c;
            }
        }
        let mut list = Vec::new();
        let mut at = from.min(read);
        while let Some(o) = crate::bucket::Bucket::get(&bucket, &crate::db::step_name(&prefix, at + 1)).await? {
            at += 1;
            if at > from {
                list.push(sealed(at, o));
            }
        }
        out["at"] = json!(at);
        out["steps"] = json!(list);
        Ok(out)
    }

    /// Reads a scope's own steps after the last this device applied, once `global`'s log is frozen: from the database
    /// its log's entries up to the freeze's end of it add up to (`steps::scope_anchor`), worked out here the first time
    /// and kept, then each applied if it counts there with `global`'s scope database as it is now. Its state is the
    /// scope's view from then on. One whose start can't be worked out yet is tried again the next time.
    pub(super) async fn read_scope_steps_now(&self, scope: &Scope) -> Result<bool> {
        if *scope == Scope::Global || !self.frozen() || !self.with(|i| Ok(i.r.holds(scope)))? {
            return Ok(false);
        }
        let keys = self.log_key(scope).await?;
        let bucket = self.bucket()?;
        if !self.with(|i| Ok(i.scope_dbs.contains_key(scope)))? {
            let (freeze, root) = self.with(|i| Ok((i.r.frozen.clone().map(|(f, _)| f), i.r.root)))?;
            let freeze = freeze.context("global's log isn't frozen")?;
            let global = self.global_log(freeze.global()).await?;
            let entries = steps::anchor_entries(&bucket, &keys, &freeze, root, &global, scope).await?;
            let db = steps::scope_anchor(&freeze, root, &global, scope, &entries, now_ms())?;
            self.with(|i| Ok(i.scope_dbs.insert(scope.clone(), (0, db))))?;
        }
        let mut read = Vec::new();
        loop {
            let from = self.with(|i| Ok(i.scope_dbs.get(scope).map_or(0, |(at, _)| *at)))? + read.len() as u64;
            let more = steps::read(&bucket, &keys, scope, from).await?;
            read.extend(more.steps.iter().cloned());
            // Its epochs are made in `global`, so one above them all is junk.
            if !more.junk_at_end(self.with(|i| Ok(i.r.db().epoch(scope)))?) {
                break;
            }
            read.push(None);
        }
        let (kept, newer, forgotten) = self.with(|i| {
            let (at, db) = i.scope_dbs.remove(scope).context("no database")?;
            let steps::TookScope { at, mut db, newer } = steps::take_scope(at, db, i.r.db(), scope, &read, now_ms())?;
            let (position, was) = i.views.get(&scope.to_string()).map(|(p, s)| (p.clone(), s.forgotten.clone())).unwrap_or(((0, Hash::from_bytes([0; 32])), Default::default()));
            let state = db.state();
            // The blobs its steps said its apps no longer need, as `forget` entries did.
            let forgotten: Vec<(String, String)> = state.forgotten.iter().filter(|b| !was.contains(*b)).map(|b| (scope.to_string(), b.clone())).collect();
            i.views.insert(scope.to_string(), (position, state));
            let kept = json!({ "at": at, "hash": crate::db::state_hash(db.conn())?, "db": data_encoding::BASE64.encode(&db.bytes()?) });
            i.scope_dbs.insert(scope.clone(), (at, db));
            Ok((kept, newer, forgotten))
        })?;
        self.keep_scope_db(scope, &kept);
        if !forgotten.is_empty() {
            self.delete_forgotten(forgotten);
        }
        // A newer release's step: this device reads no further until it's updated.
        ensure!(!newer, steps::Newer);
        Ok(!read.is_empty())
    }

    /// Writes `content` to a scope's own database (not `global`'s) as a step this device signs as `user`, once
    /// `global`'s log is frozen, at the next position.
    pub(super) async fn write_scope_step(&self, scope: &Scope, user: &str, content: Content) -> Result<Entry> {
        self.first_epoch(scope).await?;
        let _write = self.own_writes().await?;
        for _ in 0..50 {
            self.read_scope_steps_now(scope).await?;
            let (step, at) = self.with(|i| {
                let (at, db) = i.scope_dbs.get(scope).context("this device can't read that yet: try again")?;
                let step = steps::sign(&i.r.key, scope, at + 1, user, i.r.db().epoch(scope), now_ms(), content.clone())?;
                steps::apply(db, Some(i.r.db()), scope, &step)?;
                Ok((step, at + 1))
            })?;
            for blob in &step.content.blobs {
                self.share_blob(scope, blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
            }
            let keys = self.log_key(scope).await?;
            let name = crate::db::step_name(&steps::prefix(&scope.to_string()), at);
            if crate::bucket::Bucket::create(&self.bucket()?, &name, &crate::log::seal(&keys, &name, &steps::encode(&step, None))?).await? {
                self.read_scope_steps_now(scope).await?;
                let copy = self.with(|i| i.scope_dbs.get_mut(scope).map(|(_, d)| d.bytes()).transpose())?;
                if let Some(bytes) = copy
                    && let Err(err) = steps::written(&self.bucket()?, &keys, scope, at, bytes).await
                {
                    tracing::debug!("{scope}'s copy and hint: {err:#}");
                }
                self.ping(scope, None);
                return Ok(step);
            }
        }
        bail!("{scope} changed too often to write to")
    }
}
