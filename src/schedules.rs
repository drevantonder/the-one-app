//! Schedules: an app's operations that run on their own, every so often or at a time of day.
//! Which server runs each is decided from the members alone, so every server agrees without asking: no leases.

use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};

use crate::entry::{Key, Scope};
use crate::members::{Members, Principal};

/// How long each server in line waits for the ones before it to run a slot, before running it itself.
pub const GRACE: u64 = 60_000;
/// The latest a slot still runs: no later than this after its time (or its period, if shorter).
pub const LATE: u64 = 3_600_000;

/// A schedule as an app declares it: `{ op, input?, every: "15m" }` or `{ op, input?, at: "09:00", tz?, days? }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Schedule {
    pub name: String,
    pub description: String,
    pub op: String,
    pub input: Value,
    pub when: When,
}

#[derive(Clone, Debug, PartialEq)]
pub enum When {
    /// Every so many milliseconds, from 1970.
    Every(u64),
    /// At a time of day (hour, minute) in a time zone, on some days of the week (0 is Monday) or every day.
    At { hour: i8, minute: i8, tz: String, days: Vec<i8> },
}

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

impl Schedule {
    /// Reads one of an app's schedules, as its module describes it.
    pub fn parse(name: &str, v: &Value) -> Result<Schedule> {
        let op = v["op"].as_str().filter(|o| !o.is_empty()).context("a schedule names the op it runs")?.to_string();
        let when = match (v["every"].as_str(), v["at"].as_str()) {
            (Some(every), None) => When::Every(duration(every)?),
            (None, Some(at)) => {
                let (h, m) = at.split_once(':').context("at is a time of day, like 09:00")?;
                let (hour, minute): (i8, i8) = (h.parse()?, m.parse()?);
                ensure!((0..24).contains(&hour) && (0..60).contains(&minute), "at is a time of day, like 09:00");
                let tz = v["tz"].as_str().unwrap_or("UTC").to_string();
                jiff::tz::TimeZone::get(&tz).with_context(|| format!("no time zone {tz}"))?;
                let mut days = Vec::new();
                for d in v["days"].as_array().into_iter().flatten() {
                    let d = d.as_str().unwrap_or("").to_lowercase();
                    days.push(DAYS.iter().position(|x| d.starts_with(x)).with_context(|| format!("{d} isn't a day of the week"))? as i8);
                }
                When::At { hour, minute, tz, days }
            }
            _ => bail!("a schedule runs every so often (every: \"15m\") or at a time of day (at: \"09:00\")"),
        };
        Ok(Schedule { name: name.into(), description: v["description"].as_str().unwrap_or("").into(), op, input: v.get("input").cloned().unwrap_or(json!({})), when })
    }

    /// Every schedule an app's module describes; one that doesn't make sense is left out, with why.
    pub fn all(describe: &Value) -> Vec<std::result::Result<Schedule, (String, String)>> {
        describe["schedules"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                let name = s["name"].as_str().unwrap_or("").to_string();
                Schedule::parse(&name, s).map_err(|e| (name, format!("{e:#}")))
            })
            .collect()
    }

    /// What it runs and when, as written: kept with each run, so a deploy that changes it shows.
    pub fn spec(&self) -> Value {
        let mut v = json!({ "op": self.op, "input": self.input });
        match &self.when {
            When::Every(ms) => v["every"] = json!(say_duration(*ms)),
            When::At { hour, minute, tz, days } => {
                v["at"] = json!(format!("{hour:02}:{minute:02}"));
                v["tz"] = json!(tz);
                if !days.is_empty() {
                    v["days"] = json!(days.iter().map(|d| DAYS[*d as usize]).collect::<Vec<_>>());
                }
            }
        }
        v
    }

    /// How long between runs, at most.
    pub fn period(&self) -> u64 {
        match &self.when {
            When::Every(ms) => *ms,
            When::At { days, .. } if !days.is_empty() => 7 * 86_400_000,
            When::At { .. } => 86_400_000,
        }
    }

    /// The latest slot at or before `now`: when it was due to run, in milliseconds.
    pub fn slot(&self, now: u64) -> Option<u64> {
        match &self.when {
            When::Every(ms) => Some(now / ms * ms),
            When::At { .. } => self.occurrences(now).0,
        }
    }

    /// The next slot after `now`.
    pub fn next(&self, now: u64) -> Option<u64> {
        match &self.when {
            When::Every(ms) => Some((now / ms + 1) * ms),
            When::At { .. } => self.occurrences(now).1,
        }
    }

    /// The time of day's latest occurrence at or before `now`, and its next after.
    fn occurrences(&self, now: u64) -> (Option<u64>, Option<u64>) {
        let When::At { hour, minute, tz, days } = &self.when else { return (None, None) };
        let Ok(tz) = jiff::tz::TimeZone::get(tz) else { return (None, None) };
        let Ok(now_ts) = jiff::Timestamp::from_millisecond(now as i64) else { return (None, None) };
        let today = now_ts.to_zoned(tz.clone()).date();
        let at = |date: jiff::civil::Date| -> Option<u64> {
            if !days.is_empty() && !days.contains(&(date.weekday().to_monday_zero_offset())) {
                return None;
            }
            let t = date.at(*hour, *minute, 0, 0).to_zoned(tz.clone()).ok()?;
            Some(t.timestamp().as_millisecond() as u64)
        };
        let day = |n: i64| today.checked_add(jiff::Span::new().days(n)).ok();
        let last = (0..8).filter_map(|n| day(-n).and_then(at)).find(|t| *t <= now);
        let next = (0..9).filter_map(|n| day(n).and_then(at)).find(|t| *t > now);
        (last, next)
    }

    /// When it runs, for people: "every 15 minutes", "at 09:00 (Australia/Melbourne) on Mon, Fri".
    pub fn say(&self) -> String {
        say(&self.spec())
    }
}

