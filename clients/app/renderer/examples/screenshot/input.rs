//! A person's input and the clock, played as the display's events (`INPUT`).

use renderer::platform::Windows;
use renderer::touch::Phase;
use renderer::Appearance;
use serde_json::{Value, json};

use crate::replay::replay;
use crate::select::{ask, point_of};

/// The inspector's requests that a person's input answers, and its clock's: on the fake platform, the display's own
/// events, each at its own time, with the display's frames between, as a host gives a window input, and its time
/// moved on with the display ticking through it. On a phone's window (`--touch`), a press is a finger's. And what the
/// platform's keyboard was told (`keyboard`), and what its menu for text and magnifier were asked to show, with the
/// handles drawn (`text_selection`), which only the fake platform knows; the system's appearance switched
/// (`appearance`), the window resized or hidden (`resize`, `hide`), its last frame beside one drawn afresh (`fresh`),
/// and what its windows hold (`memory`, `Windows::memory`).
pub(crate) const INPUT: [&str; 22] = ["tap", "hover", "down", "up", "key", "scroll", "drag", "replay", "trace", "clock", "wait_for", "away", "put_away", "summon", "hide", "back", "keyboard", "text_selection", "appearance", "resize", "fresh", "memory"];

/// How long a click holds the button down, as a person's does.
pub(crate) const CLICK_MS: f64 = 80.0;

/// Window `id` has handled the input given it and drawn what that changed: what its system was told (its keyboard,
/// its menu for text) is told as it draws, and input waits for a busy page (`Windows::waiting`) and a late refresh puts
/// a frame off, so it's looked at once it has, as Playwright's assertions wait, the display ticking meanwhile, a second
/// at most.
pub(crate) fn drawn(windows: &mut Windows, id: renderer::platform::WindowId) {
    let (started, mut frames) = (renderer::platform::fake_time(), windows.frames(id));
    while renderer::platform::fake_time() - started < 1000.0 {
        match windows.waiting(id) {
            true => frames = windows.frames(id),
            false if !windows.wanted() || windows.frames(id) != frames => return,
            false => {}
        }
        windows.run(1000.0 / 60.0 * 1.01);
    }
}

