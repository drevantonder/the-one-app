//! `global`'s scope database once its log is frozen (`database::switch`, `database::steps`): freezing the log when an
//! admin's device finds the gate open, reading the steps others wrote in the bucket, and writing this device's own and
//! those its people's other devices sign. What this device has read of each it keeps in its folder, checked against
//! its hash when it starts again (`scope-kept/`).

use super::*;
use crate::database::{steps, switch};

/// How long a writer of the log waits for the freeze a `FREEZING` step says is coming, from when it first sees it,
/// before it writes on: its admin's device writes the freeze within half of it, or not at all, so a writer that has
/// waited this long finds it, if it's ever written.
const FREEZE_WAIT: Duration = Duration::from_secs(30);

impl Node {
    /// Whether `global`'s log is frozen here: its scope database is the truth, written as steps.
    pub fn frozen(&self) -> bool {
        self.with(|i| Ok(i.r.frozen.is_some())).unwrap_or(false)
    }

    /// Freezes `global`'s log, if the switch is on and the gate is open (or `forced`, in a throwaway network), it isn't
    /// frozen yet, and this device signs for an admin: a `FREEZING` step first, so writers of the log look whether the
    /// freeze holds what they write from then on; then where every log ends, and the freeze (`steps::freeze`), at the
    /// next free position, within half of `FREEZE_WAIT` of the `FREEZING`, or not at all. Another admin's device may
    /// freeze it first: the first freeze an admin's device signed is the one. → whether it's frozen.
    pub async fn freeze(&self, forced: bool) -> Result<bool> {
        if !(switch::SWITCH || (forced && switch::FORCEABLE.load(std::sync::atomic::Ordering::Relaxed))) || self.frozen() {
            return Ok(self.frozen());
        }
        let Ok(user) = self.user() else { return Ok(false) };
        self.read_log(&Scope::Global).await?;
        let signs = self.with(|i| {
            let db = i.r.db();
            Ok((forced || switch::gate(db, &i.reads).open()) && db.client(&self.0.id).is_some_and(|c| c.user == user) && db.may(&user, "change-settings", scopes::INSTALL))
        })?;
        if !signs {
            return Ok(false);
        }
        let _write = self.own_writes().await?;
        // `global`'s log as the bucket has it, read before the `FREEZING`, so only what's written since is read after.
        let (bucket, keys, prefix) = (self.bucket()?, self.log_key(&Scope::Global).await?, crate::log::prefix("global"));
        let before = crate::log::read(&bucket, &keys, &prefix, "global", 0, 0).await?;
        if !self.put_first(&user, Content::new(members::APP, steps::FREEZING, json!({})), None).await? {
            return Ok(self.frozen());
        }
        let started = std::time::Instant::now();
        // Every log as it is now, which no writer that looks after this adds to unawares.
        self.read_entries_now(&Scope::Global).await?;
        let after = crate::log::read(&bucket, &keys, &prefix, "global", before.at, before.floor).await?;
        ensure!(before.ahead.is_none() && after.ahead.is_none(), "global's log has a step under an epoch this device hasn't opened yet: it freezes the log once it has");
        let log = steps::Log { to: after.at, entries: [before.entries, after.entries].concat() };
        let mut logs = BTreeMap::new();
        for scope in self.with(|i| Ok(i.r.db().scopes()))?.into_iter().filter(|s| *s != Scope::Global) {
            let listed = crate::bucket::Bucket::objects(&bucket, &crate::log::prefix(&scope.to_string())).await?;
            let last = listed.iter().filter_map(|(n, _)| n.parse::<u64>().ok()).max().unwrap_or(0);
            logs.insert(scope.to_string(), last);
        }
        let (content, copy) = self.with(|i| i.r.freeze_now(&log, logs, now_ms()))?;
        if started.elapsed() > FREEZE_WAIT / 2 {
            tracing::warn!("freezing the log took too long, so writers may have written on: it's tried again");
            return Ok(false);
        }
        *self.0.frozen_log.lock().unwrap() = Some(Arc::new(log));
        self.put_first(&user, content, Some(&copy)).await?;
        Ok(self.frozen())
    }

