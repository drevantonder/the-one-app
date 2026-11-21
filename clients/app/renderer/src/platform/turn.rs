use super::*;

impl Windows {
    /// Every window's renderer, each that has one.
    pub(super) fn renderers(&self) -> impl Iterator<Item = &Renderer> {
        self.windows.values().filter_map(|window| window.renderer.as_ref())
    }

    /// How long until any window's screen has something to do by itself, a window waiting to show has waited long
    /// enough, or a renderer has drawn nothing for a while (`Renderer::idle`).
    pub fn due(&self) -> Option<std::time::Duration> {
        let screens = self.windows.values().filter_map(|window| window.screen.as_ref().ok()?.due());
        let showing = self.windows.values().filter(|window| !window.shown || matches!(window.cover, Cover::Over(_)) || window.reloading).map(|window| (if window.picture.is_some() { SHOWN_WITHIN } else { QUIET }).saturating_sub(since(window.opened)));
        let blank = self.windows.values().filter(|window| !window.looked && (!window.panel || window.visible)).map(|window| BLANK.saturating_sub(since(window.looking)));
        // Where the display says nothing of its frames, hasn't yet since they were asked for, or has stopped, the loop's
        // own clock stands in for it a refresh after its last; while its link ticks, the loop looks again once a tick is
        // a quarter of a refresh late (`ticking`).
        let refresh = self.refresh();
        let frame = self.framing().then(|| match self.ticked.filter(|_| self.ticking()) {
            Some(at) => (refresh * 5 / 4).saturating_sub(since(at)),
            None => refresh.saturating_sub(since(self.framed)),
        });
        // A pull's spring steps at a 120 Hz display's frames.
        let pulled = self.windows.values().any(|window| window.pull.as_ref().is_some_and(rail::Pull::springing)).then_some(std::time::Duration::from_micros(8_333));
        let check = self.check.as_ref().map(conformance::Check::due).into_iter().chain(self.recording.as_ref().map(|recording| recording.until.saturating_duration_since(now()))).min();
        let idle = self.renderers().filter_map(Renderer::idle_in);
        // A change held for the refresh after its window's last frame (`turned`), where the display says nothing yet.
        let held = self.windows.values().filter(|window| !self.ticking() && window.dirty && window.on_screen()).filter_map(|window| window.presented).map(|at| refresh.saturating_sub(since(at)));
        screens.chain(showing).chain(blank).chain(frame).chain(pulled).chain(check).chain(idle).chain(held).min()
    }

    /// Whether its windows are a phone's: each the whole screen, an app's over the Home Screen, which it opens out of.
    #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
    pub(super) fn on_phone(&self) -> bool {
        #[cfg(feature = "fake")]
        return self.display.is_phone();
        #[cfg(not(feature = "fake"))]
        cfg!(any(target_os = "android", target_os = "ios"))
    }

    /// Whether the display says when it shows each frame, and has within the last refresh and a quarter since its
    /// frames were last asked for: a display link just started takes a while to tick (a Mac's two or three refreshes),
    /// and a ProMotion display's stops a while as if what's on it were idle, and the loop's own clock stands in for it
    /// meanwhile, so what moves isn't held. A tick later than that finds the frame that clock drew for it (`turned`).
    pub(super) fn ticking(&self) -> bool {
        self.display.paced() && self.ticked.is_some_and(|at| since(at) < self.refresh() * 5 / 4)
    }

    /// The display's refresh interval, as it says: every platform's knows it before its first frame, so the loop's own
    /// clock stands in at the display's pace from the first motion after it starts, never a 60 Hz display's on a 120.
    pub fn refresh(&self) -> std::time::Duration {
        self.display.refresh()
    }

    /// Whether a window on screen wants the display's next frame: something on it moves by itself, or its page asked
    /// for one (`requestAnimationFrame`).
    pub(super) fn framing(&self) -> bool {
        self.check.as_ref().is_some_and(conformance::Check::moving)
            || self.windows.values().any(|window| window.on_screen() && (window.moving || (window.dirty && window.working()) || window.slide.is_some() || window.pull.as_ref().is_some_and(rail::Pull::springing) || matches!(window.cover, Cover::Fading(..)) || window.screen.as_ref().is_ok_and(Screen::framing)))
    }

