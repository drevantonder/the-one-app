//! A page's videos natively (`crate::video`): each connected as its `ice` is set, given its `answer`, and its events
//! told its page; and what the person does on one, unless it's `watch`ed, sent down its data channel, as the web's
//! are (clients/web/renderer/video.js): the pointer as it moves, its buttons, a wheel's steps, and keys, to the video
//! last pressed.

use serde_json::{Value, json};

use super::Screen;
use crate::geometry::Point;
use crate::inspect::Key;
use crate::tree::{Id, Kind, Mutation};

/// Keys as the DOM names them, as X does, which is how a screen is told them (input.ts's `KEYS`).
const KEYS: &[(&str, &str)] = &[
    ("Enter", "Return"),
    ("Backspace", "BackSpace"),
    ("Tab", "Tab"),
    ("Escape", "Escape"),
    ("Delete", "Delete"),
    ("ArrowLeft", "Left"),
    ("ArrowRight", "Right"),
    ("ArrowUp", "Up"),
    ("ArrowDown", "Down"),
    ("Home", "Home"),
    ("End", "End"),
    ("PageUp", "Prior"),
    ("PageDown", "Next"),
];
/// How far a wheel turns for one of its steps, in pixels (primitives.tsx's).
const STEP: f32 = 60.0;
/// ⌘ is the key held for a shortcut, which a screen is told as Control, as Chrome's shortcuts on Linux take it.
const MAC: bool = cfg!(target_vendor = "apple");

impl Screen {
    /// What a mutation does to a video: a new `ice` is a new connection, an `answer` is the screen's, and one dropped
    /// is closed.
    pub(super) fn video_mutation(&mut self, mutation: &Mutation) {
        match mutation {
            Mutation::Set { id, name, value }
                if self.tree.nodes.get(id).is_some_and(|node| node.kind == Kind::Video) =>
            {
                let video = self.videos.entry(*id).or_default();
                match name.as_str() {
                    "ice" => video.connect(value, self.wake.clone()),
                    "answer" => {
                        if let Some(sdp) = value.as_str() {
                            video.answer(sdp);
                        }
                    }
                    "watch" => video.watch = value.as_bool().unwrap_or(false),
                    "autofocus" if value.as_bool() == Some(true) => self.video_keys = Some(*id),
                    _ => {}
                }
            }
            Mutation::Drop { id } => {
                self.videos.remove(id);
                if self.video_keys == Some(*id) {
                    self.video_keys = None;
                }
            }
            _ => {}
        }
    }

    /// Closes each video taken out of the page, as soon as it's out, not once its JS lets go of it (`drop`): one
    /// connected before it's put in the page stays until it's been in it.
    pub(super) fn sync_videos(&mut self) {
        let ids: Vec<Id> = self.videos.keys().copied().collect();
        for id in ids {
            let attached = self.attached(id);
            let Some(video) = self.videos.get_mut(&id) else { continue };
            if attached {
                video.shown = true;
            } else if video.shown {
                self.videos.remove(&id);
            }
        }
    }

    /// Tells each video's page what it said since it was last asked. → whether anything came: an event, or a frame,
    /// which is drawn in the next frame, as a picture decoded is.
    pub(super) fn video_pump(&mut self) -> bool {
        let mut told = Vec::new();
        let mut frames = false;
        for (id, video) in &mut self.videos {
            let (events, frame) = video.take();
            frames |= frame;
            told.extend(events.into_iter().map(|(name, told)| (*id, name, told)));
        }
        if frames {
            self.pictures.decoded += 1;
        }
        let came = frames || !told.is_empty();
        for (id, name, told) in told {
            self.tell(id, name, told);
        }
        came
    }

    /// The video connected and drawn under `at`, sending what's done on it, and where on its screen `at` is.
    fn video_at(&self, at: Point<f32>) -> Option<(Id, (u32, u32))> {
        self.videos
            .iter()
            .filter(|(_, video)| video.connected && !video.watch)
            .find_map(|(id, video)| Some((*id, video.at(at)?)))
    }

    fn video_send(&self, id: Id, input: Value) {
        if let Some(video) = self.videos.get(&id) {
            video.send(input);
        }
    }

