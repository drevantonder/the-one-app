//! The scopes this device holds: reading each from the bucket, and what devices tell each other about them: that one
//! changed (pings), and who is on (presence).

use super::*;

/// The largest step a `changed` carries: a bigger one, the devices it tells read from the bucket.
const SENT_STEP: usize = 1 << 20;

impl Node {
    /// Reads what each scope this device holds has in the bucket after what it last read, `global` first, until it
    /// holds no scope it didn't.
    pub(super) async fn read_scopes(&self) -> Result<()> {
        let held = || self.with(|i| Ok(i.r.db().scopes()?.into_iter().filter(|s| i.r.holds(s)).collect::<Vec<Scope>>()));
        let mut read: Vec<Scope> = Vec::new();
        loop {
            let mut scopes: Vec<Scope> = held()?.into_iter().filter(|s| !read.contains(s)).collect();
            if scopes.is_empty() {
                break;
            }
            scopes.sort_by_key(|s| *s != Scope::Global);
            for scope in scopes {
                match self.read_scope(&scope).await {
                    // One whose newest key this device wasn't given yet: it reads the rest.
                    Err(err) if scope != Scope::Global && err.is::<NotGiven>() => tracing::debug!("{err:#}"),
                    other => other?,
                }
                read.push(scope);
            }
        }
        // Having read them all can make it ready (`ready`): its settling says so to whoever waits (`warm`).
        if !self.with(|i| Ok(std::mem::replace(&mut i.read, true)))? {
            self.0.changed.notify_one();
        }
        Ok(())
    }

    pub(super) async fn read_scope(&self, scope: &Scope) -> Result<()> {
        let _write = self.0.writes.lock().await;
        self.read_scope_now(scope).await
    }

    /// `read_scope`, while this device holds its writes: `global`'s steps, its first epochs first while it has read no
    /// freeze (`first_global_read`), or another scope's own, once it has an epoch.
    pub(super) async fn read_scope_now(&self, scope: &Scope) -> Result<()> {
        if *scope == Scope::Global {
            if self.with(|i| Ok(i.r.freeze().is_none()))? {
                self.first_global_read().await?;
            }
            self.read_steps_now().await?;
        } else if self.with(|i| Ok(i.r.db().epoch(scope)?))? > 0 {
            self.read_scope_steps_now(scope).await?;
        }
        Ok(())
    }

    /// The keys a scope's database's steps are sealed with in the bucket, by epoch. Of one this
    /// device runs but doesn't open, those a caller who does handed it.
    pub(super) async fn step_keys(&self, scope: &Scope) -> Result<crate::database::stored::Keys> {
        if !self.with(|i| self.opens(i.r.db(), scope))? {
            return self.handed_step_keys(scope).await;
        }
        self.database_keys(scope, scopes::APP, scopes::CoreDb::Scope.name(), false).await
    }

    /// The keys of `global`'s steps in each of its epochs given.
    pub(super) async fn step_keys_of(&self, epochs: &crate::database::stored::Keys) -> Result<crate::database::stored::Keys> {
        crate::keys::database_keys(&self.bucket()?, "global", scopes::APP, scopes::CoreDb::Scope.name(), epochs, false).await
    }

    /// Tells the devices that hold `scope` it changed, so they read it now rather than at their next look: its steps,
    /// or with `wrote`, step `at` of an app's database, which it sends them unless it's big. Each applies a step only
    /// once the bucket confirms it holds those bytes, so nothing a device sends can be anything but the bucket's, and
    /// QUIC proves who sent it: it needs no signature.
    pub(super) fn ping(&self, scope: &Scope, wrote: Option<(&str, &str, u64)>) {
        let Ok(devices) = self.with(|i| {
            let m = i.r.db();
            Ok(m.computes()?.into_iter().filter(|s| s.key != self.0.id && m.runs_for(&s.key, scope)).map(|s| s.key).collect::<Vec<_>>())
        }) else {
            return;
        };
        let mut input = json!({ "scope": scope.to_string() });
        if let Some((app, database, at)) = wrote {
            input = json!({ "scope": scope.to_string(), "app": app, "database": database, "at": at });
            let name = crate::database::stored::step_name(&crate::bucket::database_prefix(&scope.to_string(), app, database), at);
            let bucket = self.bucket().ok();
            if let Some(step) = bucket.as_ref().and_then(|b| b.kept(&name).ok().flatten()).filter(|s| s.len() <= SENT_STEP) {
                input["step"] = json!(data_encoding::BASE64.encode(&step));
                if let Some(sig) = bucket.as_ref().and_then(|b| b.kept(&format!("{name}.sig")).ok().flatten()) {
                    input["sig"] = json!(data_encoding::BASE64.encode(&sig));
                }
            }
        }
        self.notify(&devices, &input);
    }

