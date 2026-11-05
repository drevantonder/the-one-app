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
        let Some(from) = self.with(|i| Ok(i.r.at()))? else { return Ok(false) };
        let bucket = self.bucket()?;
        let prefix = steps::prefix("global");
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
        Ok(at > from)
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
}