    /// Writes a step of `global`'s scope database before its log is frozen (`FREEZING`, or the freeze with its `copy`),
    /// at the next free position, while this device holds its writes. → whether it was written: not if the log froze.
    async fn put_first(&self, user: &str, content: Content, copy: Option<&[u8]>) -> Result<bool> {
        for _ in 0..50 {
            self.read_steps_now().await?;
            if self.frozen() {
                return Ok(false);
            }
            let (at, epoch) = self.with(|i| Ok((i.r.looked + 1, i.r.db().epoch(&Scope::Global))))?;
            let step = self.with(|i| steps::sign(&i.r.key, &Scope::Global, at, user, epoch, now_ms(), content.clone()))?;
            let keys = self.log_key(&Scope::Global).await?;
            let name = crate::db::step_name(&steps::prefix("global"), at);
            if crate::bucket::Bucket::create(&self.bucket()?, &name, &crate::log::seal(&keys, &name, &steps::encode(&step, copy))?).await? {
                self.read_steps_now().await?;
                self.ping(&Scope::Global, None);
                return Ok(true);
            }
        }
        bail!("who's who changed too often to write to")
    }

    /// After this device wrote `entry` at step `at` of its scope's log: whether a freeze leaves it out, so it never
    /// counts. Looks only once an admin's `FREEZING` step is there, and waits for the freeze it says is coming, up to
    /// `FREEZE_WAIT` from when it first saw that one: after that it has lapsed, and the entry counts.
    pub(super) async fn left_out(&self, entry: &Entry, at: u64) -> Result<bool> {
        loop {
            self.read_steps_now().await?;
            let (frozen, freezing) = self.with(|i| Ok((i.r.frozen.clone(), i.r.freezing)))?;
            if let Some((freeze, _)) = frozen {
                return Ok(!freeze.holds(entry, at));
            }
            let Some(freezing) = freezing else { return Ok(false) };
            let seen = {
                let mut seen = self.0.freezing_seen.lock().unwrap();
                if seen.is_none_or(|(p, _)| p != freezing) {
                    *seen = Some((freezing, std::time::Instant::now()));
                }
                seen.map(|(_, t)| t).expect("just set")
            };
            if seen.elapsed() >= FREEZE_WAIT {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// Once `global`'s log is frozen, this release's types, roles, rules and tables as a step (`steps::DEFAULTS`), if
    /// this device signs for an admin, its database is an older release's, and every device reads this one
    /// (`switch::gate`).
    pub async fn say_defaults(&self) -> Result<()> {
        let Ok(user) = self.user() else { return Ok(()) };
        let due = self.with(|i| {
            let db = i.r.db();
            let signs = db.client(&self.0.id).is_some_and(|c| c.user == user) && db.may(&user, "change-roles", scopes::INSTALL);
            Ok(i.r.frozen.is_some() && signs && steps::version(db.conn())? < switch::READS && switch::gate_at(db, &i.reads, switch::READS).open())
        })?;
        if due {
            self.write_step(&user, steps::release_defaults()?).await?;
        }
        Ok(())
    }

    /// Reads `global`'s steps in the bucket after the last this device applied, once its log is frozen: each applied if
    /// it counts, passed over if not. Before, it looks at each for its freeze. A step under an epoch above every key this
    /// device opens waits for that epoch's key; one above every epoch made is looked at again once `global`'s log is
    /// read to its end, and only then, if still above, junk, passed over: an epoch's entry or step is always written
    /// before anything is sealed under it. → whether there were any.
    pub(super) async fn read_steps(&self) -> Result<bool> {
        let _write = self.0.writes.lock().await;
        self.read_steps_now().await
    }

    /// `read_steps`, while this device holds its writes. Before the log is frozen, it looks at each step for the
    /// freeze (`Replica::take_first`): one an admin's own device signed is taken once `global`'s log is read to its end
    /// (so its signer is checked against everything in the bucket) and from its steps up to the freeze's end of it.
    pub(super) async fn read_steps_now(&self) -> Result<bool> {
        // A build the switch is off in never freezes, and holds the gate back, so nothing it reads is frozen.
        if !self.frozen() && !(switch::SWITCH || switch::FORCEABLE.load(std::sync::atomic::Ordering::Relaxed)) {
            return Ok(false);
        }
        let bucket = self.bucket()?;
        let prefix = steps::prefix("global");
        let from = self.with(|i| Ok(i.r.at().unwrap_or(i.r.looked)))?;
        let mut at = from;
        let mut log_read = false;
        loop {
            let name = crate::db::step_name(&prefix, at + 1);
            let Some(o) = crate::bucket::Bucket::get(&bucket, &name).await? else {
                steps::missing(&bucket, &prefix, at + 1).await?;
                break;
            };
            let keys = self.log_key(&Scope::Global).await?;
            let given = match steps::open(&keys, &Scope::Global, at + 1, &o.bytes) {
                steps::Opened::Step(given) => Some(given),
                steps::Opened::Junk => None,
                steps::Opened::Waits(epoch) if self.with(|i| Ok(i.r.epoch_made(&Scope::Global)))? >= epoch => {
                    // Made, but not handed to this device yet: it reads on once it is.
                    self.forget_epochs(&Scope::Global);
                    break;
                }
                // Before the freeze, `global`'s epochs are its log's: read it to its end, and look again.
                steps::Opened::Waits(_) if !self.frozen() && !log_read => {
                    self.read_entries_now(&Scope::Global).await?;
                    log_read = true;
                    continue;
                }
                steps::Opened::Waits(_) => None,
            };
            let claimed = self.with(|i| match i.r.frozen {
                Some(_) => Ok(None),
                None => i.r.claimed(at + 1, given.as_ref()),
            })?;
            let log = match claimed {
                Some(end) => {
                    // Its signer is checked against everything in the bucket, so the log first, to its end.
                    if !log_read {
                        self.read_entries_now(&Scope::Global).await?;
                    }
                    Some(self.global_log(end).await?)
                }
                None => None,
            };
            let handled = self.with(|i| match i.r.frozen {
                Some(_) => i.r.take_step(at + 1, given.as_ref().map(|g| &g.step), now_ms()),
                None => i.r.take_first(at + 1, given.as_ref(), log.as_deref(), now_ms()),
            })?;
            at += 1;
            log_read = false;
            self.after(handled).await?;
        }
        if at > from {
            self.keep_global();
        }
        Ok(at > from)
    }

    /// `global`'s log up to step `end`, where its freeze says it ends, from the bucket's steps (`steps::log_to`): read
    /// once, and kept while this device runs, for each scope's database to start from.
    async fn global_log(&self, end: u64) -> Result<Arc<steps::Log>> {
        if let Some(log) = self.0.frozen_log.lock().unwrap().clone().filter(|l| l.to == end) {
            return Ok(log);
        }
        let log = Arc::new(steps::log_to(&self.bucket()?, &self.log_key(&Scope::Global).await?, end).await?);
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
        self.0.dir.join("install").join("scope-kept").join(data_encoding::HEXLOWER.encode(scope.to_string().as_bytes()))
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
        let Ok(dir) = std::fs::read_dir(self.0.dir.join("install").join("scope-kept")) else { return };
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
        let _write = self.own_writes().await?;
        for _ in 0..50 {
            self.read_steps_now().await?;
            let step = self.with(|i| i.r.make_step(&Scope::Global, user, content.clone(), now_ms()))?;
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
            ensure!(i.r.db().may(&caller.user, "use", scopes::INSTALL), "you aren't one of this install's people");
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
            let freeze = self.with(|i| Ok(i.r.frozen.clone().map(|(f, _)| f)))?.context("global's log isn't frozen")?;
            let end = freeze.logs.get(&scope.to_string()).copied().unwrap_or(0);
            let entries = crate::log::read_to(&bucket, &keys, &crate::log::prefix(&scope.to_string()), &scope.to_string(), end).await?;
            let global = self.global_log(freeze.global()).await?;
            let db = self.with(|i| steps::scope_anchor(&freeze, i.r.root, &global, scope, &entries, now_ms()))?;
            self.with(|i| Ok(i.scope_dbs.insert(scope.clone(), (0, db))))?;
        }
        let from = self.with(|i| Ok(i.scope_dbs.get(scope).map_or(0, |(at, _)| *at)))?;
        let read = steps::read(&bucket, &keys, scope, from).await?;
        let kept = self.with(|i| {
            let (mut at, mut db) = i.scope_dbs.remove(scope).context("no database")?;
            for step in &read {
                at += 1;
                let Some(given) = step else { continue };
                match steps::apply(&db, Some(i.r.db()), scope, &given.step) {
                    Ok(a) => db = ScopeDb::open(a.db, now_ms())?,
                    Err(err) => tracing::info!("{scope}'s step {at} doesn't count: {err:#}"),
                }
            }
            let position = i.views.get(&scope.to_string()).map(|(p, _)| p.clone()).unwrap_or((0, Hash::from_bytes([0; 32])));
            i.views.insert(scope.to_string(), (position, db.state()));
            let kept = json!({ "at": at, "hash": crate::db::state_hash(db.conn())?, "db": data_encoding::BASE64.encode(&db.bytes()?) });
            i.scope_dbs.insert(scope.clone(), (at, db));
            Ok(kept)
        })?;
        self.keep_scope_db(scope, &kept);
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
