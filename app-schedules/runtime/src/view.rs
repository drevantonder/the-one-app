//! An app's schedules as a user sees and switches them: what each runs and when, whom it runs as, its last run, and
//! whether a deploy changed or removed it; and those failing.

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use api::Caller;
use api::runtime::prelude::*;
use app_schedules::Schedule;
use custom_build::allowed;
use custom_build::now_ms;
use scope::content::{self, Content};
use scope::read::Read;
use scope::scopes::{self, Scope, ScopeDb, ScopeState};

use crate::State;

/// How long an app's page says a deploy changed or removed one of its schedules.
const CHANGED_SHOWN: u64 = 14 * 86_400_000;

/// The scopes a call of `app` as `user` reaches, as this device can tell: of each scope it holds, by what it read.
fn reachable(r: &Read, app: &str, user: &str) -> Result<Vec<Scope>> {
    scope::state::reachable(r.db(), user, |s| r.holds(s).then(|| r.state(s).is_some_and(|st| st.added.contains(app))))
}

/// The schedules `viewer` may switch, of the apps they may use, that are on and failed their last run, each as
/// `schedules_of` gives it with its `app`. Only an app with a failed last run where `viewer` manages it is described,
/// so it's cheap when nothing fails.
pub async fn failing(rt: &Rt, viewer: &Caller) -> Result<Vec<Value>> {
    let rows = rt.state::<State>().rows.lock().unwrap().clone();
    let apps: Vec<String> = rt.read(|r| {
        let m = r.db();
        let mut failing = Vec::new();
        for a in m.apps()?.into_iter().filter(|a| !a.erased && a.deleted.is_none()) {
            let fails = reachable(r, &a.name, &viewer.user)?.iter().any(|scope| {
                let added = r.state(scope).is_some_and(|s| s.added.contains(&a.name));
                let failed = rows.get(scope).is_some_and(|st| st.runs.iter().any(|((app, _), runs)| app == &a.name && runs.last().is_some_and(|r| !r.ok)));
                added && failed && m.may(&viewer.user, "add-apps", &scopes::object(scope))
            });
            if fails {
                failing.push(a.name);
            }
        }
        Ok(failing)
    })?;
    let here = rt.names();
    let mut out = Vec::new();
    for app in apps.into_iter().filter(|a| here.contains(a)) {
        let Ok(d) = rt.describe(&app).await else { continue };
        for mut s in schedules_of(rt, viewer, &app, &Schedule::all(&d))?.0 {
            if s["on"] == true && s["may_switch"] == true && s["last"]["ok"] == false {
                s["app"] = json!(app);
                out.push(s);
            }
        }
    }
    Ok(out)
}

/// An app's schedules where it's added for `viewer` and this device holds the scope: what each runs and when, whether
/// it's on, whom it runs as, when it runs next, its last run, and whether a deploy changed or removed it; and its runs,
/// newest first.
pub fn schedules_of(rt: &Rt, viewer: &Caller, app: &str, declared: &[std::result::Result<Schedule, (String, String)>]) -> Result<(Vec<Value>, Vec<Value>)> {
    let now = now_ms();
    let rows = rt.state::<State>().rows.lock().unwrap().clone();
    rt.read(|r| {
        let m = r.db();
        let (mut list, mut activity) = (Vec::new(), Vec::new());
        for scope in reachable(r, app, &viewer.user)? {
            let Some(state) = r.state(&scope) else { continue };
            if !state.added.contains(app) {
                continue;
            }
            let status = rows.get(&scope).cloned().unwrap_or_default();
            let scope_name = scope::state::whose(m, &scope)?;
            // The devices that run its schedules there: none, and they don't run.
            let run_by: Vec<String> = m.app_schedules_at(now)?.into_iter().filter(|((_, s, a), _)| *s == scope && a == app).map(|((k, ..), _)| m.machine(&k)).collect::<Result<Vec<_>>>()?.into_iter().flatten().map(|d| d.name).collect();
            let base = |name: &str| {
                let key = (app.to_string(), name.to_string());
                let runs = status.runs.get(&key);
                let last = runs.and_then(|r| r.last()).map(|r| json!({ "time": r.time, "ok": r.ok, "error": r.error }));
                let changed = status.changed.get(&key).filter(|(.., t)| now.saturating_sub(*t) < CHANGED_SHOWN).map(|(was, is, t)| json!({ "was": app_schedules::say(was), "is": app_schedules::say(is), "time": t }));
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
                        v["switched"] = json!(switched.map(|(on, by, t)| r.user_name(by).map(|by| json!({ "on": on, "by": by, "time": t }))).transpose()?);
                        v["runs_as"] = json!(user.as_deref().map(|u| r.user_name(u)).transpose()?);
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
                    v["was"] = json!(runs.last().map(|r| app_schedules::say(&r.spec)));
                    list.push(v);
                }
            }
            for ((_, name), runs) in status.runs.iter().filter(|((a, _), _)| a == app) {
                for run in runs {
                    activity.push(json!({ "schedule": name, "scope": scope.to_string(), "scope_name": scope_name, "time": run.time, "ok": run.ok, "error": run.error, "ms": run.ms, "device": m.machine_ever(&run.device)?.map(|s| s.name.clone()), "user": r.user_name(&run.user)? }));
                }
            }
        }
        activity.sort_by_key(|r| std::cmp::Reverse(r["time"].as_u64().unwrap_or(0)));
        activity.truncate(50);
        Ok((list, activity))
    })
}

/// Switches one of an app's schedules on or off in `scope`, for `caller`; switching one on makes it run as them.
pub(crate) async fn switch(rt: &Rt, caller: &Caller, app: &str, schedule: &str, scope: &Scope, on: bool) -> Result<()> {
    let title = rt.title(app).await?;
    let content = rt.read(|r| switch_entry(r, &caller.user, scope, app, &title, schedule, on))?;
    rt.append(scope, &caller.user, content).await
}

/// Whom a schedule of `app` in a scope runs as: whoever switched it on last, or else added the app there, while they
/// still manage the scope; nobody if it's switched off.
pub(crate) fn runs_as(m: &ScopeDb, state: &ScopeState, scope: &Scope, app: &str, schedule: &str) -> Option<String> {
    let manages = |u: &String| m.may(u, "add-apps", &scopes::object(scope));
    match state.switches.get(&(app.to_string(), schedule.to_string())) {
        Some((false, ..)) => None,
        Some((true, user, _)) if manages(user) => Some(user.clone()),
        _ => state.added_by.get(app).map(|(u, _)| u.clone()).filter(manages),
    }
}

/// A `schedule` entry `user` may write in `scope`, switching a schedule of `app` there on or off.
pub fn switch_entry(r: &Read, user: &str, scope: &Scope, app: &str, title: &str, schedule: &str, on: bool) -> Result<Content> {
    let name = r.scope_name(scope)?;
    allowed!(r.db().may(user, "add-apps", &scopes::object(scope)), "You can't switch schedules in {name}.");
    ensure!(r.state(scope).is_some_and(|s| s.added.contains(app)), "{title} isn't on {name}.");
    ensure!(r.holds(scope), "This device can't open {name} yet.");
    Ok(content::schedule(app, schedule, on))
}
