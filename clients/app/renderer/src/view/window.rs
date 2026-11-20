//! What the window says of itself over its page: its semantic tree, its notice, its own buttons and title row, whether it's shown, seen or has the keyboard, and a page loaded again in its place.

use super::*;

impl Screen {
    /// The semantic tree, as an agent and a screen reader read it, each frame's page's in its place, after the
    /// window's own: its notice, while it's drawn.
    pub fn semantics(&self) -> Value {
        let mut out = Vec::new();
        if let Some(notice) = &self.notice {
            let action = notice.action.as_ref().filter(|_| self.chrome.action.is_some());
            let children: Vec<Value> = action.map(|(label, _)| json!({"id": NOTICE_ACTION, "role": "button", "name": label, "actions": ["click"]})).into_iter().collect();
            out.push(json!({"id": NOTICE, "role": "status", "live": "polite", "name": notice.text, "children": children}));
        }
        out.extend(self.grafted(self.tree.semantics(0)));
        json!(out)
    }

    /// Presses the window's own node `id` (`NOTICE_ACTION`), as a click on it does. → whether it's one.
    pub(crate) fn press_chrome(&mut self, id: Id) -> bool {
        match id {
            NOTICE_ACTION => match self.notice.as_mut().and_then(|notice| notice.action.as_mut()) {
                Some((_, action)) => action(),
                None => return false,
            },
            _ => return false,
        }
        true
    }

    /// Its window came on screen, or went, as a panel does as it's summoned and put away: its page is told, as a
    /// browser's page is by `visibilitychange`.
    pub fn shown(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        for child in self.frames.values_mut() {
            child.screen.shown(visible);
        }
        // Shown again, it has the keyboard where it says `autofocus`, as a page opened does, from its first key on;
        // put away, nothing keeps it, so nothing left with it is pressed as it comes back.
        self.focus.autofocused.clear();
        self.focus(if visible { self.tree.autofocused().max() } else { None });
        if visible {
            self.focus.autofocused.extend(self.tree.autofocused());
        }
        // Its frames' pages are told by their own `shown`, above.
        self.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": {"visible": visible}}));
    }

    /// Its page, and each frame's, is told whether it's seen (`hostContext.visible`, as the DOM's `visibilityState`
    /// says it): not while it's put away, or its app is in the background, where it keeps what it shows and the keyboard.
    pub fn seen(&mut self, seen: bool) {
        for child in self.frames.values_mut() {
            child.screen.seen(seen);
        }
        self.message(&json!({"jsonrpc": "2.0", "method": "ui/notifications/host-context-changed", "params": {"visible": seen}}));
    }

    /// The window button last pressed, once, for its window to do.
    pub fn take_window_button(&mut self) -> Option<WindowButton> {
        self.window_button.take()
    }

    /// The system's menu for text a secondary click asked for, once.
    pub fn take_text_menu(&mut self) -> Option<TextMenu> {
        self.text_menu.take()
    }

    /// Its window got or lost the keyboard: the caret shows only while it has it, and an input losing it keeps what
    /// was being composed in it.
    pub fn activate(&mut self, active: bool) {
        self.active = active;
        self.editing.blink = self.js.clock().now();
        if let Some(id) = self.focus.focused.filter(|id| !active && self.editing.fields.contains_key(id)) {
            self.edit(id, Field::finish_composition);
        }
        self.unused();
    }

    /// Whether something is under way on its page that a new version of it would lose: a press held or dragged, text
    /// being selected or composed, or what its person typed that its app hasn't taken (`typed`).
    pub fn in_use(&self) -> bool {
        self.captured.is_some()
            || self.drag.is_some()
            || self.editing.selecting.is_some()
            || self.editing.fields.values().any(Field::composing)
            || self.typed()
    }

    /// Does what waits for its page not to be in use (`when_unused`), if it isn't: as input ends, and as its app takes
    /// what was typed.
    pub fn unused(&mut self) {
        if !self.when_unused.is_empty() && !self.in_use() {
            for unused in std::mem::take(&mut self.when_unused) {
                unused();
            }
        }
    }

    /// Shows `page`, a new version of its app's page made for the same window, in its page's place, as a browser's tab
    /// loads again: what's the window's stays (its room and insets, its title, theme and buttons, what it says over its
    /// page, the device and its clipboard, and what wakes it), and what was the page's goes with it.
    pub fn reload(&mut self, mut page: Screen) {
        page.room = self.room;
        page.insets = self.insets;
        page.theme = self.theme;
        page.appearance = self.appearance;
        page.reduced_motion = self.reduced_motion;
        page.pressure = self.pressure;
        page.text_size = self.text_size;
        page.title = self.title.take();
        page.window_buttons = std::mem::take(&mut self.window_buttons);
        page.full_screen = self.full_screen;
        page.panel = self.panel.take();
        page.notice = self.notice.take();
        page.visible = self.visible;
        page.active = self.active;
        page.touch = self.touch;
        page.rail = self.rail;
        page.hz = self.hz;
        page.shortcut = self.shortcut.take();
        page.make_frame = page.make_frame.or_else(|| self.make_frame.take());
        page.clipboard = std::mem::replace(&mut self.clipboard, Box::new(editing::Memory::default()));
        page.device = std::mem::replace(&mut self.device, Box::new(()));
        if let Some(wake) = self.wake.take() {
            page.on_post(wake);
        }
        *self = page;
    }

    /// Its title row, where its window has one over its page (a unified toolbar): how tall it is, and where in it
    /// something takes a press, which the rest of it doesn't, so the window moves held there, as a title bar does.
    /// What its page says moves the window (`appRegion`) is part of it too, with or without a title row.
    pub fn title_row(&self) -> Option<TitleRow> {
        let height = if self.title.is_some() { self.insets.top } else { 0.0 };
        if height == 0.0 && self.chrome.drag_regions.is_empty() {
            return None;
        }
        // What takes a press, or a hold, in the row or in what moves the window, keeps it: a button, an input, what's
        // held to edit it. What's under them isn't in them.
        let inside = |id: Id, placed: &Placed| placed.shown.origin.y < height || self.chrome.drag_regions.iter().any(|(region, _)| self.within(id, *region));
        let takes = |id: &Id, placed: &Placed| placed.pressable || placed.input || self.tree.nodes.get(id).is_some_and(|node| node.listens("pointerdown"));
        let mut taken: Vec<Bounds<f32>> = self.placed.iter().filter(|(id, placed)| takes(id, placed) && inside(*id, placed)).map(|(_, placed)| placed.shown).collect();
        taken.extend(self.chrome.kept_regions.iter().copied());
        taken.extend(self.chrome.buttons.iter().map(|(_, bounds)| *bounds));
        taken.extend(self.chrome.action);
        Some(TitleRow { height, taken, regions: self.chrome.drag_regions.iter().map(|(_, bounds)| *bounds).collect() })
    }
}