    /// Makes what draws again, on a new GPU device, where the one it drew with was lost, as a browser makes a lost
    /// WebGPU device's: the atlas and every window's renderer, each window drawn afresh (`Screen::gpu_lost`).
    pub(super) fn recover(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.recovered(Context::new());
        // The web's is asked for with a promise, and drawn on once it's come.
        #[cfg(target_arch = "wasm32")]
        if let Some(made) = web::recover() {
            self.recovered(made);
        }
    }

    pub(super) fn recovered(&mut self, made: anyhow::Result<Context>) {
        let made = made.map(|context| {
            let atlas = Arc::new(Atlas::new(&context));
            (context, atlas)
        });
        let (context, atlas) = match made {
            Ok(made) => made,
            Err(error) => return log::error!("no GPU to draw with again yet: {error:#}"),
        };
        for window in self.windows.values_mut() {
            window.renderer = None;
            if !self.background && window.parts.iter().all(|part| part.native.has_surface()) {
                match draws_on(&context, atlas.clone(), &window.parts) {
                    Ok(renderer) => window.renderer = Some(renderer),
                    Err(error) => log::error!("a native window can't draw: {error:#}"),
                }
            }
            if let Ok(screen) = &mut window.screen {
                screen.gpu_lost();
            }
            window.dirty = true;
        }
        (self.context, self.atlas) = (context, atlas);
        log::info!("drawing again on a new GPU device");
    }

    /// After each wait for what's due, a turn of the loop (`turned`), whose time each window's frame timing hears.
    pub fn turn(&mut self) {
        let started = web_time::Instant::now();
        self.turned();
        for window in self.windows.values_mut() {
            window.timing.turned(started.elapsed());
        }
    }

