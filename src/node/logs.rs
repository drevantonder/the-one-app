//! Each scope's log in the bucket, reading and writing its entries, and what devices tell each other: pings and presence.

use super::*;

impl Node {
    /// Reads what each scope this device holds has in the bucket after what it last read, `global` first, until it
    /// gains no scope by what it read.
    pub(super) async fn read_logs(&self) -> Result<()> {
        loop {
            let scopes = self.with(|i| {
                let mut scopes: Vec<Scope> = i.r.db().scopes().into_iter().filter(|s| i.r.holds(s)).collect();
                scopes.sort_by_key(|s| *s != Scope::Global);
                Ok(scopes)
            })?;
            let mut gained = false;
            for scope in scopes {
                gained |= match self.read_log(&scope).await {
                    // One whose newest key this device wasn't given yet: it reads the rest.
                    Err(err) if scope != Scope::Global && err.is::<NotGiven>() => {
                        tracing::debug!("{err:#}");
                        false
                    }
                    other => other?,
                };
            }
            if !gained {
                break;
            }
        }
        self.with(|i| {
            i.read = true;
            Ok(())
        })
    }

    /// Reads a scope's entries in the bucket after the last this device read: whether it now holds a scope it didn't.
    pub(super) async fn read_log(&self, scope: &Scope) -> Result<bool> {
        let _write = self.0.writes.lock().await;
        self.read_log_now(scope).await
    }

    /// `read_log`, while this device holds its writes. A device that has read none of a scope's entries starts from
    /// their newest copy. A scope with no epoch yet has nothing to read. A step under an epoch newer than any this
    /// device opens is read once that epoch's entry is (in `global`, which is read again for it), or if no entry made
    /// it, passed over as junk: an epoch's entry is always written before anything is sealed under it.
    pub(super) async fn read_log_now(&self, scope: &Scope) -> Result<bool> {
        let gained = self.read_entries_now(scope).await?;
        // Once `global`'s log is frozen, its scope database's steps: who's who from there; and each other scope's own.
        if *scope == Scope::Global {
            self.read_steps_now().await?;
        } else {
            self.read_scope_steps_now(scope).await?;
        }
        Ok(gained)
    }

    /// `read_log_now`'s entries alone: what a freeze's signer is checked against, read to the log's end first.
    pub(super) async fn read_entries_now(&self, scope: &Scope) -> Result<bool> {
        let log = scope.to_string();
        let (genesis, epoch) = self.with(|i| Ok((!i.r.db().users().is_empty(), i.r.db().epoch(scope))))?;
        if *scope != Scope::Global && epoch == 0 {
            return Ok(false);
        }
        let bucket = self.bucket()?;
        let prefix = crate::log::prefix(&log);
        let mut gained = false;
        loop {
            let (from, floor) = self.with(|i| i.r.store.position(&log))?;
            let (key, read) = match (from, genesis) {
                (0, false) if *scope == Scope::Global => {
                    let (epochs, read) = self.first_global_read().await?;
                    (self.log_key_of(&epochs).await?, read)
                }
                _ => {
                    let key = self.log_key(scope).await?;
                    let read = match from {
                        0 => crate::log::open(&bucket, &key, &prefix, &log).await?,
                        _ => crate::log::read(&bucket, &key, u32::MAX, &prefix, &log, from, floor).await?,
                    };
                    (key, read)
                }
            };
            let at = read.at;
            if at > from {
                let handled = self.with(|i| {
                    let handled = i.r.receive(read.entries, now_ms())?;
                    i.r.store.set_position(&log, at, read.floor)?;
                    Ok(handled)
                })?;
                gained |= handled.gained;
                self.after(handled).await?;
            }
            let Some(ahead) = read.ahead else {
                self.hint_log(scope, &bucket, &key, at).await;
                return Ok(gained);
            };
            if *scope != Scope::Global {
                Box::pin(self.read_log_now(&Scope::Global)).await?;
            }
            if self.with(|i| Ok(i.r.epoch_made(scope)))? >= ahead.epoch {
                self.forget_epochs(scope);
            } else {
                tracing::warn!("{log}'s step {} is under epoch {}, which no entry made: passed over", ahead.at, ahead.epoch);
                self.with(|i| i.r.store.set_position(&log, ahead.at, read.floor))?;
            }
        }
    }

