//! What's typed into an input and where its text was drawn: its editing, its caret and blink, and what the platform's keyboard asks of it (`Drawn`).

use super::*;

impl Screen {
    /// How long until the caret blinks on or off: as GTK's does, on two thirds of each 1.2 s and off a third, until
    /// it's been still 10 s, when it stays on. None while the app's clock is paused, or no input has the keyboard.
    pub(super) fn blinks(&self) -> Option<Duration> {
        (!self.js.clock().paused()).then(|| self.blink_due()).flatten().map(|ms| Duration::from_secs_f64(ms / 1000.0))
    }

    /// When the caret next blinks on or off, in ms on the app's clock, which a harness steps as a timer (`timer_due`),
    /// where no input has the keyboard.
    pub(crate) fn blink_due(&self) -> Option<f64> {
        let typing = self.active && self.focus.focused.is_some_and(|id| self.editing.fields.contains_key(&id));
        let since = self.js.clock().now() - self.editing.blink;
        if !typing || since >= BLINK_STOPS {
            return None;
        }
        let into = since % BLINK;
        // At least a microsecond on, so a clock a hair short of a blink isn't stepped by nothing, again and again.
        Some((if into < BLINK * 2.0 / 3.0 { BLINK * 2.0 / 3.0 - into } else { BLINK - into }).max(0.001))
    }

    /// Whether the caret shows now.
    pub(super) fn caret_shows(&self) -> bool {
        let since = self.js.clock().now() - self.editing.blink;
        self.active && (since >= BLINK_STOPS || since % BLINK < BLINK * 2.0 / 3.0)
    }

    /// What a key does in input `id`: typing, moving, selecting, undoing and the clipboard, by the editing model
    /// (`editing::Field::key`), then its `input` if that changed it and its `action` if it was the action key.
    pub(super) fn type_key(&mut self, id: Id, key: &Key) {
        let Some(field) = self.editing.fields.get_mut(&id) else { return };
        let before = field.value();
        let outcome = field.key(key, self.editing.drawn.get(&id).map(|drawn| drawn as &dyn editing::Layout));
        if let Some(copy) = outcome.copy.filter(|copy| !copy.is_empty()) {
            self.clipboard.write(false, &copy);
        }
        if outcome.paste
            && let Some(text) = self.clipboard.read(false)
        {
            field.paste(&text);
        }
        self.edited(id, before);
        if outcome.action && self.tree.nodes.get(&id).is_some_and(|node| node.listens("action")) {
            let node = &self.tree.nodes[&id];
            let action = node.str("action").unwrap_or(if node.is("multiline") { "newline" } else { "done" }).to_owned();
            self.tell(id, "action", json!({"action": action}));
        }
    }

    /// Whether an input holds something its person typed that its app hasn't taken (`Field::typed`).
    pub fn typed(&self) -> bool {
        self.editing.fields.values().any(Field::typed) || self.frames.values().any(|child| child.screen.typed())
    }

    /// The editing state of the input with the keyboard, for a platform's keyboard bridge (`editing`).
    pub fn editing(&self) -> Option<editing::Value> {
        if let Some(child) = self.focus.focused.and_then(|id| self.frames.get(&id)) {
            return child.screen.editing();
        }
        self.focus.focused.and_then(|id| self.editing.fields.get(&id)).map(Field::value)
    }

    /// The character of the input with the keyboard nearest to `at`, in the window, in UTF-16 units of its
    /// `editing` text, as AppKit's `characterIndexForPoint:` asks.
    pub fn index_at(&self, at: Point<f32>) -> Option<usize> {
        let id = self.focus.focused?;
        if let Some(child) = self.frames.get(&id) {
            return child.screen.index_at(point(at.x - child.rect.origin.x, at.y - child.rect.origin.y));
        }
        let (field, drawn) = (self.editing.fields.get(&id)?, self.editing.drawn.get(&id)?);
        let index = drawn.index_at(at);
        // What's drawn is its editing text, what's composed in it too, but for a password's dots.
        Some(if field.secure { editing::utf16(&field.text, field.to_model(index)) } else { editing::utf16(&field.display(), index) })
    }

