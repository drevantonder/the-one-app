//! The scopes this device holds: reading each from the bucket, and what devices tell each other about them: that one
//! changed (pings), and who is on (presence), with what their services describe of themselves.

use super::*;

/// The largest step a `changed` carries: a bigger one, the devices it tells read from the bucket.
const SENT_STEP: usize = 1 << 20;
/// How often each connection's paths are said in the log (`log_paths`).
const PATHS_EVERY: Duration = Duration::from_secs(5 * 60);

impl Node {
    /// Reads what each scope this device holds has in the bucket after what it last read, `global` first, until it
    /// holds no scope it didn't: of `only`, if given, those, and any it hasn't read yet.
    pub(super) async fn read_scopes(&self, only: Option<&HashSet<Scope>>) -> Result<()> {
        let held = || self.with(|i| Ok(i.r.db().scopes()?.into_iter().filter(|s| i.r.holds(s) && only.is_none_or(|o| o.contains(s) || (*s != Scope::Global && !i.scope_dbs.contains_key(s)))).collect::<Vec<Scope>>()));
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
        // And the relay, which counts each scope's writes for a device whose notice was lost (`Node::moved`).
        self.tell_relay(json!({ "moved": [scope.to_string()] }));
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
        self.hint_changed(&crate::bucket::database_prefix(&scope.to_string(), &app, &database));
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

    /// Tells every other device how loaded it is, now and then.
    pub(super) async fn presence_loop(self) {
        let mut paths_said = Instant::now();
        loop {
            if self.has_network() {
                if paths_said.elapsed() >= PATHS_EVERY {
                    paths_said = Instant::now();
                    self.log_paths();
                }
                self.say_hello();
                self.read_connected();
                self.check_relay();
                self.node_says();
                self.renew_app_schedules("");
                self.start_owed_keys();
                self.name_services_scopes(&crate::oned::carried_helpers()).await;
            }
            tokio::time::sleep(PRESENCE_EVERY).await;
        }
    }

    /// Says hello to every other device, now and then: how loaded this one is, and what each of its parts registered
    /// here describes of itself. How far each is, is the round trip iroh measures to it.
    pub(super) fn say_hello(&self) {
        let Ok(devices) = self.with(|i| Ok(i.r.db().computes()?.into_iter().map(|s| s.key).filter(|k| *k != self.0.id).collect::<Vec<Key>>())) else {
            return;
        };
        let hello = json!({ "load": crate::load::current(), "updating": self.updating(), "services": self.parts_described() });
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
    }

    /// Another device's hello: its load, and what the parts its runtime carries describe of themselves, kept for those
    /// who's who lists as parts of its device.
    pub(super) fn hello_from(&self, from: &Key, hello: &Value) -> Result<()> {
        let load = serde_json::from_value(hello["load"].clone()).ok();
        let changed = self.with(|i| {
            i.peers.insert(*from, Peer { load, updating: hello["updating"].as_str().map(str::to_string), seen: Instant::now() });
            let m = i.r.db();
            let device = m.device_of(from);
            let mut said = HashMap::new();
            for (key, describe) in hello["services"].as_object().into_iter().flatten() {
                if let Ok(key) = crate::keys::unhex(key)
                    && m.service(&key)?.is_some_and(|s| s.device == Some(device))
                {
                    said.insert(key, describe.clone());
                }
            }
            let mut changed = Vec::new();
            for key in i.described.keys().copied().collect::<Vec<_>>() {
                if m.service(&key)?.is_none_or(|s| s.device == Some(device)) && !said.contains_key(&key) {
                    changed.extend(i.described.remove(&key).and_then(|d| d["name"].as_str().map(str::to_string)));
                }
            }
            for (key, describe) in said {
                if i.described.get(&key) != Some(&describe) {
                    changed.extend(describe["name"].as_str().map(str::to_string));
                    i.described.insert(key, describe);
                }
            }
            Ok(changed)
        })?;
        self.0.heard.notify_waiters();
        if !changed.is_empty() {
            self.learn_apps(changed);
        }
        Ok(())
    }

    /// What `device`'s oned runs as processes of its own, and each one's state, as its onet said (`oned::keep_parts`):
    /// shown with its app runtime in Settings › Devices.
    pub(super) fn parts_from(&self, device: &Key, parts: Value) {
        let changed = self.with_or(false, |i| Ok(i.parts.insert(*device, parts.clone()) != Some(parts)));
        if changed {
            self.emit(Event::Devices);
        }
    }

    /// Each service up as this device knows it, with whom it serves (as its listing says, never what it says of
    /// itself) and what it describes of itself: this device's parts as they registered here, another's as its device's
    /// runtime said in its last hello, while it's heard.
    pub(super) fn services_up(&self, i: &Network) -> Result<Vec<(scopes::Machine, Service)>> {
        let m = i.r.db();
        let here = self.0.holds.device();
        let registered: HashMap<String, Value> = self.0.sockets.described().into_iter().collect();
        let mut out = Vec::new();
        for s in m.machines()?.into_iter().filter(|s| !s.removed && s.is(scopes::Kind::Service)) {
            let (Some(name), Some(device)) = (s.service.clone(), s.device) else { continue };
            let describe = if device == here {
                registered.get(&name).cloned()
            } else {
                m.runtime_for(&device)?.filter(|r| i.heard(r)).and_then(|_| i.described.get(&s.key).cloned())
            };
            let Some(mut describe) = describe else { continue };
            describe["runs"] = json!("service");
            let users: Vec<String> = m.users()?.into_iter().map(|u| u.id).filter(|u| m.may(u, "use", &scopes::service(&s.key))).collect();
            let listed = self.listed_as_app(&name, describe);
            out.push((s, (name, users, listed)));
        }
        Ok(out)
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

