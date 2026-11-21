use super::*;

impl Windows {
    #[cfg(not(any(target_arch = "wasm32", feature = "fake")))]
    pub(super) fn new(display: Display) -> anyhow::Result<Windows> {
        Self::made(display, Context::new())
    }

    /// Its windows, drawn with `context`, the GPU, where there is one.
    pub(super) fn made(display: Display, context: anyhow::Result<Context>) -> anyhow::Result<Windows> {
        let made = (|| {
            let context = context?;
            #[cfg(not(target_arch = "wasm32"))]
            context.prepare();
            let atlas = Arc::new(Atlas::new(&context));
            // The system's fonts where ours lack a character: a fake platform has only those the harness's host is
            // given for it (`fake::fallback_fonts`), so it draws the same everywhere.
            #[allow(unused_mut)]
            let mut fonts = crate::fonts()?;
            #[cfg(feature = "fake")]
            fonts.extend(fake::fallback_fonts()?);
            let text = TextSystem::new(crate::FONT, fonts, !cfg!(feature = "fake"));
            Ok(Windows { display, context, atlas, text, windows: HashMap::new(), next: 1, framed: now(), wanted: false, ticked: false, check: None, record: Default::default(), recording: None, deadlines: Default::default(), pressure: Pressure::Nominal, system: (now(), Pressure::Nominal), background: false })
        })();
        if let Err(error) = &made {
            *UNSTARTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(format!("{error:#}"));
        }
        made
    }

    /// Opens a window showing `screen`, which tells `on_event` what happens to it. It's shown once its app's JS
    /// has started and heard back from what it first asked, or has waited long enough (`SHOWN_WITHIN`), and drawn
    /// as that posts what it changed.
    /// Opens a window of `screen`, or one that says only why its page isn't there, as one whose page didn't start does.
    pub fn open(&mut self, options: WindowOptions, screen: Result<Screen, String>, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        self.create(options, Some(screen), on_event)
    }

    /// Opens a window whose page is still starting, as a browser opens a tab before its page has loaded: its system's
    /// window and renderer are made while its page starts elsewhere, and it shows its snapshot, where it has one, until
    /// its page is given it (`attach`) and has started and heard back, as `open`'s does.
    pub fn opening(&mut self, options: WindowOptions, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        self.create(options, None, on_event)
    }

    /// Window `id`, opened while its page started (`opening`), shows its page now, `screen`. → whether it was such a
    /// window and is still open.
    pub fn attach(&mut self, id: WindowId, mut screen: Screen) -> bool {
        let Some(window) = self.windows.get_mut(&id).filter(|window| window.pending) else { return false };
        fit(&mut screen, &window.parts[0].native, id, window.room(), window.root, window.title_row, self.pressure);
        (window.screen, window.pending, window.dirty) = (Ok(screen), false, true);
        true
    }

