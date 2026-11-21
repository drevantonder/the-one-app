use super::*;

impl Windows {
    /// Brings window `id` forward.
    pub fn activate(&mut self, id: WindowId) {
        if let Some(window) = self.windows.get(&id) {
            self.display.activate(&window.parts[0].native);
        }
    }

    /// Shows panel `id`, centred near the top of the screen the pointer is on, with the keyboard, its frame drawn as
    /// it shows; the window that had the keyboard is kept, to give it back (`put_away`). `asked` is when it was asked
    /// to, which its `Shown` says how long after.
    /// `key` is the key its shortcut ends with, where its shortcut summoned it (`Screen::summoned`).
    pub fn summon(&mut self, id: WindowId, asked: web_time::Instant, key: Option<&str>) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        // Showing, it has the keyboard already: a panel never brings the One App in front.
        if window.visible {
            return;
        }
        window.visible = true;
        window.dirty = true;
        window.summoned = Some(asked);
        if let Ok(screen) = &mut window.screen {
            screen.shown(true);
            screen.summoned(key);
        }
        let queued = asked.elapsed();
        // On the screen with what it last drew, where it has; else once its page has drawn its first frame (`turn`), so
        // a launcher summoned as the app starts never shows empty.
        match window.drawn {
            true => self.display.summon(&window.parts[0].native, &places(window.places.as_deref()), window.room),
            false => window.placing = true,
        }
        // Where the time to show it goes: waiting for this thread, then for the system's to place it, then drawing.
        let ms = |time: std::time::Duration| time.as_secs_f64() * 1000.0;
        eprintln!("the launcher's summon waited {:.1} ms for its thread and {:.1} ms to be placed", ms(queued), ms(asked.elapsed() - queued));
        // Drawn now, not on the loop's next turn, so its first frame is the one it shows with.
        self.turn();
    }

    /// Puts panel `id` away, and with `give_back`, gives the keyboard back to the window that had it before.
    pub fn put_away(&mut self, id: WindowId, give_back: bool) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        if !window.visible {
            return;
        }
        window.visible = false;
        window.summoned = None;
        window.placing = false;
        // Painted once more, put away, so what was on its way is where it was going (`Screen::shown`).
        window.dirty = true;
        // Its surfaces let go of their images, or on a Mac its next frame's taken while it's still on the screen, so
        // it's drawn at once as it's summoned (`Renderer::put_away`).
        for renderer in window.parts.iter_mut().filter_map(|part| part.renderer.as_mut()) {
            renderer.put_away();
        }
        if let Some(frames) = window.timing.end(self.display.refresh()) {
            (window.on_event)(Event::Frames(frames));
        }
        if let Ok(screen) = &mut window.screen {
            screen.shown(false);
        }
        self.display.put_away(&window.parts[0].native, give_back);
    }

    /// Hides window `id`, or shows it again, as a phone's root window is while the page over it shows an app it
    /// draws itself. Hidden, it isn't drawn, and its renderers let go of their images as a panel's put away do
    /// (`Renderer::put_away`): a phone's system does nothing with a window hidden (an iPhone's Home Screen kept its
    /// three images, 36 MB at 3x, under every app).
    pub fn hide(&mut self, id: WindowId, hidden: bool) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        window.hidden = hidden;
        if hidden {
            for renderer in window.parts.iter_mut().filter_map(|part| part.renderer.as_mut()) {
                renderer.put_away();
            }
            self.display.put_away(&window.parts[0].native, false);
        } else {
            // Drawn as it shows, though nothing it shows changed: its images went as it was hidden.
            (window.dirty, window.unshown) = (true, true);
            self.display.show(&window.parts[0].native);
        }
    }

    /// Whether panel `id` is showing, or was until something else took the keyboard just now: a click on Windows'
    /// notification-area icon, which toggles it, first gives the keyboard to the taskbar, so the click that would put
    /// it away would show it again.
    pub fn showing(&self, id: WindowId) -> bool {
        const CLICK: std::time::Duration = std::time::Duration::from_millis(300);
        self.windows.get(&id).is_some_and(|window| window.visible || window.left.is_some_and(|left| since(left) < CLICK))
    }

    /// Lays panel `id`'s page out `height` points tall, and `width` wide where it's said, as its page asks: its parts'
    /// windows follow its page (`Display::arrange`).
    pub fn resize(&mut self, id: WindowId, width: Option<f32>, height: f32) {
        if let Some(window) = self.windows.get_mut(&id) {
            let size = window.room;
            let width = width.unwrap_or(size.width.0);
            if (size.height.0 - height).abs() >= 0.5 || (size.width.0 - width).abs() >= 0.5 {
                window.room = Size { width: Pixels(width), height: Pixels(height) };
                window.dirty = true;
                window.placing = window.visible;
            }
        }
    }

    /// Closes window `id`.
    pub fn close(&mut self, id: WindowId) {
        self.keep_snapshot(id);
        if let Some(mut window) = self.windows.remove(&id) {
            self.let_go(window.parts);
            (window.on_event)(Event::Closed);
        }
    }

    /// Keeps window `id`'s frame as its snapshot (`WindowOptions::snapshot`), drawn afresh offscreen as it is now,
    /// where it shows what its page drew: not its snapshot, nor a page still starting.
    pub(crate) fn keep_snapshot(&mut self, id: WindowId) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let (room, scale) = (window.room(), window.parts[0].native.scale());
        let (Some(stem), Ok(screen)) = (&window.snapshot, &mut window.screen) else { return };
        if self.background || !window.drawn || !matches!(window.cover, Cover::Gone) || window.loading || screen.starting() {
            return;
        }
        let device = Size { width: DevicePixels((room.width.0 * scale).round() as i32), height: DevicePixels((room.height.0 * scale).round() as i32) };
        let mut renderer = Renderer::offscreen(&self.context, self.atlas.clone(), device);
        let mut scene = crate::render::scene::Scene::default();
        screen.paint_afresh(room, scale, &mut self.text, &self.atlas, &mut scene);
        renderer.draw(&scene, [0.0, 0.0]);
        if let Some((width, height, rgba)) = renderer.read() {
            snapshot::keep(stem, width, height, rgba);
        }
        renderer.destroy();
    }

    /// Lets go of a window's parts: their renderers, then their native windows, the children first.
    pub(super) fn let_go(&self, parts: Vec<Part>) {
        for mut part in parts.into_iter().rev() {
            if let Some(mut renderer) = part.renderer.take() {
                renderer.destroy();
            }
            self.display.close(&part.native);
        }
    }
}
