use super::*;

/// What a harness drives the fake platform's windows by (`fake`), as GPUI's `TestAppContext` drives its test windows:
/// it owns them, on its own thread, and says when their pages have run, when the display shows a frame and what input
/// comes, and the loop (`turn`) does the rest as it does on every host.
#[cfg(feature = "fake")]
impl Windows {
    /// The windows of a display of `scale` device pixels a point, refreshing `hz` times a second, whose time is
    /// `clock`'s, which their pages' JS reads too: paused, it moves only as the driver moves it.
    pub fn fake(scale: f32, hz: f64, clock: crate::js::Clock) -> anyhow::Result<Windows> {
        fake::keep_time(clock);
        Self::made(Display::new(scale, hz), Context::new())
    }

    /// Waits for the pipelines its windows draw with, compiled as it started (`Context::prepare`), as a harness that
    /// turns its loop itself on a paused clock does, so its first frame is drawn on its first turn, as every run's is.
    #[cfg(feature = "fake")]
    pub fn prepared(&self) {
        self.context.wait_compiled();
    }

    /// Whether the loop wants the display's next frame: something on a window moves.
    pub fn wanted(&self) -> bool {
        self.display.wanted()
    }

    /// The display's refreshes come late as `seed` decides (`fake::Display::seed`), and `random` draws from it too, and
    /// pictures' decodes are cut short now and then (`view::cut_short`).
    pub fn seed(&self, seed: u64) {
        self.display.seed(seed);
        crate::view::cut_short(seed);
    }

    /// A number from 0 to below `below`, as the seed decides: always 0 with none.
    pub fn random(&self, below: u64) -> u64 {
        self.display.random(below)
    }

    /// Each frame takes `ms` to draw from now on, the windows' thread busy meanwhile and the display's refreshes going
    /// by unseen, as on a device others load (the Home Screen on a loaded Mac); 0 for none.
    pub fn set_frame_ms(&self, ms: f64) {
        self.display.set_frame_ms(ms);
    }

