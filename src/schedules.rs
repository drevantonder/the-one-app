//! Schedules: an app's tools that run on their own, every so often or at a time of day, on the machine keeping the
//! app running (src/node/scheduled.rs).

use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};


/// A schedule as an app declares it: `{ tool, input?, every: "15m" }` or `{ tool, input?, at: "09:00", tz?, days? }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Schedule {
    pub name: String,
    pub description: String,
    pub tool: String,
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
        let tool = v["op"].as_str().filter(|o| !o.is_empty()).context("a schedule names the tool it runs")?.to_string();
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
        Ok(Schedule { name: name.into(), description: v["description"].as_str().unwrap_or("").into(), tool, input: v.get("input").cloned().unwrap_or(json!({})), when })
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
        let mut v = json!({ "op": self.tool, "input": self.input });
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

    pub fn next(&self, now: u64) -> Option<u64> {
        match &self.when {
            When::Every(ms) => Some((now / ms + 1) * ms),
            When::At { .. } => self.next_at(now),
        }
    }

    fn next_at(&self, now: u64) -> Option<u64> {
        let When::At { hour, minute, tz, days } = &self.when else { return None };
        let tz = jiff::tz::TimeZone::get(tz).ok()?;
        let now_ts = jiff::Timestamp::from_millisecond(now as i64).ok()?;
        let today = now_ts.to_zoned(tz.clone()).date();
        let at = |date: jiff::civil::Date| -> Option<u64> {
            if !days.is_empty() && !days.contains(&(date.weekday().to_monday_zero_offset())) {
                return None;
            }
            let t = date.at(*hour, *minute, 0, 0).to_zoned(tz.clone()).ok()?;
            Some(t.timestamp().as_millisecond() as u64)
        };
        let day = |n: i64| today.checked_add(jiff::Span::new().days(n)).ok();
        (0..9).filter_map(|n| day(n).and_then(at)).find(|t| *t > now)
    }

    /// When it runs, for people: "every 15 minutes", "at 09:00 (Australia/Melbourne) on Mon, Fri".
    pub fn say(&self) -> String {
        say(&self.spec())
    }
}

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