/// What a person's input request does, played as the display's events (`INPUT`).
pub(crate) fn input(windows: &mut Windows, id: renderer::platform::WindowId, request: &renderer::inspect::Request) -> anyhow::Result<Value> {
    match request.op.as_str() {
        // The pointer moved onto what it selects, or a point, as Playwright's hover does.
        "hover" => {
            let (x, y) = point_of(windows, id, request)?;
            press(windows, id, x, y, Phase::Moved);
            windows.run(0.0);
            Ok(json!({"hovered": {"x": x, "y": y}}))
        }
        "tap" => {
            let (x, y) = point_of(windows, id, request)?;
            let clicks = request.inputs.get("clicks").and_then(Value::as_u64).unwrap_or(1).clamp(1, 3);
            press(windows, id, x, y, Phase::Moved);
            windows.run(0.0);
            // With a seed, held as long as a person might: from a quick tap to a quarter of a second.
            let held = request.f64("hold").unwrap_or(CLICK_MS + windows.random(170) as f64 - windows.random(50) as f64);
            for _ in 0..clicks {
                match request.bool("shift") {
                    true => windows.pointer_down(id, x, y, 0, true),
                    false => press(windows, id, x, y, Phase::Down),
                }
                windows.run(held);
                press(windows, id, x, y, Phase::Up);
                windows.run(0.0);
            }
            Ok(json!({"tapped": {"x": x, "y": y}}))
        }
        "clock" => {
            if request.inputs.get("pause").and_then(Value::as_bool) == Some(false) {
                anyhow::bail!("the fake platform's clock is its driver's: step or seek it");
            }
            let now = renderer::platform::fake_time();
            let ms = request.f64("step").unwrap_or(0.0) + request.f64("seek").map_or(0.0, |at| at - now);
            if ms < 0.0 {
                anyhow::bail!("the clock goes forward only: it's at {now} ms");
            }
            windows.run(ms);
            Ok(json!({"time": renderer::platform::fake_time(), "paused": true}))
        }
        // The app goes to the background for `ms` and comes back, as a phone's does (`Windows::away`): `drawn` is how
        // many frames it drew meanwhile, which a phone's GPU refuses.
        "away" => Ok(json!({"drawn": windows.away(request.f64("ms").unwrap_or(1000.0))})),
        // The system's appearance switched to Dark (`dark`) or Light, as a person or their schedule does.
        "appearance" => {
            windows.appearance(if request.bool("dark") { Appearance::Dark } else { Appearance::Light });
            windows.run(0.0);
            Ok(json!({}))
        }
        // The window made `width` × `height` points, as a person drags a computer's window's edge or a phone turns.
        "resize" => {
            let (width, height) = (request.f64("width"), request.f64("height").ok_or_else(|| anyhow::anyhow!("say its height"))?);
            windows.sized(id, width.map(|width| width as f32), height as f32);
            windows.run(0.0);
            Ok(json!({}))
        }
        // The frame the window's page presented last, drawn on what it kept from frames before, beside one drawn
        // afresh as it is now (`Windows::afresh`): how many of their pixels differ, and where, as a frame must be the same however
        // it was come to; with both pictures where they differ. What's still on its way is drawn afresh a moment on
        // from its last frame, so is only said to be moving; a loop (a spinner) is drawn at the time its last frame
        // was, so it's compared; what isn't on the screen (a panel put away) shows nothing, and is judged as it's shown
        // again.
        "fresh" => {
            drawn(windows, id);
            if windows.arriving(id) {
                return Ok(json!({"moving": true}));
            }
            if windows.wanted() {
                let (now, refresh) = (renderer::platform::fake_time(), 1000.0 / 60.0);
                windows.run(((now / refresh).floor() + 1.0) * refresh - now);
            }
            if !windows.on_screen(id) {
                return Ok(json!({"shown": false}));
            }
            let (width, height, kept) = windows.presented(id).ok_or_else(|| anyhow::anyhow!("it has presented nothing"))?;
            let afresh = windows.afresh(id).ok_or_else(|| anyhow::anyhow!("it drew nothing afresh"))?;
            if (afresh.0, afresh.1) != (width, height) {
                anyhow::bail!("it presented {width}×{height} px, and drew {}×{} afresh", afresh.0, afresh.1);
            }
            let afresh = afresh.2;
            // A channel more than a few levels apart, past what blending in another order or a curve's edge rounds to.
            let (mut differ, mut left, mut top, mut right, mut bottom) = (0u32, width, height, 0, 0);
            for (at, (was, now)) in kept.chunks(4).zip(afresh.chunks(4)).enumerate() {
                if was.iter().zip(now).any(|(was, now)| was.abs_diff(*now) > 8) {
                    let (x, y) = (at as u32 % width, at as u32 / width);
                    (differ, left, top, right, bottom) = (differ + 1, left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1));
                }
            }
            let mut answer = json!({"differ": differ, "of": width * height});
            if differ > 0 {
                answer["box"] = json!({"x": left, "y": top, "width": right - left, "height": bottom - top});
                answer["kept"] = renderer::inspect::png(width, height, kept)?["png"].clone();
                answer["afresh"] = renderer::inspect::png(width, height, afresh)?["png"].clone();
            }
            Ok(answer)
        }
        // A panel (`--panel`) put away, as its shortcut or a click outside it does a launcher, and summoned again.
        "put_away" => {
            windows.put_away(id, false);
            windows.run(0.0);
            Ok(json!({}))
        }
        "summon" => {
            windows.summon(id, web_time::Instant::now(), None);
            windows.run(0.0);
            Ok(json!({}))
        }
        // Hidden while the page over it shows an app, as a phone's root window is (`Windows::hide`), or shown again
        // (`hidden` false).
        "hide" => {
            windows.hide(id, request.inputs.get("hidden").and_then(Value::as_bool) != Some(false));
            windows.run(0.0);
            Ok(json!({}))
        }
        "memory" => Ok(windows.memory()),
        // The device's Back, as Android's Back key is (`Windows::back`): what's open over the page shuts, or the window
        // leaves; `went` is whether it took it.
        "back" => {
            let went = windows.back(id);
            windows.run(0.0);
            Ok(json!({"went": went}))
        }
        // What the input with the keyboard asked the platform's keyboard for (Flutter's `TextInputConfiguration`:
        // `textCapitalization`, `autocorrect`, `inputType`…), as a phone's keyboard is told; null where none has it.
        "keyboard" => {
            drawn(windows, id);
            Ok(json!({"keyboard": windows.keyboard(id)}))
        }
        // What the system's menu for text was asked to show (its items, what it's for and where the system put it),
        // where its magnifier shows, and the handles drawn, as a phone's are, with the window's safe area.
        "text_selection" => {
            drawn(windows, id);
            Ok(windows.text_selection(id))
        }
        // As Playwright's `locator.waitFor` and its assertions do: what's selected comes to be `state` (`shown`: drawn
        // whole and in the window; `gone`: not there; `focused`: with the keyboard; `there`: there at all; `drawn`: at
        // `opacity`), with its
        // states as `is` says where it says, within `timeout` ms (1000 unless said), the display ticking meanwhile.
        "wait_for" => {
            let state = request.str("state").unwrap_or("shown");
            let timeout = request.f64("timeout").unwrap_or(1000.0);
            let mut query = json!({"op": "query"});
            for input in ["node", "role", "name", "text", "key"] {
                if let Some(value) = request.inputs.get(input) {
                    query[input] = value.clone();
                }
            }
            let is = request.inputs.get("is").and_then(Value::as_object).cloned().unwrap_or_default();
            let started = renderer::platform::fake_time();
            loop {
                let found = ask(windows, id, query.clone()).ok_or_else(|| anyhow::anyhow!("its window didn't answer"))?;
                let nodes = found["result"]["nodes"].as_array().cloned().unwrap_or_default();
                let node = nodes.first();
                let states = |node: &Value| is.iter().all(|(state, want)| node["states"].get(state).unwrap_or(&json!(false)) == want);
                let met = match state {
                    "gone" => nodes.is_empty(),
                    "focused" => nodes.iter().any(|node| node["focused"] == true && states(node)),
                    "there" => node.is_some_and(states),
                    // Drawn at `opacity` (within a hundredth), as its look says it is now.
                    "drawn" => node.is_some_and(|node| node["opacity"].as_f64().zip(request.f64("opacity")).is_some_and(|(drawn, want)| (drawn - want).abs() <= 0.01)),
                    _ => node.is_some_and(|node| node["opacity"].as_f64().is_some_and(|opacity| opacity >= 0.99) && node["seen"].as_f64().is_some_and(|seen| seen >= 0.99) && states(node)),
                };
                let waited = renderer::platform::fake_time() - started;
                if met || waited >= timeout {
                    return Ok(json!({"met": met, "ms": waited, "node": node}));
                }
                // A hair over a frame, as the display's refresh is a hair over 1000/60 ms, so a frame is always drawn
                // between one look and the next.
                windows.run(1000.0 / 60.0 * 1.01);
            }
        }
        // A button held down where `tap` would press it, and let go where it was held, for what shows mid-press.
        "down" => {
            let (x, y) = point_of(windows, id, request)?;
            press(windows, id, x, y, Phase::Moved);
            match request.bool("shift") {
                true => windows.pointer_down(id, x, y, 0, true),
                false => press(windows, id, x, y, Phase::Down),
            }
            windows.run(0.0);
            Ok(json!({"down": {"x": x, "y": y}}))
        }
        "up" => {
            let (x, y) = PRESSED.get().ok_or_else(|| anyhow::anyhow!("nothing is held down"))?;
            press(windows, id, x, y, Phase::Up);
            windows.run(0.0);
            Ok(json!({"up": {"x": x, "y": y}}))
        }
        // A trace of input as a device delivered it (`events`, each at its `ms` from the first): a trackpad's scrolls
        // with their phases and momentum, the pointer's moves, presses and keys, played as the display's events at their
        // own times, then `after` ms (1000 unless said) to come to rest.
        "trace" => {
            let events = request.inputs.get("events").and_then(Value::as_array).cloned().ok_or_else(|| anyhow::anyhow!("say what it plays: `events`, each {{ms, kind, …}}"))?;
            let begun = renderer::platform::fake_time();
            for event in &events {
                let ms = event["ms"].as_f64().unwrap_or(0.0);
                windows.run((begun + ms - renderer::platform::fake_time()).max(0.0));
                deliver(windows, id, event)?;
            }
            windows.run(request.f64("after").unwrap_or(1000.0));
            Ok(json!({"played": events.len()}))
        }
        "key" => {
            windows.key(id, request)?;
            windows.run(0.0);
            Ok(json!({"key": request.str("key")}))
        }
        "scroll" => {
            let (x, y) = if request.selects() { point_of(windows, id, request)? } else { (0.0, 0.0) };
            let (dx, dy) = (request.f32("dx").unwrap_or(0.0), request.f32("dy").unwrap_or(0.0));
            let momentum = request.inputs.get("momentum").and_then(Value::as_bool).unwrap_or(false);
            windows.wheel(id, (x, y), (dx, dy), request.str("phase"), momentum, notches(|name| request.inputs.get(name).and_then(Value::as_f64)));
            windows.run(0.0);
            let scrolled = windows.with(id, |screen| screen.scrolled_under(renderer::geometry::point(x, y))).flatten();
            Ok(json!({"scrolled": scrolled.map(|at| json!({"x": at.x, "y": at.y}))}))
        }
        "drag" => {
            let (x, y) = point_of(windows, id, request)?;
            let (dx, dy) = (request.f32("dx").unwrap_or(0.0), request.f32("dy").unwrap_or(0.0));
            let steps = request.inputs.get("steps").and_then(Value::as_u64).unwrap_or(10).max(1);
            let ms = request.f64("ms").unwrap_or(200.0).max(0.0);
            press(windows, id, x, y, Phase::Moved);
            press(windows, id, x, y, Phase::Down);
            for step in 1..=steps {
                windows.run(ms / steps as f64);
                let part = step as f32 / steps as f32;
                press(windows, id, x + dx * part, y + dy * part, Phase::Moved);
            }
            press(windows, id, x + dx, y + dy, Phase::Up);
            windows.run(0.0);
            Ok(json!({"dragged": {"x": dx, "y": dy}}))
        }
        _ => replay(windows, id, request),
    }
}

