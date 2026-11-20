//! Whether a host keeps the renderer's contract with it (`platform`'s, at its top), checked the same way on every
//! host by the inspector's `host` op: a window is kept moving for a while, then left still, as everything the host
//! does meanwhile is recorded (`Record`): each display tick, with the time the display gave it, each frame presented
//! and each input it delivered. Then it says, check by check, what held.

use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{Value, json};
use web_time::Instant;

/// How long a check keeps its window moving, and then still.
pub(super) const MOVING: Duration = Duration::from_millis(1000);
pub(super) const STILL: Duration = Duration::from_millis(500);

/// What a host did, as it did it, for as long as a check runs.
#[derive(Default)]
pub(super) struct Record {
    /// Each tick of the display: when it says it showed the frame, and when the loop heard of it.
    ticks: VecDeque<(Instant, Instant)>,
    /// When each frame was presented.
    presents: VecDeque<Instant>,
    /// Each input as it came: when, what it was, and when the device says it happened, where the host says.
    inputs: VecDeque<(Instant, &'static str, Option<Instant>)>,
}

/// The most of each it keeps: well over a check's ticks at 240 Hz.
const KEPT: usize = 4096;

fn keep<T>(all: &mut VecDeque<T>, one: T) {
    if all.len() == KEPT {
        all.pop_front();
    }
    all.push_back(one);
}

impl Record {
    pub fn tick(&mut self, shown: Instant) {
        keep(&mut self.ticks, (shown, Instant::now()));
    }

    pub fn present(&mut self) {
        keep(&mut self.presents, Instant::now());
    }

    pub fn input(&mut self, kind: &'static str, happened: Option<Instant>) {
        keep(&mut self.inputs, (Instant::now(), kind, happened));
    }
}

/// A check under way: when it started, and who's told what held.
pub(super) struct Check {
    pub window: super::WindowId,
    pub started: Instant,
    pub id: Value,
    pub reply: crate::Reply,
}

impl Check {
    /// Whether its window is kept moving now.
    pub fn moving(&self) -> bool {
        self.started.elapsed() < MOVING
    }

    /// Whether it's over, and what held is to be said.
    pub fn over(&self) -> bool {
        self.started.elapsed() >= MOVING + STILL
    }

    /// When it's next due: as it stops moving, or as it's over.
    pub fn due(&self) -> Duration {
        let since = self.started.elapsed();
        if since < MOVING { MOVING - since } else { (MOVING + STILL).saturating_sub(since) }
    }

    /// What held, from what `record` kept since it started, `paced` whether the display says when it shows each frame.
    pub fn said(&self, record: &Record, paced: bool) -> Value {
        let (start, stop) = (self.started, self.started + MOVING);
        let ms = |duration: Duration| (duration.as_secs_f64() * 100_000.0).round() / 100.0;
        // Each tick while it moved, by the display's time, and how far apart they came.
        let ticks: Vec<Instant> = record.ticks.iter().filter(|(_, heard)| *heard >= start && *heard < stop).map(|(shown, _)| *shown).collect();
        let mut gaps: Vec<Duration> = ticks.windows(2).map(|pair| pair[1].saturating_duration_since(pair[0])).collect();
        gaps.sort();
        let median = gaps.get(gaps.len() / 2).copied().unwrap_or_default();
        let late = gaps.iter().filter(|gap| gap.as_secs_f64() > median.as_secs_f64() * 1.5).count();
        let backwards = ticks.windows(2).filter(|pair| pair[1] < pair[0]).count();
        // A frame presented between each tick and the next, while it moved.
        let heard: Vec<Instant> = record.ticks.iter().filter(|(_, heard)| *heard >= start && *heard < stop).map(|(_, heard)| *heard).collect();
        let unpresented = heard.windows(2).filter(|pair| !record.presents.iter().any(|at| *at >= pair[0] && *at < pair[1])).count();
        // Once it's still, after the frame it stopped on and the one after: no ticks, nothing presented.
        let settled = stop + median * 2;
        let idle_ticks = record.ticks.iter().filter(|(_, heard)| *heard >= settled).count();
        let idle_presents = record.presents.iter().filter(|at| **at >= settled).count();
        // Inputs, in the order they came, each with the device's own time.
        let inputs: Vec<&(Instant, &'static str, Option<Instant>)> = record.inputs.iter().filter(|(came, ..)| *came >= start).collect();
        let timed = inputs.iter().filter(|(_, _, happened)| happened.is_some()).count();
        let times: Vec<Instant> = inputs.iter().filter_map(|(_, _, happened)| *happened).collect();
        let out_of_order = times.windows(2).filter(|pair| pair[1] < pair[0]).count();
        let mut kinds: Vec<&str> = inputs.iter().map(|(_, kind, _)| *kind).collect();
        kinds.sort();
        kinds.dedup();
        let expected = if median.is_zero() { 0 } else { (MOVING.as_secs_f64() / median.as_secs_f64()) as usize };
        let check = |name: &str, ok: bool, said: String| json!({"check": name, "ok": ok, "said": said});
        let checks = vec![
            check("ticks every refresh", paced && ticks.len() + 1 >= expected * 9 / 10 && late == 0 && backwards == 0, match paced {
                false => "the display says nothing of when it shows a frame: the loop times its own, as a 60 Hz display's".into(),
                true => format!("{} ticks in {} ms, {} ms apart (median), {late} more than half as late again, {backwards} before the one before", ticks.len(), ms(MOVING), ms(median)),
            }),
            check("a frame presented each tick while moving", unpresented == 0, format!("{unpresented} of {} ticks with none", heard.len().saturating_sub(1))),
            check("nothing while still", idle_ticks == 0 && idle_presents == 0, format!("{idle_ticks} ticks and {idle_presents} frames presented once still")),
            check("inputs timed by the device, in order", !inputs.is_empty() && timed == inputs.len() && out_of_order == 0, match inputs.len() {
                0 => "no input came while it ran: move the pointer or a finger over the window as it does".into(),
                count => format!("{timed} of {count} with the device's time, {out_of_order} before the one before ({})", kinds.join(", ")),
            }),
        ];
        json!({"paced": paced, "refresh_hz": if median.is_zero() { Value::Null } else { json!((1.0 / median.as_secs_f64() * 10.0).round() / 10.0) }, "ticks": ticks.len(), "presented": record.presents.iter().filter(|at| **at >= start && **at < stop).count(), "inputs": inputs.len(), "checks": checks, "ok": checks.iter().all(|check| check["ok"] == true)})
    }
}

/// The inspector's `record` under way: every input its window gets, as it comes, for `ms`, as the events a trace plays
/// (the headless harness's `trace`, a scene's `trace` gesture), so a device's input is played headless as it came.
pub(super) struct Recording {
    pub window: super::WindowId,
    pub started: Instant,
    pub until: Instant,
    pub events: Vec<Value>,
    pub id: Value,
    pub reply: crate::Reply,
}

impl Recording {
    /// Input `event` (`{kind, …}`) to window `window`, `happened` when the device says it did, where it says: kept with
    /// its `ms` from when it started, for its window.
    pub fn input(&mut self, window: super::WindowId, mut event: Value, happened: Option<Instant>) {
        if window != self.window {
            return;
        }
        let at = happened.unwrap_or_else(Instant::now).saturating_duration_since(self.started);
        event["ms"] = json!((at.as_secs_f64() * 100_000.0).round() / 100.0);
        self.events.push(event);
    }
}