    /// Where the input with the keyboard has its caret, in the window, for the platform's input method to show what
    /// it composes beside: none where it takes no typing (a password takes no input method).
    pub fn caret_rect(&self) -> Option<Bounds<f32>> {
        if let Some(caret) = self.frame_caret() {
            return caret;
        }
        let id = self.focus.focused?;
        let field = self.editing.fields.get(&id).filter(|field| !field.secure && !field.readonly)?;
        let drawn = self.editing.drawn.get(&id)?;
        let (line, x) = field.placed().unwrap_or_else(|| editing::Layout::caret(drawn, field.caret(), field.upstream()));
        Some(drawn.line_rect(line, x, x + 1.0))
    }

    /// Does `change` to input `id`'s editing, and tells its app what changed.
    pub(super) fn edit(&mut self, id: Id, change: impl FnOnce(&mut Field)) {
        let Some(field) = self.editing.fields.get_mut(&id) else { return };
        let before = field.value();
        change(field);
        self.edited(id, before);
    }

    /// Input `id`'s editing changed from `before`, or perhaps not: its caret shows again, and its app hears its
    /// editing state (`input`) if it's different, with its text as `value` too, as the DOM's event has it.
    pub(super) fn edited(&mut self, id: Id, before: editing::Value) {
        self.editing.blink = self.js.clock().now();
        let Some(value) = self.editing.fields.get(&id).map(Field::value).filter(|value| *value != before) else { return };
        if self.tree.nodes.get(&id).is_some_and(|node| node.listens("input")) {
            self.tell(id, "input", json!({"editing": value, "value": value.text}));
            self.editing.edits.insert(id, self.js.sent());
        }
    }

    /// Types `text` into input `id`, a character at a time, each an `input`.
    pub(crate) fn type_text(&mut self, id: Id, text: &str) {
        let mut buffer = [0; 4];
        for typed in text.chars() {
            self.edit(id, |field| field.insert(typed.encode_utf8(&mut buffer)));
        }
    }

    /// Each input's editing as its node says, after the app's mutations: what it is (several lines, a password, read
    /// only, what Return does), and its editing state where the app set a new one.
    pub(super) fn sync_fields(&mut self) {
        let tree = &self.tree;
        let input = |id: &Id| tree.nodes.get(id).is_some_and(|node| node.kind == Kind::Input);
        self.editing.fields.retain(|id, _| input(id));
        self.editing.set.retain(|id, _| input(id));
        self.editing.edits.retain(|id, _| input(id));
        self.editing.drawn.retain(|id, _| input(id));
        self.editing.texts.retain(|id, _| tree.nodes.contains_key(id));
        if self.editing.selected.is_some_and(|selected| !tree.nodes.contains_key(&selected.id)) {
            self.editing.selected = None;
        }
        for id in tree.inputs() {
            let node = &tree.nodes[&id];
            let field = self.editing.fields.entry(id).or_default();
            field.multiline = node.is("multiline");
            field.newline = field.multiline && node.str("action").is_none_or(|action| action == "newline");
            field.secure = node.str("type") == Some("password");
            field.readonly = node.is("readonly");
            let set = node.prop("editing").and_then(|editing| serde_json::from_value::<editing::Value>(editing.clone()).ok());
            if let Some(set) = set.filter(|set| self.editing.set.get(&id) != Some(set)) {
                if self.editing.edits.get(&id).is_none_or(|told| *told <= self.editing.heard) {
                    field.set(&set);
                }
                self.editing.set.insert(id, set);
            }
        }
    }
}

/// A caret's blink, in milliseconds on the app's clock: on two thirds of it and off a third, as GTK's, until it's been
/// still `BLINK_STOPS`.
const BLINK: f64 = 1200.0;
const BLINK_STOPS: f64 = 10_000.0;

/// Where an input's text was drawn: its paragraph (of its display text, or its placeholder while it's `empty`), its
/// first line's top left in the window, where each line starts along the paragraph, how much it's grown, and how far
/// it has scrolled, along one line or down several. Its `Layout` is the paragraph's, unscrolled.
pub(crate) struct Drawn {
    pub(super) paragraph: Paragraph,
    pub(super) empty: bool,
    pub(super) origin: Point<f32>,
    pub(super) offsets: Vec<f32>,
    pub(super) grown: f32,
    pub(super) scroll: Point<f32>,
    /// Whether it had the keyboard, where its caret was and how long its text was, to scroll to the caret only as
    /// it moves.
    pub(super) seen: (bool, usize, bool, usize),
}

