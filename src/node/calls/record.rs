//! How each app's calls went here: loop guards that stop an app, its failures recorded, and what its people see of them.

use super::*;

impl Node {
    /// Records, for an app's owners, a call that ran here: the first each day by each user from each client
    /// (`used`), and every failure (`failed`), at most `FAILURES_KEPT` an hour per app on this device. They go in the
    /// owner's scope when the caller may use it and this device holds it, else in `global`; in the background.
    pub(in crate::node) fn record(&self, app: &str, req: &CallRequest, me: &Caller, error: Option<&str>) {
        let now = now_ms();
        let client = req.client.clone().unwrap_or_else(|| "api".into());
        let said = match error {
            None => {
                let day = now / 86_400_000;
                if !self.0.calls.seen.lock().unwrap().insert((app.to_string(), me.user.clone(), client.clone(), day)) {
                    return;
                }
                status::Said::Used { app: app.into(), user: me.user.clone(), client, agent: me.agent.clone(), day }
            }
            Some(error) => {
                let hour = now / 3_600_000;
                let mut failures = self.0.calls.failures.lock().unwrap();
                let n = failures.entry((app.to_string(), hour)).or_default();
                *n += 1;
                if *n > FAILURES_KEPT {
                    return;
                }
                let error: String = error.lines().next().unwrap_or("").chars().take(300).collect();
                status::Said::Failed { app: app.into(), tool: req.runs().to_string(), error, user: me.user.clone() }
            }
        };
        self.write_insight(app, me, said);
    }

    /// Writes an insight about `app` for `me`'s call: in the app owner's scope when `me` may use it and this device
    /// holds it, else in `global`; in the background.
    pub(in crate::node) fn write_insight(&self, app: &str, me: &Caller, said: status::Said) {
        let Ok(scope) = self.with(|i| {
            let m = i.r.db();
            let owner = m.app(app)?.map(|a| a.owner).context("no such app")?;
            let own = match owner {
                Principal::User(u) => Scope::User(u),
                Principal::Group(g) => Scope::Group(g),
                _ => Scope::Global,
            };
            Ok(if m.may(&me.user, "use", &scopes::object(&own)) && i.r.holds(&own) { own } else { Scope::Global })
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
    pub(in crate::node) fn insights_of(&self, viewer: &Caller, app: &str) -> Result<Option<Value>> {
        self.with(|i| {
            let m = i.r.db();
            let Some(a) = m.app(app)? else { return Ok(None) };
            if !m.may(&viewer.user, "deploy", &scopes::app(&a.name)) {
                return Ok(None);
            }
            let name = |u: &str| user_name(m, u);
            let mut people: BTreeMap<(String, String), (u64, u64, Option<String>)> = BTreeMap::new();
            let mut errors: BTreeMap<(String, String), (u64, u64, u64, String)> = BTreeMap::new();
            let mut cost: BTreeMap<String, (f64, u64, u64, u64)> = BTreeMap::new();
            for (log, status) in &i.status {
                let Ok(scope) = Scope::parse(log) else { continue };
                let Some(ins) = status.insights.get(app).filter(|_| m.may(&viewer.user, "use", &scopes::object(&scope))) else { continue };
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
                        x.3 = name(&e.user)?;
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
            let mut usage: Vec<Value> = Vec::new();
            for ((u, client), (days, last, agent)) in people {
                usage.push(json!({ "user": name(&u)?, "client": client, "agent": agent, "days": days, "last": last }));
            }
            usage.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let mut errors: Vec<Value> = errors.into_iter().map(|((tool, error), (count, first, last, user))| json!({ "tool": tool, "error": error, "count": count, "first": first, "last": last, "user": user })).collect();
            errors.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let cost: Vec<Value> = cost.into_iter().map(|(model, (usd, input, output, calls))| json!({ "model": model, "usd": usd, "input": input, "output": output, "calls": calls })).collect();
            Ok(Some(json!({ "usage": usage, "errors": errors, "cost": cost })))
        })
    }

    /// Counts one of `app`'s calls of other apps, or one run of its schedules (`kind`). Past `RATE_LIMIT` this minute,
    /// the app is stopped here: it's looping.
    pub(in crate::node) async fn count(&self, app: &str, kind: &'static str) -> Result<()> {
        let now = now_ms();
        let tripped = {
            let mut g = self.0.calls.guard.lock().unwrap();
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
            let bundle = self.with_or(None, |i| Ok(i.r.db().app(app)?.and_then(|a| a.bundle.clone())));
            self.0.calls.guard.lock().unwrap().stopped.insert(app.to_string(), (why.clone(), now_ms() + STOPPED_FOR, bundle));
            tracing::warn!("{why}");
        }
        match self.stopped(app) {
            Some(why) => bail!("{why}"),
            None => Ok(()),
        }
    }

    /// Why `app` is stopped here, if it is: it looped, and hasn't been deployed again or waited out its time since.
    pub(in crate::node) fn stopped(&self, app: &str) -> Option<String> {
        let latest = self.with_or(None, |i| Ok(i.r.db().app(app)?.and_then(|a| a.bundle.clone())));
        let mut g = self.0.calls.guard.lock().unwrap();
        let (why, until, bundle) = g.stopped.get(app)?;
        if now_ms() < *until && *bundle == latest {
            return Some(why.clone());
        }
        g.stopped.remove(app);
        None
    }
}

/// Loop guards: an app that calls other apps, or runs its schedules, more than `RATE_LIMIT` times a minute on this
/// device is stopped here for `STOPPED_FOR`, or until it's deployed again.
#[derive(Default)]
pub(super) struct Guard {
    /// Each app's calls or runs since the minute it started counting (milliseconds): a minute from its first, not
    /// the clock's, so a loop that runs across the turn of a minute is still caught.
    counts: HashMap<(String, &'static str), (u64, u32)>,
    /// Each app stopped here: why, until when, and the bundle it was running.
    stopped: HashMap<String, (String, u64, Option<String>)>,
}
