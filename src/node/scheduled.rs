//! Schedules: the kernel's schedules service runs them, each with only the keys of the scope and app
//! its person let it use. A node tells the service what runs as a signed-in person whenever they change it, and runs
//! each slot the service hands in (`POST /v1/kernel/run`) as that person, with the run's own token.

use std::collections::BTreeSet;

use super::*;

impl Node {
    /// The schedules service the install's schedules run from, once its kernel is set up: (the install's address, the
    /// service's).
    pub fn schedules_service(&self) -> Option<(String, String)> {
        self.with(|i| {
            let s = &i.r.members.settings;
            Ok(s.get("address").cloned().zip(s.get("kernel_schedules").cloned()))
        })
        .ok()
        .flatten()
    }

    /// Brings what `me` has let run at the schedules service in line with the schedules that run as them here, in the
    /// background: after they install an app, or switch one of its schedules.
    pub(super) fn sync_schedules_later(&self, me: &Caller) {
        let (node, me) = (self.clone(), me.clone());
        tokio::spawn(async move {
            if let Err(err) = node.sync_schedules(&me).await {
                tracing::warn!("telling the schedules service what runs as {}: {err:#}", me.user);
            }
        });
    }

    /// Lets each schedule that runs as `me` here run at the schedules service, and stops each there that doesn't: one
    /// whose app was removed, that was switched off, or whose deploy changed when it runs.
    pub(super) async fn sync_schedules(&self, me: &Caller) -> Result<()> {
        let Some((address, service)) = self.schedules_service() else { return Ok(()) };
        let token = me.token.clone().filter(|t| !t.starts_with("run.")).with_context(|| format!("sign in first ({} sign-in): schedules here run from your One App's kernel, with what your sign-in lets them use", crate::config::CLI))?;
        let declared = self.declared().await;
        // (scope, app, schedule) → when it runs, as the service says it.
        let want: BTreeMap<(String, String, String), (Value, Schedule)> = self.with(|i| {
            let m = &i.r.members;
            let mut want = BTreeMap::new();
            for (log, (_, state)) in &i.views {
                let Ok(scope) = Scope::parse(log) else { continue };
                for (app, list) in declared.iter().filter(|(app, _)| state.installed.contains(app)) {
                    for s in list.iter().filter_map(|s| s.as_ref().ok()) {
                        if runs_as(m, state, &scope, app, &s.name).as_deref() == Some(me.user.as_str()) {
                            want.insert((scope.to_string(), app.clone(), s.name.clone()), (when(&s.spec()), s.clone()));
                        }
                    }
                }
            }
            Ok(want)
        })?;
        let ask = |method: &'static str, path: String, body: Option<Value>| {
            let (url, token) = (format!("{}{path}", service.trim_end_matches('/')), token.clone());
            async move {
                tokio::task::spawn_blocking(move || -> Result<Value> {
                    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
                    let bearer = format!("Bearer {token}");
                    let mut resp = match (method, body) {
                        ("GET", _) => agent.get(&url).header("authorization", &bearer).call()?,
                        ("DELETE", _) => agent.delete(&url).header("authorization", &bearer).call()?,
                        (_, body) => agent.post(&url).header("authorization", &bearer).send_json(body.unwrap_or_default())?,
                    };
                    let out: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
                    ensure!(resp.status() == 200, "the schedules service said: {}", out["error"].as_str().unwrap_or("nothing"));
                    Ok(out)
                })
                .await?
            }
        };
        let mut have = BTreeSet::new();
        for s in ask("GET", "/schedules".into(), None).await?["schedules"].as_array().cloned().unwrap_or_default() {
            let id = (s["scope"].as_str().unwrap_or("").to_string(), s["app"].as_str().unwrap_or("").to_string(), s["name"].as_str().unwrap_or("").to_string());
            if want.get(&id).is_some_and(|(w, _)| *w == when(&s)) && s["node"].as_str() == Some(address.as_str()) {
                have.insert(id);
            } else {
                ask("DELETE", format!("/schedules/{}", s["id"].as_str().unwrap_or("")), None).await?;
            }
        }
        for ((scope, app, name), (when, schedule)) in want.into_iter().filter(|(id, _)| !have.contains(id)) {
            let title = self.0.apps.describe(&app).await.ok().and_then(|d| d["title"].as_str().map(str::to_string)).unwrap_or_else(|| app.clone());
            let mut body = json!({
                "app": app, "name": name, "scope": scope, "op": schedule.op, "input": schedule.input, "node": address,
                "title": if schedule.description.is_empty() { name.clone() } else { schedule.description.clone() },
                "what": title,
            });
            for (k, v) in when.as_object().into_iter().flatten() {
                body[k] = v.clone();
            }
            ask("POST", "/schedules".into(), Some(body)).await?;
        }
        Ok(())
    }

    /// Runs a slot the schedules service handed in, as the person who let it run, with the run's token (its keys
    /// reach only that scope and app): once, whichever server takes its slot first, and only while it still runs as
    /// them here. How it went is written as every run's is.
    pub async fn kernel_run(&self, me: &Caller, run: &crate::kernel::Run) -> Result<Value> {
        let scope = Scope::parse(&run.scope)?;
        let slot: u64 = run.run.rsplit_once('/').and_then(|(_, s)| s.parse().ok()).context("that isn't a run of a schedule's slot")?;
        let schedule = self.0.apps.describe(&run.app).await.ok().and_then(|d| Schedule::all(&d).into_iter().filter_map(Result::ok).find(|s| s.name == run.schedule));
        let schedule = schedule.with_context(|| format!("{}'s latest deploy has no schedule {}", run.app, run.schedule))?;
        self.with(|i| {
            let state = i.views.get(&scope.to_string()).map(|(_, s)| s).with_context(|| format!("this device doesn't hold {scope}"))?;
            ensure!(state.installed.contains(&run.app), "{} isn't installed in {scope}", run.app);
            ensure!(runs_as(&i.r.members, state, &scope, &run.app, &run.schedule).as_deref() == Some(me.user.as_str()), "{} doesn't run as you in {scope} any more", run.schedule);
            Ok(())
        })?;
        let lease = format!("{}/{}/slots/{}/{}/{slot}", crate::bucket::folder(&scope.to_string()), members::APP, run.app, schedule.name);
        if !crate::bucket::Bucket::create(&self.bucket()?, &lease, hex(&self.0.id).as_bytes()).await? {
            return Ok(json!({ "ran": "already" }));
        }
        let started = Instant::now();
        let req = CallRequest { app: run.app.clone(), op: schedule.op.clone(), input: schedule.input.clone(), scopes: Some(vec![scope.to_string()]), client: Some("schedule".into()), ..Default::default() };
        let ran = match self.count(&run.app, "schedule").await {
            Err(err) => Err(err),
            Ok(()) => Box::pin(self.route(req, me.clone())).await,
        };
        let error = ran.as_ref().err().map(|e| format!("{e:#}").chars().take(500).collect::<String>());
        let said = status::Run { slot, time: now_ms(), server: self.0.id, user: me.user.clone(), ok: error.is_none(), error, ms: started.elapsed().as_millis() as u64, spec: schedule.spec() };
        self.say(&scope, status::Said::Ran { app: run.app.clone(), schedule: schedule.name.clone(), run: said }).await?;
        Ok(json!({ "ran": ran.is_ok() }))
    }
}

