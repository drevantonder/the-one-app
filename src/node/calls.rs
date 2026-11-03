//! Calls: where each runs, running it here or on another device, what it wrote, and what its app asks the node while it runs.

use super::*;

impl Node {
    /// Runs an operation for this device's local user: here, or on the nearest device that can.
    pub async fn call(&self, req: CallRequest) -> Result<Value> {
        self.call_as(req, Caller { key: self.0.id, user: self.user()?, agent: None, token: None }).await
    }

    /// Runs an operation for `me`, whom this device acts for: here, or on the nearest device that can.
    pub async fn call_as(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let me = Caller { agent: req.agent.clone().or(me.agent), ..me };
        self.route(req, me).await
    }

    /// A call from a kernel sign-in, which is its person's in every way but one: the node's own operations are its
    /// devices' to ask, and a sign-in's caller has this device's key.
    pub fn for_sign_in(&self, req: CallRequest) -> Result<CallRequest> {
        let req = self.resolve(req);
        ensure!(req.app != members::APP, "a sign-in can't ask that");
        Ok(req)
    }

    /// Tries the devices that might run a call, nearest first, one at a time, the next only if one can't be reached
    /// or says to go elsewhere.
    pub(super) async fn route(&self, req: CallRequest, me: Caller) -> Result<Value> {
        let mut req = self.resolve(req);
        // A client's own id, so a call it sends again runs once, names its lease in the bucket: only a short word.
        let id = req.id.get_or_insert_with(new_id);
        ensure!(id.len() <= 64 && !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "a call's id is up to 64 letters, digits, - and _");
        if req.device.as_deref() == Some("all") {
            return self.everywhere(req, me).await;
        }
        let mut why = Vec::new();
        for device in self.candidates(&req, &me)? {
            let started = Instant::now();
            let reply = match device == self.0.id {
                // Keys it lacks are its caller's to hand this device, which they were sealed to: no other runs it.
                true => Ok(self.run_or_need(&req, &me).await?),
                false => self.ask_device(device, &req, &me).await,
            };
            let name = self.name_of(&device);
            match reply {
                Ok(Reply::Ok(v)) => {
                    let _ = self.with(|i| Ok(i.nearest.insert(device, started.elapsed())));
                    return Ok(v);
                }
                Ok(Reply::Error(e)) => bail!("{e}"),
                Ok(Reply::Elsewhere(w)) => why.push(format!("{name}: {w}")),
                Err(err) => {
                    let _ = self.with(|i| Ok(i.nearest.insert(device, Duration::MAX)));
                    why.push(format!("{name}: {err:#}"));
                }
            }
        }
        if why.is_empty() {
            bail!("None of your devices can do this: no device runs {} for you", req.app);
        }
        bail!("None of your devices can do this now. Try again soon. ({})", why.join("; "))
    }

    /// Runs a sidecar's call on every device that runs it for the caller, one at a time: `{ <device>: answer }`, or
    /// `{ error }` for one that failed. Only for sidecars: an app's call runs once.
    pub(super) async fn everywhere(&self, req: CallRequest, me: Caller) -> Result<Value> {
        ensure!(req.sidecar.is_some(), "{} is an app: its calls run once, on one device", req.app);
        let req = CallRequest { device: None, ..req };
        let mut out = serde_json::Map::new();
        for device in self.candidates(&req, &me)? {
            let reply = match device == self.0.id {
                true => Ok(self.run(&req, &me).await),
                false => self.ask_device(device, &req, &me).await,
            };
            let answer = match reply {
                Ok(Reply::Ok(v)) => v,
                Ok(Reply::Error(e) | Reply::Elsewhere(e)) => json!({ "error": e }),
                Err(err) => json!({ "error": format!("{err:#}") }),
            };
            out.insert(self.name_of(&device), answer);
        }
        ensure!(!out.is_empty(), "None of your devices can do this: no device runs {} for you", req.app);
        Ok(Value::Object(out))
    }

    /// The devices that might run a call, in the order it tries them: those that are up with room first; then the
    /// near before the far (FAR); then by the install's order (src/rank.rs); this device before another as good; then
    /// the nearest. A sidecar about its own machine runs here first.
    pub(super) fn candidates(&self, req: &CallRequest, me: &Caller) -> Result<Vec<Key>> {
        if let Some(device) = &req.device {
            // A sidecar's call goes to the service offering it on that device, if one serves the caller: their own
            // first, then the device itself, then one its person shares with everyone.
            if req.sidecar.is_some() {
                let found = self.find_service(&req.app, device, &me.user, true).or_else(|| self.find_device(device).ok()).or_else(|| self.find_service(&req.app, device, &me.user, false));
                if let Some(found) = found {
                    return Ok(vec![found]);
                }
            }
            return Ok(vec![self.find_device(device)?]);
        }
        let here = self.runs_sidecar_here(&req.app);
        // A sidecar, or one of its operations, that's about its own machine (`"prefer": "here"`: Shell, models_here) runs
        // on the caller's first.
        let about_here = |d: &Value| d["prefer"] == "here" || d["ops"].as_array().is_some_and(|ops| ops.iter().any(|o| o["name"] == req.op.as_str() && o["prefer"] == "here"));
        let here_first = here && req.sidecar.is_some() && self.with(|i| Ok(self.sidecars_here(i).iter().any(|d| d.0 == req.app && about_here(&d.2))))?;
        self.with(|i| {
            let m = &i.r.members;
            let mut devices: Vec<Key> = if req.sidecar.is_none() {
                m.computes().map(|s| s.key).filter(|s| m.serves(s, &me.user)).collect()
            } else {
                let mut s: Vec<Key> = i.peers.iter().filter(|(k, _)| i.online(k)).filter(|(_, s)| s.sidecars.iter().any(|d| d.0 == req.app && d.1.contains(&me.user))).map(|(k, _)| *k).collect();
                if here && (me.user == i.user || (self.0.sidecars.serves_everyone(&req.app) && i.r.members.serves(&self.0.id, &me.user))) {
                    s.push(self.0.id);
                }
                s
            };
            let set = rank::set(&i.r.members.settings);
            let here_load = crate::load::current();
            let rank = |k: &Key| {
                let here = *k == self.0.id;
                let load = if here { Some(&here_load) } else { i.peers.get(k).and_then(|p| p.load.as_ref()) };
                let kind = load.map(|l| l.kind.as_str()).unwrap_or("");
                let far = !here && i.far.get(k).is_some_and(|d| *d > FAR);
                (!here && !i.online(k), here_first && !here, load.is_some_and(full), far, rank::order(&set, &hex(k), kind), !here, i.nearest.get(k).copied().unwrap_or(Duration::from_secs(1)))
            };
            devices.sort_by_cached_key(rank);
            Ok(devices)
        })
    }

