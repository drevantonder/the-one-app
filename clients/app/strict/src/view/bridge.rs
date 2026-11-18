//! What a phone's platform asks of a screen beyond a computer's pointer and keys: its keyboard bridge (Android's
//! `InputConnection`, through Flutter's `TextInputPlugin`), which keeps the input's whole editing state itself and
//! says it back as the renderer's `Editing` (`editing::Value`); its autofill, which fills any field; its screen
//! reader, which presses, focuses and scrolls by node; and a finger, which pans what scrolls and may be lifted off what
//! it pressed.

use serde_json::{Value, json};

use super::Screen;
use crate::editing;
use crate::geometry::{Point, point};
use crate::inspect::Rect;
use crate::tree::{Id, Kind};

impl Screen {
    /// What has the keyboard.
    pub fn focused(&self) -> Option<Id> {
        self.focused
    }

    /// Gives the keyboard to `id`, or takes it from what has it, as a screen reader or a keyboard's Next asks.
    pub fn set_focused(&mut self, id: Option<Id>) {
        self.focus(id);
    }

    /// A keyboard that keeps the editing state itself says input `id`'s is now `value` (what it typed, selected and is
    /// composing), or a password manager filled it: taken as it is, and its app told.
    pub fn set_editing(&mut self, id: Id, value: &editing::Value) {
        self.edit(id, |field| field.keyboard(value));
    }

    /// The inputs, in the order Tab goes, as last painted.
    pub fn inputs(&self) -> Vec<Id> {
        self.tab_order.iter().copied().filter(|id| self.fields.contains_key(id)).collect()
    }

    /// Whether what a press at `at` presses is an input, which a finger gives the keyboard only as it lifts.
    pub fn input_at(&self, at: Point<f32>) -> bool {
        self.under(at).into_iter().find(|id| self.placed(*id).is_some_and(|placed| placed.pressable)).is_some_and(|id| self.fields.contains_key(&id))
    }

    /// What a keyboard bridge needs to know of input `id`: what kind of text it takes (`type`, `inputmode`), what may
    /// fill it (`autocomplete`), how a keyboard capitalises and checks it, its action key, its name, and where it is.
    pub fn field(&self, id: Id) -> Option<Value> {
        let node = self.tree.nodes.get(&id).filter(|node| node.kind == Kind::Input)?;
        let rect = self.rects.get(&id).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
        let multiline = node.is("multiline");
        Some(json!({
            "id": id,
            "type": node.str("type").unwrap_or("text"),
            "inputmode": node.str("inputmode"),
            "autocomplete": node.str("autocomplete"),
            "autocapitalize": node.str("autocapitalize"),
            "spellcheck": node.prop("spellcheck").is_none_or(|spellcheck| spellcheck != &json!(false) && spellcheck != &json!("false")),
            "multiline": multiline,
            "readonly": node.is("readonly"),
            "action": node.str("action").unwrap_or(if multiline { "newline" } else { "done" }),
            "label": node.str("aria-label").or_else(|| node.str("placeholder")),
            "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height},
            "text": self.fields.get(&id).map(|field| field.value().text),
        }))
    }

    /// Input `id`'s action key was pressed on the keyboard (`next`, `done`, `go`…): its app hears it if it listens for
    /// `action`; if not, `next` and `previous` move the keyboard, as a browser's form does.
    pub fn action(&mut self, id: Id, action: &str) {
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("action")) {
            return self.tell(id, "action", json!({"action": action}));
        }
        let inputs = self.inputs();
        let at = inputs.iter().position(|input| *input == id);
        match action {
            "next" => self.focus(at.and_then(|at| inputs.get(at + 1)).copied()),
            "previous" => self.focus(at.and_then(|at| at.checked_sub(1)).and_then(|at| inputs.get(at)).copied()),
            _ => {}
        }
    }

    /// The pointer's press is taken from what it went down on, as a finger that pans what's under it: told as
    /// `pointercancel`, and nothing is pressed or being selected.
    pub fn pointer_cancel(&mut self, at: Point<f32>) {
        if let Some(captured) = self.captured.take() {
            let chain = self.chain_from(captured);
            self.pointer_event(&chain, "pointercancel", at, 0, point(0.0, 0.0));
        }
        self.states.pressed = None;
        self.selecting = None;
    }

    /// Selects the word at `at` in the input there, as a finger held on it does; → whether there was one.
    pub fn select_word(&mut self, at: Point<f32>) -> bool {
        let Some(id) = self.under(at).into_iter().find(|id| self.fields.contains_key(id)) else { return false };
        self.focus(Some(id));
        let index = self.drawn.get(&id).map_or(usize::MAX, |drawn| drawn.index_at(at));
        self.edit(id, |field| field.press(index.min(field.display().len()), 2, false));
        true
    }

    /// Where node `id` was drawn in the last frame.
    pub fn rect(&self, id: Id) -> Option<Rect> {
        self.rects.get(&id).copied()
    }

    /// Scrolls what node `id` is in, the innermost first, so it shows whole: the input with the keyboard, as the
    /// keyboard comes up over where it was, as a browser does. → whether anything scrolled.
    pub fn reveal(&mut self, id: Id) -> bool {
        let Some(rect) = self.rects.get(&id).copied() else { return false };
        // Room below it, as Chrome leaves.
        const ROOM: f32 = 16.0;
        let mut moved = 0.0;
        for container in self.chain_from(id).into_iter().skip(1) {
            let Some(placed) = self.placed(container) else { continue };
            let Some((most, _, false)) = placed.scroll else { continue };
            let (top, bottom) = (placed.shown.origin.y, placed.shown.origin.y + placed.shown.size.height);
            let (from, to) = (rect.y - moved, rect.y - moved + rect.height);
            let by = if to + ROOM > bottom { (to + ROOM - bottom).min(from - top) } else if from < top { from - top } else { 0.0 };
            let now = self.scrolled.entry(container).or_insert(point(0.0, 0.0));
            let was = now.y;
            now.y = (now.y + by).clamp(0.0, most);
            moved += now.y - was;
        }
        moved != 0.0
    }
}