    /// How pressed the windows are, by Compute Pressure's names, as their frames and the system say.
    pub fn pressure(&self) -> &'static str {
        self.pressure.name()
    }

    /// The display's link ticks `starts` refreshes after it's started from now on, and with `pauses`, (ticks,
    /// refreshes), not for that many once it's ticked that many (`fake::Link`); 0 and none for one that ticks at once.
    pub fn set_link(&self, starts: u32, pauses: Option<(u32, u32)>) {
        self.display.set_link(fake::Link { starts, pauses });
    }

    /// The display refreshes `hz` times a second from now on.
    pub fn set_refresh(&self, hz: f64) {
        self.display.set_refresh(hz);
    }

    /// Each window's page has run all it's been sent, and what it posted is applied, as `posted` applies it once its
    /// JS wakes the loop. → whether anything came.
    pub fn wait(&mut self) -> bool {
        let mut came = false;
        for window in self.windows.values_mut() {
            if let Ok(screen) = &mut window.screen
                && screen.idle()
            {
                window.dirty = true;
                came = true;
            }
        }
        came
    }

    /// The display refreshes now, its link saying so to the loop on its turn where it ticks (`set_link`). A panel just
    /// put on the screen with nothing drawn on it (a page that hadn't drawn yet) is wrong (`wrong`), as a host's
    /// compositor shows its window then, its backdrop an empty box, until the next frame.
    pub fn tick(&mut self) {
        if self.display.ticks() {
            self.display.tick(now());
        }
        for id in self.display.summoned() {
            let Some(window) = self.windows.get_mut(&id) else { continue };
            let parts = window.parts.len();
            let drawn = window.renderer.as_mut().is_some_and(|renderer| (0..parts).filter_map(|at| renderer.read(at)).any(|(_, _, rgba)| rgba.chunks_exact(4).any(|pixel| pixel[3] > 0)));
            if !drawn {
                self.display.wrong(format!("{} was put on the screen with nothing drawn on it", window.title));
            }
        }
    }

    /// What was wrong with what the display showed, since it was last asked.
    pub fn wrong(&self) -> Vec<String> {
        self.display.wrongs()
    }

    /// What window `id`'s input with the keyboard asked the platform's keyboard for (Flutter's
    /// `TextInputConfiguration`), if one has it.
    pub fn keyboard(&self, id: WindowId) -> Option<serde_json::Value> {
        self.display.keyboard(id)
    }

    /// What window `id`'s system was last asked to show of a finger's selection: its menu for text, what of it can be
    /// done, what it's for and where the system put it; its magnifier; and the handles the window draws, each in its
    /// points, with its safe area.
    pub fn text_selection(&self, id: WindowId) -> serde_json::Value {
        use serde_json::json;
        let rect = |rect: crate::geometry::Bounds<f32>| json!({"x": rect.origin.x, "y": rect.origin.y, "width": rect.size.width, "height": rect.size.height});
        let (menu, magnifier) = self.display.text_selection(id);
        let screen = self.windows.get(&id).and_then(|window| window.screen.as_ref().ok());
        let menu = menu.map(|(menu, placed)| {
            let items: Vec<&str> = [("cut", menu.cut), ("copy", menu.copy), ("paste", menu.paste), ("select", menu.select), ("selectAll", menu.select_all)].into_iter().filter(|(_, shown)| *shown).map(|(item, _)| item).collect();
            json!({"items": items, "for": rect(menu.rect), "rect": rect(placed)})
        });
        let handles: Vec<serde_json::Value> = screen.map(|screen| screen.handles().iter().map(|handle| json!({"end": format!("{:?}", handle.end).to_lowercase(), "rect": rect(handle.bounds())})).collect()).unwrap_or_default();
        let insets = screen.map(|screen| screen.insets).unwrap_or_default();
        json!({
            "menu": menu,
            "magnifier": magnifier.map(|magnifier| json!({"x": magnifier.at.x, "y": magnifier.at.y})),
            "handles": handles,
            "safeArea": {"top": insets.top, "right": insets.right, "bottom": insets.bottom, "left": insets.left},
        })
    }

    /// Window `id` is drawn again on the next turn, though nothing on it changed.
    pub fn redraw(&mut self, id: WindowId) {
        if let Some(window) = self.windows.get_mut(&id) {
            window.dirty = true;
        }
    }

    /// Whether window `id`'s page is still under what covers it as it opens (`Cover`).
    pub fn revealing(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| !matches!(window.cover, Cover::Gone))
    }

    /// It's a phone's screen, `screen` points big: every window but a panel is the whole of it, whatever it was asked
    /// to be, as an iPhone's and an Android phone's are.
    pub fn phone(&self, screen: Size<Pixels>) {
        self.display.phone(screen);
    }

    /// The windows on the screen, the root window first and the rest as they opened, each with how it's drawn over
    /// what's under it (`rail::Moved`).
    pub(super) fn stacked(&self) -> Vec<WindowId> {
        let mut ids: Vec<WindowId> = self.windows.iter().filter(|(_, window)| window.parts[0].native.composed().1).map(|(id, _)| *id).collect();
        ids.sort_by_key(|id| (!self.windows[id].root, *id));
        ids
    }

    /// Each window on the screen, bottom first: its id, its title, how it's drawn over what's under it, whether it
    /// shows its snapshot, and the room its page was last told and the room it has, in points.
    pub fn composed(&self) -> serde_json::Value {
        let listed: Vec<serde_json::Value> = self
            .stacked()
            .into_iter()
            .map(|id| {
                let window = &self.windows[&id];
                let (moved, _) = window.parts[0].native.composed();
                let told = window.screen.as_ref().ok().and_then(Screen::told_room);
                let room = window.room();
                serde_json::json!({
                    "window": id,
                    "title": window.title,
                    "moved": {"scale": moved.scale, "x": moved.x, "y": moved.y, "opacity": moved.opacity, "dim": moved.dim},
                    "snapshot": window.picture.is_some(),
                    "covered": window.cover.opacity(),
                    "starting": window.screen.as_ref().is_ok_and(Screen::starting),
                    "told": told.map(|told| serde_json::json!({"width": told.width, "height": told.height})),
                    "room": {"width": room.width.0, "height": room.height.0},
                })
            })
            .collect();
        serde_json::json!(listed)
    }

    /// The screen as its compositor shows it: each window on it what it last presented, over those under it, grown and
    /// moved about its middle, as opaque and with corners as round as it's moved to, what's under it dimmed by its
    /// `dim` (`rail::Moved`), as an iPhone's Core Animation draws its card: its width, height and pixels.
    pub fn screen(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let stacked = self.stacked();
        let first = *stacked.first()?;
        let (width, height, mut rgba) = self.presented(first)?;
        for id in stacked.into_iter().skip(1) {
            let scale = self.windows[&id].parts[0].native.scale();
            let (moved, _) = self.windows[&id].parts[0].native.composed();
            let Some((w, h, drawn)) = self.presented(id) else { continue };
            for pixel in rgba.chunks_exact_mut(4) {
                for channel in &mut pixel[..3] {
                    *channel = (f32::from(*channel) * (1.0 - moved.dim)).round() as u8;
                }
            }
            // Where each pixel of the screen comes from on the window: back through its transform, about its middle.
            let (middle_x, middle_y) = (w as f32 / 2.0, h as f32 / 2.0);
            let corner = moved.corner * scale;
            for (y, x) in (0..height).flat_map(|y| (0..width).map(move |x| (y, x))) {
                let from_x = middle_x + (x as f32 + 0.5 - middle_x - moved.x * scale) / moved.scale;
                let from_y = middle_y + (y as f32 + 0.5 - middle_y - moved.y * scale) / moved.scale;
                if from_x < 0.0 || from_y < 0.0 || from_x >= w as f32 || from_y >= h as f32 {
                    continue;
                }
                // Outside its round corners.
                let (dx, dy) = ((corner - from_x).max(from_x - (w as f32 - corner)), (corner - from_y).max(from_y - (h as f32 - corner)));
                if dx > 0.0 && dy > 0.0 && dx.hypot(dy) > corner {
                    continue;
                }
                let (from, to) = (((from_y as u32 * w + from_x as u32) * 4) as usize, ((y * width + x) * 4) as usize);
                let alpha = f32::from(drawn[from + 3]) / 255.0 * moved.opacity;
                for channel in 0..3 {
                    rgba[to + channel] = (f32::from(drawn[from + channel]) * alpha + f32::from(rgba[to + channel]) * (1.0 - alpha)).round() as u8;
                }
            }
        }
        Some((width, height, rgba))
    }

    /// What window `id` last presented, as its compositor shows it: its width, height and pixels (`composed`).
    pub fn presented(&mut self, id: WindowId) -> Option<(u32, u32, Vec<u8>)> {
        let window = self.windows.get_mut(&id)?;
        let parts = window.parts.len();
        let drawn = (0..parts).map(|at| window.renderer.as_mut().and_then(|renderer| renderer.read(at))).collect();
        composed(window, drawn)
    }

    /// Window `id` drawn afresh as it is now, as its compositor would show it (`presented`): its page painted afresh,
    /// and each of its parts drawn where it is by a renderer of its own, so nothing a frame before kept (a layer, a
    /// blur, a surface's last image) is in it. A frame drawn on what was kept must be the same.
    pub fn afresh(&mut self, id: WindowId) -> Option<(u32, u32, Vec<u8>)> {
        let window = self.windows.get_mut(&id)?;
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let mut scene = crate::render::scene::Scene::default();
        window.screen.as_mut().ok()?.paint_afresh(room, scale, &mut self.text, &self.atlas, &mut scene);
        let size = |length: f32| DevicePixels((length * scale).round() as i32);
        let drawn = window
            .parts
            .iter()
            .map(|part| {
                let device = if window.panel { Size { width: size(part.bounds.size.width), height: size(part.bounds.size.height) } } else { part.native.device_size() };
                let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), device);
                renderer.draw(&scene, &[[(part.bounds.origin.x * scale).round(), (part.bounds.origin.y * scale).round()]]);
                let read = renderer.read(0);
                renderer.destroy();
                read
            })
            .collect();
        composed(window, drawn)
    }

    /// The fake platform's time moves on by `ms`, its pages' clock with it: each of their timers due meanwhile runs at
    /// its own time, and while the loop wants frames the display ticks each refresh, the loop taking a turn for each,
    /// as a host's do. What each page sent is run before each turn (`wait`), and what a turn sent one (input that
    /// waited for it) is run and drawn at once, on a turn of its own, as its post wakes a host's loop (`posted`).
    pub fn run(&mut self, ms: f64) {
        let refresh = self.display.refresh().as_secs_f64() * 1000.0;
        let started = fake::time();
        let mut late = 0;
        loop {
            self.wait();
            let drawn: u64 = self.windows.values().map(|window| window.frames).sum();
            self.turn();
            if self.wait() {
                self.turn();
            }
            // A frame drawn on a device others load keeps its thread busy, the display's refreshes going by unseen.
            let busy = self.display.frame_ms();
            if busy > 0.0 && self.windows.values().map(|window| window.frames).sum::<u64>() > drawn {
                fake::advance(busy);
                for window in self.windows.values_mut() {
                    if let Ok(screen) = &mut window.screen {
                        window.dirty |= screen.advance(0.0);
                    }
                }
            }
            // What the turn set off decoding has decoded, so when it's there is known before time moves on past it.
            self.wait();
            let now = fake::time();
            let left = started + ms - now;
            // The display's next refresh: the one it's at now, give or take a microsecond, unless it's taken it, as the
            // clock's sums of ms land a hair either side of one. Where it took the next a hair on instead, a page's timer
            // due a hair before it moved the clock there, and the refresh was never taken: a frame held (the Launcher's
            // mouse wheel page turn at 200 ms, 60 Hz). The display ticks while the loop wants frames, or a window has
            // what came meanwhile to draw (a picture that's there at its time).
            let at = (now / refresh).round();
            let reached = (now - at * refresh).abs() < 0.001;
            let due = if reached && self.display.refreshed() != Some(at as i64) { now } else { (if reached { at + 1.0 } else { (now / refresh).floor() + 1.0 }) * refresh };
            let tick = (self.display.wanted() || self.windows.values().any(|window| window.dirty && window.on_screen())).then(|| (due - now).max(0.0));
            let timer = self.windows.values().filter_map(|window| window.screen.as_ref().ok()?.timer_due()).chain(self.display.answer_due().map(|at| at - now)).reduce(f64::min).map(|due| due.max(0.0));
            // What the loop has due by itself (`due`), as a host's waits for it: the loop's own clock standing in for
            // a display link that's stopped, a refresh after its last frame, not at the display's next refresh.
            let woken = self.due().map(|due| due.as_secs_f64() * 1000.0).filter(|due| *due > 0.001);
            // A refresh a hair past the end, as a driver's own refresh, `1000 / hz` ms, is a hair short of the
            // display's (a `Duration`, whole nanoseconds), is the one it ends on, where it was never shown.
            let next = tick.into_iter().chain(timer).chain(woken).reduce(f64::min).filter(|next| *next <= left + 0.001);
            // The platform's time moves on once, its pages' clock with it, then each page runs what's due by then.
            let by = next.unwrap_or(left.max(0.0));
            fake::advance(by);
            for (id, answer) in self.display.answers_due(fake::time()) {
                self.with(id, |screen| screen.message(&answer));
            }
            for window in self.windows.values_mut() {
                if let Ok(screen) = &mut window.screen {
                    window.dirty |= screen.advance(0.0);
                }
            }
            if next.is_none() {
                break;
            }
            if next == tick {
                self.display.reached((fake::time() / refresh).round() as i64);
                // A late refresh shows nothing new: what's waiting is shown on the next.
                match self.display.late(late) {
                    true => late += 1,
                    false => {
                        late = 0;
                        self.tick();
                    }
                }
            }
        }
        self.wait();
        self.turn();
    }

    /// Where `x`, `y` of window `id`'s page is to its system: in the uppermost of its native windows there (each of a
    /// panel's surfaces of glass is one), at that point of it, as each says the pointer's in its own.
    pub(super) fn part_at(&self, id: WindowId, x: f32, y: f32) -> (usize, crate::geometry::Point<f32>) {
        let at = crate::geometry::point(x, y);
        let parts = self.windows.get(&id).map(|window| &window.parts[..]).unwrap_or_default();
        let part = parts.iter().rposition(|part| part.bounds.contains(&at)).unwrap_or(0);
        let origin = parts.get(part).map_or(crate::geometry::point(0.0, 0.0), |part| part.bounds.origin);
        (part, crate::geometry::point(x - origin.x, y - origin.y))
    }

    /// Finger `finger` at `x`, `y` on window `id`, as a phone's screen says it.
    pub fn finger(&mut self, id: WindowId, finger: u64, x: f32, y: f32, phase: crate::view::touch::Phase) {
        let (part, at) = self.part_at(id, x, y);
        self.display.push_to(id, part, Input::Touch(finger, at, phase));
    }

    /// The pointer moved to `x`, `y` on window `id`, as a mouse does.
    pub fn pointer_moved(&mut self, id: WindowId, x: f32, y: f32) {
        let (part, at) = self.part_at(id, x, y);
        self.display.push_to(id, part, Input::Moved(at));
    }

    /// A button went down at `x`, `y` on window `id`: 0 the main one, 1 the middle, 2 the secondary, as the DOM's.
    pub fn pointer_down(&mut self, id: WindowId, x: f32, y: f32, button: u8, shift: bool) {
        let (part, at) = self.part_at(id, x, y);
        self.display.push_to(id, part, Input::Down(at, button, shift));
    }

    pub fn pointer_up(&mut self, id: WindowId, x: f32, y: f32, button: u8) {
        let (part, at) = self.part_at(id, x, y);
        self.display.push_to(id, part, Input::Up(at, button));
    }

    /// A wheel's or a trackpad's scroll at `at` on window `id`, by `by` in points, in a trackpad's phase where it says
    /// one (its fingers' by AppKit's names, `crate::view::Phase::FINGERS`, and whether it's the momentum after), or a
    /// wheel's `notches` where it turned by clicks.
    pub fn wheel(&mut self, id: WindowId, at: (f32, f32), by: (f32, f32), fingers: Option<&str>, momentum: bool, notches: Option<(f32, f32)>) {
        let fingers = fingers.and_then(|named| crate::view::Phase::FINGERS.into_iter().find(|one| *one == named));
        let phase = (fingers.is_some() || momentum).then_some(crate::view::Phase { fingers, momentum });
        let notches = notches.map(|(x, y)| crate::geometry::point(x, y));
        let (part, at) = self.part_at(id, at.0, at.1);
        self.display.push_to(id, part, Input::Wheel(at, crate::geometry::point(by.0, by.1), phase, notches));
    }

    /// A key on window `id`, as an inspector's `key` request says it.
    pub fn key(&mut self, id: WindowId, request: &crate::inspect::Request) -> anyhow::Result<()> {
        self.display.push(id, Input::Key(crate::inspect::Key::of(request)?));
        Ok(())
    }

    /// What window `id`'s page asked its client comes back as `answers` has it on its way, each at its time on the
    /// page's clock, as the clock moves (`run`), as a page's timers fire: a server's answer a round trip after it's asked.
    pub fn answering(&mut self, id: WindowId, answers: std::rc::Rc<std::cell::RefCell<Vec<(f64, serde_json::Value)>>>) {
        self.display.answering(id, answers);
    }

    /// Window `id` made `width` × `height` points, as a phone turns, a computer's window's edge is dragged, or a system
    /// lays a window out from nothing, 0 tall: a panel's room is what its page says (`resize`), any other's is its
    /// native window's, which its system tells it of (`Input::Resized`).
    pub fn sized(&mut self, id: WindowId, width: Option<f32>, height: f32) {
        let Some(window) = self.windows.get(&id) else { return };
        if window.panel {
            return self.resize(id, width, height);
        }
        let (scale, now) = (window.parts[0].native.scale(), window.parts[0].native.size());
        let pixels = |points: f32| crate::geometry::DevicePixels((points * scale).round() as i32);
        self.display.push(id, Input::Resized(Size { width: pixels(width.unwrap_or(now.width.0)), height: pixels(height) }));
    }

    /// The app goes to the background for `ms` of its time and comes back, as an iPhone's does: its windows lose the
    /// keyboard, then iOS has them drawn in the other appearance and back, for the app switcher's snapshot of each
    /// (WWDC 2019's "Implementing Dark Mode on iOS"), while their pages run; then they have it back. → how many frames
    /// its windows presented while it was there, each lost on an iPhone, its GPU's work refused.
    pub fn away(&mut self, ms: f64) -> u64 {
        let frames = |windows: &Windows| windows.windows.values().map(|window| window.frames).sum::<u64>();
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        let other = match appearance() {
            crate::Appearance::Light => crate::Appearance::Dark,
            crate::Appearance::Dark => crate::Appearance::Light,
        };
        for &id in &ids {
            self.display.push(id, Input::Focus(false));
        }
        self.run(0.0);
        self.background(true);
        let before = frames(self);
        for appearance in [other, appearance()] {
            for &id in &ids {
                self.display.push(id, Input::Appearance(appearance));
            }
            self.run(ms / 2.0);
        }
        let drawn = frames(self) - before;
        self.background(false);
        for &id in &ids {
            self.display.push(id, Input::Focus(true));
        }
        self.run(0.0);
        drawn
    }

    /// How many frames window `id` has presented, all told.
    pub fn frames(&self, id: WindowId) -> u64 {
        self.windows.get(&id).map_or(0, |window| window.frames)
    }

    /// Whether something on window `id` is on its way somewhere it'll arrive at (`Screen::arriving`): moving, but not
    /// by a loop alone (a spinner), which never arrives.
    pub fn arriving(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| window.screen.as_ref().is_ok_and(Screen::arriving))
    }

    /// Whether window `id` is on screen (`Window::on_screen`): not a panel put away.
    pub fn on_screen(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(Window::on_screen)
    }

    /// The system's appearance becomes `appearance`, as a person switching between Light and Dark makes it (or their
    /// schedule does): every window is told, as its host tells it.
    pub fn appearance(&mut self, appearance: crate::Appearance) {
        fake::appear(appearance);
        for id in self.windows.keys().copied().collect::<Vec<_>>() {
            self.display.push(id, Input::Appearance(appearance));
        }
    }

    /// Whether input to window `id` waits for its page to catch up (`waits`), not handled yet.
    pub fn waiting(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| !window.waiting.is_empty())
    }

    /// What window `id`'s frames cost since this was last asked, as the inspector's `profile` says it (`Screen::cost`),
    /// its GPU waited for, to say its time for the last.
    pub fn cost(&mut self, id: WindowId) -> Option<serde_json::Value> {
        let window = self.windows.get_mut(&id)?;
        let renderer = window.renderer.as_mut()?;
        renderer.finish();
        Some(window.screen.as_mut().ok()?.cost(renderer))
    }

    /// What window `id` last drew, the GPU it drew it with and its atlas.
    pub fn drawn(&self, id: WindowId) -> Option<(&crate::render::scene::Scene, &Context, &Arc<Atlas>)> {
        Some((&self.windows.get(&id)?.scene, &self.context, &self.atlas))
    }
}

