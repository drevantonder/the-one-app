//! What a platform asks of a screen beyond a pointer and keys: its keyboard bridge (Flutter's text input plugin, on
//! Android through its `InputConnection`, on a Mac and Windows through their input methods), which keeps the input's
//! whole editing state itself and says it back (`editing::TextInputClient`), and is told of the input with the
//! keyboard as Flutter's framework tells it (`TextInputConnection`); a phone's autofill, which fills any field; its
//! screen reader, which presses, focuses and scrolls by node; and a finger, which pans what scrolls and may be lifted
//! off what it pressed.

use serde_json::{Value, json};

use super::Screen;
use crate::editing::{self, TextInput, TextInputClient};
use crate::geometry::{Bounds, Point, Size, point};
use crate::inspect::Rect;
use crate::tree::{Id, Kind};

/// What a window's text input plugin was last told, as Flutter's framework keeps its `TextInputConnection`: the input
/// it's attached to, that input's editing state as the plugin has it, and where its caret is, so only what changed is
/// told again.
#[derive(Default)]
pub struct TextInputConnection {
    client: Option<Id>,
    value: Option<editing::Value>,
    caret: Option<Bounds<f32>>,
    /// Where the input is in the window, as its size and transform last said.
    rect: Option<Rect>,
    origin: Point<f32>,
}

impl Screen {
    /// What window's text input plugin is to be told, as the input with the keyboard, its editing state or its caret
    /// changed since `connection` was: attached to an input that takes typing as it takes the keyboard, detached as it
    /// loses it, as Flutter's `TextInput.attach` and `TextInputConnection.close` call it.
    pub fn text_input(&self, connection: &mut TextInputConnection) -> Vec<TextInput> {
        let typing = self.focus.focused.filter(|id| self.editing.fields.get(id).is_some_and(|field| !field.readonly));
        let mut calls = Vec::new();
        if connection.client != typing {
            if connection.client.is_some() {
                calls.extend([TextInput::ClearClient, TextInput::Hide]);
            }
            *connection = TextInputConnection::default();
            if let (Some(id), Some(field), Some(value)) = (typing, typing.and_then(|id| self.field(id)), self.editing()) {
                // An input with an autofill hint is in a group with every other on the screen, which a password manager
                // fills together, as Flutter's `TextInputConfiguration.fields`.
                let mut configuration = editing::configuration(&field, &value);
                if configuration.get("autofill").is_some() {
                    let fields: Vec<Value> = self
                        .inputs()
                        .into_iter()
                        .filter_map(|other| Some(editing::configuration(&self.field(other)?, &self.editing.fields.get(&other)?.value())))
                        .filter(|other| other.get("autofill").is_some())
                        .collect();
                    configuration["fields"] = json!(fields);
                }
                calls.extend([
                    TextInput::SetClient(id, configuration),
                    TextInput::SetEditingState(value.clone()),
                    TextInput::Show,
                ]);
                (connection.client, connection.value) = (Some(id), Some(value));
            }
        }
        let Some(client) = connection.client else { return calls };
        // Where it is, as it opens and each time it moves (a sheet sliding up, what it's in scrolled): its size and
        // the transform from its own coordinates to the window's, where a platform puts what's typed through (the web's
        // real field, an input method's window), before it's shown.
        let rect = self.rects.get(&client).copied().unwrap_or(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
        if connection.rect != Some(rect) {
            let transform = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, rect.x, rect.y, 0.0, 1.0];
            let show = calls.iter().position(|call| matches!(call, TextInput::Show)).unwrap_or(calls.len());
            calls.insert(show, TextInput::SetEditableSizeAndTransform { width: rect.width, height: rect.height, transform });
            (connection.rect, connection.origin) = (Some(rect), point(rect.x, rect.y));
        }
        let value = self.editing();
        if value.is_some() && connection.value != value {
            calls.push(TextInput::SetEditingState(value.clone().expect("checked")));
            connection.value = value;
        }
        // In the input's own coordinates.
        let origin = connection.origin;
        let caret = self.caret_rect().map(|caret| Bounds { origin: point(caret.origin.x - origin.x, caret.origin.y - origin.y), size: caret.size });
        if connection.caret != caret {
            if let Some(caret) = caret {
                calls.extend([TextInput::SetCaretRect(caret), TextInput::SetMarkedTextRect(caret)]);
            }
            connection.caret = caret;
        }
        calls
    }