    /// The pointer moved to `at`: the video it's over, or holding a press in, is told where.
    pub(super) fn video_moved(&mut self, at: Point<f32>) {
        let over = self
            .video_pressed
            .and_then(|id| Some((id, self.videos.get(&id)?.at_clamped(at)?)))
            .or_else(|| self.video_at(at));
        if let Some((id, (x, y))) = over {
            self.video_send(id, json!({"t": "move", "x": x, "y": y}));
        }
    }

    /// A button went down at `at`: on a video, its screen's, and keys go to it from now on. → whether it was on one.
    pub(super) fn video_down(&mut self, at: Point<f32>, button: u8, shift: bool) -> bool {
        let Some((id, (x, y))) = self.video_at(at) else {
            self.video_keys = None;
            return false;
        };
        self.video_keys = Some(id);
        self.video_pressed = Some(id);
        self.video_send(id, json!({"t": "move", "x": x, "y": y}));
        self.video_send(
            id,
            json!({"t": "button", "b": button + 1, "down": true, "mods": if shift { vec!["Shift_L"] } else { vec![] }}),
        );
        true
    }

    /// A button came up at `at`: the video it went down on has it. → whether it went down on one.
    pub(super) fn video_up(&mut self, at: Point<f32>, button: u8) -> bool {
        let Some(id) = self.video_pressed.take() else { return false };
        if let Some((x, y)) = self.videos.get(&id).and_then(|video| video.at_clamped(at)) {
            self.video_send(id, json!({"t": "move", "x": x, "y": y}));
        }
        self.video_send(id, json!({"t": "button", "b": button + 1, "down": false, "mods": []}));
        true
    }

    /// Scrolled by `delta` at `at`: over a video, its screen's wheel turns, a step (buttons 4 to 7) for each `STEP`
    /// turned. → whether it was over one.
    pub(super) fn video_scroll(&mut self, at: Point<f32>, delta: Point<f32>) -> bool {
        let Some((id, (x, y))) = self.video_at(at) else { return false };
        self.video_turned = crate::geometry::point(self.video_turned.x + delta.x, self.video_turned.y + delta.y);
        self.video_send(id, json!({"t": "move", "x": x, "y": y}));
        for (horizontal, less, more) in [(false, 4, 5), (true, 6, 7)] {
            loop {
                let turned = if horizontal { &mut self.video_turned.x } else { &mut self.video_turned.y };
                if turned.abs() < STEP {
                    break;
                }
                let b = if *turned > 0.0 { more } else { less };
                *turned -= turned.signum() * STEP;
                self.video_send(id, json!({"t": "button", "b": b, "down": true, "mods": []}));
                self.video_send(id, json!({"t": "button", "b": b, "down": false, "mods": []}));
            }
        }
        true
    }

    /// A key, while a video was last pressed: its screen's, but the device's full screen keys (F11, ⌃⌘F), which are
    /// the page's. → whether it took it.
    pub(super) fn video_key(&mut self, key: &Key) -> bool {
        let Some(id) =
            self.video_keys.filter(|id| self.videos.get(id).is_some_and(|video| video.connected && !video.watch))
        else {
            return false;
        };
        if key.key == "F11" || (key.ctrl && key.meta && key.key.eq_ignore_ascii_case("f")) {
            return false;
        }
        let modified = key.ctrl || key.meta || key.alt;
        let single = key.key.chars().count() == 1;
        let name = KEYS
            .iter()
            .find(|(dom, _)| *dom == key.key)
            .map(|(_, x)| (*x).to_owned())
            .or_else(|| single.then(|| key.key.to_lowercase()));
        let mods: Vec<&str> = [
            (key.ctrl || (MAC && key.meta), "Control_L"),
            (key.alt, "Alt_L"),
            (key.shift, "Shift_L"),
            (!MAC && key.meta, "Super_L"),
        ]
        .into_iter()
        .filter_map(|(held, name)| held.then_some(name))
        .collect();
        match name {
            Some(name) if modified || !single => self.video_send(id, json!({"t": "key", "key": name, "mods": mods})),
            _ if single => self.video_send(id, json!({"t": "text", "text": key.key})),
            _ => return false,
        }
        true
    }
}
