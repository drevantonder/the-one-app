//! Schedules: an app's schedules run on the machine its user chose for them (its app schedules, src/node/
//! app_schedules.rs), from its node, with their keys alone: each slot once, whichever such machine takes it first,
//! and none while every one is off.

use crate::protocol::allowed;
use std::collections::BTreeSet;

use super::*;

/// How long an app's page says a deploy changed or removed one of its schedules.
const CHANGED_SHOWN: u64 = 14 * 86_400_000;

const TICK: Duration = Duration::from_secs(10);

impl Node {
    /// Runs the schedules due of each app whose schedules this device runs, every `TICK`, for as long as the node runs.
    pub(super) async fn schedule_loop(self) {
        // Each schedule's next slot, by (scope, app, schedule), with what it was worked out from.
        let mut next: HashMap<(Scope, String, String), (Value, u64)> = HashMap::new();
        loop {
            tokio::time::sleep(TICK).await;
            self.forget_app_schedules();
            if let Err(err) = self.run_due(&mut next).await {
                tracing::debug!("schedules: {err:#}");
            }
        }
    }

    /// Starts each schedule whose slot came, of the apps whose schedules this device runs: one first seen waits for its next
    /// slot, so nothing missed while it was off runs late.
    async fn run_due(&self, next: &mut HashMap<(Scope, String, String), (Value, u64)>) -> Result<()> {
        let now = now_ms();
        let declared = self.declared().await;
        let due = self.with(|i| {
            let m = i.r.db();
            let mut due = Vec::new();
            let mut seen = BTreeSet::new();
            // A device that dropped its Compute for an app runtime of its own leaves its schedules to that.
            if m.compute(&self.0.id)?.is_none() {
                return Ok(due);
            }
            let me = self.0.holds.device();
            for ((_, scope, app), _) in m.app_schedules_at(now)?.into_iter().filter(|((k, ..), _)| *k == me) {
                let Some(state) = i.views.get(&scope.to_string()).filter(|s| s.added.contains(&app)) else { continue };
                let Some((_, list)) = declared.iter().find(|(a, _)| *a == app) else { continue };
                for s in list.iter().filter_map(|s| s.as_ref().ok()) {
                    let Some(user) = runs_as(m, state, &scope, &app, &s.name) else { continue };
                    let id = (scope.clone(), app.clone(), s.name.clone());
                    seen.insert(id.clone());
                    let spec = s.spec();
                    let at = match next.get(&id).filter(|(was, _)| *was == spec) {
                        Some((_, at)) => *at,
                        None => s.next(now).unwrap_or(u64::MAX),
                    };
                    if at <= now {
                        due.push((scope.clone(), app.clone(), s.clone(), user, at));
                    }
                    next.insert(id, (spec, if at <= now { s.next(now).unwrap_or(u64::MAX) } else { at }));
                }
            }
            next.retain(|id, _| seen.contains(id));
            Ok(due)
        })?;
        for (scope, app, schedule, user, slot) in due {
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(err) = node.run_schedule(&scope, &app, &schedule, &user, slot).await {
                    tracing::warn!("{app}'s schedule {} in {scope}: {err:#}", schedule.name);
                }
            });
        }
        Ok(())
    }

    /// Runs one slot of a schedule here, as whom it runs as, with its app schedules' keys: once, whichever device takes its
    /// slot first. How it went is written as every run's is, where this device opens the scope's own rows.
    async fn run_schedule(&self, scope: &Scope, app: &str, schedule: &Schedule, user: &str, slot: u64) -> Result<()> {
        self.app_schedules_keys(scope, app).await?;
        let lease = format!("{}/{}/slots/{app}/{}/{slot}", crate::bucket::folder(&scope.to_string()), scopes::APP, schedule.name);
        if !crate::bucket::Bucket::create(&self.bucket()?, &lease, hex(&self.0.id).as_bytes()).await? {
            return Ok(());
        }
        let started = Instant::now();
        let traced = trace::Trace::quiet();
        let ran = trace::within(Some(traced.clone()), async {
            match self.count(app, "schedule").await {
                Err(err) => Err(err),
                Ok(()) => self.app_schedules_call(scope, app, user, &schedule.tool, schedule.input.clone(), "schedule").await,
            }
        })
        .await;
        self.finish_schedule(app, &schedule.name, user, started.elapsed(), &traced, ran.as_ref().err());
        let error = ran.as_ref().err().map(|e| format!("{e:#}").chars().take(500).collect::<String>());
        let said = status::Run { slot, time: now_ms(), device: self.0.id, user: user.to_string(), ok: error.is_none(), error, ms: started.elapsed().as_millis() as u64, spec: schedule.spec() };
        // The scope's own rows open only with its epochs, which a machine that only runs the app's schedules hasn't.
        if let Err(err) = self.say(scope, status::Said::Ran { app: app.to_string(), schedule: schedule.name.clone(), run: said }).await {
            tracing::debug!("{app}'s run of {} in {scope} isn't written: {err:#}", schedule.name);
        }
        ran.map(|_| ())
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

    /// The schedules `viewer` may switch, of the apps they may use, that are on and failed their last run, each as
    /// `schedules_of` gives it with its `app`. Only an app with a failed last run where `viewer` manages it is
    /// described, so it's cheap when nothing fails.
    pub async fn failing_schedules(&self, viewer: &Caller) -> Result<Vec<Value>> {
        let apps: Vec<String> = self.with(|i| {
            let m = i.r.db();
            let mut failing = Vec::new();
            for a in m.apps()?.into_iter().filter(|a| !a.erased && a.deleted.is_none()) {
                let fails = reachable(i, &a.name, &viewer.user)?.iter().any(|scope| {
                    let added = i.views.get(&scope.to_string()).is_some_and(|s| s.added.contains(&a.name));
                    let failed = i.status.get(&scope.to_string()).is_some_and(|st| st.runs.iter().any(|((app, _), runs)| app == &a.name && runs.last().is_some_and(|r| !r.ok)));
                    added && failed && m.may(&viewer.user, "add-apps", &scopes::object(scope))
                });
                if fails {
                    failing.push(a.name);
                }
            }
            Ok(failing)
        })?;
        let mut out = Vec::new();
        for app in apps.into_iter().filter(|a| self.0.apps.has(a)) {
            let Ok(d) = self.0.apps.describe(&app).await else { continue };
            for mut s in self.schedules_of(viewer, &app, &Schedule::all(&d))?.0 {
                if s["on"] == true && s["may_switch"] == true && s["last"]["ok"] == false {
                    s["app"] = json!(app);
                    out.push(s);
                }
            }
        }
        Ok(out)
    }

    /// An app's schedules where it's added for `viewer` and this device holds the scope: what each runs and when,
    /// whether it's on, whom it runs as, when it runs next, its last run, and whether a deploy changed or removed it;
    /// and its runs, newest first.
    pub(super) fn schedules_of(&self, viewer: &Caller, app: &str, declared: &[std::result::Result<Schedule, (String, String)>]) -> Result<(Vec<Value>, Vec<Value>)> {
        let now = now_ms();
        self.with(|i| {
            let m = i.r.db();
            let user_name = |u: &str| user_name(m, u);
            let (mut list, mut activity) = (Vec::new(), Vec::new());
            for scope in reachable(i, app, &viewer.user)? {
                let Some(state) = i.views.get(&scope.to_string()) else { continue };
                if !state.added.contains(app) {
                    continue;
                }
                let status = i.status.get(&scope.to_string()).cloned().unwrap_or_default();
                let scope_name = whose(m, &scope)?;
                // The devices that run its schedules there: none, and they don't run.
                let run_by: Vec<String> = m.app_schedules_at(now)?.into_iter().filter(|((_, s, a), _)| *s == scope && a == app).map(|((k, ..), _)| m.machine(&k)).collect::<Result<Vec<_>>>()?.into_iter().flatten().map(|d| d.name).collect();
                let base = |name: &str| {
                    let key = (app.to_string(), name.to_string());
                    let runs = status.runs.get(&key);
                    let last = runs.and_then(|r| r.last()).map(|r| json!({ "time": r.time, "ok": r.ok, "error": r.error }));
                    let changed = status.changed.get(&key).filter(|(.., t)| now.saturating_sub(*t) < CHANGED_SHOWN).map(|(was, is, t)| json!({ "was": schedules::say(was), "is": schedules::say(is), "time": t }));
                    json!({ "name": name, "scope": scope.to_string(), "scope_name": scope_name, "may_switch": m.may(&viewer.user, "add-apps", &scopes::object(&scope)), "may_set_app_schedules": m.may(&viewer.user, "choose-devices", &scopes::object(&scope)), "run_by": run_by, "last": last, "changed": changed })
                };
                for s in declared {
                    let mut v = match s {
                        Ok(s) => {
                            let mut v = base(&s.name);
                            let user = runs_as(m, state, &scope, app, &s.name);
                            let switched = state.switches.get(&(app.to_string(), s.name.clone()));
                            v["description"] = json!(s.description);
                            v["when"] = json!(s.say());
                            v["tool"] = json!(s.tool);
                            v["on"] = json!(switched.is_none_or(|(on, ..)| *on));
                            v["switched"] = json!(switched.map(|(on, by, t)| user_name(by).map(|by| json!({ "on": on, "by": by, "time": t }))).transpose()?);
                            v["runs_as"] = json!(user.as_deref().map(user_name).transpose()?);
                            v["next"] = json!(user.filter(|_| !run_by.is_empty()).and(s.next(now)));
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
                for ((_, name), runs) in status.runs.iter().filter(|((a, _), _)| a == app) {
                    for r in runs {
                        activity.push(json!({ "schedule": name, "scope": scope.to_string(), "scope_name": scope_name, "time": r.time, "ok": r.ok, "error": r.error, "ms": r.ms, "device": m.machine_ever(&r.device)?.map(|s| s.name.clone()), "user": user_name(&r.user)? }));
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
        let title = self.title(app).await;
        let content = self.with(|i| switch_entry(i, &caller.user, scope, app, &title, schedule, on))?;
        self.append_as(scope, &caller.user, content).await?;
        Ok(())
    }
}

/// Whom a schedule of `app` in a scope runs as: whoever switched it on last, or else added the app there, while
/// they still manage the scope; nobody if it's switched off.
fn runs_as(m: &ScopeDb, state: &ScopeState, scope: &Scope, app: &str, schedule: &str) -> Option<String> {
    let manages = |u: &String| m.may(u, "add-apps", &scopes::object(&scope));
    match state.switches.get(&(app.to_string(), schedule.to_string())) {
        Some((false, ..)) => None,
        Some((true, user, _)) if manages(user) => Some(user.clone()),
        _ => state.added_by.get(app).map(|(u, _)| u.clone()).filter(manages),
    }
}

/// A `schedule` entry `user` may write in `scope`, switching a schedule of `app` there on or off.
pub(super) fn switch_entry(i: &Network, user: &str, scope: &Scope, app: &str, title: &str, schedule: &str, on: bool) -> Result<Content> {
    let name = scope_name(i, &scope.to_string())?;
    allowed!(i.r.db().may(user, "add-apps", &scopes::object(scope)), "You can't switch schedules in {name}.");
    ensure!(i.views.get(&scope.to_string()).is_some_and(|s| s.added.contains(app)), "{title} isn't on {name}.");
    ensure!(i.r.holds(scope), "This device can't open {name} yet.");
    Ok(content::schedule(app, schedule, on))
}
