//! Running what's due: each slot of the schedules of the apps whose app schedules this device holds, once, whichever
//! of their machines takes it first.

use std::collections::BTreeSet;
use std::time::Instant;

use anyhow::Result;
use api::runtime::prelude::*;
use api::trace;
use app_schedules::Schedule;
use custom_build::now_ms;
use key::hex;
use scope::scopes::Scope;

use crate::rows::{self, Run};
use crate::view::runs_as;
use crate::State;

/// Starts each schedule whose slot came, of the apps whose schedules this device runs: one first seen waits for its next
/// slot, so nothing missed while it was off runs late (`Runs.every`).
pub(crate) async fn tick(rt: Rt) -> Result<()> {
    if !rt.ready() {
        return Ok(());
    }
    rows::read_all(&rt).await;
    let now = now_ms();
    let declared = declared(&rt).await;
    let (me, device) = (rt.id(), rt.device());
    let due = rt.read(|r| {
        let m = r.db();
        let mut next = rt.state::<State>().next.lock().unwrap();
        let mut due = Vec::new();
        let mut seen = BTreeSet::new();
        // A device that dropped its Compute for an app runtime of its own leaves its schedules to that.
        if m.compute(&me)?.is_none() {
            return Ok(due);
        }
        for ((_, scope, app), _) in m.app_schedules_at(now)?.into_iter().filter(|((k, ..), _)| *k == device) {
            let Some(state) = r.state(&scope).filter(|s| s.added.contains(&app)) else { continue };
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
        let rt = rt.clone();
        tokio::spawn(async move {
            if let Err(err) = run_schedule(&rt, &scope, &app, &schedule, &user, slot).await {
                tracing::warn!("{app}'s schedule {} in {scope}: {err:#}", schedule.name);
            }
        });
    }
    Ok(())
}

/// Runs one slot of a schedule here, as whom it runs as, with its app schedules' keys: once, whichever device takes its
/// slot first. How it went is written as every run's is, where this device opens the scope's own rows.
async fn run_schedule(rt: &Rt, scope: &Scope, app: &str, schedule: &Schedule, user: &str, slot: u64) -> Result<()> {
    rt.app_schedules_keys(scope, app).await?;
    let lease = format!("{}/{}/slots/{app}/{}/{slot}", bucket::folder(&scope.to_string()), key::APP, schedule.name);
    if !bucket::Bucket::create(&rt.bucket()?, &lease, hex(&rt.id()).as_bytes()).await? {
        return Ok(());
    }
    let started = Instant::now();
    let traced = trace::Trace::quiet();
    let ran = trace::within(Some(traced.clone()), rt.call_scheduled(scope, app, user, &schedule.tool, schedule.input.clone())).await;
    insights_runtime::finish_schedule(rt, app, &schedule.name, user, started.elapsed(), &traced, ran.as_ref().err());
    let error = ran.as_ref().err().map(|e| format!("{e:#}").chars().take(500).collect::<String>());
    let run = Run { slot, time: now_ms(), device: rt.id(), user: user.to_string(), ok: error.is_none(), error, ms: started.elapsed().as_millis() as u64, spec: schedule.spec() };
    // The scope's own rows open only with its epochs, which a machine that only runs the app's schedules hasn't.
    if let Err(err) = rows::ran(rt, scope, app, &schedule.name, &run).await {
        tracing::debug!("{app}'s run of {} in {scope} isn't written: {err:#}", schedule.name);
    }
    ran.map(|_| ())
}

/// Each app's schedules here, as its latest deploy declares them, with why any don't make sense.
async fn declared(rt: &Rt) -> Vec<(String, Vec<std::result::Result<Schedule, (String, String)>>)> {
    let mut out = Vec::new();
    for app in rt.names() {
        if let Ok(d) = rt.describe(&app).await {
            let list = Schedule::all(&d);
            if !list.is_empty() {
                out.push((app, list));
            }
        }
    }
    out
}