    /// What window's text input plugin says, to the input it's attached to (`connection`): its whole editing state,
    /// which it then has, or its action key.
    pub fn text_input_client(&mut self, connection: &mut TextInputConnection, said: TextInputClient) {
        match said {
            TextInputClient::UpdateEditingState(client, value) if connection.client == Some(client) => {
                self.set_editing(client, &value);
                connection.value = Some(value);
            }
            TextInputClient::PerformAction(client, action) if connection.client == Some(client) => {
                self.action(client, action.trim_start_matches("TextInputAction."));
            }
            TextInputClient::DismissSystemContextMenu(client) if connection.client == Some(client) => self.text_menu_closed(),
            TextInputClient::PerformCustomAction(client, action) if connection.client == Some(client) && action == "select" => self.select_at_caret(),
            TextInputClient::UpdateEditingStateWithTag(client, filled) if connection.client == Some(client) => {
                for (id, value) in filled {
                    if self.editing.fields.contains_key(&id) {
                        self.set_editing(id, &value);
                    }
                }
            }
            _ => {}
        }
    }

    /// What has the keyboard.
    pub fn focused(&self) -> Option<Id> {
        self.focus.focused
    }

    /// Gives the keyboard to `id`, or takes it from what has it, as a screen reader or a keyboard's Next asks.
    pub fn set_focused(&mut self, id: Option<Id>) {
        self.focus(id);
    }

    /// A keyboard that keeps the editing state itself says input `id`'s is now `value` (what it typed, selected and is
    /// composing), or a password manager filled it: taken as it is, and its app told.
    pub fn set_editing(&mut self, id: Id, value: &editing::Value) {
        let Some(before) = self.value(id) else { return };
        self.edit(id, |field| field.keyboard(value));
        self.overlay_keyboard(id, &before);
    }

    /// Input `id`'s editing state.
    pub fn value(&self, id: Id) -> Option<editing::Value> {
        self.editing.fields.get(&id).map(editing::Field::value)
    }

    /// The inputs, in the order Tab goes, as last painted.
    pub fn inputs(&self) -> Vec<Id> {
        self.focus.tab_order.iter().copied().filter(|id| self.editing.fields.contains_key(id)).collect()
    }

    /// Whether what a press at `at` presses is an input, which a finger gives the keyboard only as it lifts.
    pub fn input_at(&self, at: Point<f32>) -> bool {
        self.under(at).into_iter().find(|id| self.placed(*id).is_some_and(|placed| placed.pressable)).is_some_and(|id| self.editing.fields.contains_key(&id))
    }

