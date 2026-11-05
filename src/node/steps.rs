//! `global`'s scope database once its log is frozen (`database::switch`, `database::steps`): freezing the log when an
//! admin's device finds the gate open, reading the steps others wrote in the bucket, and writing this device's own and
//! those its people's other devices sign.

use super::*;
use crate::database::{steps, switch};

impl Node {
    /// Whether `global`'s log is frozen here: its scope database is the truth, written as steps.
    pub fn frozen(&self) -> bool {
        self.with(|i| Ok(i.r.frozen.is_some())).unwrap_or(false)
    }

    /// Freezes `global`'s log, if the switch is on (or `forced`, in a try install), the gate is open, it isn't frozen
    /// yet, and this device signs for an admin: its last entry says the scope database, built from everything this
    /// device holds, comes to its state hash. → whether it froze it.
    pub async fn freeze(&self, forced: bool) -> Result<bool> {
        if !(switch::SWITCH || (forced && switch::FORCEABLE.load(std::sync::atomic::Ordering::Relaxed))) || self.frozen() {
            return Ok(false);
        }
        let Ok(user) = self.user() else { return Ok(false) };
        self.read_log(&Scope::Global).await?;
        let anchor = self.with(|i| {
            let db = i.r.db();
            let open = switch::gate(db, &i.reads).open();
            let signs = db.client(&self.0.id).is_some_and(|c| c.user == user) && db.may(&user, "change-settings", scopes::INSTALL);
            Ok((open && signs).then(|| i.r.anchor(now_ms())).transpose()?)
        })?;
        let Some((hash, frontier)) = anchor else { return Ok(false) };
        self.append(&Scope::Global, members::freeze(&hash, &frontier)).await?;
        // The database's first step: the release's types, roles and rules, which readers keep from here on.
        self.write_step(&user, steps::release_defaults()?).await?;
        Ok(self.frozen())
    }

    /// Once `global`'s log is frozen, the release's types, roles and rules as a step (`steps::DEFAULTS`), if this
    /// device signs for an admin and its database's aren't this release's: the database's first step, and one after
    /// each release that changes them.
    pub async fn say_defaults(&self) -> Result<()> {
        let Ok(user) = self.user() else { return Ok(()) };
        let wanted = steps::release_defaults()?;
        let due = self.with(|i| {
            let db = i.r.db();
            let signs = db.client(&self.0.id).is_some_and(|c| c.user == user) && db.may(&user, "change-roles", scopes::INSTALL);
            Ok(i.r.frozen.is_some() && signs && steps::rules_of(db.conn())? != wanted.data)
        })?;
        if due {
            self.write_step(&user, wanted).await?;
        }
        Ok(())
    }

    /// Reads `global`'s steps in the bucket after the last this device applied, once its log is frozen: each applied if
    /// it counts, passed over if not. A step under an epoch above every key this device opens waits for that epoch's
    /// step; with none, it's junk, passed over. → whether there were any.
    pub(super) async fn read_steps(&self) -> Result<bool> {
        let _write = self.0.writes.lock().await;
        self.read_steps_now().await
    }

    /// `read_steps`, while this device holds its writes.
    pub(super) async fn read_steps_now(&self) -> Result<bool> {
        let Some(mut from) = self.with(|i| Ok(i.r.at()))? else { return Ok(false) };
        let bucket = self.bucket()?;
        let prefix = steps::prefix("global");
        // Steps this device hasn't read were deleted, a copy covering them: on from that copy, checked against what it
        // read before (`scope-at`), as it kept it.
        if crate::bucket::Bucket::get(&bucket, &crate::db::step_name(&prefix, from + 1)).await?.is_none()
            && let Some((at, hash)) = self.kept_at().filter(|(at, _)| *at > from)
        {
            let keys = self.log_key(&Scope::Global).await?;
            let db = steps::from_copy(&bucket, &keys, at, &hash, now_ms()).await?;
            self.with(|i| i.r.restart_at(at, db))?;
            from = at;
        }
        let mut at = from;
        loop {
            let name = crate::db::step_name(&prefix, at + 1);
            let Some(o) = crate::bucket::Bucket::get(&bucket, &name).await? else { break };
            let keys = self.log_key(&Scope::Global).await?;
            let step = match crate::keys::epoch_of(&o.bytes) {
                Ok(epoch) if keys.get(&epoch).is_some() => crate::keys::open(&keys[&epoch], &name, &o.bytes).ok().and_then(|b| steps::decode(&Scope::Global, at + 1, &b).ok()),
                Ok(epoch) if keys.keys().next_back().is_some_and(|newest| epoch > *newest) && self.with(|i| Ok(i.r.epoch_made(&Scope::Global)))? >= epoch => {
                    // Made, but not handed to this device yet: it reads on once it is.
                    self.forget_epochs(&Scope::Global);
                    break;
                }
                _ => None,
            };
            let handled = self.with(|i| i.r.take_step(at + 1, step.as_ref(), now_ms()))?;
            at += 1;
            self.after(handled).await?;
        }
        if at > from {
            self.keep_at();
        }
        Ok(at > from)
    }