    /// A call as the node runs it: one that names a sidecar is that sidecar's, whatever app shares its name. As older
    /// machines ask for a sidecar, so is one that names an app with no deploy, or that runs on every device.
    pub(super) fn resolve(&self, mut req: CallRequest) -> CallRequest {
        if let Some(sidecar) = &req.sidecar {
            req.app = sidecar.clone();
        } else if req.app != members::APP && (!self.deployed(&req.app) || req.device.as_deref() == Some("all")) {
            req.sidecar = Some(req.app.clone());
        }
        req
    }

    /// Asks another device to run a call: a sign-in's with the keys its caller handed this device, sealed to that one.
    pub(super) async fn ask_device(&self, device: Key, req: &CallRequest, me: &Caller) -> Result<Reply> {
        // A sign-in's token goes along, with the keys it was handed: to a service only if it serves everyone, which
        // checks it itself, and never keys. A call handed keys with no sign-in passes on its keys alone (below).
        let service = self.is_service(&device);
        let shared = self.with(|i| Ok(i.r.members.service(&device).is_some_and(|s| s.shared)))?;
        let token = me.token.as_ref().filter(|t| !sessions::unsigned(t) && (!service || (shared && sessions::is_sign_in(t))));
        let mut request = json!({ "app": req.app, "sidecar": req.sidecar, "op": req.op, "input": req.input, "scopes": req.scopes, "depth": req.depth, "from": req.from, "client": req.client, "me": { "key": hex(&me.key), "user": me.user, "agent": me.agent, "token": token } });
        // Keys handed here with no sign-in (a client device's, or with a call passed on) go on to another node sealed
        // to it for this node's key, of the scopes the call reaches alone, which it runs on them alone.
        if let Some(unsigned) = me.token.as_ref().filter(|t| sessions::unsigned(t))
            && !service
        {
            let scopes = self.with(|i| call_scopes(i, &self.resolve(req.clone()), &me.user))?;
            let (keys, until) = self.passing_on(unsigned, &scopes);
            let seal = self.with(|i| Ok(i.r.members.seal_key(&device)))?.context("that device has no key to seal to")?;
            let named: Vec<String> = scopes.iter().map(Scope::to_string).collect();
            request = crate::keys::forwarded(&request, &me.user, req.client.as_deref().unwrap_or(""), &self.0.id, &seal, until, keys, &named)?["body"]["call"].take();
            request["me"]["key"] = json!(hex(&self.0.id));
            request["me"]["agent"] = json!(me.agent);
        }
        if let Some(token) = token
            && !service
            && let Some(seal) = self.with(|i| Ok(i.r.members.seal_key(&device)))?
            && let Some(keys) = self.hand_on(token, &seal)
        {
            request["keys"] = json!(keys);
        }
        let reply = self.ask(device, request, self.limit(req).await + Duration::from_secs(5)).await?;
        Ok(Reply::from_json(reply))
    }

    /// Runs a call here, for `me`, or says why it can't.
    pub(super) async fn run(&self, req: &CallRequest, me: &Caller) -> Reply {
        self.run_or_need(req, me).await.unwrap_or_else(|need| Reply::Error(need.to_string()))
    }

    /// `run`, but a call that lacks keys its caller can hand fails with what it needs.
    pub(super) async fn run_or_need(&self, req: &CallRequest, me: &Caller) -> Result<Reply, sessions::Needs> {
        let req = &self.resolve(req.clone());
        let limit = self.limit(req).await;
        let result = tokio::time::timeout(limit, self.run_here(req, me, limit)).await;
        Ok(match result {
            Ok(Ok(reply)) => reply,
            Ok(Err(err)) => match err.downcast::<sessions::Needs>() {
                Ok(need) => return Err(need),
                Err(err) => Reply::Error(format!("{err:#}")),
            },
            Err(_) => Reply::Error(format!("it took longer than {}s and was stopped", limit.as_secs())),
        })
    }

    /// How long a call may run: an app's, CALL_LIMIT; a sidecar's, and that of an app that uses sidecars (which waits
    /// on theirs), SIDECAR_LIMIT.
    pub(super) async fn limit(&self, req: &CallRequest) -> Duration {
        if req.app == members::APP {
            return CALL_LIMIT;
        }
        if req.sidecar.is_some() || !self.uses_here(&req.app).await.1.is_empty() { SIDECAR_LIMIT } else { CALL_LIMIT }
    }

