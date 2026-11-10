//! `global`'s scope database (`database::steps`): reading the steps others wrote in the bucket, from the freeze the
//! network's recovery key signed, and writing this device's own and those its users' other devices sign. What this
//! device has read of each it keeps in its folder, checked against its hash when it starts again (`scope-kept/`).

use crate::protocol::allowed;
use super::*;
use crate::database::steps;
use crate::protocol::{Coded, code};

impl Node {
    /// Whether this device has read who's who: `global`'s freeze, and the steps after it.
    pub fn frozen(&self) -> bool {
        self.with(|i| Ok(i.r.freeze().is_some())).unwrap_or(false)
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
        if self.with(|i| Ok(i.r.read_again))?
            && let Err(err) = self.read_global_again().await
        {
            tracing::warn!("reading who's who again: {err:#}: it's tried again next time");
        }
        let bucket = self.bucket()?;
        let from = self.with(|i| Ok(i.r.looked()))?;
        loop {
            let keys = match self.step_keys(&Scope::Global).await {
                Ok(keys) => keys,
                // Not given `global`'s newest epoch yet, as between its own new epoch's step and its seals: it reads
                // with those it holds, and a step under the newest waits for it, as one under any epoch not handed yet.
                Err(err) if err.is::<NotGiven>() => self.step_keys_of(&self.epochs_held(&Scope::Global).await?.0).await?,
                Err(err) => return Err(err),
            };
            let at = self.with(|i| Ok(i.r.looked()))?;
            let sealed = steps::fetch(&bucket, at, None).await?;
            let (handled, waits) = self.with(|i| Ok(i.r.add(&sealed, &keys, now_ms())))?;
            self.after(handled).await?;
            if waits?.is_none() {
                break;
            }
            // Made, but not handed to this device yet: it reads on once it is.
            if self.with(|i| Ok(i.r.looked()))? == at {
                self.forget_epochs(&Scope::Global);
                break;
            }
        }
        let read = self.with(|i| Ok(i.r.looked()))? > from;
        if read {
            self.keep_global();
        }
        Ok(read)
    }