/// A mouse wheel's clicks as it turns (`notchesX`, `notchesY`), where an event says them.
pub(crate) fn notches(at: impl Fn(&str) -> Option<f64>) -> Option<(f32, f32)> {
    let (x, y) = (at("notchesX"), at("notchesY"));
    (x.is_some() || y.is_some()).then(|| (x.unwrap_or(0.0) as f32, y.unwrap_or(0.0) as f32))
}

/// One event of a trace, as the display's (`trace`'s `events`): a wheel's or a trackpad's scroll (`dx`, `dy`, its
/// `phase` and `momentum`, or a mouse wheel's `notchesX` and `notchesY`), the pointer moved, a button down or up (a finger's on a phone's window), a finger
/// (`touch`, its `phase`, `down`, `moved`, `up` or `cancelled`, and which `finger`, as a phone's screen says it), each
/// at `x`, `y`, or a `key`.
pub(crate) fn deliver(windows: &mut Windows, id: renderer::platform::WindowId, event: &Value) -> anyhow::Result<()> {
    let at = |name: &str| event[name].as_f64().unwrap_or(0.0) as f32;
    let (x, y) = (at("x"), at("y"));
    match event["kind"].as_str().unwrap_or_default() {
        "wheel" => windows.wheel(id, (x, y), (at("dx"), at("dy")), event["phase"].as_str(), event["momentum"].as_bool().unwrap_or(false), notches(|name| event[name].as_f64())),
        "move" => press(windows, id, x, y, Phase::Moved),
        "down" => press(windows, id, x, y, Phase::Down),
        "up" => press(windows, id, x, y, Phase::Up),
        "touch" => {
            let phase = event["phase"].as_str().and_then(Phase::named).ok_or_else(|| anyhow::anyhow!("say a touch's phase: down, moved, up or cancelled"))?;
            windows.finger(id, event["finger"].as_u64().unwrap_or(0), x, y, phase);
        }
        "key" => {
            let key = renderer::inspect::Request::parse(&json!({"v": renderer::inspect::VERSION, "op": "key", "key": event["key"], "shift": event["shift"], "alt": event["alt"], "ctrl": event["ctrl"], "meta": event["meta"]}).to_string()).map_err(|refused| anyhow::anyhow!("{refused}"))?;
            windows.key(id, &key)?;
        }
        kind => anyhow::bail!("there's no kind of event {kind}: wheel, move, down, up, touch and key are"),
    }
    Ok(())
}

thread_local! {
    /// Where the main button or the finger is held down, if it is, for `up`.
    pub(crate) static PRESSED: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// A press at `x`, `y`: on a phone's window (`--touch`) a finger's, as its screen says it; elsewhere the main button's,
/// a move the pointer's.
pub(crate) fn press(windows: &mut Windows, id: renderer::platform::WindowId, x: f32, y: f32, phase: Phase) {
    match phase {
        Phase::Down | Phase::Moved if PRESSED.get().is_some() || phase == Phase::Down => PRESSED.set(Some((x, y))),
        Phase::Up | Phase::Cancelled => PRESSED.set(None),
        _ => {}
    }
    if windows.with(id, |screen| screen.touch).unwrap_or(false) {
        return windows.finger(id, 0, x, y, phase);
    }
    match phase {
        Phase::Down => windows.pointer_down(id, x, y, 0, false),
        Phase::Moved => windows.pointer_moved(id, x, y),
        Phase::Up | Phase::Cancelled => windows.pointer_up(id, x, y, 0),
    }
}
