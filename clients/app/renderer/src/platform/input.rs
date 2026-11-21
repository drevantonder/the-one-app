use super::*;

impl Windows {
    /// What the display said happened to part `part` of window `id`, where it is in its page.
    pub(super) fn input(&mut self, id: WindowId, part: usize, input: Input) {
        if let Input::Touch(finger, at, phase) = input {
            self.touch(id, finger, at, phase, None, crate::view::touch::Fingers::PHONE);
            return;
        }
        if self.check.is_some() {
            self.record.input(input.kind(), None);
        }
        if let Some((recording, event)) = self.recording.as_mut().zip(input.traced()) {
            recording.input(id, event, None);
        }
        // The wheel, which waits for no page, waits for a frame that's overdue (`overdue`), as what does.
        let waits = match input.waits() {
            Some(discrete) => self.waits(id, discrete),
            None => matches!(input, Input::Wheel(..)) && self.overdue(),
        };
        if waits {
            if let Some(window) = self.windows.get_mut(&id) {
                window.waiting.push_back(Waiting::Input(part, input));
            }
            return;
        }
        self.act(id, part, input);
    }

    /// What the display said happened to part `part` of window `id`, its page caught up where it waits (`input`).
    pub(super) fn act(&mut self, id: WindowId, part: usize, input: Input) {
        // Ctrl-W closes the window, as a Linux or Windows app's does, and Ctrl-Q on Linux quits the app (a Mac's are
        // ⌘W and ⌘Q, its menu bar's).
        let (closes, quits) = match &input {
            Input::Close => (true, false),
            Input::Key(key) if !cfg!(target_os = "macos") && key.ctrl && !key.alt && !key.shift && !key.meta => (key.key == "w", cfg!(target_os = "linux") && key.key == "q"),
            _ => (false, false),
        };
        if quits {
            if let Some(window) = self.windows.get_mut(&id) {
                (window.on_event)(Event::Quit);
            }
            return;
        }
        if closes {
            // A panel's put away, as Raycast's is by ⌘W or Alt-F4: it's summoned again, never closed.
            if !self.windows.get(&id).is_some_and(|window| window.panel) {
                return self.close(id);
            }
            self.put_away(id, true);
            if let Some(window) = self.windows.get_mut(&id) {
                (window.on_event)(Event::PutAway);
            }
            return;
        }
        let background = self.background;
        let Some(window) = self.windows.get_mut(&id) else { return };
        let Some(at) = window.parts.get(part).map(|part| part.bounds.origin) else { return };
        let input = input.moved_by(at);
        window.dirty = true;
        match input {
            Input::Resized(size) => return window.parts[part].native.resized(size),
            // A part's surface came, or a new one did: drawn on from now, by a renderer made again for all of its parts,
            // the one for those it had let go first.
            Input::Surface => {
                if let Some(mut renderer) = window.renderer.take() {
                    renderer.destroy();
                }
                if !background && window.parts.iter().all(|part| part.native.has_surface()) {
                    match draws_on(&self.context, self.atlas.clone(), &window.parts) {
                        Ok(renderer) => window.renderer = Some(renderer),
                        Err(error) => log::error!("native window {id} can't draw: {error:#}"),
                    }
                }
                return;
            }
            Input::SurfaceLost => {
                if let Some(mut renderer) = window.renderer.take() {
                    renderer.destroy();
                }
                return;
            }
            _ => {}
        }
        let Ok(screen) = &mut window.screen else { return };
        match input {
            Input::Resized(_) | Input::Surface | Input::SurfaceLost | Input::Touch(..) => {}
            Input::Expose => {}
            // The system went light or dark: so do its own parts of the window, where its Colour Mode says nothing.
            Input::FullScreen(full) => screen.full_screen = full,
            Input::Dropped(at, files) => screen.dropped(at, &files),
            Input::Appearance(appearance) => {
                screen.appearance = appearance;
                window.chrome = None;
            }
            Input::Moved(at) => {
                screen.pointer_moved(at);
                let cursor = screen.cursor(at);
                self.display.set_cursor(&window.parts[part].native, cursor);
            }
            Input::Predicted(at) => screen.predicted(at),
            Input::Left => screen.pointer_left(),
            Input::Down(at, button, shift) => {
                screen.pointer_down(at, button, shift);
                // The system's own menu for text, where it was clicked in this part.
                if let Some(mut menu) = screen.take_text_menu() {
                    let origin = window.parts[part].bounds.origin;
                    menu.rect.origin = crate::geometry::point(menu.rect.origin.x - origin.x, menu.rect.origin.y - origin.y);
                    self.display.text_menu(&window.parts[part].native, Some(menu));
                }
            }
            Input::Up(at, button) => {
                screen.pointer_up(at, button);
                // Its own window buttons do what they say (an unframed Linux window's).
                #[cfg(target_os = "linux")]
                match screen.take_window_button() {
                    Some(crate::WindowButton::Close) => return self.close(id),
                    Some(crate::WindowButton::Minimize) => self.display.minimize(&window.parts[0].native),
                    Some(crate::WindowButton::Maximize) => self.display.toggle_maximized(window.parts[0].native.x_window()),
                    None => {}
                }
            }
            Input::Wheel(at, delta, phase, notches) => {
                window.timing.input("scroll");
                screen.scroll(at, delta, phase, notches);
            }
            Input::Key(key) => drop(screen.key(&key)),
            Input::Ime(ime) => screen.ime(ime),
            Input::TextInputClient(said) => screen.text_input_client(&mut window.text_input, said),
            // A panel goes as something else takes the keyboard, as Spotlight does.
            Input::Focus(false) if window.panel && window.visible => {
                window.left = Some(now());
                screen.activate(false);
                self.put_away(id, false);
                if let Some(window) = self.windows.get_mut(&id) {
                    (window.on_event)(Event::PutAway);
                }
                // Its frame as it was put away is what it shows as it opens again (`WindowOptions::snapshot`).
                return self.keep_snapshot(id);
            }
            Input::Focus(active) => {
                screen.activate(active);
                if !active {
                    (window.on_event)(Event::Unfocused);
                    return self.keep_snapshot(id);
                }
            }
            // Dragged somewhere on a screen, a panel is summoned there on that screen from now on.
            Input::Placed(on, at) => {
                if let Some(file) = &window.places {
                    let mut kept = places(Some(file));
                    kept.insert(on, at);
                    let kept: HashMap<&String, [f32; 2]> = kept.iter().map(|(screen, at)| (screen, [at.x, at.y])).collect();
                    if let Err(error) = serde_json::to_vec(&kept).map_err(anyhow::Error::from).and_then(|bytes| Ok(std::fs::write(file, bytes)?)) {
                        log::warn!("where the panel was dragged to isn't kept: {error}");
                    }
                }
            }
            Input::Picked(node, value) => screen.picked(node, &value),
            Input::Close => {}
        }
        screen.unused();
        // What it changed reaches the text input plugin at once, before its next key, not with the frame.
        let calls = screen.text_input(&mut window.text_input);
        if !calls.is_empty() {
            self.display.text_input(&window.parts[0].native, &calls);
        }
    }
}