    /// Once this device has read a scope's entries up to step `at`, and hasn't said so since it started: a copy of them
    /// in the bucket if it's due, and their newest hint moved on (`src/log.rs`).
    pub(super) async fn hint_log(&self, scope: &Scope, bucket: &impl crate::bucket::Bucket, key: &crate::db::Keys, at: u64) {
        let log = scope.to_string();
        if self.0.hinted.lock().unwrap().get(&log) == Some(&at) {
            return;
        }
        // With any fork's evidence, so a device that reads only the copy finds the fork too.
        let entries = || self.with(|i| Ok([i.r.store.scope(&log)?, i.r.store.forked(&log)?].concat()));
        match crate::log::hint(bucket, key, &crate::log::prefix(&log), at, entries).await {
            Ok(()) => drop(self.0.hinted.lock().unwrap().insert(log, at)),
            Err(err) => tracing::debug!("{log}'s copy: {err:#}"),
        }
    }

    /// Writes an entry this device made to its scope's entries in the bucket, at the first position free.
    /// → the position it was written at.
    pub(super) async fn put_entry(&self, scope: &Scope, entry: &Entry) -> Result<u64> {
        use crate::bucket::Bucket;
        let (bucket, key, prefix) = (self.bucket()?, self.log_key(scope).await?, crate::log::prefix(&scope.to_string()));
        let bytes = crate::entry::pack(&[entry]);
        let written = self.0.written.lock().unwrap().get(scope).copied().unwrap_or(0);
        let mut at = self.with(|i| i.r.store.position(&scope.to_string()))?.0.max(written);
        for _ in 0..1000 {
            at += 1;
            let name = crate::db::step_name(&prefix, at);
            if Bucket::create(&bucket, &name, &crate::log::seal(&key, &name, &bytes)?).await? {
                self.0.written.lock().unwrap().insert(scope.clone(), at);
                return Ok(at);
            }
        }
        bail!("{scope} changed too often to write to")
    }

    /// The keys a scope's entries are sealed with in the bucket, by epoch: the node's `log` of the scope. Of one this
    /// device runs but doesn't open, those a caller who does handed it.
    pub(super) async fn log_key(&self, scope: &Scope) -> Result<crate::db::Keys> {
        if !self.with(|i| Ok(self.opens(i.r.db(), scope)))? {
            return self.handed_log(scope).await;
        }
        self.database_keys(scope, members::APP, crate::log::NAME, false).await
    }

    /// The keys of `global`'s entries in each of its epochs given.
    async fn log_key_of(&self, epochs: &crate::db::Keys) -> Result<crate::db::Keys> {
        let bucket = self.bucket()?;
        let mut out = crate::db::Keys::new();
        for (n, epoch) in epochs {
            if let Some(app) = crate::db::app_key(&bucket, epoch, "global", members::APP, *n, false).await? {
                out.insert(*n, crate::cloudflare::database_key(&app, crate::log::NAME));
            }
        }
        Ok(out)
    }

    /// Tells the devices that hold `scope` it changed, so they read it now rather than at their next look: its entries,
    /// or with `wrote`, step `at` of an app's database, which it sends them unless it's big. Each applies a step only
    /// once the bucket confirms it holds those bytes, so nothing a device sends can be anything but the bucket's, and
    /// QUIC proves who sent it: it needs no signature.
    pub(super) fn ping(&self, scope: &Scope, wrote: Option<(&str, &str, u64)>) {
        let Ok(devices) = self.with(|i| {
            let m = i.r.db();
            Ok(m.computes().into_iter().filter(|s| s.key != self.0.id && m.runs_for(&s.key, scope)).map(|s| s.key).collect::<Vec<_>>())
        }) else {
            return;
        };
        let mut input = json!({ "scope": scope.to_string() });
        if let Some((app, database, at)) = wrote {
            input = json!({ "scope": scope.to_string(), "app": app, "database": database, "at": at });
            let name = crate::db::step_name(&database_prefix(scope, app, database), at);
            let bucket = self.bucket().ok();
            if let Some(step) = bucket.as_ref().and_then(|b| b.kept(&name).ok().flatten()).filter(|s| s.len() <= SENT_STEP) {
                input["step"] = json!(data_encoding::BASE64.encode(&step));
                // And the signature beside it, which an older device ignores.
                if let Some(sig) = bucket.as_ref().and_then(|b| b.kept(&format!("{name}.sig")).ok().flatten()) {
                    input["sig"] = json!(data_encoding::BASE64.encode(&sig));
                }
            }
        }
        self.notify(&devices, &input);
    }