    pub(super) async fn run_here(&self, req: &CallRequest, me: &Caller, limit: Duration) -> Result<Reply> {
        let input = if req.input.is_null() { json!({}) } else { req.input.clone() };
        if req.app == members::APP {
            return self.node_op(&req.op, &input, me).await.map(Reply::Ok);
        }
        if req.sidecar.is_some() {
            if !self.runs_sidecar_here(&req.app) {
                return Ok(Reply::Elsewhere(format!("{} doesn't run here", req.app)));
            }
            return match self.run_sidecar(&req.app, &req.op, input, me, req.from.clone()).await {
                Err(err) if err.is::<crate::sidecars::Elsewhere>() => Ok(Reply::Elsewhere(err.to_string())),
                other => other.map(Reply::Ok),
            };
        }
        let title = self.title(&req.app).await;
        let prepared = self.with(|i| prepare(self, i, req, me, &title));
        let (ctx, scopes) = match prepared? {
            Ok(p) => p,
            Err(why) => return Ok(Reply::Elsewhere(why)),
        };
        if !self.ready() {
            return Ok(Reply::Elsewhere("it isn't ready yet".into()));
        }
        let deployed_here = self.0.bundles.lock().unwrap().get(&req.app).cloned();
        let latest = self.with(|i| Ok(i.r.members.apps.get(&req.app).and_then(|a| a.bundle.clone())))?;
        if deployed_here.is_none() || deployed_here != latest {
            return Ok(Reply::Elsewhere(format!("it doesn't have {}'s latest version yet", req.app)));
        }
        if self.deleted(&req.app) {
            return Ok(Reply::Error(format!("{title} was deleted: its owners can restore it from Recently Deleted in the App Store")));
        }
        if let Some(why) = self.stopped(&req.app) {
            self.record(&req.app, req, me, Some(&why));
            return Ok(Reply::Error(why));
        }
        // A sign-in's call that lacks a key of a scope it names fails here, before it runs or writes anything: its
        // app's database's, and the Vault's there too if it says it uses items.
        let mut want: Vec<(Scope, &str, &str)> = scopes.iter().map(|s| (s.clone(), req.app.as_str(), DATABASE)).collect();
        if self.0.apps.describe(&req.app).await?["vault"].is_object() {
            want.extend(scopes.iter().map(|s| (s.clone(), members::APP, crate::vault::DATABASE)));
        }
        self.needs(me, &want).await?;
        // A call that writes, tried on another device before this one, ran there if it took the lease first.
        if let (Some(id), Some(scope)) = (&req.id, scopes.first()) {
            let reads = self.0.apps.describe(&req.app).await?["ops"].as_array().into_iter().flatten().any(|o| o["name"] == req.op.as_str() && o["reads"] == true);
            let lease = format!("{}/{}/calls/{id}", crate::bucket::folder(&scope.to_string()), req.app);
            if !reads && !crate::bucket::Bucket::create(&self.bucket()?, &lease, hex(&self.0.id).as_bytes()).await? {
                return Ok(Reply::Error("This may already be done: it ran on another device, whose answer was lost. Check before trying again.".into()));
            }
        }
        let mut ctx = ctx;
        ctx.from = req.from.clone();
        let granted = self.grant(&req.app, me, &mut ctx, req.depth);
        // Just under the call's own limit, so a call that's stopped is recorded first.
        let ran = tokio::time::timeout(limit - Duration::from_secs(1), async {
            let mut outcome = self.0.apps.call(&req.app, &req.op, input, &ctx, limit).await?;
            self.write_appends(&req.app, std::mem::take(&mut outcome.appends), &scopes, me, req.from.as_deref()).await?;
            anyhow::Ok(outcome)
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("it took longer than {}s and was stopped", limit.as_secs())));
        let outcome = match ran {
            Ok(outcome) => outcome,
            Err(err) => {
                self.record(&req.app, req, me, Some(&format!("{err:#}")));
                return Err(err);
            }
        };
        self.record(&req.app, req, me, None);
        if outcome.later {
            self.keep_for_later(granted);
        }
        Ok(Reply::Ok(outcome.result))
    }

    /// Records, for an app's owners, a call that ran here: the first each day by each user from each client
    /// (`used`), and every failure (`failed`), at most `FAILURES_KEPT` an hour per app on this device. They go in the
    /// owner's scope when the caller may use it and this device holds it, else in `global`; in the background.
    pub(super) fn record(&self, app: &str, req: &CallRequest, me: &Caller, error: Option<&str>) {
        let now = now_ms();
        let client = req.client.clone().unwrap_or_else(|| "api".into());
        let said = match error {
            None => {
                let day = now / 86_400_000;
                if !self.0.seen.lock().unwrap().insert((app.to_string(), me.user.clone(), client.clone(), day)) {
                    return;
                }
                status::Said::Used { app: app.into(), user: me.user.clone(), client, agent: me.agent.clone(), day }
            }
            Some(error) => {
                let hour = now / 3_600_000;
                let mut failures = self.0.failures.lock().unwrap();
                let n = failures.entry((app.to_string(), hour)).or_default();
                *n += 1;
                if *n > FAILURES_KEPT {
                    return;
                }
                let error: String = error.lines().next().unwrap_or("").chars().take(300).collect();
                status::Said::Failed { app: app.into(), op: req.op.clone(), error, user: me.user.clone() }
            }
        };
        self.write_insight(app, me, said);
    }

