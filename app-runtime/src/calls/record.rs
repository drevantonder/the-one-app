//! How each app's calls went here: loop guards that stop an app.

use super::*;

impl Node {
    /// Counts one of `app`'s calls of other apps, or one run of its schedules (`kind`). Past `RATE_LIMIT` this minute,
    /// the app is stopped here: it's looping.
    pub(in crate) async fn count(&self, app: &str, kind: &'static str) -> Result<()> {
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
    pub(in crate) fn stopped(&self, app: &str) -> Option<String> {
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