    /// What a keyboard bridge needs to know of input `id`: what kind of text it takes (`type`, `inputmode`), what may
    /// fill it (`autocomplete`), how a keyboard capitalises, corrects and checks it, its action key, its name, and where it is.
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
            "autocorrect": node.prop("autocorrect").is_none_or(|autocorrect| autocorrect != &json!(false) && autocorrect != &json!("off")),
            "multiline": multiline,
            "readonly": node.is("readonly"),
            "action": node.str("action").unwrap_or(if multiline { "newline" } else { "done" }),
            "label": node.str("aria-label").or_else(|| node.str("placeholder")),
            "rect": {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height},
            "text": self.editing.fields.get(&id).map(|field| field.value().text),
        }))
    }

    /// Input `id`'s action key was pressed on the keyboard (`next`, `done`, `go`…): its app hears it if it listens for
    /// `action` (`onSubmitEditing`, as Return is on the DOM); if not, `next` and `previous` move the keyboard, as a
    /// browser's form does. Under a finger, a key that ends the editing (Done, Go, Search, Send) puts the keyboard
    /// away once its app has heard it, as Flutter's text fields have it by default and iOS's own do.
    pub fn action(&mut self, id: Id, action: &str) {
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("action")) {
            self.tell(id, "action", json!({"action": action}));
            if self.touch && matches!(action, "done" | "go" | "search" | "send") && self.focus.focused == Some(id) {
                self.focus(None);
            }
            return;
        }
        if self.touch && matches!(action, "done" | "go" | "search" | "send") {
            return self.focus(None);
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
        if !self.swiping() {
            self.drag_up(at, true);
        }
        if let Some(captured) = self.captured.take() {
            let chain = self.chain_from(captured);
            self.pointer_event(&chain, "pointercancel", at, 0, None);
        }
        self.states.pressed = None;
        self.editing.selecting = None;
    }

    /// Where the rail's line is drawn, while it shows (`rail`): iOS's home indicator on its side, 4 by 64 points, in the
    /// middle of the right edge, 6 in from what the safe area leaves, as the window page's is (client.css's `#line`).
    /// It's away while something is drawn over the page (a sheet, a menu: a layer) and while it's pulled.
    pub fn line(&self) -> Option<Bounds<f32>> {
        if !self.rail || self.pulled || self.layer {
            return None;
        }
        Some(Bounds { origin: point(self.room.width - self.insets.right - 10.0, self.room.height / 2.0 - 32.0), size: Size { width: 4.0, height: 64.0 } })
    }

    /// Whether a finger landing at `at` takes the rail's line: a fingertip's width in from the edge, and taller than the
    /// line, as the window page's is.
    pub fn on_line(&self, at: Point<f32>) -> bool {
        self.line().is_some() && at.x >= self.room.width - self.insets.right - 24.0 && (at.y - self.room.height / 2.0).abs() <= 56.0
    }

    /// The device's Back: what's open over its page shuts, as the page's Escape would (`notifications/one/back`).
    /// → whether something was.
    /// What's open over its page, if anything, and whether that's drawn over it (a sheet, a menu) rather than a screen
    /// of its own.
    pub fn covered(&self) -> Option<bool> {
        self.over.then_some(self.layer)
    }

    pub fn back(&mut self) -> bool {
        if !self.over {
            return false;
        }
        self.message(&serde_json::json!({"jsonrpc": "2.0", "method": "notifications/one/back"}));
        true
    }

    /// A finger moved past the slop, keeping its moves (`pans`): it's no tap, so what it pressed isn't pressed, and
    /// isn't clicked as it lifts, as a browser clicks only on a tap.
    pub fn dragged(&mut self) {
        if self.states.pressed.take().is_some() {
            self.changed();
        }
    }

    /// Where node `id` was drawn in the last frame.
    pub fn rect(&self, id: Id) -> Option<Rect> {
        let chrome = match id {
            super::NOTICE_ACTION => self.chrome.action,
            _ => return self.rects.get(&id).copied(),
        };
        chrome.map(|bounds| Rect { x: bounds.origin.x, y: bounds.origin.y, width: bounds.size.width, height: bounds.size.height })
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
            // What the window keeps at its foot (a keyboard, a home indicator) covers what scrolls under it.
            let (top, bottom) = (placed.shown.origin.y, (placed.shown.origin.y + placed.shown.size.height).min(self.room.height - self.insets.bottom));
            let (from, to) = (rect.y - moved, rect.y - moved + rect.height);
            let by = if to + ROOM > bottom { (to + ROOM - bottom).min(from - top) } else if from < top { from - top } else { 0.0 };
            let now = self.scrolled.entry(container).or_insert(point(0.0, 0.0));
            let was = now.y;
            now.y = (now.y + by).clamp(0.0, most);
            moved += now.y - was;
            let scrolled = *now;
            if scrolled.y != was && self.tree.nodes.get(&container).is_some_and(|node| node.listens("scroll")) {
                self.tell(container, "scroll", json!({"scrollTop": scrolled.y, "scrollLeft": scrolled.x}));
            }
        }
        moved != 0.0
    }
}