    /// Writes an insight about `app` for `me`'s call: in the app owner's scope when `me` may use it and this device
    /// holds it, else in `global`; in the background.
    pub(super) fn write_insight(&self, app: &str, me: &Caller, said: status::Said) {
        let Ok(scope) = self.with(|i| {
            let m = &i.r.members;
            let owner = m.apps.get(app).map(|a| a.owner.clone()).context("no such app")?;
            let own = match owner {
                Principal::User(u) => Scope::User(u),
                Principal::Group(g) => Scope::Group(g),
                _ => Scope::Global,
            };
            Ok(if m.may_use(&me.user, &own) && i.r.holds(&own) { own } else { Scope::Global })
        }) else {
            return;
        };
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(err) = node.say(&scope, said).await {
                tracing::debug!("insights: {err:#}");
            }
        });
    }

    /// What an app's owners see of it, from every scope `viewer` may use here: who used it, when and from what;
    /// its errors grouped, with counts; and what it spent on AI, by model. Only for someone acting for its owner.
    pub(super) fn insights_of(&self, viewer: &Caller, app: &str) -> Result<Option<Value>> {
        self.with(|i| {
            let m = &i.r.members;
            let Some(a) = m.apps.get(app) else { return Ok(None) };
            if !m.acts_for(&viewer.user, &a.owner) {
                return Ok(None);
            }
            let name = |u: &str| m.users.get(u).map(|u| u.name.clone()).unwrap_or_default();
            let mut people: BTreeMap<(String, String), (u64, u64, Option<String>)> = BTreeMap::new();
            let mut errors: BTreeMap<(String, String), (u64, u64, u64, String)> = BTreeMap::new();
            let mut cost: BTreeMap<String, (f64, u64, u64, u64)> = BTreeMap::new();
            for (log, status) in &i.status {
                let Ok(scope) = Scope::parse(log) else { continue };
                let Some(ins) = status.insights.get(app).filter(|_| m.may_use(&viewer.user, &scope)) else { continue };
                for ((user, client), u) in &ins.used {
                    let p = people.entry((user.clone(), client.clone())).or_default();
                    p.0 += u.days;
                    p.1 = p.1.max(u.last);
                    if u.agent.is_some() {
                        p.2 = u.agent.clone();
                    }
                }
                for (key, e) in &ins.errors {
                    let x = errors.entry(key.clone()).or_insert((0, u64::MAX, 0, String::new()));
                    x.0 += e.count;
                    x.1 = x.1.min(e.first);
                    if e.last >= x.2 {
                        x.2 = e.last;
                        x.3 = name(&e.user);
                    }
                }
                for (model, c) in &ins.cost {
                    let x = cost.entry(model.clone()).or_default();
                    x.0 += c.usd;
                    x.1 += c.input;
                    x.2 += c.output;
                    x.3 += c.calls;
                }
            }
            let mut usage: Vec<Value> = people.into_iter().map(|((u, client), (days, last, agent))| json!({ "user": name(&u), "client": client, "agent": agent, "days": days, "last": last })).collect();
            usage.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let mut errors: Vec<Value> = errors.into_iter().map(|((op, error), (count, first, last, user))| json!({ "op": op, "error": error, "count": count, "first": first, "last": last, "user": user })).collect();
            errors.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let cost: Vec<Value> = cost.into_iter().map(|(model, (usd, input, output, calls))| json!({ "model": model, "usd": usd, "input": input, "output": output, "calls": calls })).collect();
            Ok(Some(json!({ "usage": usage, "errors": errors, "cost": cost })))
        })
    }

    /// Writes a call's appends as its caller. Every one is checked before any is written: a call that fails writes
    /// nothing. Installs come last, since installing a built-in app the install doesn't have yet adds it first.
    pub(super) async fn write_appends(&self, app: &str, appends: Vec<Append>, scopes: &[Scope], me: &Caller, from: Option<&str>) -> Result<()> {
        // What it spent on AI counts against the app that called it, if one did.
        let (costs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Cost { .. }));
        for cost in costs {
            if let Append::Cost { data } = cost {
                let to = from.unwrap_or(app);
                let said = status::Said::Cost {
                    app: to.into(),
                    model: data["model"].as_str().unwrap_or("?").into(),
                    usd: data["usd"].as_f64().unwrap_or(0.0),
                    input: data["input"].as_u64().unwrap_or(0),
                    output: data["output"].as_u64().unwrap_or(0),
                };
                self.write_insight(to, me, said);
            }
        }
        let schedules_changed = appends.iter().any(|a| matches!(a, Append::Install { .. } | Append::Switch { .. }));
        let scheduling: Vec<String> = appends
            .iter()
            .filter_map(|a| match a {
                Append::Switch { app, on: true, .. } | Append::Install { app, installed: true, .. } => Some(app.clone()),
                _ => None,
            })
            .collect();
        let (installs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Install { .. }));
        let (execs, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Exec { .. }));
        let (items, appends): (Vec<_>, Vec<_>) = appends.into_iter().partition(|a| matches!(a, Append::Vault { .. }));
        // Each scope's statements, in the order the call ran them, and the blobs they keep.
        type Statement = (String, Vec<Value>, Option<String>);
        let mut statements: Vec<(Scope, Vec<Statement>, Vec<String>)> = Vec::new();
        for exec in execs {
            let Append::Exec { scope, sql, params, blobs, check } = exec else { continue };
            let scope = Scope::parse(&scope)?;
            ensure!(scopes.contains(&scope), "the call didn't name {scope}");
            ensure!(blobs.iter().all(|b| b.len() == 64 && b.bytes().all(|c| c.is_ascii_hexdigit())), "a blob is named by its hash");
            match statements.iter_mut().find(|(s, _, _)| *s == scope) {
                Some((_, list, kept)) => {
                    list.push((sql, params, check));
                    kept.extend(blobs);
                }
                None => statements.push((scope, vec![(sql, params, check)], blobs)),
            }
        }
        let mut writes = Vec::new();
        for append in appends {
            let title = match &append {
                Append::Switch { app, .. } | Append::Retire { app, .. } | Append::Source { app, .. } => self.title(app).await,
                _ => String::new(),
            };
            writes.push(self.with(|i| check_append(i, append, scopes, me, &title))?);
        }
        self.may_schedule(me, &scheduling.iter().map(String::as_str).collect::<Vec<_>>()).await?;
        let wrote = !statements.is_empty();
        for (scope, list, kept) in statements {
            let (db, bucket) = self.call_database(me, &scope, app, DATABASE, None).await?;
            let mut db = db.lock().await;
            let at = db.at();
            self.write_in(&scope, &mut db, &bucket, |c| crate::db::statements(c, &list)).await?;
            if db.at() > at {
                self.ping(&scope, Some((app, DATABASE, db.at())));
            }
            drop(db);
            if !kept.is_empty() {
                writes.push((scope, Content { blobs: kept, ..members::node("keep", json!({})) }));
            }
        }
        if wrote {
            self.emit(Event::Changed { apps: vec![app.to_string()] });
        }
        for item in items {
            if let Append::Vault { id, scope, data, same, release } = item {
                self.call_item(app, me, scopes, &id, scope, data, same, release).await?;
            }
        }
        for append in installs {
            if let Append::Install { app, scope, installed } = append {
                let scope = if scope == "user" { Scope::User(me.user.clone()) } else { Scope::parse(&scope)? };
                writes.extend(self.install_entries(me, &app, &scope, installed).await?);
            }
        }
        for (scope, content) in writes {
            self.append_as(&scope, &me.user, content).await?;
        }
        if schedules_changed && self.schedules_service().is_some() && me.token.is_some() {
            self.sync_schedules_later(me);
        }
        Ok(())
    }

    /// Keeps a call's grant after it returned, while the work it left running (`ctx.waitUntil`) goes on: its requests
    /// still go out as the call's, and it hands in its appends (`POST /appends`) until it says it's done, or for
    /// `LATER_LIMIT` at most.
    pub(super) fn keep_for_later(&self, granted: Granted) {
        let done = Arc::new(tokio::sync::Notify::new());
        self.0.later.lock().unwrap().insert(granted.1.clone(), done.clone());
        let node = self.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(LATER_LIMIT, done.notified()).await;
            node.0.later.lock().unwrap().remove(&granted.1);
            drop(granted);
        });
    }

    /// Runs a built-in sidecar or sidecar here, for this device's local user.
    pub(super) async fn run_sidecar(&self, app: &str, op: &str, input: Value, me: &Caller, from: Option<String>) -> Result<Value> {
        let everyone = self.0.sidecars.serves_everyone(app) && self.with(|i| Ok(i.r.members.serves(&self.0.id, &me.user)))?;
        let user = if everyone { me.user.clone() } else { self.user()? };
        ensure!(me.user == user, "{} runs only for {}'s own user", self.sidecar_title(app), self.device_name());
        let caller = self.me_for(me)?;
        // An app's call has the fields of the items it's switched on for filled in, a secret one only for a sidecar
        // named among its item's hosts, and blotted out of the answer.
        let mut input = input;
        let used = std::sync::Mutex::new(Vec::<String>::new());
        if let Some(from) = from.as_ref().filter(|_| input.to_string().contains("{{")) {
            let scopes = self.with(|i| Ok(reachable(i, from, &me.user).into_iter().filter(|s| i.r.holds(s)).collect::<Vec<_>>()))?;
            let lookup = self.lookup(me, from, &scopes).await?;
            crate::vault::fill_value(&mut input, app, &|name| {
                let found = lookup.get(name);
                if let Some((hosts, value)) = &found {
                    if !hosts.is_empty() {
                        used.lock().unwrap().push(value.clone());
                    }
                }
                found
            })?;
        }
        let built_in = self.0.built_in_sidecars.iter().find(|n| n.name == app);
        let mut answer = match built_in {
            Some(n) => (n.call)(op.to_string(), input, caller.clone(), if n.records { self.records(app).await? } else { vec![] }).await?,
            None => self.0.sidecars.call(app, op, input, caller.clone(), from).await?,
        };
        crate::vault::blot(&mut answer.ok, &used.into_inner().unwrap());
        let scope = Scope::User(user.clone());
        if !answer.records.is_empty() {
            let (db, bucket) = self.call_database(me, &scope, app, SIDECAR_DATABASE, Some(vec![crate::sidecars::RECORDS.into()])).await?;
            let (time, by, device) = (now_ms(), caller.user_name.clone(), caller.name.clone());
            let mut db = db.lock().await;
            self.write_in(&scope, &mut db, &bucket, |c| {
                for (kind, data) in &answer.records {
                    let row = [json!(kind), json!(data.to_string()), json!(time), json!(by), json!(device)];
                    crate::db::execute(c, "insert into records (kind, data, time, by, device) values (?, ?, ?, ?, ?)", &row)?;
                }
                Ok(())
            })
            .await?;
            self.ping(&scope, Some((app, SIDECAR_DATABASE, db.at())));
            drop(db);
            if let Some(n) = built_in.filter(|n| n.sync.is_some()) {
                self.sync_built_in(n.name);
            }
        }
        if !answer.blobs.is_empty() {
            self.append_as(&scope, &user, Content { blobs: answer.blobs, ..members::node("keep", json!({})) }).await?;
        }
        Ok(answer.ok)
    }

    /// The node's own operations, which other devices ask for.
    pub(super) async fn node_op(&self, op: &str, input: &Value, me: &Caller) -> Result<Value> {
        match op {
            // Another device's hints, which only a device of the install gives.
            "hello" | "changed" if self.with(|i| Ok(i.r.members.compute(&me.key).is_some()))? => {
                if op == "hello" {
                    let sidecars: Vec<Sidecar> = serde_json::from_value(input["sidecars"].clone()).unwrap_or_default();
                    let load = serde_json::from_value(input["load"].clone()).ok();
                    // The sidecars that came, went or changed there, whose apps shells list again.
                    let (new, changed) = self.with(|i| {
                        let before = i.peers.get(&me.key).map(|p| p.sidecars.clone());
                        let was = before.as_deref().unwrap_or_default();
                        let missing = |a: &[Sidecar], b: &[Sidecar]| a.iter().filter(|s| !b.contains(s)).map(|s| s.0.clone()).collect::<Vec<_>>();
                        let mut changed = missing(&sidecars, was);
                        changed.extend(missing(was, &sidecars));
                        changed.sort();
                        changed.dedup();
                        i.peers.insert(me.key, Peer { sidecars, load, seen: Instant::now() });
                        Ok((before.is_none(), changed))
                    })?;
                    if new || !changed.is_empty() {
                        self.emit(Event::Devices);
                    }
                    if !changed.is_empty() {
                        self.emit(Event::Changed { apps: changed });
                    }
                    return Ok(json!(null));
                }
                let scope = Scope::parse(input["scope"].as_str().unwrap_or(""))?;
                let wrote = input["app"].as_str().zip(input["database"].as_str()).map(|(app, db)| (app, db, input["at"].as_u64().unwrap_or(0)));
                self.changed_there(&scope, wrote, None);
                Ok(json!(null))
            }
            other => bail!("the node has no operation {other}"),
        }
    }

    /// Lets the requests of a call, widget or hook of `app` through the node as `caller`'s, while it runs.
    pub(super) fn grant(&self, app: &str, caller: &Caller, ctx: &mut Context, depth: u32) -> Granted {
        let token = hex(&rand::random::<[u8; 32]>());
        let scopes = ctx.scopes.iter().filter_map(|s| Scope::parse(&s.id).ok()).collect();
        let grant = Grant { app: app.to_string(), caller: caller.clone(), scopes, read_only: ctx.read_only, puts: Vec::new(), caught_up: Vec::new(), depth, from: ctx.from.clone() };
        self.0.grants.lock().unwrap().insert(token.clone(), grant);
        ctx.token = token.clone();
        Granted(self.clone(), token)
    }

    /// Counts one of `app`'s calls of other apps, or one run of its schedules (`kind`). Past `RATE_LIMIT` this minute,
    /// the app is stopped here: it's looping.
    pub(super) async fn count(&self, app: &str, kind: &'static str) -> Result<()> {
        let now = now_ms();
        let tripped = {
            let mut g = self.0.guard.lock().unwrap();
            let (since, n) = g.counts.entry((app.to_string(), kind)).or_insert((now, 0));
            if now.saturating_sub(*since) >= 60_000 {
                (*since, *n) = (now, 0);
            }
            *n += 1;
            *n == RATE_LIMIT + 1
        };
        if tripped {
            let what = if kind == "call" { "called other apps" } else { "ran its schedules" };
            let why = format!("{} {what} more than {RATE_LIMIT} times in a minute on {}, so it was stopped there: is it looping? It runs again once it's deployed again, or in {} minutes.", self.title(app).await, self.device_name(), STOPPED_FOR / 60_000);
            let bundle = self.with(|i| Ok(i.r.members.apps.get(app).and_then(|a| a.bundle.clone()))).ok().flatten();
            self.0.guard.lock().unwrap().stopped.insert(app.to_string(), (why.clone(), now_ms() + STOPPED_FOR, bundle));
            tracing::warn!("{why}");
        }
        match self.stopped(app) {
            Some(why) => bail!("{why}"),
            None => Ok(()),
        }
    }

    /// Why `app` is stopped here, if it is: it looped, and hasn't been deployed again or waited out its time since.
    pub(super) fn stopped(&self, app: &str) -> Option<String> {
        let latest = self.with(|i| Ok(i.r.members.apps.get(app).and_then(|a| a.bundle.clone()))).ok().flatten();
        let mut g = self.0.guard.lock().unwrap();
        let (why, until, bundle) = g.stopped.get(app)?;
        if now_ms() < *until && *bundle == latest {
            return Some(why.clone());
        }
        g.stopped.remove(app);
        None
    }

    /// The ports this node serves on this machine: its local API, workerd, and apps' outbound requests.
    pub fn own_ports(&self) -> Vec<u16> {
        let api = crate::api::addr().rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(crate::config::PORT);
        vec![api, self.0.apps.port(), self.0.outbound]
    }

    /// The grant of a call running now.
    pub fn grant_of(&self, token: &str) -> Option<Grant> {
        self.0.grants.lock().unwrap().get(token).cloned()
    }

    /// Runs an app's hook for an agent harness on this machine. The node remembers, per
    /// session, when the hook last ran, and tells the app as `since` (null on a session's first call).
    pub async fn hook(&self, app: &str, name: &str, event: Value) -> Result<Value> {
        let session = event["session_id"].as_str().unwrap_or("").to_string();
        let path = self.0.dir.join("hooks.json");
        let mut seen: serde_json::Map<String, Value> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let key = format!("{app}/{name}/{session}");
        let since = seen.get(&key).cloned().unwrap_or(Value::Null);
        let now = now_ms();
        let caller = Caller { key: self.0.id, user: self.user()?, agent: None, token: None };
        let mut ctx = self.read_context(app, &caller)?;
        let _granted = self.grant(app, &caller, &mut ctx, 0);
        let out = self.0.apps.hook(app, name, json!({ "event": event, "since": since }), &ctx).await?;
        seen.insert(key, json!(now));
        let week_ago = now.saturating_sub(7 * 86_400_000);
        seen.retain(|_, t| t.as_u64().is_some_and(|t| t > week_ago));
        std::fs::write(&path, serde_json::to_vec(&seen)?)?;
        Ok(out)
    }

    /// What the shim asks the node for a running call (`src/outbound.rs`): `POST /call { app, op, input }` runs an
    /// operation of an app or sidecar the calling app uses, as its caller; `POST /blob` keeps a blob, `GET
    /// /blob/<hash>` gets one the caller may read; `GET /catalog` lists the install's apps, `GET /devices` its devices.
    pub async fn internal(&self, token: &str, method: &str, path: &str, body: Vec<u8>) -> Result<axum::response::Response> {
        use axum::response::IntoResponse;
        let grant = &self.grant_of(token).context("only a running call asks the node")?;
        let ok = |v: Value| axum::Json(v).into_response();
        Ok(match (method, path) {
            ("POST", "/call") => {
                ensure!(!grant.read_only, "this call only reads: it can't run other apps' operations");
                let req: Value = serde_json::from_slice(&body)?;
                let (app, op) = (req["app"].as_str().unwrap_or(""), req["op"].as_str().unwrap_or(""));
                // `ctx.sidecars` says it's a sidecar's; `ctx.apps`, an app's (or, from older shims, either).
                let sidecar = req["sidecar"] == true;
                let (apps, sidecars) = self.uses(&grant.app).await;
                let used = if sidecar { sidecars.iter().any(|u| u == app) } else { apps.iter().chain(&sidecars).any(|u| u == app) };
                ensure!(used, "{} doesn't use {app}", grant.app);
                ensure!(grant.depth < MAX_DEPTH, "{} called {app} {} calls deep, and apps call each other at most {MAX_DEPTH} deep: is it looping?", grant.app, grant.depth + 1);
                self.count(&grant.app, "call").await?;
                // An app may say which device runs it, like the one with a local model.
                let device = req["device"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                let call = CallRequest { app: app.into(), sidecar: sidecar.then(|| app.into()), op: op.into(), input: req["input"].clone(), device, depth: grant.depth + 1, from: Some(grant.app.clone()), client: Some("app".into()), ..Default::default() };
                ok(json!({ "ok": Box::pin(self.route(call, grant.caller.clone())).await? }))
            }
            ("POST", "/query") => {
                let req: Value = serde_json::from_slice(&body)?;
                let scope = Scope::parse(req["scope"].as_str().unwrap_or(""))?;
                ensure!(grant.scopes.contains(&scope), "the call didn't name {scope}");
                let params = req["params"].as_array().cloned().unwrap_or_default();
                let (db, bucket) = self.call_database(&grant.caller, &scope, &grant.app, DATABASE, None).await?;
                let mut db = db.lock().await;
                // Once a call, so it reads one state of the scope throughout.
                if !grant.caught_up.contains(&scope) {
                    self.read_ready(&(scope.clone(), format!("{}/{DATABASE}", grant.app)), &mut db, &bucket).await?;
                    if let Some(g) = self.0.grants.lock().unwrap().get_mut(token) {
                        g.caught_up.push(scope.clone());
                    }
                }
                ok(json!({ "ok": db.query(req["sql"].as_str().unwrap_or(""), &params)? }))
            }
            ("GET", "/vault") => ok(json!(self.lookup(&grant.caller, &grant.app, &grant.scopes).await?.for_app())),
            ("POST", "/vault/match") => {
                // The item a sign-in would change rather than make: one of that type whose plain field has that value.
                let req: Value = serde_json::from_slice(&body)?;
                let scope = match req["scope"].as_str() {
                    None | Some("user") => Scope::User(grant.caller.user.clone()),
                    Some(s) => Scope::parse(s)?,
                };
                ensure!(grant.scopes.contains(&scope), "the call didn't name {scope}");
                let (kind, field, value) = (req["type"].as_str().unwrap_or(""), req["field"].as_str().unwrap_or(""), req["value"].as_str());
                let items = self.items(&grant.caller, &scope).await?;
                let id = items.iter().filter(|(_, it)| it.kind == kind && it.fields.get(field).is_some_and(|f| !f.secret && Some(f.value.as_str()) == value)).min_by_key(|(_, it)| it.made).map(|(id, _)| id.clone());
                ok(json!({ "id": id }))
            }
            ("POST", "/blob") => {
                let hash = self.put_blob(&body)?;
                if let Some(g) = self.0.grants.lock().unwrap().get_mut(token) {
                    g.puts.push(hash.clone());
                }
                ok(json!({ "blob": hash }))
            }
            ("GET", p) if p.starts_with("/blob/") => {
                let hash = &p["/blob/".len()..];
                let built_in = self.0.built_in.values().any(|h| h == hash);
                // A call this machine's own programs asked may read what its local API would give them: a file put there to
                // attach. A sign-in's asks here too, but reads only what its person may.
                let local = grant.caller.key == self.0.id && grant.caller.token.is_none() && release::blob_path(&self.0.dir, hash).is_ok_and(|p| p.exists());
                ensure!(built_in || local || grant.puts.iter().any(|h| h == hash) || self.may_read_blob(&grant.caller.user, hash)?, "no such blob for you");
                self.blob(hash).await?.into_response()
            }
            ("POST", "/appends") => {
                ensure!(self.0.later.lock().unwrap().contains_key(token), "only work a call left running after it returned hands in appends later");
                let req: Value = serde_json::from_slice(&body)?;
                let appends = crate::call::appends(&req["appends"])?;
                self.write_appends(&grant.app, appends, &grant.scopes, &grant.caller, grant.from.as_deref()).await?;
                ok(json!({}))
            }
            ("POST", "/done") => {
                let req: Value = serde_json::from_slice(&body).unwrap_or_default();
                if let Some(err) = req["error"].as_str() {
                    tracing::warn!("{}'s work after a call failed: {err}", grant.app);
                }
                if let Some(done) = self.0.later.lock().unwrap().remove(token) {
                    done.notify_one();
                }
                ok(json!({}))
            }
            ("GET", "/catalog") => ok(json!(self.catalog(&grant.caller, true).await?)),
            ("GET", "/failing") => ok(json!(self.failing_schedules(&grant.caller).await?)),
            // A push to the caller's phones, from this app: the install delivers it through the sidecar its `push`
            // setting names, wherever that runs (the device with Apple's push key); with push off, it isn't sent.
            ("POST", "/notify") => {
                ensure!(!grant.read_only, "this call only reads: it can't push");
                let push = self.with(|i| Ok(crate::state::settings(&i.r.members).remove("push").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()))?;
                if push.is_empty() {
                    return Ok(ok(json!({ "ok": { "sent": false, "why": "push is off in this install" } })));
                }
                let input: Value = serde_json::from_slice(&body)?;
                let call = CallRequest { app: push.clone(), sidecar: Some(push), op: "send".into(), input, depth: grant.depth + 1, from: Some(grant.app.clone()), client: Some("app".into()), ..Default::default() };
                ok(json!({ "ok": Box::pin(self.route(call, grant.caller.clone())).await? }))
            }
            ("GET", "/devices") => ok(self.state_for(&grant.caller)["install"]["devices"].take()),
            // Where things run first, as the person the call is for: only an admin changes it.
            ("GET", "/use") => {
                let install = self.state_for(&grant.caller)["install"].take();
                let mut order: Vec<Value> = install["devices"].as_array().cloned().unwrap_or_default().into_iter().filter(|s| s["service"].is_null()).map(|s| json!({ "name": s["name"], "key": s["key"], "kind": s["kind"], "use": s["use"], "online": s["online"], "here": s["me"] })).collect();
                if let Some(use_) = install["serverless"]["use"].as_str() {
                    order.push(json!({ "name": "Cloudflare", "key": rank::CLOUDFLARE, "kind": rank::CLOUDFLARE, "use": use_, "online": true, "here": false }));
                }
                let set = self.with(|i| Ok(rank::set(&i.r.members.settings)))?;
                order.sort_by_cached_key(|s| (rank::order(&set, s["key"].as_str().unwrap_or(""), s["kind"].as_str().unwrap_or("")), s["name"].as_str().unwrap_or("").to_lowercase()));
                ok(json!({ "order": order, "admin": install["role"] == "admin" }))
            }
            ("POST", "/use") => {
                ensure!(!grant.read_only, "this call only reads: it can't change where things run");
                let req: Value = serde_json::from_slice(&body)?;
                self.set_use(&grant.caller, req["device"].as_str().unwrap_or(""), req["use"].as_str().unwrap_or("")).await?;
                ok(json!({}))
            }
            _ => bail!("the node has no {method} {path}"),
        })
    }

    /// Runs a call another device, `remote`, asked for. A sign-in's runs only if its token is its caller's, with the
    /// keys handed with it, never this device's own: from a node, only in scopes that node runs. One with no token runs
    /// only from a client device of its caller, as that device's own: only then are this device's keys used, and only
    /// for its own person, whose device that is.
    pub(super) async fn accept_call(&self, remote: Key, request: &Value) -> Reply {
        match self.accept(remote, request).await {
            Ok((req, me)) => self.run(&req, &me).await,
            Err(err) => Reply::Error(format!("{err:#}")),
        }
    }

    /// Runs a call one of the install's services asked for, as its person, wherever it runs: a service writes nothing
    /// itself, and asks for the calls the program it offers makes (`/v1/call` where it's hosted).
    pub(super) async fn accept_service_call(&self, remote: Key, request: &Value) -> Reply {
        match self.accept_service(remote, request) {
            Ok((req, me)) => self.run(&req, &me).await,
            Err(err) => Reply::Error(format!("{err:#}")),
        }
    }

    /// What a service may have run, as `service::limited` says: only its own app, in its person's own scope, here on
    /// a node of that person's own, whose keys it is run with until grants land, calling no other app. A hacked
    /// service gets no more than its own app's operations for its own person.
    fn accept_service(&self, remote: Key, request: &Value) -> Result<(CallRequest, Caller)> {
        let me = self.check_caller(remote, request)?;
        ensure!(request["me"]["token"].is_null(), "a service asks with no sign-in");
        let (name, owner) = self.with(|i| Ok(i.r.members.service(&remote).map(|s| (s.service.clone().unwrap_or_default(), s.user.clone()))))?.context("not a service of this install")?;
        ensure!(me.key == remote && me.user == owner && owner == self.user()?, "a service's calls run only on its own person's node");
        let mut req: CallRequest = serde_json::from_value(crate::service::limited(&name, request)?)?;
        let resolved = self.resolve(req.clone());
        ensure!(resolved.app == name, "a service asks only for its own app, {name}");
        (req.from, req.depth) = (None, MAX_DEPTH);
        Ok((req, me))
    }

    async fn accept(&self, remote: Key, request: &Value) -> Result<(CallRequest, Caller)> {
        let mut me = self.check_caller(remote, request)?;
        let passed = request["me"]["token"].is_null() && request["keys"].is_string();
        if let Some(token) = request["me"]["token"].as_str() {
            me.token = Some(self.token_of(token, &me.user).await.context("the sign-in it came with isn't its caller's, or has expired")?);
            if let Some(keys) = request["keys"].as_str() {
                self.hand(token, &me.user, keys)?;
            }
        } else if let Some(keys) = request["keys"].as_str() {
            // Passed on with no sign-in: run on the keys it was handed alone, never this device's own.
            me.token = Some(self.take_passed(&remote, &me.user, request, keys, true)?);
        }
        let mut req: CallRequest = serde_json::from_value(request.clone())?;
        let (own, serves) = self.with(|i| Ok((!passed && i.r.members.client(&remote).is_some_and(|c| c.user == me.user), i.r.members.serves(&remote, &me.user))))?;
        // Only a node that runs the user's calls says an app made a call (whose items fill it in), and how deep it is.
        if !serves {
            (req.from, req.depth) = (None, 0);
        }
        // A sidecar that runs as this device's own person (Shell, say) runs only for a device of theirs, never for a
        // sign-in a node passes on.
        let resolved = self.resolve(req.clone());
        ensure!(own || resolved.sidecar.is_none() || self.0.sidecars.serves_everyone(&resolved.app), "{} runs only for its person's own devices", resolved.app);
        if !own && req.sidecar.is_none() && req.app != members::APP {
            let req = self.resolve(req.clone());
            let named = req.scopes.clone();
            self.with(|i| {
                let scopes: Vec<Scope> = match &named {
                    None => reachable(i, &req.app, &me.user),
                    Some(named) => named.iter().map(|s| if s == "user" { Ok(Scope::User(me.user.clone())) } else { Scope::parse(s) }).collect::<Result<_>>()?,
                };
                let m = &i.r.members;
                ensure!(scopes.iter().all(|s| m.runs_for(&remote, s)), "{} doesn't run every scope that call reaches", short(&remote));
                Ok(())
            })?;
        }
        Ok((req, me))
    }

    /// The keys a call `from` another node passed on for `user` with no sign-in was handed (`sealed`): sealed here for
    /// `from`'s key, of the scopes the call names alone, every one of which `user` may use and, for a node on a machine
    /// (`machine`), `from` runs. → what its keys are kept by.
    pub(crate) fn take_passed(&self, from: &Key, user: &str, call: &Value, sealed: &str, machine: bool) -> Result<String> {
        let named = call["scopes"].as_array().context("a call passed on names its scopes")?;
        let scopes: Vec<Scope> = named.iter().map(|s| match s.as_str() {
            Some("user") => Ok(Scope::User(user.to_string())),
            Some(s) => Scope::parse(s),
            None => bail!("a scope is its name"),
        }).collect::<Result<_>>()?;
        self.with(|i| {
            let m = &i.r.members;
            ensure!(scopes.iter().all(|s| m.may_use(user, s)), "that call reaches a scope its person may not use");
            ensure!(!machine || scopes.iter().all(|s| m.runs_for(from, s)), "{} doesn't run every scope that call names", short(from));
            Ok(())
        })?;
        self.hand_passed(from, user, sealed, &scopes)
    }

    /// Who a call another device passed on is for: with a sign-in's token, someone that device's node serves or whose
    /// client it is; with none, only the person whose client device `remote` is, asking itself. The node's own hints
    /// (`hello`, `changed`) come from any compute endpoint of the install, as itself; a person's service asks as them, as itself.
    pub(super) fn check_caller(&self, remote: Key, request: &Value) -> Result<Caller> {
        let key = unhex(request["me"]["key"].as_str().context("who's asking?")?)?;
        let user = request["me"]["user"].as_str().context("who's asking?")?.to_string();
        let token = request["me"]["token"].as_str().is_some();
        // Passed on by another node with no sign-in, the keys it was handed with it, which `take_passed` checks.
        let passed = !token && request["keys"].is_string();
        let hint = request["app"] == members::APP && matches!(request["op"].as_str(), Some("hello" | "changed"));
        self.with(|i| {
            let m = &i.r.members;
            let client = m.client(&remote).is_some_and(|c| c.user == user);
            let service = m.service(&remote).is_some_and(|s| s.user == user);
            let allowed = match () {
                _ if hint => remote == key && m.compute(&remote).is_some(),
                _ if token => client || m.serves(&remote, &user),
                _ if passed => remote == key && m.compute(&remote).is_some(),
                _ => (client || service) && remote == key,
            };
            ensure!(allowed, "{} may not call as that user", short(&remote));
            Ok(())
        })?;
        let agent = request["me"]["agent"].as_str().map(str::to_string);
        Ok(Caller { key, user, agent, token: None })
    }
}

