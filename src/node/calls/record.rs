//! How each app's calls went here: loop guards that stop an app, who used it, and what its users see of that.

use super::*;

impl Node {
    /// Records, for an app's owners, who used it: the first call each day by each user from each client (`used`).
    pub(in crate::node) fn record(&self, app: &str, req: &CallRequest, me: &Caller) {
        let client = req.client.clone().unwrap_or_else(|| "api".into());
        let day = now_ms() / 86_400_000;
        if !self.0.calls.seen.lock().unwrap().insert((app.to_string(), me.user.clone(), client.clone(), day)) {
            return;
        }
        self.write_insight(app, me, status::Said::Used { app: app.into(), user: me.user.clone(), client, agent: me.agent.clone(), day });
    }

    /// Where what's said of `me`'s call of `app` goes: the owner's scope when `me` may use it and this device holds it, else
    /// `global`. `None` for no such app, as what a caller typed is theirs and may be anything.
    pub(in crate::node) fn destination(&self, app: &str, me: &Caller) -> Option<Destination> {
        self.with(|i| {
            let m = i.r.db();
            let Some(owner) = m.app(app)?.map(|a| a.owner) else { return Ok(None) };
            let own = match owner {
                Principal::User(u) => Scope::User(u),
                Principal::Group(g) => Scope::Group(g),
                _ => Scope::Global,
            };
            let kind = trace::kind(&own);
            Ok(Some(if m.may(&me.user, "use", &scopes::object(&own)) && i.r.holds(&own) {
                Destination { scope: own, named: true, kind }
            } else {
                Destination { named: true, scope: Scope::Global, kind }
            }))
        })
        .ok()
        .flatten()
    }

    /// Writes an insight about `app` for `me`'s call, where `destination` says, in the background.
    pub(in crate::node) fn write_insight(&self, app: &str, me: &Caller, said: status::Said) {
        let Some(Destination { scope, .. }) = self.destination(app, me) else { return };
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
            let mut users: BTreeMap<(String, String), (u64, u64, Option<String>)> = BTreeMap::new();
            let mut errors: BTreeMap<(String, String), status::Fingerprint> = BTreeMap::new();
            let mut cost: BTreeMap<String, (f64, u64, u64, u64)> = BTreeMap::new();
            for (log, status) in &i.status {
                let Ok(scope) = Scope::parse(log) else { continue };
                let Some(ins) = status.insights.get(app).filter(|_| m.may(&viewer.user, "use", &scopes::object(&scope))) else { continue };
                for ((user, client), u) in &ins.used {
                    let p = users.entry((user.clone(), client.clone())).or_default();
                    p.0 += u.days;
                    p.1 = p.1.max(u.last);
                    if u.agent.is_some() {
                        p.2 = u.agent.clone();
                    }
                }
                for (key, e) in &ins.errors {
                    let x = errors.entry(key.clone()).or_insert_with(|| status::Fingerprint { count: 0, first: u64::MAX, ..e.clone() });
                    x.count += e.count;
                    x.first = x.first.min(e.first);
                    x.last = x.last.max(e.last);
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
            for ((u, client), (days, last, agent)) in users {
                usage.push(json!({ "user": name(&u)?, "client": client, "agent": agent, "days": days, "last": last }));
            }
            usage.sort_by_key(|v| std::cmp::Reverse(v["last"].as_u64()));
            let mut errors: Vec<Value> = errors.into_iter().filter(|(_, e)| e.status == "error").map(|((tool, _), e)| json!({ "tool": tool, "error": format!("{} at {}", e.kind, e.frame), "build": e.build, "count": e.count, "first": e.first, "last": e.last })).collect();
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

/// Where what's said of a call goes.
pub(in crate::node) struct Destination {
    pub scope: Scope,
    /// Whether there's an app to name: every member can list every app's name, so only a call of no such app has none.
    pub named: bool,
    /// The kind of the app's owner's scope.
    pub kind: &'static str,
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