    /// What another device said changed in `scope`, which this device holds: its entries, which it reads now, or with
    /// `wrote`, step `at` of a database. If it has the database open, it fetches that step into its copy now: the one
    /// sent, once the bucket confirms it, else the bucket's. Until then a read asks the bucket first.
    pub(super) fn changed_there(&self, scope: &Scope, wrote: Option<(&str, &str, u64)>, step: Option<Vec<u8>>, sig: Option<Vec<u8>>) {
        if !self.with(|i| Ok(i.r.holds(scope))).unwrap_or(false) {
            return;
        }
        let (node, scope) = (self.clone(), scope.clone());
        let Some((app, database, at)) = wrote.map(|(a, d, at)| (a.to_string(), d.to_string(), at)) else {
            tokio::spawn(async move {
                let _ = node.read_log(&scope).await;
            });
            return;
        };
        let id = (scope, format!("{app}/{database}"));
        self.0.behind.lock().unwrap().insert(id.clone());
        let open = self.open_dbs().iter().any(|(open, _)| *open == id);
        match step.filter(|_| open) {
            Some(step) => self.confirm_sent(Sent { name: crate::db::step_name(&database_prefix(&id.0, &app, &database), at), step, sig, id, at }),
            None => drop(tokio::spawn(async move { node.fill_to(&[(id, at)]).await })),
        }
    }

    /// Asks the bucket to confirm a step another device sent, with every other sent while it's asked about the last
    /// lot: a burst of steps is one ask.
    fn confirm_sent(&self, sent: Sent) {
        let first = {
            let mut waiting = self.0.sent.lock().unwrap();
            let first = waiting.is_none();
            waiting.get_or_insert_default().push(sent);
            first
        };
        if !first {
            return;
        }
        let node = self.clone();
        tokio::spawn(async move {
            loop {
                let lot = {
                    let mut waiting = node.0.sent.lock().unwrap();
                    let lot = std::mem::take(waiting.as_mut().expect("asking"));
                    if lot.is_empty() {
                        *waiting = None;
                        break;
                    }
                    lot
                };
                let mut steps: Vec<(String, Vec<u8>)> = lot.iter().map(|s| (s.name.clone(), s.step.clone())).collect();
                // The signatures sent beside them after them, so what's taken of the steps comes first.
                steps.extend(lot.iter().filter_map(|s| Some((format!("{}.sig", s.name), s.sig.clone()?))));
                // A bucket slow to answer holds up no other: what it doesn't confirm in time is read from it.
                let took = match node.bucket() {
                    Ok(bucket) => tokio::time::timeout(BUCKET_WAIT, bucket.take_sent(&steps)).await.unwrap_or_else(|_| Err(anyhow!("the bucket didn't answer"))),
                    Err(err) => Err(err),
                };
                match took {
                    Ok(took) => {
                        for (sent, _) in lot.iter().zip(took).filter(|(_, took)| !took) {
                            tracing::debug!("{}: the bucket didn't confirm the step sent, so it's read from the bucket", sent.name);
                        }
                    }
                    Err(err) => tracing::debug!("confirming steps other devices sent: {err:#}"),
                }
                node.fill_to(&lot.into_iter().map(|s| (s.id, s.at)).collect::<Vec<_>>()).await;
            }
        });
    }

    /// Brings this device's copy of each open database up to step `at`: from what it has, then from the bucket for what
    /// it lacks. One that's there no longer needs a read to ask the bucket first.
    async fn fill_to(&self, wrote: &[((Scope, String), u64)]) {
        let Ok(bucket) = self.bucket() else { return };
        let mut apps = Vec::new();
        for (id, at) in wrote {
            if wrote.iter().any(|(other, later)| other == id && later > at) {
                continue;
            }
            let open = self.open_dbs().into_iter().find(|(open, _)| open == id).map(|(_, db)| db);
            let filled = match open.as_ref().map(|db| db.try_lock().map(|db| (db.prefix().to_string(), db.at()))) {
                Some(Ok((prefix, from))) => {
                    let here = from + bucket.here_only().fill(&prefix, from).await.unwrap_or(0);
                    here >= *at || bucket.fill(&prefix, here).await.is_ok_and(|n| here + n >= *at)
                }
                _ => false,
            };
            if filled {
                self.0.behind.lock().unwrap().remove(id);
            }
            apps.extend(id.1.split('/').next().filter(|app| *app != members::APP && !apps.iter().any(|a: &String| a == app)).map(str::to_string));
        }
        if !apps.is_empty() {
            self.emit(Event::Changed { apps });
        }
    }

    /// Tells every other device which services serve its local user here and how loaded it is, now and then.
    pub(super) async fn presence_loop(self) {
        loop {
            if self.has_install() {
                self.say_hello();
                self.end_expired_grants();
                self.start_owed_keys();
            }
            tokio::time::sleep(PRESENCE_EVERY).await;
        }
    }

