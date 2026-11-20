//! What the pointer and a finger do on a screen: what's under it, hover, presses, clicks, scrolling and panning, and what each node keeps for them (`Placed`, `States`).

use super::*;

impl Screen {
    /// Tells node `id` it was clicked, at `at` in the window, or in its middle by a key or an agent, with where it's
    /// drawn (`rect`), which what it opens is placed by, as a menu is under its button.
    pub(crate) fn click(&mut self, id: Id, at: Option<Point<f32>>) {
        let rect = self.rects.get(&id).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
        // A picker opens the device's, as a `<select>` does a phone's browser's.
        if let Some(node) = self.tree.nodes.get(&id).filter(|node| node.kind == Kind::Picker) {
            let (options, chosen) = (node.prop("options").cloned().unwrap_or_default(), node.str("value").map(str::to_owned));
            self.device.pick(id, rect, &options, chosen.as_deref());
            return;
        }
        let at = at.unwrap_or(point(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0));
        self.tell(id, "click", json!({"clientX": at.x, "clientY": at.y, "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}}));
    }

    /// The device's picker for picker node `id` chose `value`, which it's told as a `<select>`'s `input` says it.
    pub fn picked(&mut self, id: Id, value: &str) {
        self.tell(id, "input", json!({"editing": {"text": value}}));
    }

    pub(super) fn placed(&self, id: Id) -> Option<&Placed> {
        self.placed.iter().rev().find(|(placed, _)| *placed == id).map(|(_, placed)| placed)
    }

    /// The topmost node that takes the pointer at `at`, and what it's in, the innermost first.
    pub(super) fn under(&self, at: Point<f32>) -> Vec<Id> {
        let Some(top) = self.placed.iter().rev().find(|(_, placed)| placed.pointer && placed.shown.contains(&at)).map(|(id, _)| *id) else { return Vec::new() };
        let mut chain = vec![top];
        while let Some(parent) = chain.last().and_then(|id| self.placed(*id)).and_then(|placed| placed.parent) {
            chain.push(parent);
        }
        chain
    }

    /// What a press on `chain`, from the innermost out, reaches: the first in it that takes one.
    pub(super) fn pressed_in(&self, chain: &[Id]) -> Option<Id> {
        chain.iter().copied().find(|id| self.placed(*id).is_some_and(|placed| placed.pressable))
    }

    /// The pointer's shape at `at`: what its style says, or a hand over what's pressed and a beam over an input.
    pub fn cursor(&self, at: Point<f32>) -> Cursor {
        if let Some(cursor) = self.frame_cursor(at) {
            return cursor;
        }
        if self.chrome.action.is_some_and(|action| action.contains(&at)) {
            return Cursor::Pointer;
        }
        for id in self.under(at) {
            let Some(placed) = self.placed(id) else { continue };
            if let Some(cursor) = placed.cursor {
                return cursor;
            }
            if placed.input {
                return Cursor::Text;
            }
            if placed.pressable {
                return Cursor::Pointer;
            }
        }
        Cursor::Arrow
    }

    /// A pointer event to each node in `chain` that listens for it, the innermost first, as the DOM's bubble.
    pub(super) fn pointer_event(&mut self, chain: &[Id], name: &str, at: Point<f32>, button: u8, wheel: Option<(Point<f32>, Option<Phase>, Option<Point<f32>>)>) {
        for &id in chain {
            if !self.tree.nodes.get(&id).is_some_and(|node| node.listens(name)) {
                continue;
            }
            let rect = self.rects.get(&id).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
            let mut told = json!({
                "clientX": at.x, "clientY": at.y, "pointerId": 1, "pointerType": if self.touch { "touch" } else { "mouse" }, "button": button,
                // When it happened, in milliseconds by the page's clock, as `performance.now()` says it: what a
                // swipe's speed is worked out by.
                "timeStamp": self.input_time(),
                "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height},
            });
            if let Some((delta, phase, notches)) = wheel {
                told["deltaX"] = json!(delta.x);
                told["deltaY"] = json!(delta.y);
                if let Some(notches) = notches {
                    told["notchesX"] = json!(notches.x);
                    told["notchesY"] = json!(notches.y);
                }
                if let Some(phase) = phase {
                    told["momentum"] = json!(phase.momentum);
                    if let Some(fingers) = phase.fingers {
                        told["phase"] = json!(fingers);
                    }
                }
            }
            self.tell(id, name, told);
        }
    }

    /// What the pointer at `at` is over is hovered, what it's left and what it's come into told so, as the DOM's
    /// `pointerleave` and `pointerenter`. → what it's over, the innermost first.
    pub(super) fn hover(&mut self, at: Point<f32>) -> Vec<Id> {
        let chain = self.under(at);
        let mut hovered = chain.clone();
        hovered.reverse();
        if hovered == self.states.hovered {
            return chain;
        }
        let left: Vec<Id> = self.states.hovered.iter().copied().filter(|id| !hovered.contains(id)).collect();
        let entered: Vec<Id> = hovered.iter().copied().filter(|id| !self.states.hovered.contains(id)).collect();
        self.states.hovered = hovered;
        self.changed();
        for id in left.into_iter().rev() {
            self.pointer_event(&[id], "pointerleave", at, 0, None);
        }
        for id in entered {
            self.pointer_event(&[id], "pointerenter", at, 0, None);
        }
        chain
    }

    /// The pointer moved to `at`: what it's over is hovered, told as it comes and goes, and a press it holds is told
    /// it moved.
    /// Whether a pointer's press, or a finger, is held on it, so what moves follows the person.
    pub fn held(&self) -> bool {
        self.captured.is_some()
    }

    pub fn pointer_moved(&mut self, at: Point<f32>) {
        self.frame_moved(at);
        #[cfg(not(target_arch = "wasm32"))]
        self.video_moved(at);
        self.pointer = Some(at);
        let chain = self.hover(at);
        let target = match self.captured {
            Some(captured) => self.chain_from(captured),
            None => chain,
        };
        if !self.swiping() {
            self.drag_moved(at);
        }
        self.pointer_event(&target, "pointermove", at, 0, None);
        self.select_moved(at);
        // A press held in an input selects as it's dragged.
        if let Some(id) = self.editing.selecting
            && let (Some(field), Some(drawn)) = (self.editing.fields.get_mut(&id), self.editing.drawn.get(&id))
        {
            let before = field.value();
            let (index, upstream) = drawn.hit(at);
            field.drag(index, upstream);
            self.edited(id, before);
        }
    }

    /// The pointer left the window: nothing's hovered.
    pub fn pointer_left(&mut self) {
        self.frame_left();
        let at = self.pointer.take().unwrap_or(point(0.0, 0.0));
        for id in std::mem::take(&mut self.states.hovered).into_iter().rev() {
            self.pointer_event(&[id], "pointerleave", at, 0, None);
        }
    }

    /// `id` and what it's in, the innermost first, as last painted.
    /// Whether node `id` is `outer` or in it, as the page's tree has them, whatever's laid out between.
    pub(super) fn within(&self, id: Id, outer: Id) -> bool {
        let mut at = Some(id);
        while let Some(node) = at {
            if node == outer {
                return true;
            }
            at = self.tree.parent(node).filter(|parent| *parent != 0);
        }
        false
    }

    /// Whether node `id` is in the page's tree: under its root, not taken out of it, though its JS may still hold it.
    pub(super) fn attached(&self, id: Id) -> bool {
        let mut at = Some(id);
        while let Some(node) = at.filter(|node| !self.tree.leaving(*node)) {
            if node == 0 {
                return true;
            }
            at = self.tree.parent(node);
        }
        false
    }

    pub(super) fn chain_from(&self, id: Id) -> Vec<Id> {
        let mut chain = vec![id];
        while let Some(parent) = chain.last().and_then(|id| self.placed(*id)).and_then(|placed| placed.parent) {
            chain.push(parent);
        }
        chain
    }

    /// A button went down at `at` (0 the main one, 1 the middle, 2 the secondary), with Shift held or not: what's
    /// under it is told, and with the main one what's pressed there is pressed and takes the keyboard if it can, an
    /// input putting its caret there (a double click selects a word, a triple its line, and Shift extends the
    /// selection); the middle one in an input pastes the primary selection there, as on Linux; the secondary is a
    /// `contextmenu` too.
    pub fn pointer_down(&mut self, at: Point<f32>, button: u8, shift: bool) {
        // The window's own buttons are the window's, not its page's.
        if let Some((kind, _)) = self.chrome.buttons.iter().find(|(_, bounds)| bounds.contains(&at)) {
            self.chrome.pressing_button = (button == 0).then_some(*kind);
            self.changed();
            return;
        }
        // Its notice's action is the window's, not its page's.
        if self.chrome.action.is_some_and(|action| action.contains(&at)) {
            self.chrome.pressing_action = button == 0;
            return;
        }
        if self.frame_down(at, button, shift) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.video_down(at, button, shift) {
            return;
        }
        let chain = self.under(at);
        self.captured = chain.first().copied();
        if button == 0 && !self.swiping() {
            self.drag_down(&chain, at);
        }
        self.pointer_event(&chain, "pointerdown", at, button, None);
        if button == 2 {
            // On a field, it has the keyboard and the system's menu for text, its caret where the click was unless
            // that's in what's selected, as every system's fields have.
            if let Some(input) = self.pressed_in(&chain).filter(|id| self.editing.fields.contains_key(id)) {
                self.focus(Some(input));
                let (index, _) = self.editing.drawn.get(&input).map_or((usize::MAX, false), |drawn| drawn.hit(at));
                let field = &self.editing.fields[&input];
                let (start, end) = field.range();
                if !(start..end).contains(&index) {
                    self.press_input(input, at, 1, false);
                }
                self.text_menu = Some(self.text_menu_of(input, Bounds { origin: at, size: Size::default() }));
            }
            return self.pointer_event(&chain, "contextmenu", at, button, None);
        }
        let pressed = self.pressed_in(&chain);
        let input = pressed.filter(|id| self.editing.fields.contains_key(id));
        if button == 1 {
            if let Some(input) = input.filter(|id| self.editing.fields.get(id).is_some_and(|field| !field.secure && !field.readonly)) {
                self.states.focus_visible = false;
                self.focus(Some(input));
                self.press_input(input, at, 1, false);
                if let Some(text) = self.clipboard.read(true) {
                    self.edit(input, |field| field.insert(&text));
                }
            }
            return;
        }
        self.states.pressed = pressed;
        self.changed();
        self.states.focus_visible = false;
        self.click_focus(&chain);
        // Presses close together in time and place count as a double or triple click, by GTK's defaults; a finger's
        // taps further apart (`touch::DOUBLE_TAP_SLOP`).
        let now = self.input_time();
        let near = if self.touch { touch::DOUBLE_TAP_SLOP } else { 5.0 };
        let clicks = match self.editing.last_press {
            Some((when, was, clicks)) if now - when < 400.0 && (was.x - at.x).abs() < near && (was.y - at.y).abs() < near => (clicks + 1).min(3),
            _ => 1,
        };
        self.editing.last_press = Some((now, at, clicks));
        // Text that's neither a field nor in something pressed is selected from here, by a pointer (a finger drags to
        // scroll); a press anywhere else lets go of what was.
        let text = (pressed.is_none() && !self.touch).then(|| chain.iter().copied().find(|id| self.editing.texts.contains_key(id))).flatten();
        self.select_down(text, at, clicks);
        if let Some(input) = input {
            self.editing.selecting = Some(input);
            self.press_input(input, at, clicks, shift);
        }
    }

    /// Input `id` pressed at `at`, `clicks` times in a row, or with Shift held.
    pub(super) fn press_input(&mut self, id: Id, at: Point<f32>, clicks: u8, shift: bool) {
        let (index, upstream) = self.editing.drawn.get(&id).map_or((usize::MAX, false), |drawn| drawn.hit(at));
        self.edit(id, |field| field.press(index.min(field.display().len()), upstream, clicks, shift));
    }

    /// A button came up at `at`: what it went down on is told, and with the main one a press, if it's still over
    /// what was pressed.
    pub fn pointer_up(&mut self, at: Point<f32>, button: u8) {
        if let Some(kind) = self.chrome.pressing_button.take() {
            if self.chrome.buttons.iter().any(|(was, bounds)| *was == kind && bounds.contains(&at)) {
                self.window_button = Some(kind);
            }
            self.changed();
            return;
        }
        if std::mem::take(&mut self.chrome.pressing_action) {
            if self.chrome.action.is_some_and(|action| action.contains(&at))
                && let Some((_, action)) = self.notice.as_mut().and_then(|notice| notice.action.as_mut())
            {
                action();
            }
            return;
        }
        if self.frame_up(at, button) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.video_up(at, button) {
            return;
        }
        let target = match self.captured.take() {
            Some(captured) => self.chain_from(captured),
            None => self.under(at),
        };
        if !self.swiping() {
            self.drag_up(at, false);
        }
        self.pointer_event(&target, "pointerup", at, button, None);
        self.select_up();
        // What the pointer selected goes on the primary selection, as Linux's fields put it, never a password.
        if let Some(field) = self.editing.selecting.take().and_then(|id| self.editing.fields.get(&id))
            && !field.secure
            && !field.selected().is_empty()
        {
            let selected = field.selected().to_owned();
            self.clipboard.write(true, &selected);
        }
        let Some(pressed) = self.states.pressed.take() else { return };
        let over = self.under(at).contains(&pressed);
        if over && self.placed(pressed).is_some_and(|placed| !placed.input) {
            self.click(pressed, Some(at));
        }
    }

    /// The wheel or a trackpad scrolled by `delta` pixels (down and right positive) at `at`, in `phase` where the
    /// system says, or by a wheel's `notches`, its clicks as it turns, where it has them: what's under it is told
    /// (`wheel`), and the innermost thing there that can still scroll that way does, telling its app if it listens.
    /// → what scrolled.
    pub fn scroll(&mut self, at: Point<f32>, delta: Point<f32>, phase: Option<Phase>, notches: Option<Point<f32>>) -> Option<Id> {
        // A frame's page scrolls first, and what it can't, this one does, as a browser chains them.
        if let Some(frame) = self.frame_scroll(at, delta, Some((phase, notches))) {
            return Some(frame);
        }
        if self.drag_wheel(at, delta, phase) {
            return None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.video_scroll(at, delta) {
            return None;
        }
        let chain = self.under(at);
        self.pointer_event(&chain, "wheel", at, 0, Some((delta, phase, notches)));
        self.pan(at, delta)
    }

    /// Whether a finger that landed at `at` and has moved `by` (its first move past the slop) pans, as a browser decides
    /// by `touch-action`: from what it's on outwards, what can still scroll that way scrolls; a node that listens for
    /// its pointer keeps the finger, its moves its own, unless it says the device pans that way (`touchAction`); and
    /// with neither, the device pans. So a sheet's content scrolled to its top, pulled down, is the sheet's to pull,
    /// as an iPhone's and Android's sheets take it.
    pub fn pans(&self, at: Point<f32>, by: Point<f32>) -> bool {
        let along = by.x.abs() > by.y.abs();
        for id in self.under(at) {
            let Some(placed) = self.placed(id) else { continue };
            if let Some((most_y, most_x, _)) = placed.scroll {
                let now = self.scrolled.get(&id).copied().unwrap_or(point(0.0, 0.0));
                let (finger, now, most) = if along { (by.x, now.x, most_x) } else { (by.y, now.y, most_y) };
                if (finger < 0.0 && now < most) || (finger > 0.0 && now > 0.0) {
                    return true;
                }
            }
            // What drags that way keeps the finger, and what drags the other doesn't.
            match self.drags(id, by) {
                Some(true) => return false,
                Some(false) => continue,
                None => {}
            }
            if self.tree.nodes.get(&id).is_some_and(|node| node.listens("pointermove") || node.listens("pointerdown")) {
                return placed.touch_action == Some(if along { style::Pan::X } else { style::Pan::Y });
            }
        }
        true
    }

    /// A finger (or a screen reader) moved what's at `at` by `delta` pixels (down and right positive): the innermost
    /// thing there that can still scroll that way does, telling its app if it listens. → what scrolled.
    pub fn pan(&mut self, at: Point<f32>, delta: Point<f32>) -> Option<Id> {
        if let Some(frame) = self.frame_scroll(at, delta, None) {
            return Some(frame);
        }
        let chain = self.under(at);
        let target = chain.iter().copied().find_map(|id| {
            let (most_y, most_x, along) = self.placed(id)?.scroll?;
            let now = self.scrolled.get(&id).copied().unwrap_or(point(0.0, 0.0));
            // A scroll along takes the wheel's turn as along where it doesn't go down, as a row of pages does.
            let (by, now, most) = if along { (if delta.x != 0.0 { delta.x } else { delta.y }, now.x, most_x) } else { (delta.y, now.y, most_y) };
            ((by > 0.0 && now < most) || (by < 0.0 && now > 0.0)).then_some((id, by, most, along))
        })?;
        let (id, by, most, along) = target;
        let now = self.scrolled.entry(id).or_insert(point(0.0, 0.0));
        if along {
            now.x = (now.x + by).clamp(0.0, most);
        } else {
            now.y = (now.y + by).clamp(0.0, most);
        }
        let scrolled = *now;
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("scroll")) {
            self.tell(id, "scroll", json!({"scrollTop": scrolled.y, "scrollLeft": scrolled.x}));
        }
        Some(id)
    }

    /// A tap on an iPhone's status bar: everything that scrolls goes back to its top, as the DOM's page does (`toTop` in
    /// the UI library's bridge.ts).
    pub fn scroll_to_top(&mut self) {
        let scrolled: Vec<(Id, Point<f32>)> = self.scrolled.iter().filter(|(_, at)| at.y > 0.0).map(|(id, at)| (*id, point(at.x, 0.0))).collect();
        for (id, at) in scrolled {
            self.scrolled.insert(id, at);
            if self.tree.nodes.get(&id).is_some_and(|node| node.listens("scroll")) {
                self.tell(id, "scroll", json!({"scrollTop": at.y, "scrollLeft": at.x}));
            }
        }
        self.changed();
    }

    /// The input about to be handled happened `ago` milliseconds before now, as its device says (a touch's own time),
    /// or nothing says when (`None`), which times it as it's handled: how fast a press went is worked out from when
    /// each of its moves happened, not when each was handled, which moves handled together get wrong.
    pub fn happened(&mut self, ago: Option<f64>) {
        self.happened = ago.map(|ago| self.js.clock().now() - ago.max(0.0));
    }

    /// When the input being handled happened, on the page's clock.
    pub fn input_time(&self) -> f64 {
        self.happened.unwrap_or_else(|| self.js.clock().now())
    }

    /// Files dropped on the window at `at` from the system (a Finder's, an Explorer's): what's under it that takes a
    /// drop, the innermost, is told them (`drop`).
    pub fn dropped(&mut self, at: Point<f32>, files: &[std::path::PathBuf]) {
        if let Some(id) = self.under(at).into_iter().find(|id| self.tree.nodes.get(id).is_some_and(|node| node.listens("drop"))) {
            self.tell(id, "drop", json!({"files": files_told(files)}));
        }
    }
}

/// What the painter keeps of each node it paints, for the pointer and the keyboard.
/// What a click on a node does with the keyboard (`Placed::focus`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Clicked {
    Takes,
    Keeps,
    Passes,
}

#[derive(Clone)]
pub(crate) struct Placed {
    pub parent: Option<Id>,
    /// Where it shows, clipped, in the window.
    pub shown: Bounds<f32>,
    /// How opaque it's drawn, with what it's in, as it's moving now: what a person sees of it, which the inspector
    /// says.
    pub opacity: f32,
    /// Whether it takes the pointer (`pointerEvents`): what doesn't is still in the chain of what's in it.
    pub pointer: bool,
    pub cursor: Option<Cursor>,
    pub pressable: bool,
    pub input: bool,
    /// What a click gives the keyboard, as the DOM's does: it, where it takes the keyboard (an input, what's pressed as
    /// a button is, what says its `tabindex`, a frame), or nothing, or the keyboard stays where it is, as a press of
    /// what's chosen from a list another holds it for (`option`) leaves it (core/ui's drawing-primitives/primitives.tsx
    /// `chosen`).
    pub focus: Clicked,
    /// It scrolls: how far it can, down and along, and whether along is its way.
    pub scroll: Option<(f32, f32, bool)>,
    /// Which way a finger on it pans, where it listens for its pointer (`Look::touch_action`).
    pub touch_action: Option<style::Pan>,
}

/// What a node's state is, for its styles: where the pointer is, what's pressed and what has the keyboard.
#[derive(Default, Clone, PartialEq)]
pub(crate) struct States {
    /// What the pointer is over, from the root to the innermost.
    pub hovered: Vec<Id>,
    pub pressed: Option<Id>,
    pub focused: Option<Id>,
    /// The keyboard came there by a key, so it shows (CSS's `:focus-visible`).
    pub focus_visible: bool,
    /// The pointer is a finger, which a style's `hover` never is: that's a mouse's (`State` in the UI library), as the
    /// DOM's styles hover only where `(hover: hover)`.
    pub touch: bool,
}

impl States {
    /// What's in a different state from in `was`.
    pub(super) fn changed(&self, was: &States) -> Vec<Id> {
        let mut changed: Vec<Id> = self.hovered.iter().filter(|id| !was.hovered.contains(id)).chain(was.hovered.iter().filter(|id| !self.hovered.contains(id))).copied().collect();
        if self.touch != was.touch {
            changed.extend(self.hovered.iter().copied());
        }
        if self.pressed != was.pressed {
            changed.extend(self.pressed.into_iter().chain(was.pressed));
        }
        if self.focused != was.focused || self.focus_visible != was.focus_visible {
            changed.extend(self.focused.into_iter().chain(was.focused));
        }
        changed
    }

    pub(super) fn of(&self, id: Id, input: bool) -> State {
        State {
            hover: !self.touch && self.hovered.contains(&id),
            pressed: self.pressed == Some(id),
            focused: self.focused == Some(id) && (self.focus_visible || input),
            entering: false,
            exiting: false,
        }
    }
}