    /// Reads `global`'s steps again from the first, for what an older build kept here, which lacks those it took for junk
    /// (`steps::read_again`). What it read takes the place of what was kept, and if it doesn't reach as far, what's kept
    /// stays until this device next starts.
    async fn read_global_again(&self) -> Result<()> {
        let (root, at) = self.with(|i| Ok((i.r.root(), i.r.looked())))?;
        let Some((fresh, epochs)) = steps::read_again(&self.bucket()?, root, at, &hex(&self.0.holds.device()), &self.0.holds, now_ms()).await? else {
            tracing::warn!("who's who read again doesn't reach step {at}, where this device was: it stays as it was");
            self.with(|i| Ok(i.r.read_again = false))?;
            return Ok(());
        };
        let handled = self.with(|i| i.r.replace(fresh))?;
        self.0.databases.epochs.lock().unwrap().insert(Scope::Global, epochs);
        self.after(handled).await?;
        self.keep_global();
        Ok(())
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
            let opened = steps::ScopeReader::kept(&kept, &scope, now_ms());
            match opened {
                Ok(db) => drop(self.with(|i| Ok(i.scope_dbs.insert(scope.clone(), db)))),
                Err(err) => tracing::warn!("{scope}'s database this device kept: {err:#}: it's read again from the bucket"),
            }
        }
    }

    /// Writes `content` to `global`'s scope database as a step this device signs as `user`, at the next position: when
    /// another device wrote there first, it reads that, and signs it again for the one after. Signed by the device's key,
    /// never this node's own, which may be its app runtime's.
    pub(super) async fn write_step(&self, user: &str, content: Content) -> Result<Step> {
        self.write_step_as(self.0.holds.signs_global(), user, content).await
    }

    /// `write_step`, signed by `key`: this device's, or the network's recovery key, which only ever signs on the machine
    /// it's kept on.
    pub(super) async fn write_step_as(&self, key: &SigningKey, user: &str, content: Content) -> Result<Step> {
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
    async fn put_step(&self, step: &Step) -> Result<bool> {
        let at = u64::from(step.seq());
        ensure!(self.with(|i| Ok(i.r.at()))? == Some(at - 1), "step {at} isn't the next here");
        for blob in &step.content.blobs {
            self.share_blob(&Scope::Global, blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
        }
        let keys = self.step_keys(&Scope::Global).await?;
        let name = crate::database::stored::step_name(&steps::prefix("global"), at);
        if !crate::bucket::Bucket::create(&self.bucket()?, &name, &steps::seal(&keys, &name, &steps::encode(step, None))?).await? {
            return Ok(false);
        }
        let handled = self.with(|i| i.r.take_step(at, step, now_ms()))?;
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
    pub async fn put_signed_steps(&self, caller: &Caller, list: Vec<Step>) -> Result<()> {
        self.read_callers_scopes(caller).await?;
        for step in list {
            ensure!(step.user() == caller.user, "step {} isn't written as you", step.seq());
            let _write = self.0.writes.lock().await;
            self.read_steps_now().await?;
            let next = self.with(|i| Ok(i.r.at()))?.context("who's who here isn't written as steps yet: update this device")? + 1;
            if u64::from(step.seq()) != next {
                return Err(Coded::error(code::TAKEN, format!("{} is taken: sign it again for {next}", step.seq())));
            }
            self.with(|i| steps::apply(i.r.db(), None, &Scope::Global, &step).map(drop))?;
            if !self.put_step(&step).await? {
                return Err(Coded::error(code::TAKEN, format!("{} is taken: sign it again for {}", step.seq(), next + 1)));
            }
        }
        Ok(())
    }

    /// `global`'s steps after `from` as the bucket holds them, sealed, each `{ at, sealed }` (base64url), its freeze
    /// among them: a client device opens them with its own keys and applies them itself, never taking who's who, nor
    /// that a step isn't one, from a node. With `{ at, epoch }`, the last there and `global`'s newest epoch, after which
    /// and under which a writer signs its own. Fails
    /// before the log is frozen here, so a writer writes entries. For a device that starts from its `welcome`
    /// (`steps::Welcome`), those after the copy before its step instead, with that copy and the freeze's step, sealed:
    /// `{ welcome, freeze: { at, sealed }, start: { at, sealed } }`, without `start` if the copy isn't there.
    pub async fn global_steps(&self, caller: &Caller, from: u64, welcome: Option<steps::Welcome>) -> Result<Value> {
        self.read_steps().await?;
        let read = self.with(|i| {
            allowed!(i.r.db().may(&caller.user, "use", scopes::NETWORK), "you aren't one of this network's users");
            Ok(i.r.at())
        })?;
        let read = read.context("global's log isn't frozen")?;
        let from = from.min(read);
        let mut out = steps::fetch(&self.bucket()?, from, welcome.filter(|w| w.at <= read).as_ref()).await?.to_json(from);
        out["epoch"] = json!(self.with(|i| Ok(i.r.db().epoch(&Scope::Global)?))?);
        Ok(out)
    }

    /// Reads a scope's own steps after the last this device applied, once `global`'s log is frozen: from its start
    /// (`steps::started`), then each applied if it counts there with `global`'s scope database as it is now. Its state is the
    /// scope's view from then on. One whose start can't be worked out yet is tried again the next time.
    pub(super) async fn read_scope_steps_now(&self, scope: &Scope) -> Result<bool> {
        if *scope == Scope::Global || !self.frozen() || !self.with(|i| Ok(i.r.holds(scope)))? {
            return Ok(false);
        }
        let keys = self.step_keys(scope).await?;
        let bucket = self.bucket()?;
        if !self.with(|i| Ok(i.scope_dbs.contains_key(scope)))? {
            let start = steps::ScopeReader::fetch_start(&bucket, &keys, scope).await?;
            let reader = self.with(|i| steps::ScopeReader::start(i.r.truth().context("global's log isn't frozen")?, scope, start.as_ref(), now_ms()))?;
            self.with(|i| Ok(i.scope_dbs.insert(scope.clone(), reader)))?;
        }
        if self.with(|i| Ok(i.scope_dbs.get(scope).is_some_and(|r| !r.whole)))?
            && let Err(err) = self.read_scope_again(scope, &keys).await
        {
            tracing::warn!("reading {scope} again: {err:#}: it's tried again next time");
        }
        let (from, made) = self.with(|i| Ok((i.scope_dbs.get(scope).map_or(0, |r| r.at), i.r.db().epoch(scope)?)))?;
        let read = steps::read(&bucket, &keys, scope, from, made).await?;
        let (kept, took, forgotten) = self.with(|i| {
            let mut reader = i.scope_dbs.remove(scope).context("no database")?;
            let took = reader.take(from, &read, i.r.db(), scope, now_ms());
            let was = i.views.get(&scope.to_string()).map(|s| s.forgotten.clone()).unwrap_or_default();
            let state = reader.db.state()?;
            // The blobs its steps said its apps no longer need, as `forget` entries did.
            let forgotten: Vec<(String, String)> = state.forgotten.iter().filter(|b| !was.contains(*b)).map(|b| (scope.to_string(), b.clone())).collect();
            i.views.insert(scope.to_string(), state);
            if !read.steps.is_empty() {
                note_change(i, scope);
            }
            let kept = reader.keep()?;
            i.scope_dbs.insert(scope.clone(), reader);
            Ok((kept, took, forgotten))
        })?;
        self.keep_scope_db(scope, &kept);
        if !forgotten.is_empty() {
            self.delete_forgotten(forgotten);
        }
        // A newer release's step: this device reads no further until it's updated.
        took?;
        Ok(!read.steps.is_empty())
    }

    /// A scope's steps read again from its start, for what an older build kept here (`read_global_again`), in its place
    /// if it read to the end of them. Where that ends is the bucket's to say: what's kept can stand at a position past it.
    async fn read_scope_again(&self, scope: &Scope, keys: &crate::database::stored::Keys) -> Result<()> {
        let bucket = self.bucket()?;
        let start = steps::ScopeReader::fetch_start(&bucket, keys, scope).await?;
        let (mut fresh, made) = self.with(|i| Ok((steps::ScopeReader::start(i.r.truth().context("global's log isn't frozen")?, scope, start.as_ref(), now_ms())?, i.r.db().epoch(scope)?)))?;
        let read = steps::read(&bucket, keys, scope, 0, made).await?;
        self.with(|i| {
            fresh.take(0, &read, i.r.db(), scope, now_ms())?;
            if read.waits.is_none() {
                i.scope_dbs.insert(scope.clone(), fresh);
            }
            Ok(())
        })
    }

    /// Writes `content` to a scope's own database (not `global`'s) as a step this device signs as `user`, once
    /// `global`'s log is frozen, at the next position.
    pub(super) async fn write_scope_step(&self, scope: &Scope, user: &str, content: Content) -> Result<Step> {
        self.first_epoch(scope).await?;
        let _write = self.own_writes().await?;
        for _ in 0..50 {
            self.read_scope_steps_now(scope).await?;
            let (step, at) = self.with(|i| {
                let r = i.scope_dbs.get(scope).context("this device can't read that yet: try again")?;
                let step = steps::sign(&i.r.key, scope, r.at + 1, user, i.r.db().epoch(scope)?, now_ms(), content.clone())?;
                steps::apply(&r.db, Some(i.r.db()), scope, &step)?;
                Ok((step, r.at + 1))
            })?;
            for blob in &step.content.blobs {
                self.share_blob(scope, blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
            }
            let keys = self.step_keys(scope).await?;
            let name = crate::database::stored::step_name(&steps::prefix(&scope.to_string()), at);
            if crate::bucket::Bucket::create(&self.bucket()?, &name, &steps::seal(&keys, &name, &steps::encode(&step, None))?).await? {
                self.read_scope_steps_now(scope).await?;
                let copy = self.with(|i| i.scope_dbs.get_mut(scope).map(|r| r.db.bytes()).transpose())?;
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

impl Node {
    pub async fn append(&self, scope: &Scope, content: Content) -> Result<Step> {
        let user = self.user()?;
        self.append_as(scope, &user, content).await
    }

    pub(super) async fn append_as(&self, scope: &Scope, user: &str, content: Content) -> Result<Step> {
        if !self.frozen() {
            self.read_steps().await?;
        }
        match scope {
            Scope::Global => self.write_step(user, content).await,
            _ => self.write_scope_step(scope, user, content).await,
        }
    }

    /// A scope's first epoch, before anything is written there, made outside this device's writes, since its entry is
    /// one: a scope with none has nothing to read or write until it has.
    pub(super) async fn first_epoch(&self, scope: &Scope) -> Result<()> {
        if *scope != Scope::Global && self.with(|i| Ok(i.r.db().epoch(scope)?))? == 0 {
            self.epoch_keys(scope, true).await?;
        }
        Ok(())
    }

    /// Follows what changed in who's who, and brings each scope's state up to date.
    pub(super) async fn after(&self, handled: Handled) -> Result<()> {
        if handled.dropped.is_empty() && !handled.members {
            return Ok(());
        }
        // What `global`'s scope database forgets in a step, once its log is frozen, as its `forget` entries said before.
        let forgotten_before = self.with(|i| Ok(i.views.get("global").map(|s| s.forgotten.clone()).unwrap_or_default()))?;
        let forgotten_now = self.with(|i| {
            if handled.members {
                let db = i.r.db();
                let mut listed = BTreeSet::new();
                for peer in i.peers.keys() {
                    if db.compute(peer)?.is_some() || db.service(peer)?.is_some() {
                        listed.insert(*peer);
                    }
                }
                i.peers.retain(|peer, _| listed.contains(peer));
            }
            for log in &handled.dropped {
                i.views.remove(log);
            }
            if handled.members {
                note_change(i, &Scope::Global);
            }
            update_views(i)?;
            Ok(i.views.get("global").map(|s| s.forgotten.clone()).unwrap_or_default())
        })?;
        self.close_dbs(|scope, _| handled.dropped.contains(&scope.to_string()));
        if let Ok(bucket) = self.bucket() {
            for log in &handled.dropped {
                if let Err(err) = bucket.forget(&crate::bucket::folder(log)) {
                    tracing::warn!("forgetting this device's copy of {log}: {err:#}");
                }
            }
        }
        if !handled.lost.is_empty() {
            self.forget_sessions_of(&handled.lost);
        }
        if handled.members {
            // A device removed keeps nothing it handed this one, nor its connections.
            self.forget_removed_clients();
            self.close_removed();
            self.0.changed.notify_one();
            self.emit(Event::Devices);
            self.seal_epochs();
            self.settle_own();
            self.follow_recovery();
            self.follow_erased().await;
        }
        if handled.members {
            self.follow_storage();
            self.follow_net();
        }
        let forgotten: Vec<(String, String)> = forgotten_now.into_iter().filter(|b| !forgotten_before.contains(b)).map(|b| ("global".to_string(), b)).collect();
        self.emit(Event::Changed { apps: Vec::new() });
        if !forgotten.is_empty() {
            self.delete_forgotten(forgotten);
        }
        Ok(())
    }

    /// Brings this device in line with the members: runs each app's latest deploy, fetching bundles it lacks, and
    /// says which build it runs.
    pub(super) async fn settle(&self) -> Result<()> {
        let _settling = self.0.settling.lock().await;
        let Ok((wanted, unadded, author)) = self.with(|i| {
            let wanted: BTreeMap<String, String> = i.r.db().apps()?.into_iter().filter(|a| !a.erased).filter_map(|a| Some((a.name, a.bundle?))).collect();
            let mut unadded = BTreeMap::new();
            for (name, bundle) in self.0.built_in.iter() {
                if i.r.db().app(name)?.is_none() {
                    unadded.insert(name.clone(), bundle.clone());
                }
            }
            Ok((wanted, unadded, i.r.db().compute(&self.0.id)?.is_some()))
        }) else {
            return Ok(());
        };
        if !author {
            return Ok(());
        }
        // Each app's deploy, and the bundle it's of: its newest, or while that can't be fetched yet, the one it ran.
        let (mut modules, mut used) = (Vec::new(), BTreeMap::new());
        let mut unfetched = false;
        let broken = self.0.broken.lock().unwrap().clone();
        let ran = self.0.bundles.lock().unwrap().clone();
        for (app, bundle) in wanted.iter().filter(|(a, b)| broken.get(*a) != Some(*b)) {
            let mut fetched = self.blob(bundle).await.and_then(|b| crate::app::bundle::module(&b)).map(|m| (m, bundle));
            if let Err(err) = &fetched {
                tracing::warn!("{app}'s deploy: {err:#}: tried again soon");
                unfetched = true;
                if let Some(old) = ran.get(app).filter(|old| *old != bundle)
                    && let Ok(module) = self.blob(old).await.and_then(|b| crate::app::bundle::module(&b))
                {
                    fetched = Ok((module, old));
                }
            }
            if let Ok((module, bundle)) = fetched {
                modules.push((app.clone(), module));
                used.insert(app.clone(), bundle.clone());
            }
        }
        // The built-in apps no one has added run no calls: they're there to be described, for the App Store to list.
        let mut waiting = Vec::new();
        for (app, bundle) in unadded.iter().filter(|(a, b)| broken.get(*a) != Some(*b)) {
            if let Ok(module) = self.blob(bundle).await.and_then(|b| crate::app::bundle::module(&b)) {
                waiting.push((app.clone(), module));
            }
        }
        self.0.unfetched.store(unfetched, std::sync::atomic::Ordering::Relaxed);
        let changed: Vec<String> = modules.iter().map(|(a, _)| a.clone()).filter(|a| ran.get(a) != used.get(a)).collect();
        if !changed.is_empty() || modules.len() != self.0.apps.names().len() || waiting.iter().map(|(a, _)| a).ne(self.0.apps.unadded().iter()) {
            // Writes wait for the new deploys, so what's written next runs on them.
            let _write = self.0.writes.lock().await;
            // An app whose code doesn't start is left out, so the rest run.
            while let Err(err) = self.0.apps.load(modules.clone(), waiting.clone()).await {
                let Some(broken) = err.downcast_ref::<crate::app::runtime::Broken>() else { return Err(err) };
                tracing::warn!("{broken}: it's left out until it's deployed again");
                if let Some(bundle) = used.get(&broken.app).or(unadded.get(&broken.app)) {
                    self.0.broken.lock().unwrap().insert(broken.app.clone(), bundle.clone());
                }
                let before = modules.len() + waiting.len();
                modules.retain(|(a, _)| *a != broken.app);
                waiting.retain(|(a, _)| *a != broken.app);
                ensure!(modules.len() + waiting.len() < before, "{err:#}");
            }
            *self.0.bundles.lock().unwrap() = modules.iter().map(|(a, _)| (a.clone(), used[a].clone())).collect();
            // Read again, so each is brought to its new deploy's schema.
            self.close_dbs(|_, db| changed.iter().any(|app| db.starts_with(&format!("{app}/"))));
            self.emit(Event::Changed { apps: changed });
            self.say_hello();
        }
        let running = self.with(|i| Ok(i.builds.get(&self.0.id).map(|r| r.commit.clone())))?;
        if running.as_deref() != Some(crate::config::BUILD) {
            self.say(&Scope::Global, status::Said::Build { device: self.0.id, build: crate::config::BUILD.into(), target: crate::config::TARGET.into() }).await?;
        }
        Ok(())
    }
}