/// The scopes a call reaches for `user`: those it names, or every one its app reaches; a sidecar's, those it names or
/// its person's own.
fn call_scopes(i: &Install, req: &CallRequest, user: &str) -> Result<Vec<Scope>> {
    let named = |s: &String| if s == "user" { Ok(Scope::User(user.to_string())) } else { Scope::parse(s) };
    match (&req.scopes, &req.sidecar) {
        (Some(named_), _) => named_.iter().map(named).collect(),
        (None, Some(_)) => Ok(vec![Scope::User(user.to_string())]),
        (None, None) => Ok(reachable(i, &req.app, user)),
    }
}

/// Checks a call may run here, and makes its context; or says why it should run elsewhere.
fn prepare(node: &Node, i: &Install, req: &CallRequest, me: &Caller, title: &str) -> Result<std::result::Result<(Context, Vec<Scope>), String>> {
    let m = &i.r.members;
    let here = node.0.id;
    if !m.serves(&here, &me.user) {
        return Ok(Err(format!("{} doesn't serve you", node.device_name())));
    }
    let reach = reachable(i, &req.app, &me.user);
    let scopes: Vec<Scope> = match &req.scopes {
        None => reach.clone(),
        Some(named) => {
            let mut out = Vec::new();
            for s in named {
                let s = if s == "user" { Scope::User(me.user.clone()) } else { Scope::parse(s)? };
                ensure!(reach.contains(&s), "{title} can't reach {} for you", scope_name(i, &s.to_string()));
                out.push(s);
            }
            out
        }
    };
    if reach.is_empty() {
        bail!("{title} isn't on your pages: add it first");
    }
    if let Some(s) = scopes.iter().find(|s| !i.r.holds(s)) {
        return Ok(Err(format!("it can't open {} yet", scope_name(i, &s.to_string()))));
    }
    let ctx = Context { me: node_me(node, m, me), now: now_ms(), scopes: handles(m, &scopes, &me.user), read_only: false, token: String::new(), from: None };
    Ok(Ok((ctx, scopes)))
}