impl crate::view::touch::Phase {
    /// Whether a finger in this phase is discrete input (`Windows::waits`): it lands, or lifts to press.
    pub(super) fn discrete(self, screen: &Screen, finger: u64) -> bool {
        use crate::view::touch::Phase;
        match self {
            Phase::Down => true,
            Phase::Moved => false,
            Phase::Up | Phase::Cancelled => screen.lift_presses(finger),
        }
    }
}

impl Input {
    /// Whether it's the page's, and waits for it (`Windows::waits`): a press, a key, what's typed or picked, which do,
    /// or the pointer moving, which only follows what does. The wheel scrolls at once, as a browser's compositor
    /// scrolls, and what's the window's or the system's is the window's.
    pub(super) fn waits(&self) -> Option<bool> {
        match self {
            Input::Down(..) | Input::Up(..) | Input::Key(_) | Input::Ime(_) | Input::TextInputClient(_) | Input::Picked(..) | Input::Dropped(..) => Some(true),
            Input::Moved(_) | Input::Predicted(_) | Input::Left => Some(false),
            _ => None,
        }
    }

    /// What it is, as the inspector's `host` check names it.
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Input::Moved(_) => "moved",
            Input::Predicted(_) => "predicted",
            Input::Down(..) => "down",
            Input::Up(..) => "up",
            Input::Wheel(..) => "wheel",
            Input::Key(_) => "key",
            Input::Left => "left",
            _ => "other",
        }
    }

    /// It as a trace's event (`{kind, x, y, …}`, as the headless harness's `trace` plays it), where it's one a person
    /// gives: the pointer, a button, the wheel or a key.
    pub(super) fn traced(&self) -> Option<serde_json::Value> {
        use serde_json::json;
        Some(match self {
            Input::Moved(at) => json!({"kind": "move", "x": at.x, "y": at.y}),
            Input::Down(at, button, shift) => json!({"kind": "down", "x": at.x, "y": at.y, "button": button, "shift": shift}),
            Input::Up(at, button) => json!({"kind": "up", "x": at.x, "y": at.y, "button": button}),
            Input::Wheel(at, by, phase, notches) => {
                let mut wheel = json!({"kind": "wheel", "x": at.x, "y": at.y, "dx": by.x, "dy": by.y, "phase": phase.as_ref().and_then(|phase| phase.fingers), "momentum": phase.as_ref().is_some_and(|phase| phase.momentum)});
                if let Some(notches) = notches {
                    (wheel["notchesX"], wheel["notchesY"]) = (json!(notches.x), json!(notches.y));
                }
                wheel
            }
            Input::Key(key) => json!({"kind": "key", "key": key.key, "shift": key.shift, "alt": key.alt, "ctrl": key.ctrl, "meta": key.meta}),
            _ => return None,
        })
    }

    /// The same, at `by` further along and down: a part's point, in its page.
    pub(super) fn moved_by(self, by: crate::geometry::Point<f32>) -> Input {
        let moved = |at: crate::geometry::Point<f32>| crate::geometry::point(at.x + by.x, at.y + by.y);
        match self {
            Input::Moved(at) => Input::Moved(moved(at)),
            Input::Predicted(at) => Input::Predicted(moved(at)),
            Input::Down(at, button, shift) => Input::Down(moved(at), button, shift),
            Input::Up(at, button) => Input::Up(moved(at), button),
            Input::Dropped(at, files) => Input::Dropped(moved(at), files),
            Input::Wheel(at, delta, phase, notches) => Input::Wheel(moved(at), delta, phase, notches),
            input => input,
        }
    }
}

