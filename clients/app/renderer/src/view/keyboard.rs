//! What has the keyboard and what a key does: focus, Tab, the key a shortcut summoned a panel with, and the input method.

use super::*;

impl Screen {
    /// A modal that's gone gives the keyboard back to what had it as it came, if the keyboard was in it or is nowhere,
    /// and that's still there; one that came takes it.
    pub(super) fn settle_focus(&mut self, modal: Option<Id>) {
        while let Some(&(gone, was)) = self.focus.modals.last() {
            if self.attached(gone) && self.tree.nodes.get(&gone).is_some_and(|node| node.str("aria-modal") == Some("true")) {
                break;
            }
            self.focus.modals.pop();
            if self.focus.focused.is_none_or(|focused| !self.attached(focused) || self.within(focused, gone)) {
                self.focus(was.filter(|was| self.attached(*was)));
                self.again = true;
            }
        }
        if let Some(modal) = modal {
            // What had the keyboard as it came: what has it now, or, where what's in it took it as it came
            // (`autofocus`), what had it before that.
            if !self.focus.modals.iter().any(|(shown, _)| *shown == modal) {
                let was = [self.focus.focused, self.focus.blurred].into_iter().flatten().find(|had| !self.within(*had, modal));
                self.focus.modals.push((modal, was));
            }
            // The keyboard goes into it: an alert's to its first action, what does least by convention (Cancel, Don't
            // Allow), as WAI-ARIA's alertdialog pattern has it and the UI SDK's alert does; any other dialog's to itself.
            if !self.focus.focused.is_some_and(|focused| self.within(focused, modal)) {
                let alert = self.tree.nodes.get(&modal).is_some_and(|node| node.str("role") == Some("alertdialog"));
                let first = alert.then(|| self.focus.tab_order.iter().copied().find(|id| self.within(*id, modal))).flatten();
                self.focus(Some(first.unwrap_or(modal)));
                self.again = true;
            }
        }
    }

    /// Gives the keyboard to what a click on `chain`, from the innermost out, gives it to, as the DOM does: the first
    /// in it that takes it, or nothing, but where what's clicked keeps it where it is (`Placed::focus`).
    pub(crate) fn click_focus(&mut self, chain: &[Id]) {
        let first = chain.iter().find_map(|id| self.placed(*id).map(|placed| (*id, placed.focus)).filter(|(_, focus)| *focus != Clicked::Passes));
        match first {
            Some((_, Clicked::Keeps)) => {}
            // A click on what takes no keyboard leaves it with what has it by its `autofocus`, as a native app's field
            // keeps it while what's around it is clicked, and a launcher's search while its card is.
            None if self.focus.focused.is_some_and(|id| self.tree.nodes.get(&id).is_some_and(|node| node.is("autofocus"))) => {}
            first => self.focus(first.map(|(id, _)| id)),
        }
    }

    /// Gives the keyboard to `id`, or to nothing, telling the app what had it and what has it, where they listen. An
    /// input that loses it keeps what was being composed in it.
    pub(super) fn focus(&mut self, id: Option<Id>) {
        if self.focus.focused == id {
            return;
        }
        self.changed();
        self.editing.overlay = None;
        if let Some(input) = self.focus.focused.filter(|focused| self.editing.fields.contains_key(focused)) {
            self.edit(input, Field::blur);
        }
        self.editing.blink = self.js.clock().now();
        let blurred = std::mem::replace(&mut self.focus.focused, id);
        self.focus.blurred = blurred;
        if let Some(blurred) = blurred.filter(|blurred| self.tree.nodes.get(blurred).is_some_and(|node| node.listens("blur"))) {
            self.event(blurred, "blur", None);
        }
        if let Some(id) = id.filter(|id| self.tree.nodes.get(id).is_some_and(|node| node.listens("focus"))) {
            self.event(id, "focus", None);
        }
        // An input that says `selecttextonfocus` (React Native's `selectTextOnFocus`) has all its text selected as it
        // takes the keyboard.
        if let Some(id) = id.filter(|id| self.editing.fields.contains_key(id) && self.tree.nodes.get(id).is_some_and(|node| node.is("selecttextonfocus"))) {
            self.edit(id, Field::select_all);
        }
    }