    pub(super) fn create(&mut self, options: WindowOptions, screen: Option<Result<Screen, String>>, on_event: impl FnMut(Event) + 'static) -> anyhow::Result<WindowId> {
        let id = self.next;
        self.next += 1;
        let asked = web_time::Instant::now();
        let native = self.display.open(id, &options)?;
        let windowed = asked.elapsed();
        if let Some(owner) = options.owner.and_then(|owner| self.windows.get(&owner)) {
            self.display.attach(&native, &owner.parts[0].native);
        }
        // An app's window on Linux is unframed, its title row a GNOME header bar with the desktop's window buttons, and
        // on Windows its title row has Windows' caption buttons, drawn by the window: so from the start, before its page
        // is there, so the system's frame never shows first.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if options.title_row {
            self.display.unframe(&native);
        }
        let pending = screen.is_none();
        let mut screen = screen.unwrap_or_else(|| Err(String::new()));
        if let Ok(screen) = &mut screen {
            // Its page is laid out in the room its window has from its first frame, which only the window knows: a
            // phone's is its screen whatever it was asked to be, and a computer's app opens where it was left.
            let room = if options.panel.is_some() { options.size } else { native.size() };
            fit(screen, &native, id, room, options.root, options.title_row, self.pressure);
        }
        let rendering = web_time::Instant::now();
        let whole = crate::geometry::Bounds { origin: crate::geometry::point(0.0, 0.0), size: Size { width: options.size.width.0, height: options.size.height.0 } };
        let parts = vec![Part { native, bounds: whole, radius: options.panel.unwrap_or(0.0) }];
        let renderer = self.renderer(&parts)?;
        let native = &parts[0].native;
        // Where a window's opening went, which a page's own (`Opening` in clients/app/src/native.rs) has as one step.
        eprintln!("{} opened in {} ms: the system's window {} ms, its renderer {} ms", options.title, asked.elapsed().as_millis(), windowed.as_millis(), rendering.elapsed().as_millis());
        let opened = now();
        let device = native.device_size();
        let picture = options.snapshot.as_deref().filter(|_| pending || screen.is_ok()).and_then(|stem| snapshot::read(stem, (device.width.0 as u32, device.height.0 as u32)));
        let screen_failed = screen.is_err() && !pending;
        let panel = options.panel.is_some();
        self.windows.insert(
            id,
            Window {
                parts,
                renderer,
                room: options.size,
                placing: false,
                opened,
                shown: false,
                panel,
                visible: false,
                hidden: false,
                summoned: None,
                left: None,
                chrome: None,
                screen,
                pending,
                title_row: options.title_row,
                title: options.title,
                scene: Default::default(),
                dirty: true,
                moving: false,
                drawn: false,
                live: false,
                looked: false,
                looking: opened,
                reloading: false,
                missed: 0,
                unshown: false,
                text_input: Default::default(),
                text_menu: None,
                magnifier: None,
                on_event: Box::new(on_event),
                tag: options.tag,
                places: options.places,
                root: options.root,
                icon: options.icon,
                pull: None,
                slide: None,
                snapshot: options.snapshot,
                picture,
                pictured: false,
                // A panel is summoned once its page has drawn; one that says why it didn't open has nothing to cover.
                cover: if panel || screen_failed { Cover::Gone } else { Cover::Over(None) },
                loading: false,
                uncovered: false,
                presented: false,
                frames: 0,
                timing: Default::default(),
                waiting: VecDeque::new(),
            },
        );
        Ok(id)
    }

    /// Window `id`'s app's JS posted what it changed: it's applied, and drawn on the loop's turn.
    pub(super) fn posted(&mut self, id: WindowId) {
        let started = web_time::Instant::now();
        if let Some(window) = self.windows.get_mut(&id)
            && let Ok(screen) = &mut window.screen
        {
            if screen.pump() {
                window.dirty = true;
            }
            screen.unused();
            window.timing.applied(started.elapsed());
        }
        self.catch_up(id);
    }

    /// Whether what the person does to window `id` now waits, behind what already does, or for its page: what's
    /// `discrete` (a press, a key) waits until the page has run what they did before (`Screen::caught_up`), as a
    /// browser runs each input event once its page has run the one before, so a Tab or a press acts on what the page
    /// made of the key or press before it, never on a frame from before it. What's continuous (the pointer or a finger
    /// moving) waits only behind what does, so it comes after it.
    pub(super) fn waits(&mut self, id: WindowId, discrete: bool) -> bool {
        let Some(window) = self.windows.get(&id) else { return false };
        let Ok(screen) = &window.screen else { return false };
        if !window.waiting.is_empty() || (discrete && !screen.caught_up()) {
            return true;
        }
        if discrete {
            self.lay_out(id);
        }
        false
    }

    /// Window `id`'s page laid out as it is now, where what it changed isn't yet, for what the person does next to hit,
    /// as a browser lays its page out before it hit-tests a click: it's painted now, and its next frame shows that.
    pub(super) fn lay_out(&mut self, id: WindowId) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let Ok(screen) = &mut window.screen else { return };
        if !screen.laid_out() {
            screen.paint(room, scale, &mut self.text, &self.atlas, &mut window.scene);
            window.dirty = true;
        }
    }

    /// What waited for window `id`'s page is done, in order, while its page has caught up.
    pub(super) fn catch_up(&mut self, id: WindowId) {
        loop {
            #[cfg(feature = "fake")]
            self.wait();
            let Some(window) = self.windows.get_mut(&id) else { return };
            let Ok(screen) = &window.screen else { return window.waiting.clear() };
            let discrete = match window.waiting.front() {
                None => return,
                Some(Waiting::Input(_, input)) => input.waits() == Some(true),
                Some(Waiting::Touch { finger, phase, .. }) => phase.discrete(screen, *finger),
            };
            if discrete && !screen.caught_up() {
                return;
            }
            if discrete {
                self.lay_out(id);
            }
            let Some(window) = self.windows.get_mut(&id) else { return };
            match window.waiting.pop_front().expect("there's one") {
                Waiting::Input(part, input) => self.act(id, part, input),
                Waiting::Touch { finger, at, phase, happened, fingers } => {
                    let ago = happened.map(|happened| since(happened).as_secs_f64() * 1000.0);
                    self.touched(id, finger, at, phase, ago, fingers);
                }
            }
        }
    }

    /// The app goes to the background, or comes back, as an iPhone's does. While it's there, its system refuses its
    /// GPU's work: Metal fails each command buffer it's given, and what the queue uploads with one, as the glyphs and
    /// pictures a frame added to the atlas, which the atlas holds as there from then on (letters missing from an
    /// iPhone's labels, each where a frame drawn in the background first drew it). So its windows' renderers go, as
    /// a lost surface's do, and what changes meanwhile is drawn once it's back.
    pub fn background(&mut self, background: bool) {
        if self.background == background {
            return;
        }
        self.background = background;
        // Its pages aren't seen meanwhile, as a browser's page in a tab behind isn't, so what each does only while it's
        // seen waits (a toast's time, a refresh).
        for window in self.windows.values_mut() {
            let put_away = window.panel && !window.visible;
            if let Ok(screen) = &mut window.screen {
                screen.seen(!background && !put_away);
            }
        }
        let parts: Vec<(WindowId, usize)> = self.windows.iter().flat_map(|(id, window)| (0..window.parts.len()).map(|part| (*id, part))).collect();
        for (id, part) in parts {
            self.input(id, part, if background { Input::SurfaceLost } else { Input::Surface });
        }
    }

    /// What draws a window's `parts`, where its first has a surface to draw on yet, the app isn't in the background,
    /// and the pipelines it draws with are compiled: until they are, it has none, so the windows' thread never waits
    /// for them, and is given one as they are (`compiled`).
    pub(super) fn renderer(&self, parts: &[Part]) -> anyhow::Result<Option<Renderer>> {
        if self.background || !parts[0].native.has_surface() || !drawable(&self.context, &parts[0].native) {
            return Ok(None);
        }
        draws_on(&self.context, self.atlas.clone(), parts).map(Some)
    }

    /// The pipelines a kind of window draws with are compiled (`Context::prepare`): each window waiting for them has
    /// its renderer now, and is drawn.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn compiled(&mut self) {
        let waiting: Vec<WindowId> = self.windows.iter().filter(|(_, window)| window.renderer.is_none()).map(|(id, _)| *id).collect();
        for id in waiting {
            let made = match self.renderer(&self.windows[&id].parts) {
                Ok(made) => made,
                Err(error) => {
                    log::error!("native window {id} can't draw: {error:#}");
                    continue;
                }
            };
            if let Some(window) = self.windows.get_mut(&id) {
                window.renderer = made;
                window.dirty = true;
            }
        }
    }
}
