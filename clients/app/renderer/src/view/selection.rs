//! Text that isn't a field, selected as a browser's is (`Text::selectable`): dragged over, a word by a double click and
//! all of it by a triple, drawn with what's selected tinted, and copied by ⌘C or Ctrl+C, and on Linux put on the
//! primary selection as it's let go, as a field's is.

use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use super::{Id, Screen};
use crate::geometry::{Bounds, Point};
use crate::inspect::Key;
use crate::text::Paragraph;

/// A selectable text as it was last drawn: its words as laid out, where, how wide and grown, and how its lines go
/// along its box, to find which of its words a point is at and to draw what's selected.
pub(super) struct Shown {
    pub paragraph: Arc<Paragraph>,
    pub words: String,
    pub content: Bounds<f32>,
    pub width: f32,
    pub grown: f32,
    pub align: f32,
}

impl Shown {
    /// Where line `line` starts across its box, in the window.
    pub fn left(&self, line: usize) -> f32 {
        let width = self.paragraph.lines.get(line).map_or(0.0, |drawn| drawn.width);
        self.content.origin.x + (self.width - width).max(0.0) * self.align * self.grown
    }

    /// The byte of its words a point is at, on the line it's beside.
    fn index_at(&self, at: Point<f32>) -> usize {
        let line_height = self.paragraph.line_height * self.grown;
        let lines = self.paragraph.lines.len().max(1);
        let line = (((at.y - self.content.origin.y) / line_height).floor().max(0.0) as usize).min(lines - 1);
        let x = (at.x - self.left(line)) / self.grown;
        self.paragraph.index_on(line, x).unwrap_or(0).min(self.words.len())
    }

    /// The word, or run of spaces or punctuation, byte `index` is in.
    fn word(&self, index: usize) -> (usize, usize) {
        self.words
            .split_word_bound_indices()
            .map(|(start, word)| (start, start + word.len()))
            .find(|(start, end)| (*start..*end).contains(&index))
            .unwrap_or((index, index))
    }
}

/// What's selected of a text that isn't a field: its node, where the selection began and where it's been dragged to,
/// and the extent the press began with (a word, for a double click), which dragging keeps whole.
#[derive(Clone, Copy, Debug)]
pub(super) struct Selected {
    pub id: Id,
    began: (usize, usize),
    pub anchor: usize,
    pub focus: usize,
}

impl Selected {
    /// Its start and end, in order.
    pub fn range(&self) -> (usize, usize) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }
}

impl Screen {
    /// A main-button press `clicks` times in a row at `at` on text `id`, or on none: it begins a selection there, or
    /// lets go of the one there was.
    pub(super) fn select_down(&mut self, id: Option<Id>, at: Point<f32>, clicks: u8) {
        let Some(shown) = id.and_then(|id| self.editing.texts.get(&id)) else {
            if self.editing.selected.take().is_some() {
                self.changed();
            }
            return;
        };
        let index = shown.index_at(at);
        let began = match clicks {
            1 => (index, index),
            2 => shown.word(index),
            _ => (0, shown.words.len()),
        };
        self.editing.selected = Some(Selected { id: id.expect("it's shown"), began, anchor: began.0, focus: began.1 });
        self.editing.selecting_text = true;
        self.changed();
    }

    /// The pointer moved to `at`, with a selection's press held: it goes to there, keeping what the press began with.
    pub(super) fn select_moved(&mut self, at: Point<f32>) {
        let Some(selected) = self.editing.selected.filter(|_| self.editing.selecting_text) else { return };
        let Some(shown) = self.editing.texts.get(&selected.id) else { return };
        let index = shown.index_at(at);
        let (start, end) = selected.began;
        let (anchor, focus) = if index < start { (end, index) } else { (start, index.max(end)) };
        self.editing.selected = Some(Selected { anchor, focus, ..selected });
        self.changed();
    }

    /// A selection's press was let go: what it selected goes on Linux's primary selection, as a field's does.
    pub(super) fn select_up(&mut self) {
        if std::mem::take(&mut self.editing.selecting_text)
            && let Some(text) = self.selected_text()
        {
            self.clipboard.write(true, &text);
        }
    }

    /// What's selected of a text that isn't a field, where something is.
    pub(super) fn selected_text(&self) -> Option<String> {
        let selected = self.editing.selected?;
        let shown = self.editing.texts.get(&selected.id)?;
        let (start, end) = selected.range();
        shown.words.get(start..end.min(shown.words.len())).filter(|text| !text.is_empty()).map(str::to_owned)
    }

    /// ⌘C on a Mac, or Ctrl+C, with no field to copy from: what's selected of a text is copied. → whether it was.
    pub(super) fn copy_selected(&mut self, key: &Key) -> bool {
        let command = if crate::platform::mac_keys() { key.meta && !key.ctrl } else { key.ctrl && !key.meta };
        if !command || key.shift || key.alt || !key.key.eq_ignore_ascii_case("c") || self.focus.focused.is_some_and(|id| self.editing.fields.contains_key(&id)) {
            return false;
        }
        let Some(text) = self.selected_text() else { return false };
        self.clipboard.write(false, &text);
        true
    }
}