    /// What another device said changed in `scope`, which this device holds: its steps, which it reads now, or with
    /// `wrote`, step `at` of a database. If it has the database open, it fetches that step into its copy now: the one
    /// sent, once the bucket confirms it, else the bucket's. Until then a read asks the bucket first.
    pub(super) fn changed_there(&self, scope: &Scope, wrote: Option<(&str, &str, u64)>, step: Option<Vec<u8>>, sig: Option<Vec<u8>>) {
        if !self.with(|i| Ok(i.r.holds(scope))).unwrap_or(false) {
            return;
        }
        let (node, scope) = (self.clone(), scope.clone());
        let Some((app, database, at)) = wrote.map(|(a, d, at)| (a.to_string(), d.to_string(), at)) else {
            tokio::spawn(async move {
                if let Err(err) = node.read_scope(&scope).await {
                    tracing::warn!("reading {scope}, which another device said changed: {err:#}");
                }
            });
            return;
        };
        let id = (scope, format!("{app}/{database}"));
        self.0.databases.behind.lock().unwrap().insert(id.clone());
        let open = self.open_dbs().iter().any(|(open, _)| *open == id);
        match step.filter(|_| open) {
            Some(step) => self.confirm_sent(Sent { name: crate::database::stored::step_name(&crate::bucket::database_prefix(&id.0.to_string(), &app, &database), at), step, sig, id, at }),
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
                self.0.databases.behind.lock().unwrap().remove(id);
            }
            apps.extend(id.1.split('/').next().filter(|app| *app != scopes::APP && !apps.iter().any(|a: &String| a == app)).map(str::to_string));
        }
        if !apps.is_empty() {
            self.emit(Event::Changed { apps });
        }
    }

    /// Tells every other device how loaded it is and what its parts are doing, now and then.
    pub(super) async fn presence_loop(self) {
        loop {
            if self.has_network() {
                self.say_hello();
                self.check_relay();
                self.renew_app_schedules("");
                self.start_owed_keys();
            }
            tokio::time::sleep(PRESENCE_EVERY).await;
        }
    }

    /// Says hello to every other device: as now and then, and when this device's parts change. How far each is, is
    /// the round trip iroh measures to it.
    pub(super) fn say_hello(&self) {
        let Ok((devices, parts)) = self.with(|i| {
            let devices: Vec<Key> = i.r.db().computes()?.into_iter().map(|s| s.key).filter(|k| *k != self.0.id).collect();
            Ok((devices, self.parts_here(i)?))
        }) else {
            return;
        };
        let hello = json!({ "load": crate::load::current(), "parts": parts });
        for device in devices {
            let (node, hello) = (self.clone(), hello.clone());
            tokio::spawn(async move {
                if node.say_hello_to(&device, &hello).await.is_ok()
                    && let Some(far) = node.round_trip(&device)
                {
                    node.with_or(None, |i| Ok(i.far.insert(device, far)));
                }
            });
        }
        self.hello_services();
    }

    pub(super) fn hello_from(&self, from: &Key, hello: &Value) -> Result<()> {
        let load = serde_json::from_value(hello["load"].clone()).ok();
        let parts = hello["parts"].as_array().map(|_| hello["parts"].clone());
        let new = self.with(|i| {
            let changed = i.peers.get(from).is_none_or(|p| p.parts != parts);
            i.peers.insert(*from, Peer { services: Vec::new(), load, parts, seen: Instant::now() });
            Ok(changed)
        })?;
        if new {
            self.emit(Event::Devices);
        }
        Ok(())
    }

    /// Says hello to each service endpoint of the network, which says what it offers and whom it serves: it's listed
    /// as a device offering that service, as another device's hello lists those there.
    pub(super) fn hello_services(&self) {
        let Ok(services) = self.with(|i| {
            let mut services = Vec::new();
            for m in i.r.db().machines()? {
                if i.r.db().service(&m.key)?.is_some() {
                    services.push((m.key, m.service.clone().unwrap_or_default()));
                }
            }
            Ok(services)
        }) else {
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
        let Some(name) = self.with_or(None, |i| Ok(i.r.db().machine_ever(&key)?.and_then(|m| m.service.clone()))) else { return false };
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
        let (describe, _) = offers;
        // Whom it serves is what its listing says, never what it says of itself.
        let users: Vec<String> = self.with_or(Default::default(), |i| { let m = i.r.db(); Ok(m.users()?.into_iter().map(|u| u.id).filter(|u| m.may(u, "use", &scopes::service(&key))).collect()) });
        self.learn_app(&name).await;
        let services: Vec<Service> = vec![(name.clone(), users, self.listed_as_app(&name, describe))];
        let far = self.round_trip(&key);
        let changed = self.with(|i| {
            if let Some(far) = far {
                i.far.insert(key, far);
            }
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