/// What the display says happened to a window.
#[cfg_attr(any(target_os = "android", target_os = "ios", target_arch = "wasm32", feature = "fake"), allow(dead_code))]
pub(crate) enum Input {
    Resized(Size<crate::geometry::DevicePixels>),
    /// It has a surface to draw on, or a new one (Android's view's), or the app's back from the background.
    Surface,
    /// Its surface is going, as its view is hidden or gone, or the app to the background (`Windows::background`):
    /// nothing draws on it.
    SurfaceLost,
    Expose,
    /// The system went light or dark.
    Appearance(crate::Appearance),
    /// Files were dropped on it from the system, at a point in it.
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows", target_os = "linux")), allow(dead_code))]
    Dropped(crate::geometry::Point<f32>, Vec<std::path::PathBuf>),
    /// It went full screen, or came out of it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FullScreen(bool),
    Close,
    Moved(crate::geometry::Point<f32>),
    /// A finger, as a phone's screen says it, in the window's points (`Windows::touch`).
    #[cfg_attr(not(feature = "fake"), allow(dead_code))]
    Touch(u64, crate::geometry::Point<f32>, crate::view::touch::Phase),
    /// Where the device predicts the pointer or finger down is going, a little after its last move
    /// (`Screen::predicted`).
    Predicted(crate::geometry::Point<f32>),
    Left,
    /// A button went down, with Shift held or not, or up: 0 the main one, 1 the middle, 2 the secondary, as the DOM
    /// numbers them.
    Down(crate::geometry::Point<f32>, u8, bool),
    Up(crate::geometry::Point<f32>, u8),
    /// Scrolled along and down by so many pixels (left and up negative), in the trackpad's phase where the system
    /// says it, and by so many notches along and down where a wheel turned by clicks, as a mouse's does: what tells
    /// it from a trackpad's or a smooth wheel's scroll, which has none.
    Wheel(crate::geometry::Point<f32>, crate::geometry::Point<f32>, Option<crate::view::Phase>, Option<crate::geometry::Point<f32>>),
    /// A key the page hears first, as Flutter's keyboard manager gives its framework each key.
    Key(crate::inspect::Key),
    /// What X11's input method composes or commits, which has no text input plugin.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Ime(crate::editing::Ime),
    /// What its text input plugin says.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    TextInputClient(crate::editing::TextInputClient),
    /// A panel was dragged to `at` on the screen named the first, in points from its top left (`Places`).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    Placed(String, crate::geometry::Point<f32>),
    /// The window got the keyboard, or lost it.
    Focus(bool),
    /// The device's picker for picker node `0` chose value `1` (`Device::pick`).
    #[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
    Picked(crate::tree::Id, String),
}