    /// A turn of the loop: what the display said, timers that are due, then each window that changed drawn.
    pub(super) fn turned(&mut self) {
        let turned = now();
        if self.context.lost() {
            self.recover();
        }
        // Each window waiting for its pipelines has its renderer once they're compiled, on whatever turns the loop: a
        // harness's own turns too, as well as the windows' thread woken as they are (`Context::prepare`).
        #[cfg(not(target_arch = "wasm32"))]
        self.compiled();
        for (id, part, input) in self.display.events() {
            // The fake platform's pages are as quick as what comes, the same each time: each has run what came before it
            // (`wait`), unless a seed has it busy (`Thread::seed`).
            #[cfg(feature = "fake")]
            self.wait();
            self.input(id, part, input);
        }
        for window in self.windows.values_mut() {
            if let Some(renderer) = window.renderer.as_mut()
                && renderer.idle_in().is_some_and(|due| due.is_zero())
            {
                renderer.idle();
            }
        }
        #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
        self.spring();
        #[cfg(any(target_os = "ios", feature = "fake"))]
        self.slide();
        // The display showed a frame: what moves on a window on screen is drawn again, and its page's
        // `requestAnimationFrame` runs, told when, as a browser's does once a frame.
        let framing = self.framing();
        let ticked = self.display.frame();
        self.ticked = ticked.or(self.ticked);
        self.vsync = ticked.or(self.vsync);
        // Where the loop's own clock stands in, its frame is the one the display would have shown a refresh after its
        // last, at most a refresh late (a link that stopped, noticed a quarter of a refresh on), so what moves keeps the
        // display's pace: at the clock's own time, a link's pause had it move a step and a half, then half a step.
        let refresh = self.refresh();
        let shown = ticked.or_else(|| (framing && !self.ticking() && since(self.framed) >= refresh).then(|| Some(self.framed + refresh).filter(|next| since(*next) < refresh).unwrap_or_else(now)));
        // What moves is drawn where it is at the time of the display's refresh its frame is for, as a link's tick says
        // it: the nearest to the loop's own clock's, which, counting from a frame drawn as input came, is out of step
        // with them. Drawn at its own time, a step as the link started came two thirds of a refresh after the one
        // before; at the refresh before it, a frame due a hair before a refresh was drawn at the last one's time.
        let time = shown.map(|at| match self.vsync.filter(|vsync| *vsync <= at) {
            Some(vsync) => vsync + refresh.mul_f64(((at - vsync).as_secs_f64() / refresh.as_secs_f64()).round()),
            None => at,
        });
        // When the display showed the frame before, which a frame presented since was drawn after (`presented`).
        let before = shown.map(|at| std::mem::replace(&mut self.showed, at));
        if let Some(at) = shown {
            self.framed = at;
            if let Some(check) = &self.check {
                self.record.tick(at);
                // The window it checks moves on each tick while it's kept moving.
                if check.moving()
                    && let Some(window) = self.windows.get_mut(&check.window)
                {
                    window.moving = true;
                }
            }
            for window in self.windows.values_mut().filter(|window| window.on_screen()) {
                window.dirty |= std::mem::take(&mut window.moving);
                if let (Ok(screen), Some(time)) = (&mut window.screen, time) {
                    screen.frame(time);
                }
            }
        }
        for window in self.windows.values_mut() {
            if let Ok(screen) = &mut window.screen
                && screen.due().is_some_and(|due| due.is_zero())
            {
                screen.fire();
                window.dirty = true;
            }
        }
        // A window whose page's module didn't run says only that its app didn't open.
        for window in self.windows.values_mut() {
            if let Some(why) = window.screen.as_ref().ok().and_then(Screen::didnt_start).map(str::to_owned) {
                window.screen = Err(format!("{} didn't open", window.title));
                window.dirty = true;
                (window.on_event)(Event::Failed(why));
            }
            // One whose page crashed shows nothing of it, never what it drew last as if it were alive.
            if let Some(why) = window.screen.as_ref().ok().and_then(Screen::crashed) {
                window.screen = Err(format!("{} stopped working", window.title));
                window.dirty = true;
                (window.on_event)(Event::Crashed(why));
            }
            // One that's still blank a while after it opened says so, where nothing else said why.
            if window.panel && !window.visible {
                window.looking = now();
            }
            if !window.looked && since(window.looking) >= BLANK {
                window.looked = true;
                let why = match &window.screen {
                    _ if window.pending => Some("its page hadn't started"),
                    Ok(_) if !window.live => Some("it hadn't drawn its page"),
                    Ok(screen) if screen.drew_nothing() => Some("its page had drawn nothing"),
                    _ => None,
                };
                if let Some(why) = why {
                    (window.on_event)(Event::Blank(format!("{why} {} s after it was on screen", BLANK.as_secs())));
                }
            }
        }
        // The system's own parts of a window (a Mac's glass and title bar, Windows' frame, a Linux window manager's)
        // are drawn light or dark as its Colour Mode says, or as the system is where it says nothing.
        for window in self.windows.values_mut() {
            let theme = window.screen.as_ref().ok().and_then(|screen| screen.theme);
            if window.chrome != Some(theme) {
                window.chrome = Some(theme);
                for part in &window.parts {
                    self.display.set_appearance(&part.native, theme);
                }
            }
        }
        // A window whose page uses what this host can't draw says so, and why.
        for window in self.windows.values_mut() {
            if let Some(why) = window.screen.as_ref().ok().and_then(|screen| screen.mismatched.clone()) {
                window.screen = Err(format!("{} didn't open: {why}", window.title));
                window.dirty = true;
                (window.on_event)(Event::Mismatched(why));
            }
        }
        // A window hands the display at most one frame a refresh: what changes after it has drawn since the last tick is
        // drawn on the next, as a browser draws on its frames, and where the display's link isn't ticking (idle, just
        // asked for, or stopped), a refresh after the frame it drew. Drawn as each input or change came, frames queued
        // ahead of the display, each shown a refresh late, and the thread waited for an image to draw on every frame
        // (an iPhone's main thread, 120 times a second); and a finger lifting in the refresh after its last move drew
        // again, so where it last was never showed, and a fling jumped as it was let go. A change before it has is
        // drawn at once, so what the page answers just after a tick, or the first input, isn't held a refresh.
        let ticking = self.ticking();
        let mut held = false;
        #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
        let phone = self.on_phone();
        // A frame presented since the display showed the one before, in the quarter refresh before it showed this one
        // or after, is too late for it: it's the next refresh's, which the window draws no other for. Forgotten, a
        // link's tick at the moment the loop's own clock or input drew drew again at once, and so did input on the frame
        // after, a frame never shown (a fling's lift).
        if let (Some(at), Some(before)) = (shown, before) {
            for window in self.windows.values_mut() {
                window.presented = window.presented.filter(|presented| *presented > before && at.saturating_duration_since(*presented) < refresh / 4);
            }
        }
        for window in self.windows.values_mut() {
            // A new window shows at once, its page covered (`Cover`) by its snapshot until its page has heard back from
            // what it asked, or has waited long enough, or with no snapshot by its canvas, until its page is ready or a
            // moment has gone by; then what covers it fades into it.
            let starting = (window.pending || window.screen.as_ref().is_ok_and(Screen::starting)) && since(window.opened) < SHOWN_WITHIN;
            let device = window.parts[0].native.device_size();
            if window.picture.as_ref().is_some_and(|picture| picture.len() != (device.width.0 * device.height.0 * 4) as usize) {
                window.picture = None;
            }
            if matches!(window.cover, Cover::Over(_)) && (!starting || (window.picture.is_none() && since(window.opened) >= QUIET)) {
                window.loading = starting;
                window.cover = match std::mem::replace(&mut window.cover, Cover::Gone) {
                    Cover::Over(Some(cover)) => Cover::Fading(cover, now()),
                    _ => Cover::Gone,
                };
            }
            // A page shown as it started is ready: what was on the screen, as it was last drawn, fades into it.
            if window.loading && !starting {
                window.loading = false;
                let shown = covered(&window.scene, window.cover.scene(), window.cover.opacity(), device);
                window.cover = Cover::Fading(shown, now());
            }
            if let Cover::Fading(_, began) = window.cover {
                window.dirty = true;
                if since(began) >= FADES_IN {
                    (window.cover, window.picture, window.uncovered) = (Cover::Gone, None, true);
                }
            }
            if !window.shown {
                // Not before it can be drawn, so it never shows with nothing drawn on it: as its pipelines compile
                // (`compiled`); and a panel, which nothing covers and which is drawn once put away and then summoned
                // with that frame, not before its page has heard back, or has waited long enough.
                if (window.panel && starting) || (window.parts[0].native.has_surface() && window.renderer.is_none() && !self.background) {
                    continue;
                }
                window.shown = true;
                // On a phone, an app's window grows out of the icon it was opened from, from the frame it first shows
                // in, as the reverse of its move home (`rail::Pull::opening`).
                #[cfg(any(target_os = "android", target_os = "ios", feature = "fake"))]
                if phone && !window.root && !window.panel && window.icon.is_some() && window.pull.is_none() {
                    let mut opening = rail::Pull::opening();
                    let reduced = window.screen.as_ref().is_ok_and(|screen| screen.reduced_motion);
                    let room = window.room();
                    let (moved, _) = opening.step(Size { width: room.width.0, height: room.height.0 }, window.icon, reduced);
                    self.display.set_moved(&window.parts[0].native, moved);
                    window.pull = Some(opening);
                }
                // A panel shows as it's summoned.
                if !window.panel {
                    self.display.show(&window.parts[0].native);
                }
            }
            // Its page loading again in its place (`reload`) is drawn as a new window shows, the frame before staying
            // on screen until then.
            if window.reloading && starting {
                continue;
            }
            // A panel put away is painted as its page changes, never shown, so its text is shaped and its glyphs and
            // pictures are in the atlas, and drawn once, so its surfaces have had a frame, by the time it's summoned,
            // and the frame it shows with is as quick as any after (26 to 52 ms on Linux's first summon after the app started, where
            // later ones take 7). What moves asks for no more frames while it's away. A window with no surface yet
            // (Android's view, until it's laid out) is drawn once it has one.
            // Its first frame drawn so is when it's opened (`Drawn`), as it's ready to show: when it's first summoned is
            // its person's (`Shown`).
            if window.panel && !window.visible {
                if std::mem::take(&mut window.dirty)
                    && let Ok(screen) = &mut window.screen
                    && window.renderer.is_some()
                {
                    window.unshown |= screen.paint(None, window.room, window.parts[0].native.scale(), &mut self.text, &self.atlas, &mut window.scene);
                    // Drawn once, so its surfaces have had a frame; after that it's painted alone, and the image it
                    // took as it went (`put_away`) is kept for the frame it's summoned with.
                    if !window.drawn {
                        let glass = screen.glass().to_vec();
                        window.arrange(&self.display, &self.context, &glass);
                        (window.drawn, window.live) = (true, true);
                        (window.on_event)(Event::Drawn);
                        (window.on_event)(Event::Live);
                    }
                }
                continue;
            }
            // Hidden (`hide`), it's drawn again as it shows, where a frame would make again the images it let go of.
            if window.hidden {
                continue;
            }
            let (width, height) = (device.width.0 as u32, device.height.0 as u32);
            if let Some(at) = window.presented
                && window.summoned.is_none()
                && (ticking || since(at) < refresh)
            {
                held |= window.dirty;
                continue;
            }
            if !window.dirty || window.renderer.is_none() || window.working() {
                if window.timing.idle()
                    && let Some(frames) = window.timing.end(self.display.refresh())
                {
                    (window.on_event)(Event::Frames(frames));
                }
                continue;
            }
            window.dirty = false;
            let began = web_time::Instant::now();
            let (room, scale) = (window.room(), window.parts[0].native.scale());
            let mut changed = true;
            {
                match &mut window.screen {
                    Ok(screen) => {
                        // In its first second, its page is laid out in the room its window has, never in another it
                        // was told of first, which it's drawn at for a frame as it's told the real one.
                        #[cfg(feature = "fake")]
                        if since(window.opened) < std::time::Duration::from_secs(1)
                            && let Some(told) = screen.told_room()
                            && (told.width != room.width.0 || told.height != room.height.0)
                        {
                            self.display.wrong(format!("{} was laid out {}×{} in its first second, in a window {}×{}", window.title, told.width, told.height, room.width.0, room.height.0));
                        }
                        // Under what covers it, painted only where some of it shows: a computer's title row, above its
                        // canvas; under its snapshot, or a canvas over all of it, it's left until it's revealed, as
                        // it was before it showed, so what it asks for as it's laid out is for its real layout.
                        if !matches!(window.cover, Cover::Over(_)) || (window.picture.is_none() && screen.title.is_some()) {
                            changed = screen.paint(time, room, scale, &mut self.text, &self.atlas, &mut window.scene);
                        }
                        window.moving = screen.take_again();
                        let calls = screen.text_input(&mut window.text_input);
                        if !calls.is_empty() {
                            self.display.text_input(&window.parts[0].native, &calls);
                        }
                        // The system's menu for what a finger selected, and its magnifier, where they are now.
                        let menu = screen.text_menu_shown();
                        if menu != window.text_menu {
                            window.text_menu = menu;
                            self.display.text_menu(&window.parts[0].native, menu);
                        }
                        let magnifier = screen.magnifier();
                        if magnifier != window.magnifier {
                            window.magnifier = magnifier;
                            self.display.magnifier(&window.parts[0].native, magnifier);
                        }
                        self.display.set_title_row(&window.parts[0].native, screen.title_row());
                    }
                    // One whose page isn't there yet (`opening`) shows its loading state, its spinner turning, never an
                    // empty window: a phone's first start showed black for 5 s and more as its page was read (21 Nov).
                    Err(_) if window.pending => {
                        crate::view::paint_loading(since(window.opened), QUIET, FADES_IN, appearance(), room, scale, &mut self.text, &self.atlas, &mut window.scene);
                        window.moving = true;
                    }
                    Err(why) => crate::view::paint_message(why, appearance(), room, scale, &mut self.text, &self.atlas, &mut window.scene),
                }
                // What covers it as it opens, made as it's first drawn: its snapshot, or its canvas, which a window whose
                // page isn't there yet (`opening`) has drawn bare.
                if let Cover::Over(cover @ None) = &mut window.cover {
                    let mut made = crate::render::scene::Scene::default();
                    match (&window.picture, &window.screen) {
                        (Some(picture), _) => {
                            if !std::mem::replace(&mut window.pictured, true)
                                && let Some(renderer) = &mut window.renderer
                            {
                                renderer.picture(snapshot::RASTER, width, height, picture);
                            }
                            made.picture(snapshot::RASTER, whole(device), 1.0);
                        }
                        (None, Ok(screen)) => screen.paint_canvas(room, scale, 1.0, &mut made),
                        (None, Err(_)) => made.splice(&window.scene, 1.0, [0.0, 0.0], 1.0, whole(device)),
                    }
                    made.finish();
                    *cover = Some(made);
                }
            }
            // What it shows is its page with what covers it over it, as opaque as it's faded to, drawn in its page's
            // scene's place for this frame; and once more as it's gone.
            changed |= !matches!(window.cover, Cover::Gone) || std::mem::take(&mut window.uncovered);
            let page = window.cover.scene().map(|cover| covered(&window.scene, Some(cover), window.cover.opacity(), device)).map(|shown| std::mem::replace(&mut window.scene, shown));
            // A character no font had draws as a font's missing glyph: a gap where a symbol or an emoji goes.
            for character in self.text.undrawn() {
                let what = format!("{} drew {character:?} (U+{:04X}) as a missing glyph: no font has it", window.title, u32::from(character));
                #[cfg(feature = "fake")]
                self.display.wrong(what);
                #[cfg(not(feature = "fake"))]
                log::warn!("{what}");
            }
            let painted = web_time::Instant::now();
            let glass = window.screen.as_ref().map(|screen| screen.glass().to_vec()).unwrap_or_default();
            // Placed for the room it's drawn in before that frame shows, so it shows where it stays.
            if window.drawn && std::mem::take(&mut window.placing) {
                self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room);
            }
            // Nothing it shows changed (its page asked for a frame, or something moved that ended where it was): the
            // frame on the screen is it, so none is drawn or presented, as a compositor draws none without damage.
            // On the Surface, each was half a ms of this thread and 0.7 to 2 of its GPU, for every part of a panel.
            if !changed && !window.unshown && window.drawn && window.summoned.is_none() && window.missed == 0 {
                continue;
            }
            let arranged = window.arrange(&self.display, &self.context, &glass);
            if let Some(page) = page {
                window.scene = page;
            }
            if !arranged {
                window.missed(&self.context, &self.atlas);
                continue;
            }
            window.unshown = false;
            window.missed = 0;
            window.presented = Some(now());
            window.frames += 1;
            #[cfg(feature = "fake")]
            if let Ok(screen) = &mut window.screen {
                screen.presented();
            }
            // Drawn where the display's link doesn't tick, as input came, it's the frame the display shows next, which the
            // loop's own clock counts its next from; drawn on that clock's frame, it counts from that.
            if !ticking && shown.is_none() {
                self.framed = now();
            }
            // Its opening out of its icon starts from the frame it first shows something in.
            if window.frames == 1
                && let Some(opening) = window.pull.as_mut().filter(|pull| pull.opens())
            {
                opening.shown();
            }
            if self.check.is_some() {
                self.record.present();
            }
            let moving = window.moving || window.screen.as_ref().is_ok_and(Screen::framing);
            if window.screen.as_ref().is_ok_and(Screen::held) {
                window.timing.input("drag");
            }
            let refresh = self.display.refresh();
            let renderer = window.renderer.as_ref();
            let (submit, present) = renderer.map(Renderer::submitted).unwrap_or_default();
            let (acquire, gpu, passes) = (renderer.map(Renderer::acquired).unwrap_or_default(), renderer.and_then(Renderer::gpu), renderer.map_or(0, Renderer::passes));
            let frame = frame_timing::Frame { at: now(), paint: painted - began, draw: painted.elapsed(), acquire, submit, present, gpu: gpu.map(|(all, _)| all), blur: gpu.map(|(_, blur)| blur), passes, applied: Default::default(), turned: Default::default() };
            let (interval, frames) = window.timing.drawn(frame, moving, refresh);
            if let Some(interval) = interval {
                self.deadlines.frame(frame_timing::Timing::long(interval, refresh));
            }
            if let Some(frames) = frames {
                (window.on_event)(Event::Frames(frames));
            }
            // Summoned before it had drawn, it's on the screen now it has, arranged again so its parts show with it.
            if std::mem::take(&mut window.placing) {
                self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room);
                window.arrange(&self.display, &self.context, &glass);
            }
            if !window.drawn {
                window.drawn = true;
                window.reloading = false;
                (window.on_event)(Event::Drawn);
            }
            if !window.pending && !matches!(window.cover, Cover::Over(_)) && !std::mem::replace(&mut window.live, true) {
                (window.on_event)(Event::Live);
            }
            if let Some(asked) = window.summoned.take() {
                let ms = |time: std::time::Duration| time.as_secs_f64() * 1000.0;
                // Where its time went: to the turn it's drawn on, on that turn before it (input, timers, other windows
                // drawn first), painting it and drawing it.
                let (before, on) = (turned.saturating_duration_since(asked), began.saturating_duration_since(turned));
                eprintln!("the launcher's first frame as it was summoned: {:.1} ms before its turn, {:.1} ms on it before it was painted, {:.1} ms to paint and {:.1} ms to draw", ms(before), ms(on), ms(painted - began), ms(painted.elapsed()));
                (window.on_event)(Event::Shown(asked.elapsed()));
            }
        }
        // What waited for a page that's caught up now is done.
        let waiting: Vec<WindowId> = self.windows.iter().filter(|(_, window)| !window.waiting.is_empty()).map(|(id, _)| *id).collect();
        for id in waiting {
            self.catch_up(id);
        }
        self.pressed();
        // The display's frames are wanted while something moves, and while a change waits for its next tick; a link
        // just started, or stopped, hasn't ticked.
        let waiting = held || (ticking && self.windows.values().any(|window| window.on_screen() && window.dirty));
        let wanted = self.framing() || waiting;
        // With no frame wanted, what moved is still: its frames are said, so the next one, drawn whenever something
        // comes, isn't counted as a refresh the display missed, which pressed the device for 5 s.
        if !wanted {
            for window in self.windows.values_mut() {
                if let Some(frames) = window.timing.end(self.display.refresh()) {
                    (window.on_event)(Event::Frames(frames));
                }
            }
        }
        self.ticked = self.ticked.filter(|_| wanted && self.wanted);
        self.wanted = wanted;
        self.display.request_frame(wanted);
        self.display.flush();
        if let Some(recording) = self.recording.take_if(|recording| recording.until <= now()) {
            (recording.reply)(crate::inspect::answer(&recording.id, Ok(serde_json::json!({"events": recording.events}))));
        }
        if let Some(check) = self.check.take_if(|check| check.over()) {
            let said = check.said(&self.record, self.display.paced());
            (check.reply)(crate::inspect::answer(&check.id, Ok(said)));
        }
    }

    /// How pressed the device is, the worse of what its system and its frames say, given to each window's screen as
    /// it changes, which draws as it says (`Screen::pressure`).
    pub(super) fn pressed(&mut self) {
        if since(self.system.0) >= std::time::Duration::from_secs(1) {
            self.system = (now(), self.display.pressure());
        }
        let pressure = self.system.1.max(self.deadlines.pressure());
        if pressure == self.pressure {
            return;
        }
        log::info!("pressure is {} now, where it was {}", pressure.name(), self.pressure.name());
        self.pressure = pressure;
        for window in self.windows.values_mut() {
            if let Ok(screen) = &mut window.screen {
                screen.pressure = pressure;
                window.dirty = true;
            }
        }
    }
}