/// When a schedule's definition (`spec`) runs, for people.
pub fn say(spec: &Value) -> String {
    if let Some(every) = spec["every"].as_str() {
        let ms = duration(every).unwrap_or(0);
        let (n, unit) = [(86_400_000, "day"), (3_600_000, "hour"), (60_000, "minute"), (1000, "second")].into_iter().find(|(u, _)| ms % u == 0).unwrap_or((1, "millisecond"));
        return match ms / n {
            1 => format!("every {unit}"),
            k => format!("every {k} {unit}s"),
        };
    }
    let days: Vec<String> = spec["days"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|d| d[..1].to_uppercase() + &d[1..]).collect();
    let on = if days.is_empty() { "every day".to_string() } else { format!("on {}", days.join(", ")) };
    format!("at {} {on} ({})", spec["at"].as_str().unwrap_or("?"), spec["tz"].as_str().unwrap_or("UTC"))
}

/// "90s", "15m", "6h", "1d" in milliseconds; at least a minute.
pub fn duration(text: &str) -> Result<u64> {
    let text = text.trim();
    let (n, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit()).context("every is like 15m, 6h or 1d")?);
    let n: u64 = n.parse().context("every is like 15m, 6h or 1d")?;
    let ms = n * match unit {
        "s" => 1000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => bail!("every is like 15m, 6h or 1d"),
    };
    ensure!(ms >= 60_000, "a schedule runs at most once a minute");
    Ok(ms)
}

fn say_duration(ms: u64) -> String {
    match ms {
        _ if ms % 86_400_000 == 0 => format!("{}d", ms / 86_400_000),
        _ if ms % 3_600_000 == 0 => format!("{}h", ms / 3_600_000),
        _ if ms % 60_000 == 0 => format!("{}m", ms / 60_000),
        _ => format!("{}s", ms / 1000),
    }
}

/// The servers that may run a schedule of `app` in `scope` as `user`, in the order they take it: privileged servers
/// first, then the user's own, then the rest; each group in an order the schedule picks, so different schedules spread
/// over the servers. Every server works out the same line from the same members.
pub fn line(m: &Members, scope: &Scope, user: &str, app: &str, schedule: &str) -> Vec<Key> {
    let mut servers: Vec<(bool, bool, [u8; 32], Key)> = m
        .servers()
        .filter(|s| m.holds(&s.key, scope) && m.serves(&s.key, user))
        .map(|s| {
            let own = m.owners(&s.key).contains(&Principal::User(user.into()));
            let mut h = blake3::Hasher::new();
            h.update(&s.key);
            h.update(format!("{scope}/{app}/{schedule}").as_bytes());
            (!m.privileged(&s.key), !own, *h.finalize().as_bytes(), s.key)
        })
        .collect();
    servers.sort();
    servers.into_iter().map(|s| s.3).collect()
}

/// Whether a slot is still worth running at `now`: it's due, and not too late.
pub fn due(schedule: &Schedule, slot: u64, now: u64) -> bool {
    slot <= now && now - slot < LATE.min(schedule.period())
}
