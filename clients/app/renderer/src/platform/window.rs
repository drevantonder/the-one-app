use super::*;

/// Fits `screen` to window `id`, its `native` window: the `room` its page is laid out in, what it's told of its system
/// and device and of its window's title row (`WindowOptions::title_row`), and the windows' thread woken as its page
/// posts.
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), allow(unused_variables))]
pub(super) fn fit(screen: &mut Screen, native: &NativeWindow, id: WindowId, room: Size<Pixels>, root: bool, title_row: bool, pressure: Pressure) {
    screen.room = Size { width: room.width.0, height: room.height.0 };
    // A phone's app window has the rail's home line; its Home Screen doesn't.
    screen.rail = cfg!(any(target_os = "android", target_os = "ios")) && !root;
    screen.appearance = appearance();
    screen.reduced_motion = reduced_motion();
    screen.pressure = pressure;
    if let Some(clipboard) = clipboard() {
        screen.clipboard = clipboard;
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        screen.device = native.device(id);
        screen.text_size = Some(text_size());
    }
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        let _ = native;
        screen.device = Box::new(Computer);
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if title_row {
        #[cfg(any(target_os = "linux", feature = "fake"))]
        {
            screen.window_buttons = button_layout();
        }
        #[cfg(all(target_os = "windows", not(feature = "fake")))]
        {
            screen.window_buttons = (Vec::new(), vec![crate::WindowButton::Minimize, crate::WindowButton::Maximize, crate::WindowButton::Close]);
        }
    }
    if let Some(native) = Native::started() {
        let native = std::sync::Mutex::new(native);
        screen.on_post(Arc::new(move || native.lock().unwrap_or_else(std::sync::PoisonError::into_inner).run(move |windows| windows.posted(id))));
    }
}

impl Window {
    /// Whether its GPU hasn't finished the frame it drew last (`Renderer::working`): its next waits.
    pub(super) fn working(&self) -> bool {
        self.renderer.as_ref().is_some_and(crate::render::renderer::Renderer::working)
    }

    /// Whether it's on screen: shown, not hidden, and for a panel, summoned.
    pub(super) fn on_screen(&self) -> bool {
        self.shown && !self.hidden && (!self.panel || self.visible)
    }

    /// How big its page is laid out: a panel's as its page asked, the rest as big as their window.
    pub(super) fn room(&self) -> Size<Pixels> {
        if self.panel { self.room } else { self.parts[0].native.size() }
    }

    /// Draws its scene into its parts, a panel's first made to follow its page: one part for each box its page asked
    /// the system's material for (`glass`), or one for all of it until it has, each window's frame and what's drawn in
    /// it put on screen together (`Display::arrange`). A display with no windows of a panel's own keeps one, as big
    /// as its page.
    /// It had nothing to draw on this frame, as when its surface is lost (the computer slept, its display changed,
    /// its GPU was reset): it's drawn again on the next, and after a few such, on a surface made again from its native
    /// window's, as when the system gives it a new one (`Input::Surface`).
    pub(super) fn missed(&mut self, context: &Context, atlas: &Arc<Atlas>) {
        self.dirty = true;
        self.moving = true;
        self.missed += 1;
        if self.missed < 3 {
            return;
        }
        self.missed = 0;
        if let Some(mut renderer) = self.renderer.take() {
            renderer.destroy();
        }
        match draws_on(context, atlas.clone(), &self.parts) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(error) => log::error!("a native window can't draw: {error:#}"),
        }
    }

    /// → whether each of its parts was drawn, which isn't so where one had nothing to draw on (`missed`).
    pub(super) fn arrange(&mut self, display: &Display, context: &Context, glass: &[Glass]) -> bool {
        let whole = crate::geometry::Bounds { origin: crate::geometry::point(0.0, 0.0), size: Size { width: self.room().width.0, height: self.room().height.0 } };
        let corner = self.parts[0].radius;
        let mut boxes: Vec<(crate::geometry::Bounds<f32>, f32)> = match self.panel {
            true if !glass.is_empty() => glass.iter().map(|glass| (whole_points(glass.bounds), glass.radius)).collect(),
            _ => vec![(whole, if self.panel { corner } else { 0.0 })],
        };
        let Some(renderer) = self.renderer.as_mut() else { return false };
        while self.panel && self.parts.len() < boxes.len() {
            let made = display.open_part(&self.parts[0].native, self.parts.len()).and_then(|native| {
                also_on(renderer, context, &native)?;
                Ok(Part { native, bounds: whole, radius: 0.0 })
            });
            match made {
                Ok(part) => self.parts.push(part),
                Err(error) => {
                    log::debug!("a panel's parts are one window here: {error:#}");
                    boxes = vec![(whole, corner)];
                }
            }
        }
        while self.parts.len() > boxes.len() {
            let part = self.parts.pop().expect("there's one");
            renderer.remove();
            display.close(&part.native);
        }
        for (part, (bounds, radius)) in self.parts.iter_mut().zip(&boxes) {
            part.bounds = *bounds;
            part.radius = *radius;
        }
        let scale = self.parts[0].native.scale();
        let placed: Vec<(Handle, crate::geometry::Bounds<f32>, f32)> = self.parts.iter().map(|part| (part.native.handle(), part.bounds, part.radius)).collect();
        let (parts, scene, panel) = (&self.parts, &self.scene, self.panel);
        let mut drawn = true;
        let mut draw = || {
            for (at, part) in parts.iter().enumerate() {
                // A panel's part is as big as it was just made (`Display::arrange`), which its window says later.
                let size = |length: f32| DevicePixels((length * scale).round() as i32);
                renderer.update_drawable_size(at, if panel { Size { width: size(part.bounds.size.width), height: size(part.bounds.size.height) } } else { part.native.device_size() });
            }
            let origins: Vec<[f32; 2]> = parts.iter().map(|part| [(part.bounds.origin.x * scale).round(), (part.bounds.origin.y * scale).round()]).collect();
            drawn = renderer.draw(scene, &origins);
        };
        if self.panel {
            display.arrange(&placed, &mut draw);
        } else {
            draw();
        }
        drawn
    }
}