impl Drawn {
    /// Where the caret before byte `index` of its display text is drawn, in the window: a line tall, a point wide.
    pub(super) fn caret_at(&self, index: usize, upstream: bool) -> Bounds<f32> {
        let (line, x) = editing::Layout::caret(self, index, upstream);
        self.line_rect(line, x, x + 1.0)
    }

    /// Where bytes `start..end` of its display text are drawn, in the window: a rectangle on each line they're on.
    pub(super) fn spans(&self, start: usize, end: usize) -> Vec<Bounds<f32>> {
        self.paragraph.spans(start, end).into_iter().map(|(line, x0, x1)| self.line_rect(line, x0 + self.offsets.get(line).copied().unwrap_or(0.0), x1 + self.offsets.get(line).copied().unwrap_or(0.0))).collect()
    }

    /// Line `line` from `x0` to `x1` along its paragraph, in the window.
    fn line_rect(&self, line: usize, x0: f32, x1: f32) -> Bounds<f32> {
        let height = self.paragraph.line_height * self.grown;
        Bounds { origin: point(self.origin.x + (x0 - self.scroll.x) * self.grown, self.origin.y + line as f32 * height), size: Size { width: (x1 - x0) * self.grown, height } }
    }

    /// The byte of its display text nearest to `at`, in the window.
    pub(super) fn index_at(&self, at: Point<f32>) -> usize {
        self.hit(at).0
    }

    /// The byte of its display text nearest to `at`, in the window, and whether the caret there is upstream: at the
    /// end of the line `at` is on, where it wraps with no space to hang.
    pub(super) fn hit(&self, at: Point<f32>) -> (usize, bool) {
        if self.empty {
            return (0, false);
        }
        let y = (at.y - self.origin.y) / self.grown;
        let line = ((y / self.paragraph.line_height).floor().max(0.0) as usize).min(self.paragraph.lines.len().saturating_sub(1));
        let x = (at.x - self.origin.x) / self.grown + self.scroll.x;
        let index = editing::Layout::index_on(self, line, x).unwrap_or(0);
        (index, self.paragraph.caret(index, false).0 != line)
    }
}

impl editing::Layout for Drawn {
    fn caret(&self, index: usize, upstream: bool) -> (usize, f32) {
        if self.empty {
            return (0, 0.0);
        }
        let (line, x) = self.paragraph.caret(index, upstream);
        (line, x + self.offsets.get(line).copied().unwrap_or(0.0))
    }

    fn index_on(&self, line: usize, x: f32) -> Option<usize> {
        if self.empty {
            return (line == 0).then_some(0);
        }
        self.paragraph.index_on(line, x - self.offsets.get(line).copied().unwrap_or(0.0))
    }

    fn line_edge(&self, line: usize, end: bool) -> usize {
        match self.paragraph.lines.get(line) {
            _ if self.empty => 0,
            Some(_) if end => self.paragraph.end(line),
            Some(drawn) => drawn.start,
            None => 0,
        }
    }

    fn bidi(&self) -> bool {
        !self.empty && self.paragraph.lines.iter().any(|line| line.rtl || line.glyphs.iter().any(|(_, _, glyph)| glyph.rtl))
    }

    fn rtl(&self, line: usize) -> bool {
        self.paragraph.lines.get(line).is_some_and(|line| line.rtl)
    }

    fn stops(&self, line: usize) -> Vec<editing::Stop> {
        let (Some(drawn), false) = (self.paragraph.lines.get(line), self.empty) else { return Vec::new() };
        let offset = self.offsets.get(line).copied().unwrap_or(0.0);
        drawn
            .glyphs
            .iter()
            .flat_map(|(_, _, glyph)| {
                let (left, right) = if glyph.rtl { (glyph.end, glyph.index) } else { (glyph.index, glyph.end) };
                let x = offset + glyph.position.x.0;
                [editing::Stop { x, index: left, right: false }, editing::Stop { x: x + glyph.advance, index: right, right: true }]
            })
            .collect()
    }
}