/// When a schedule runs, as the schedules service keeps it: `{ every }` or `{ at, tz, days }`.
fn when(spec: &Value) -> Value {
    match spec["every"].as_str() {
        Some(every) => json!({ "every": every }),
        None => json!({ "at": spec["at"], "tz": spec["tz"].as_str().unwrap_or("UTC"), "days": spec["days"].as_array().cloned().unwrap_or_default() }),
    }
}

impl Node {
    /// Each loaded app's schedules, as its latest deploy declares them, with why any don't make sense.
    pub(super) async fn declared(&self) -> Vec<(String, Vec<std::result::Result<Schedule, (String, String)>>)> {
        let mut out = Vec::new();
        for app in self.0.apps.names().into_iter().filter(|a| !self.deleted(a)) {
            if let Ok(d) = self.0.apps.describe(&app).await {
                let list = Schedule::all(&d);
                if !list.is_empty() {
                    out.push((app, list));
                }
            }
        }
        out
    }

    /// An app's schedules where it's installed for `viewer` and this server holds the scope: what each runs and when,
    /// whether it's on, whom it runs as, when it runs next, its last run, and whether a deploy changed or removed it;
    /// and its runs, newest first.
    pub(super) fn schedules_of(&self, viewer: &Caller, app: &str, declared: &[std::result::Result<Schedule, (String, String)>]) -> Result<(Vec<Value>, Vec<Value>)> {
        let now = now_ms();
        self.with(|i| {
            let m = &i.r.members;
            let user_name = |u: &str| m.users.get(u).map(|u| u.name.clone()).unwrap_or_default();
            let (mut list, mut activity) = (Vec::new(), Vec::new());
            for scope in reachable(i, app, &viewer.user) {
                let Some((_, state)) = i.views.get(&scope.to_string()) else { continue };
                if !state.installed.contains(app) {
                    continue;
                }
                let status = i.status.get(&scope.to_string()).cloned().unwrap_or_default();
                let scope_name = handles(m, std::slice::from_ref(&scope), &viewer.user).remove(0).name;
                let base = |name: &str| {
                    let key = (app.to_string(), name.to_string());
                    let runs = status.runs.get(&key);
                    let last = runs.and_then(|r| r.last()).map(|r| json!({ "time": r.time, "ok": r.ok, "error": r.error }));
                    let changed = status.changed.get(&key).filter(|(.., t)| now.saturating_sub(*t) < CHANGED_SHOWN).map(|(was, is, t)| json!({ "was": schedules::say(was), "is": schedules::say(is), "time": t }));
                    json!({ "name": name, "scope": scope.to_string(), "scope_name": scope_name, "may_switch": members::manages_scope(m, &viewer.user, &scope), "last": last, "changed": changed })
                };
                for s in declared {
                    let mut v = match s {
                        Ok(s) => {
                            let mut v = base(&s.name);
                            let user = runs_as(m, state, &scope, app, &s.name);
                            let switched = state.switches.get(&(app.to_string(), s.name.clone()));
                            v["description"] = json!(s.description);
                            v["when"] = json!(s.say());
                            v["op"] = json!(s.op);
                            v["on"] = json!(switched.is_none_or(|(on, ..)| *on));
                            v["switched"] = json!(switched.map(|(on, by, t)| json!({ "on": on, "by": user_name(by), "time": t })));
                            v["runs_as"] = json!(user.as_deref().map(user_name));
                            v["next"] = json!(user.and(s.next(now)));
                            v
                        }
                        Err((name, why)) => {
                            let mut v = base(name);
                            v["invalid"] = json!(why);
                            v
                        }
                    };
                    v["removed"] = json!(false);
                    list.push(v);
                }
                // Schedules a deploy removed that ran lately.
                for ((a, name), runs) in &status.runs {
                    let gone = a == app && !declared.iter().any(|s| match s {
                        Ok(s) => &s.name == name,
                        Err((n, _)) => n == name,
                    });
                    if gone && runs.last().is_some_and(|r| now.saturating_sub(r.time) < CHANGED_SHOWN) {
                        let mut v = base(name);
                        v["removed"] = json!(true);
                        v["was"] = json!(runs.last().map(|r| schedules::say(&r.spec)));
                        list.push(v);
                    }
                }
                for ((a, name), runs) in status.runs.iter().filter(|((a, _), _)| a == app) {
                    let _ = a;
                    for r in runs {
                        activity.push(json!({ "schedule": name, "scope": scope.to_string(), "scope_name": scope_name, "time": r.time, "ok": r.ok, "error": r.error, "ms": r.ms, "server": m.machines.get(&r.server).map(|s| s.name.clone()), "user": user_name(&r.user) }));
                    }
                }
            }
            activity.sort_by_key(|r| std::cmp::Reverse(r["time"].as_u64().unwrap_or(0)));
            activity.truncate(50);
            Ok((list, activity))
        })
    }