    /// Says hello to every other device: as now and then, and when this device's services change. How long each takes
    /// to answer is how far it is.
    pub(super) fn say_hello(&self) {
        let Ok((devices, me, services, parts)) = self.with(|i| {
            let devices: Vec<Key> = i.r.db().computes().into_iter().map(|s| s.key).filter(|k| *k != self.0.id).collect();
            Ok((devices, Caller { key: self.0.id, user: i.user.clone(), agent: None, token: None }, self.services_here(i), self.parts_here(i)))
        }) else {
            return;
        };
        let input = json!({ "services": services, "load": crate::load::current(), "parts": parts });
        let req = CallRequest { app: members::APP.into(), op: "hello".into(), input, ..Default::default() };
        for device in devices {
            let (node, req, me) = (self.clone(), req.clone(), me.clone());
            tokio::spawn(async move {
                let asked = Instant::now();
                if node.ask_device(device, &req, &me).await.is_ok() {
                    let _ = node.with(|i| Ok(i.far.insert(device, asked.elapsed())));
                }
            });
        }
        self.hello_services();
    }

    /// Says hello to each service endpoint of the install, which says what it offers and whom it serves: it's listed
    /// as a device offering that service, as another device's hello lists those there.
    pub(super) fn hello_services(&self) {
        let Ok(services) = self.with(|i| Ok(i.r.db().machines().into_iter().filter(|m| i.r.db().service(&m.key).is_some()).map(|m| (m.key, m.service.clone().unwrap_or_default())).collect::<Vec<_>>())) else {
            return;
        };
        for (key, _) in services {
            let node = self.clone();
            tokio::spawn(async move {
                node.hello_service_now(&key).await;
            });
        }
    }

    /// Says hello to one service endpoint now, and lists what it offers as another device's hello does. → whether it
    /// answered as one that takes this device's calls.
    pub(super) async fn hello_service_now(&self, key: &Key) -> bool {
        let key = *key;
        let Some(name) = self.with(|i| Ok(i.r.db().machine_ever(&key).and_then(|m| m.service.clone()))).ok().flatten() else { return false };
        let asked = Instant::now();
        let offers = match self.hello_service(key).await {
            Ok(Some(offers)) => offers,
            Ok(None) => {
                tracing::debug!("service {} doesn't take calls from this device yet", short(&key));
                return false;
            }
            Err(err) => {
                tracing::debug!("service {}: {err:#}", short(&key));
                return false;
            }
        };
        let (mut describe, _) = offers;
        // Whom it serves is what its listing says, never what it says of itself.
        let users: Vec<String> = self.with(|i| { let m = i.r.db(); Ok(m.users().into_iter().map(|u| u.id).filter(|u| m.may(u, "use", &scopes::service(&key))).collect()) }).unwrap_or_default();
        describe["name"] = json!(name);
        let services: Vec<Service> = vec![(name.clone(), users, describe)];
        let changed = self.with(|i| {
            i.far.insert(key, asked.elapsed());
            let before = i.peers.insert(key, Peer { services: services.clone(), load: None, parts: None, seen: Instant::now() });
            Ok(before.is_none_or(|p| p.services != services || p.seen.elapsed() >= 3 * PRESENCE_EVERY))
        });
        if changed.unwrap_or(false) {
            self.emit(Event::Changed { apps: vec![name] });
        }
        true
    }
}

/// A step another device sent, waiting for the bucket to confirm it: its name, its bytes, the signature beside it if it
/// came with one, its database and position.
pub(super) struct Sent {
    name: String,
    step: Vec<u8>,
    sig: Option<Vec<u8>>,
    id: (Scope, String),
    at: u64,
}

/// Where a database's steps are in the bucket.
fn database_prefix(scope: &Scope, app: &str, database: &str) -> String {
    format!("{}/{app}/{database}", crate::bucket::folder(&scope.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node whose who's who stops building answers nobody, yet still reads `global`, so the entry after builds it
    /// again: one that couldn't read it would answer nobody for good.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_node_whose_whos_who_stops_building_answers_nobody_until_it_reads_global_and_builds_again() {
        let dir = std::env::temp_dir().join(format!("one-whos-who-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("device.json"), json!({ "secret": hex(&rand::random()), "name": "a" }).to_string()).unwrap();
        let a = Node::start(&dir, crate::services::built_in()).await.unwrap();
        a.create("Home", "Andre").await.unwrap();
        let me = Caller { key: a.id(), user: a.user().unwrap(), agent: None, token: None };
        let may = || a.with(|i| Ok(i.r.db().may(&me.user, "use", scopes::INSTALL))).unwrap();
        assert!(may());

        // A setting is written, but who's who doesn't build with it: no one may do anything.
        a.with(|i| Ok(i.r.fail = true)).unwrap();
        assert!(a.set_setting(&me, "fails", Some("yes")).await.is_err());
        assert!(!may());

        // Once it builds, reading `global` finds the setting and builds it again.
        a.with(|i| Ok(i.r.fail = false)).unwrap();
        a.read_log(&Scope::Global).await.unwrap();
        assert!(may());
        assert_eq!(a.with(|i| Ok(i.r.db().settings().get("fails").cloned())).unwrap().as_deref(), Some("yes"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