/// What `window`'s parts drew (`drawn`, each part's, where it drew), as its compositor shows them: a window's one part
/// as it is, and a panel's put together where each is, over the system's glass, a flat grey of its Colour Mode here.
#[cfg(feature = "fake")]
fn composed(window: &Window, drawn: Vec<Option<(u32, u32, Vec<u8>)>>) -> Option<(u32, u32, Vec<u8>)> {
    if window.parts.len() == 1 && !window.panel {
        return drawn.into_iter().next().flatten();
    }
    let scale = window.parts[0].native.scale();
    let (width, height) = ((window.room.width.0 * scale).round() as u32, (window.room.height.0 * scale).round() as u32);
    let glass = glass(window);
    let mut rgba: Vec<u8> = (0..width * height).flat_map(|_| [glass[0], glass[1], glass[2], 255]).collect();
    for (part, drawn) in window.parts.iter().zip(drawn) {
        let (left, top) = ((part.bounds.origin.x * scale).round() as i64, (part.bounds.origin.y * scale).round() as i64);
        let Some((w, h, drawn)) = drawn else { continue };
        for (y, x) in (0..h).flat_map(|y| (0..w).map(move |x| (y, x))) {
            let (to_x, to_y) = (i64::from(x) + left, i64::from(y) + top);
            if to_x < 0 || to_y < 0 || to_x >= i64::from(width) || to_y >= i64::from(height) {
                continue;
            }
            let (from, to) = (((y * w + x) * 4) as usize, ((to_y as u32 * width + to_x as u32) * 4) as usize);
            let alpha = u32::from(drawn[from + 3]);
            for channel in 0..3 {
                rgba[to + channel] = ((u32::from(drawn[from + channel]) * alpha + u32::from(rgba[to + channel]) * (255 - alpha)) / 255) as u8;
            }
        }
    }
    Some((width, height, rgba))
}

/// The system's glass a panel is shown over, a flat grey of its Colour Mode here.
#[cfg(feature = "fake")]
fn glass(window: &Window) -> [u8; 3] {
    match window.screen.as_ref().ok().and_then(|screen| screen.theme).unwrap_or(appearance()) {
        crate::Appearance::Dark => [30, 30, 32],
        crate::Appearance::Light => [236, 236, 238],
    }
}