    /// A key went down where the keyboard is, as the DOM's `KeyboardEvent` has it: Tab moves the keyboard, what
    /// listens for `keydown` hears it, Enter and Space press what has it, and an input takes what's typed. The page
    /// hears every key too, once, whatever has the keyboard, even nothing, as a browser's `document` does (`keydown()`:
    /// Escape closing what's open, ⌘K). → what had the keyboard.
    pub fn key(&mut self, key: &Key) -> Option<Id> {
        let focused = self.focus.focused;
        // The key of the shortcut that just summoned it, still held or pressed again at once, is the shortcut's, not
        // the page's: it types nothing and presses nothing (`summoned`).
        if let Some((held, since)) = self.shortcut.take() {
            if key.key == held && since.elapsed() < SHORTCUT_HELD {
                self.shortcut = Some((held, web_time::Instant::now()));
                return focused;
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.video_key(key) {
            return focused;
        }
        // A frame with the keyboard: its page has the key, and Tab off its page's end goes on from the frame here.
        if let Some(None) = self.frame_key(key) {
            return focused;
        }
        // A key pressed without a shortcut's modifier is the keyboard in use: what has it shows it from now on, as a
        // browser's `:focus-visible` does, so the arrows moving through a list a click went into show where they are.
        if !key.ctrl && !key.meta && !key.alt {
            self.states.focus_visible = true;
        }
        // Tab is told to what has the keyboard and what it's in, as any key is, then moves the keyboard, as a
        // browser's does unless the page stops it, which a native host can't hear in time.
        if key.key == "Tab" && !key.ctrl && !key.meta && !key.alt {
            if let Some(id) = focused {
                for listener in self.chain_from(id) {
                    if self.tree.nodes.get(&listener).is_some_and(|node| node.listens("keydown")) {
                        self.tell(listener, "keydown", key.told());
                    }
                }
            }
            self.tell(PAGE, "keydown", key.told());
            // It goes round, but out of a frame's page at its ends, back to the page the frame's in; into a frame's
            // page, to its first stop or its last, and past a frame whose page has none.
            let order = self.focus.tab_order.clone();
            let count = order.len();
            let mut index = focused.and_then(|focused| order.iter().position(|id| *id == focused));
            let mut next = None;
            for _ in 0..count {
                let step = match index {
                    Some(at) if key.shift => at.checked_sub(1).or((!self.nested).then(|| count - 1)),
                    Some(at) => (at + 1 < count).then_some(at + 1).or((!self.nested).then_some(0)),
                    None if key.shift => count.checked_sub(1),
                    None => (count > 0).then_some(0),
                };
                let Some(step) = step else { break };
                index = Some(step);
                if self.frames.contains_key(&order[step]) && !self.enter_frame(order[step], key.shift) {
                    continue;
                }
                next = Some(order[step]);
                break;
            }
            if next.is_none() && self.nested {
                self.left = Some(key.shift);
                self.focus(None);
                return focused;
            }
            self.focus(next);
            // What it comes to scrolls into view, as a browser's does, so Tab goes on through a long list.
            if let Some(next) = next {
                self.reveal(next);
            }
            // An input the keyboard comes to by Tab has its text selected, as a browser's and GTK's do.
            if let Some(next) = next.filter(|next| self.editing.fields.contains_key(next)) {
                self.edit(next, Field::select_all);
            }
            return focused;
        }
        // A key goes to what has the keyboard and to each node it's in that listens, the innermost first, then the page.
        if let Some(id) = focused {
            for listener in self.chain_from(id) {
                if self.tree.nodes.get(&listener).is_some_and(|node| node.listens("keydown")) {
                    self.tell(listener, "keydown", key.told());
                }
            }
        }
        // Enter or Space on a button presses it alone, as every system's does: the page's `preventDefault` would come
        // too late to stop the press, so the page isn't told of it too (a launcher opening its chosen app behind it).
        // A radio or a checkbox only Space, as a browser's input of that type: Enter there is a form's.
        let plain = !key.ctrl && !key.meta && !key.alt;
        let presses = plain && focused.is_some_and(|id| self.pressed_by(id, key) && (id == NOTICE_ACTION || (!self.editing.fields.contains_key(&id) && self.tree.nodes.get(&id).is_some_and(|node| node.listens("click")))));
        if !presses {
            self.tell(PAGE, "keydown", key.told());
        }
        // What's selected of a text that isn't a field is copied, as a browser's is.
        if self.copy_selected(key) {
            return focused;
        }
        let id = focused?;
        // Escape empties a search field with something typed in it, as a browser's `type="search"` does.
        let search = self.tree.nodes.get(&id).is_some_and(|node| node.str("type") == Some("search"));
        if search && key.key == "Escape" && self.editing.fields.get(&id).is_some_and(|field| !field.value().text.is_empty()) {
            // What's selected is replaced by nothing, as a cut is, which undo brings back.
            self.edit(id, |field| {
                field.select_all();
                field.paste("");
            });
        } else if self.editing.fields.contains_key(&id) {
            self.type_key(id, key);
        } else if presses && self.press_chrome(id) {
        } else if self.pressed_by(id, key) && self.tree.nodes.get(&id).is_some_and(|node| node.listens("click")) {
            self.click(id, None)
        }
        Some(id)
    }

    /// Whether `key` presses node `id`, as a browser's keys press what has the keyboard: Space, and Enter but on a radio
    /// or a checkbox, where it's a form's.
    fn pressed_by(&self, id: Id, key: &Key) -> bool {
        key.key == " " || (key.key == "Enter" && !self.tree.nodes.get(&id).is_some_and(|node| matches!(node.str("role"), Some("radio" | "checkbox"))))
    }

    /// What the platform's input method says, to the input with the keyboard.
    pub fn ime(&mut self, ime: Ime) {
        if self.frame_ime(ime.clone()) {
            return;
        }
        if let Some(id) = self.focus.focused.filter(|id| self.editing.fields.contains_key(id)) {
            self.edit(id, |field| field.ime(ime));
        }
    }

    /// It was summoned by a shortcut that ends with `key` (as the DOM's `KeyboardEvent.key` names it): that key, still
    /// held as it shows or pressed again right after, is the shortcut's (`key`).
    pub fn summoned(&mut self, key: Option<&str>) {
        self.shortcut = key.map(|key| (key.to_owned(), web_time::Instant::now()));
    }
}