    /// Switches one of an app's schedules on or off in `scope`, for `caller`; switching one on makes it run as them.
    pub async fn switch_schedule(&self, caller: &Caller, app: &str, schedule: &str, scope: &Scope, on: bool) -> Result<()> {
        let content = self.with(|i| switch_entry(i, &caller.user, scope, app, schedule, on))?;
        let kernel = self.schedules_service().is_some();
        ensure!(!kernel || caller.token.is_some(), "sign in first ({} sign-in): schedules here run from your One App's kernel, with what your sign-in lets them use", crate::config::CLI);
        let by = (caller.key != self.0.id).then_some(caller.key);
        self.append_as(scope, &caller.user, by, content).await?;
        if kernel {
            self.sync_schedules(caller).await.context("it's switched, but the schedules service didn't take it")?;
        }
        Ok(())
    }
}

/// Whom a schedule of `app` in a scope runs as: whoever switched it on last, or else installed the app there, while
/// they still manage the scope; nobody if it's switched off.
fn runs_as(m: &Members, state: &ScopeState, scope: &Scope, app: &str, schedule: &str) -> Option<String> {
    let manages = |u: &String| members::manages_scope(m, u, scope);
    match state.switches.get(&(app.to_string(), schedule.to_string())) {
        Some((false, ..)) => None,
        Some((true, user, _)) if manages(user) => Some(user.clone()),
        _ => state.installers.get(app).map(|(u, _)| u.clone()).filter(manages),
    }
}