    /// Where this device read `global`'s scope database to, and what it came to, kept in its folder (`scope-at`): what
    /// it starts again from, from a copy, once the steps before it are deleted.
    fn keep_at(&self) {
        let kept = self.with(|i| Ok(json!({ "at": i.r.at(), "hash": crate::db::state_hash(i.r.db().conn())? })));
        if let Ok(kept) = kept {
            let _ = write_private(&self.0.dir.join("install").join("scope-at"), kept.to_string().as_bytes());
        }
    }

    fn kept_at(&self) -> Option<(u64, String)> {
        let kept: Value = serde_json::from_slice(&std::fs::read(self.0.dir.join("install").join("scope-at")).ok()?).ok()?;
        Some((kept["at"].as_u64()?, kept["hash"].as_str()?.to_string()))
    }

    /// Writes `content` to `global`'s scope database as a step this device signs as `user`, at the next position: when
    /// another device wrote there first, it reads that, and signs it again for the one after.
    pub(super) async fn write_step(&self, user: &str, content: Content) -> Result<Entry> {
        let _write = self.0.writes.lock().await;
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
        if !crate::bucket::Bucket::create(&self.bucket()?, &name, &crate::log::seal(&keys, &name, &steps::encode(step))?).await? {
            return Ok(false);
        }
        let handled = self.with(|i| i.r.take_step(at, Some(step), now_ms()))?;
        self.after(handled).await?;
        self.keep_at();
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

    /// `global`'s steps after `from`, as the bucket holds them opened, each `{ at, step }` (`steps::encode`,
    /// base64url; none for one that isn't a step, passed over), for a client device that applies them itself, never
    /// taking who's who from a node: with `{ frozen }`, the entry that froze the log, and `at`, the last position.
    pub async fn global_steps(&self, caller: &Caller, from: u64) -> Result<Value> {
        self.read_steps().await?;
        let (frozen, at) = self.with(|i| {
            ensure!(i.r.db().may(&caller.user, "use", scopes::INSTALL), "you aren't one of this install's people");
            Ok(i.r.frozen)
        })?
        .context("global's log isn't frozen")?;
        let bucket = self.bucket()?;
        let keys = self.log_key(&Scope::Global).await?;
        let prefix = steps::prefix("global");
        let mut list = Vec::new();
        for n in from.saturating_add(1)..=at {
            let name = crate::db::step_name(&prefix, n);
            let opened = match crate::bucket::Bucket::get(&bucket, &name).await? {
                Some(o) => crate::keys::epoch_of(&o.bytes).ok().and_then(|e| keys.get(&e)).and_then(|k| crate::keys::open(k, &name, &o.bytes).ok()),
                None => None,
            };
            list.push(json!({ "at": n, "step": opened.map(|b| data_encoding::BASE64URL_NOPAD.encode(&b)) }));
        }
        Ok(json!({ "frozen": frozen.to_hex(), "at": at, "steps": list }))
    }

    /// Reads a scope's own steps after the last this device applied, once `global`'s log is frozen: its first, the
    /// anchor, checked against what this device's copy of the scope's log comes to, then each applied if it counts
    /// there with `global`'s scope database as it is now. Its state is the scope's view from then on.
    pub(super) async fn read_scope_steps_now(&self, scope: &Scope) -> Result<bool> {
        if *scope == Scope::Global || !self.frozen() || !self.with(|i| Ok(i.r.holds(scope)))? {
            return Ok(false);
        }
        let from = self.with(|i| Ok(i.scope_dbs.get(scope).map_or(0, |(at, _)| *at)))?;
        let keys = self.log_key(scope).await?;
        let read = steps::read(&self.bucket()?, &keys, scope, from).await?;
        if read.is_empty() {
            return Ok(false);
        }
        self.with(|i| {
            let entries = i.r.store.scope(&scope.to_string())?;
            let (mut at, mut db) = i.scope_dbs.remove(scope).unwrap_or((0, None));
            for step in read {
                at += 1;
                let Some(step) = step else { continue };
                let applied = match &db {
                    None if at == 1 => steps::apply_anchor(i.r.db(), &i.r.members, &i.r.member_entries, &entries, scope, &step),
                    None => Err(anyhow!("{scope}'s anchor didn't check out here")),
                    Some(d) => steps::apply(d, Some(i.r.db()), scope, &step),
                };
                match applied {
                    Ok(a) => db = Some(ScopeDb::open(a.db, now_ms())?),
                    Err(err) => tracing::info!("{scope}'s step {at} doesn't count: {err:#}"),
                }
            }
            if let Some(d) = &db {
                let position = i.views.get(&scope.to_string()).map(|(p, _)| p.clone()).unwrap_or((0, Hash::from_bytes([0; 32])));
                i.views.insert(scope.to_string(), (position, d.state()));
            }
            i.scope_dbs.insert(scope.clone(), (at, db));
            Ok(())
        })?;
        Ok(true)
    }

    /// Writes `content` to a scope's own database (not `global`'s) as a step this device signs as `user`, once
    /// `global`'s log is frozen: its anchor first, if it has none yet, at the final position of the scope's log this
    /// device holds.
    pub(super) async fn write_scope_step(&self, scope: &Scope, user: &str, content: Content) -> Result<Entry> {
        self.first_epoch(scope).await?;
        let _write = self.0.writes.lock().await;
        self.read_log_now(scope).await?;
        for _ in 0..50 {
            self.read_scope_steps_now(scope).await?;
            let (at, anchored) = self.with(|i| Ok(i.scope_dbs.get(scope).map_or((0, false), |(at, db)| (*at, db.is_some()))))?;
            let content = match (at, anchored) {
                (0, _) => self.with(|i| {
                    let entries = i.r.store.scope(&scope.to_string())?;
                    let (_, hash, frontier) = steps::anchor(&i.r.members, &i.r.member_entries, &entries, scope)?;
                    Ok(steps::anchor_content(&hash, &frontier))
                })?,
                (_, true) => content.clone(),
                (_, false) => bail!("{scope}'s first step didn't check out on this device, so it writes nothing there"),
            };
            let step = self.with(|i| {
                let step = steps::sign(&i.r.key, scope, at + 1, user, i.r.db().epoch(scope), now_ms(), content.clone())?;
                if let Some((_, Some(d))) = i.scope_dbs.get(scope) {
                    steps::apply(d, Some(i.r.db()), scope, &step)?;
                }
                Ok(step)
            })?;
            for blob in &step.content.blobs {
                self.share_blob(scope, blob, false).await.with_context(|| format!("blob {} isn't here: put it first", &blob[..12.min(blob.len())]))?;
            }
            let keys = self.log_key(scope).await?;
            let name = crate::db::step_name(&steps::prefix(&scope.to_string()), at + 1);
            if crate::bucket::Bucket::create(&self.bucket()?, &name, &crate::log::seal(&keys, &name, &steps::encode(&step))?).await? {
                self.read_scope_steps_now(scope).await?;
                let copy = self.with(|i| match i.scope_dbs.get_mut(scope) {
                    Some((_, Some(d))) => d.bytes().map(Some),
                    _ => Ok(None),
                })?;
                if let Some(bytes) = copy
                    && let Err(err) = steps::written(&self.bucket()?, &keys, scope, at + 1, bytes).await
                {
                    tracing::debug!("{scope}'s copy and hint: {err:#}");
                }
                self.ping(scope, None);
                if step.content.kind != steps::ANCHOR {
                    return Ok(step);
                }
            }
        }
        bail!("{scope} changed too often to write to")
    }
}
