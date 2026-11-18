// The text editing model from Rune (github.com/anomalyco/rune, crates/rune/src/edit.rs, edit/input.rs,
// edit/caret_motion.rs, edit/history.rs, edit/ime.rs and runtime/selection/{click,selectable}.rs) at commit
// 78efd4a7876b5478dd72033ce6fd2b2f13ab9f9a, cut down to plain text for the strict protocol's `input`: a caret and a
// selection, grapheme and word steps, undo and redo grouped as Chromium's editor groups them, an input method's
// composition, a password's mask, and pointer selection by character, word and line. Not its rich text (marks,
// blocks, pills), its visual (bidi) arrow motion, or its parley layouts: a field's layout is ours (`Layout`).
// Copyright (c) 2026 Anomaly. Licensed under the MIT License:
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the Software without restriction, including without limitation the
// rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to
// permit persons to whom the Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or substantial portions of
// the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
// THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
// TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//
// This file is changed from the original.

//! A text input's editing: what's typed, selected and composed, as every platform's keyboard bridge says it
//! (`Value`, Flutter's `TextEditingValue`, the strict protocol's `Editing`), and what keys, an input method, a paste
//! and a pointer do to it (`Field`). Offsets in a `Field` are bytes of its text, on character boundaries; a `Value`'s
//! are UTF-16 code units, as JS and Android count them.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::inspect::Key;

/// An input's editing state, as Flutter's `TextEditingValue` and the strict protocol's `Editing` have it: its text,
/// what's selected, and what an input method is composing, which is in the text. Offsets in UTF-16 code units.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Value {
    pub text: String,
    pub selection: Selection,
    pub composing: Option<Composing>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct Selection {
    pub base: usize,
    pub extent: usize,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Composing {
    pub start: usize,
    pub end: usize,
}

/// What a platform's input method says (dead keys, Chinese and Japanese input, the emoji picker, dictation), as
/// winit and Rune have it. Plain typing arrives as keys.
#[derive(Clone, Debug, PartialEq)]
pub enum Ime {
    /// The composition so far, drawn at the caret and not yet committed, with where its cursor is in it, in bytes.
    /// Empty text ends it.
    Preedit(String, Option<usize>),
    /// Text that replaces the composition, or the selection where there's none.
    Commit(String),
}

/// The computer's clipboards: the one copy and paste use, and on Linux the primary selection, which what's selected
/// goes on and a middle click pastes.
pub trait Clipboard {
    fn read(&mut self, primary: bool) -> Option<String>;
    fn write(&mut self, primary: bool, text: &str);
}

/// A clipboard kept in memory, for a screen with no computer's clipboard (the headless harness).
#[derive(Default)]
pub struct Memory([Option<String>; 2]);

impl Clipboard for Memory {
    fn read(&mut self, primary: bool) -> Option<String> {
        self.0[usize::from(primary)].clone()
    }

    fn write(&mut self, primary: bool, text: &str) {
        self.0[usize::from(primary)] = Some(text.to_owned());
    }
}

/// Where a field's text is drawn, in its display text's bytes (`Field::display`), for the moves that depend on how
/// it's drawn: up and down, a line's ends, and the arrows across text that runs both ways. Along a line is from where
/// its paragraph starts, in its own pixels.
pub trait Layout {
    /// The line byte `index` is on, and how far along it the caret before it is drawn.
    fn caret(&self, index: usize) -> (usize, f32);
    /// The byte nearest to `x` along line `line`, or `None` if there's no such line.
    fn index_on(&self, line: usize, x: f32) -> Option<usize>;
    /// Where the line byte `index` is on begins or ends.
    fn line_edge(&self, index: usize, end: bool) -> usize;
    /// Text runs right to left somewhere in it, so the arrows move across it as it's drawn.
    fn bidi(&self) -> bool;
    /// Line `line`'s paragraph runs right to left.
    fn rtl(&self, line: usize) -> bool;
    /// Where a caret can be on line `line`: each glyph's left and right edge.
    fn stops(&self, line: usize) -> Vec<Stop>;
}

/// A place a caret can be: `x` along its line, before byte `index`, at a glyph's right edge or its left. Where runs
/// of either direction meet, two bytes have a place at one `x`, as Rune's carets have an affinity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub x: f32,
    pub index: usize,
    pub right: bool,
}

/// What a key asks of whoever holds the field.
#[derive(Default, Debug, PartialEq)]
pub struct Outcome {
    /// The action key was pressed (Return, but where it makes a new line): the input's `action`.
    pub action: bool,
    /// Put this on the clipboard (copy, cut).
    pub copy: Option<String>,
    /// Paste the clipboard here (`Field::paste`).
    pub paste: bool,
}

/// What a pointer's press selects by: a character, or with a double or triple click a word or a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Unit {
    #[default]
    Char,
    Word,
    Paragraph,
}

/// An input's text as it's edited.
#[derive(Default)]
pub struct Field {
    pub text: String,
    anchor: usize,
    focus: usize,
    /// What an input method is composing, at the caret, not yet part of `text`, and where its cursor is in it.
    preedit: Option<(String, usize)>,
    history: History,
    /// The x a run of Up and Down keeps, in its layout's pixels, so a caret passing a shorter line comes back to
    /// its column, as a browser's does. Any other key lets it go.
    column: Option<f32>,
    /// Where an arrow across text that runs both ways left the caret, `(focus, line, x)`, which is where it's drawn
    /// while the focus is still there: one of the byte's two places where runs meet.
    place: Option<(usize, usize, f32)>,
    /// What the press that started a pointer's selection selects by, and the range it began on, which a drag keeps
    /// whole.
    unit: Unit,
    origin: (usize, usize),
    /// More than one line, as a `<textarea>`.
    pub multiline: bool,
    /// Return makes a new line, not its action.
    pub newline: bool,
    /// A password: drawn as dots, never copied, and nothing recorded to undo.
    pub secure: bool,
    /// Selected and copied, but not typed in.
    pub readonly: bool,
}

/// The previous grapheme boundary, so a caret or backspace takes a ZWJ emoji, flag or skin tone whole.
fn prev_char(s: &str, i: usize) -> usize {
    s[..i].grapheme_indices(true).next_back().map_or(0, |(j, _)| j)
}

/// Where Backspace deletes back to from `i`, as Chromium's `BackspaceStateMachine` and native fields do: one code
/// point (a combining accent, a Thai or Indic vowel sign, the last jamo of a decomposed syllable), so a typist fixes
/// the last thing they typed without retyping the cluster; but a CR LF and an emoji sequence (a variation selector, a
/// skin tone, a keycap, a flag's pair, a tag sequence, a ZWJ sequence) go whole.
fn prev_backspace(s: &str, i: usize) -> usize {
    let start = prev_char(s, i);
    let cluster = &s[start..i];
    let emoji = cluster
        .chars()
        .any(|c| matches!(c as u32, 0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0x1F1E6..=0x1F1FF | 0xE0020..=0xE007F))
        || (cluster.contains('\u{200D}') && cluster.chars().any(|c| c as u32 >= 0x1F000));
    if cluster == "\r\n" || emoji {
        return start;
    }
    s[..i].char_indices().next_back().map_or(0, |(j, _)| j)
}

fn next_char(s: &str, i: usize) -> usize {
    s[i..].graphemes(true).next().map_or(i, |g| i + g.len())
}

/// Byte `index` of `text` as UTF-16 code units, as JS counts it.
pub fn utf16(text: &str, index: usize) -> usize {
    text[..index.min(text.len())].encode_utf16().count()
}

/// UTF-16 offset `units` of `text` as a byte, on the character boundary at or before it.
pub fn byte(text: &str, units: usize) -> usize {
    let mut counted = 0;
    for (at, c) in text.char_indices() {
        if counted + c.len_utf16() > units {
            return at;
        }
        counted += c.len_utf16();
    }
    text.len()
}

/// What a password's dot is, one for each character typed.
pub const DOT: char = '•';

impl Field {
    pub fn new(text: &str) -> Field {
        Field { text: text.to_owned(), anchor: text.len(), focus: text.len(), ..Field::default() }
    }

    /// Its editing state, in the protocol's shape: what it composes is in its text, from the caret.
    pub fn value(&self) -> Value {
        let (a, _) = self.range();
        match &self.preedit {
            Some((preedit, cursor)) => {
                let text = format!("{}{preedit}{}", &self.text[..a], &self.text[a..]);
                let caret = utf16(&text, a + cursor);
                Value {
                    selection: Selection { base: caret, extent: caret },
                    composing: Some(Composing { start: utf16(&text, a), end: utf16(&text, a + preedit.len()) }),
                    text,
                }
            }
            None => Value {
                selection: Selection { base: utf16(&self.text, self.anchor), extent: utf16(&self.text, self.focus) },
                composing: None,
                text: self.text.clone(),
            },
        }
    }

    /// What its app set its editing state to: kept as it is if it's the same text, as the DOM's `value` setter
    /// leaves the caret where it was when it's given what it has, so an app that sets back what it was told moves
    /// nothing; any other text replaces it, with the selection it says, its history gone, as Rune's `replace_text`.
    pub fn set(&mut self, value: &Value) {
        if value.text == self.value().text {
            return;
        }
        self.text = value.text.clone();
        let clamp = |units: usize| byte(&value.text, units);
        self.anchor = clamp(value.selection.base);
        self.focus = clamp(value.selection.extent);
        self.preedit = None;
        self.history = History::default();
        self.column = None;
        self.place = None;
    }

    /// The selection as ordered byte offsets.
    pub fn range(&self) -> (usize, usize) {
        let t = &self.text;
        (t.floor_char_boundary(self.anchor.min(self.focus)), t.floor_char_boundary(self.anchor.max(self.focus)))
    }

    pub fn selected(&self) -> &str {
        let (a, b) = self.range();
        &self.text[a..b]
    }

    pub fn composing(&self) -> bool {
        self.preedit.is_some()
    }

    /// The text as it's drawn: dots for a password, and what's being composed at the caret.
    pub fn display(&self) -> String {
        if self.secure {
            return DOT.to_string().repeat(self.text.chars().count());
        }
        match &self.preedit {
            Some((preedit, _)) => {
                let (a, _) = self.range();
                format!("{}{preedit}{}", &self.text[..a], &self.text[a..])
            }
            None => self.text.clone(),
        }
    }

    /// Byte `i` of its text as a byte of its display text.
    pub fn to_display(&self, i: usize) -> usize {
        if self.secure {
            return self.text[..i.min(self.text.len())].chars().count() * DOT.len_utf8();
        }
        match &self.preedit {
            Some((preedit, _)) if i > self.range().0 => i + preedit.len(),
            _ => i,
        }
    }

    /// Byte `j` of its display text as a byte of its text.
    pub fn to_model(&self, j: usize) -> usize {
        if self.secure {
            return self.text.char_indices().nth(j / DOT.len_utf8()).map_or(self.text.len(), |(at, _)| at);
        }
        match &self.preedit {
            Some((preedit, _)) => {
                let caret = self.range().0;
                if j <= caret {
                    j
                } else if j < caret + preedit.len() {
                    caret
                } else {
                    j - preedit.len()
                }
            }
            None => j.min(self.text.len()),
        }
    }

    /// Where the caret was put by an arrow across text that runs both ways: its line and how far along it, in its
    /// `Layout`'s pixels. Otherwise it's drawn before its byte (`caret`).
    pub fn placed(&self) -> Option<(usize, f32)> {
        self.place.filter(|place| place.0 == self.focus && self.preedit.is_none()).map(|(_, line, x)| (line, x))
    }

    /// Where the caret is drawn, in its display text: in what's being composed, at its cursor.
    pub fn caret(&self) -> usize {
        match &self.preedit {
            Some((_, cursor)) => self.range().0 + cursor,
            None => self.to_display(self.focus),
        }
    }

    /// What's selected, in its display text.
    pub fn selection(&self) -> (usize, usize) {
        let (a, b) = self.range();
        (self.to_display(a), self.to_display(b))
    }

    /// What's being composed, in its display text.
    pub fn composed(&self) -> Option<(usize, usize)> {
        let (preedit, _) = self.preedit.as_ref()?;
        let at = self.range().0;
        Some((at, at + preedit.len()))
    }

    pub fn select_all(&mut self) {
        self.history.run = Run::Closed;
        self.anchor = 0;
        self.focus = self.text.len();
    }

    /// It lost the keyboard: what's being composed is kept, and the open undo step closes.
    pub fn blur(&mut self) {
        self.finish_composition();
        self.history.run = Run::Closed;
    }

    /// Writes what's composed into the text and ends the composition, as a field that loses the keyboard mid-way
    /// keeps it (a browser's field and an `NSTextView` do).
    pub fn finish_composition(&mut self) {
        if let Some((preedit, _)) = self.preedit.take() {
            self.commit(&preedit);
        }
    }

    /// Text typed, pasted or committed. One line drops control characters, line breaks too, as `<input type=text>`
    /// does; several fold CR LF to LF.
    fn sanitize(&self, s: &str) -> String {
        if !self.multiline {
            s.chars().filter(|c| !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}')).collect()
        } else {
            s.replace("\r\n", "\n").replace('\r', "\n")
        }
    }

    fn set_focus(&mut self, i: usize, extend: bool) {
        self.focus = i;
        if !extend {
            self.anchor = i;
        }
    }

    /// Replaces bytes `a..b` with `s`, the caret after it, recording it to undo as `kind` joins the open step.
    fn replace(&mut self, (a, b): (usize, usize), s: &str, kind: EditKind) {
        if self.readonly {
            return;
        }
        self.column = None;
        self.place = None;
        if &self.text[a..b] == s {
            (self.anchor, self.focus) = (a + s.len(), a + s.len());
            return;
        }
        let before = self.snapshot();
        let into_empty = self.text.is_empty();
        self.text.replace_range(a..b, s);
        (self.anchor, self.focus) = (a + s.len(), a + s.len());
        if !self.secure {
            let after = self.snapshot();
            self.history.record(before, after, kind, into_empty);
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), anchor: self.anchor, focus: self.focus }
    }

    fn restore(&mut self, to: &Snapshot) {
        self.text = to.text.clone();
        self.place = None;
        (self.anchor, self.focus) = (to.anchor, to.focus);
    }

    /// Writes a composition's text over the selection as typing: it joins the open undo step, as each Hangul
    /// syllable or kana phrase a person commits is part of what they type.
    fn commit(&mut self, s: &str) {
        let clean = self.sanitize(s);
        if clean.is_empty() {
            return;
        }
        let kind = if clean.contains('\n') { EditKind::LineBreak } else { EditKind::Text };
        self.replace(self.range(), &clean, kind);
    }

    /// Inserts `s` where the selection is, as typing does.
    pub fn insert(&mut self, s: &str) {
        self.commit(s);
    }

    pub fn paste(&mut self, s: &str) {
        let s = self.sanitize(s);
        self.history.run = Run::Closed;
        self.replace(self.range(), &s, EditKind::Other);
    }

    /// What the input method says. A password shows nothing being composed, but takes what's committed, as a dead
    /// key's é.
    pub fn ime(&mut self, ime: Ime) {
        if self.readonly || (self.secure && matches!(ime, Ime::Preedit(..))) {
            self.preedit = None;
            return;
        }
        match ime {
            Ime::Preedit(text, cursor) => {
                // A composition starting over a selection replaces it, as a browser's field's does.
                if self.preedit.is_none() && self.anchor != self.focus {
                    self.replace(self.range(), "", EditKind::Delete);
                }
                let (a, _) = self.range();
                (self.anchor, self.focus) = (a, a);
                let cursor = cursor.filter(|cursor| text.is_char_boundary(*cursor)).unwrap_or(text.len());
                self.preedit = (!text.is_empty()).then_some((text, cursor));
            }
            Ime::Commit(text) => {
                self.preedit = None;
                self.commit(&text);
            }
        }
    }

    /// A pointer pressed at byte `at` of its display text, `clicks` times in a row (a double click selects a word, a
    /// triple its line), or with Shift held extending the selection there. What's composed is written first.
    pub fn press(&mut self, at: usize, clicks: u8, shift: bool) {
        self.finish_composition();
        self.history.run = Run::Closed;
        self.column = None;
        self.place = None;
        let at = self.to_model(at);
        if shift {
            self.unit = Unit::Char;
            self.origin = (self.anchor, self.anchor);
            self.focus = at;
            return;
        }
        self.unit = match clicks {
            0 | 1 => Unit::Char,
            2 => Unit::Word,
            _ => Unit::Paragraph,
        };
        self.origin = self.unit_at(at, self.unit);
        (self.anchor, self.focus) = self.origin;
    }

    /// The pressed pointer moved over byte `at` of its display text: the selection runs from where the press began,
    /// by what it selects by.
    pub fn drag(&mut self, at: usize) {
        let at = self.unit_at(self.to_model(at), self.unit);
        (self.anchor, self.focus) =
            if at.0 < self.origin.0 { (self.origin.1, at.0) } else { (self.origin.0, at.1.max(self.origin.1)) };
    }

    /// The range of `unit` at byte `i`: a word (or run of spaces or punctuation) by Unicode's word boundaries, or a
    /// paragraph with its line break. Past the end of a line it takes the line's last word, as AppKit does. A
    /// password is one word.
    fn unit_at(&self, i: usize, unit: Unit) -> (usize, usize) {
        let text = &self.text;
        let mut i = text.floor_char_boundary(i);
        if unit == Unit::Char {
            return (i, i);
        }
        if self.secure || (unit == Unit::Paragraph && !self.multiline) {
            return (0, text.len());
        }
        let prev = |i: usize| text.floor_char_boundary(i.saturating_sub(1));
        if i > 0 && i == text.len() {
            i = prev(i);
        }
        match unit {
            Unit::Char => (i, i),
            Unit::Word => {
                if i > 0 && text[i..].starts_with('\n') && !text[..i].ends_with('\n') {
                    i = prev(i);
                }
                text.split_word_bound_indices()
                    .map(|(at, word)| (at, at + word.len()))
                    .find(|(_, end)| i < *end)
                    .unwrap_or((i, i))
            }
            Unit::Paragraph => {
                let start = text[..i].rfind('\n').map_or(0, |n| n + 1);
                let end = text[i..].find('\n').map_or(text.len(), |n| i + n + 1);
                (start, end)
            }
        }
    }

    /// The word-boundary segment ending at byte `j` (UAX #29, within its line; a line break is a segment of its
    /// own): its start, and whether it's a word (holds a letter or digit).
    fn segment_before(&self, j: usize) -> Option<(usize, bool)> {
        if j == 0 {
            return None;
        }
        let s = &self.text;
        let line = s[..j].rfind('\n').map_or(0, |k| k + 1);
        if line == j {
            return Some((prev_char(s, j), false));
        }
        let (at, seg) = s[line..j].split_word_bound_indices().next_back()?;
        Some((line + at, seg.chars().any(char::is_alphanumeric)))
    }

    /// The word-boundary segment starting at byte `j`: its end, and whether it's a word.
    fn segment_after(&self, j: usize) -> Option<(usize, bool)> {
        let s = &self.text;
        if j >= s.len() {
            return None;
        }
        let line = s[j..].find('\n').map_or(s.len(), |k| j + k);
        if line == j {
            return Some((next_char(s, j), false));
        }
        let seg = s[j..line].split_word_bounds().next()?;
        Some((j + seg.len(), seg.chars().any(char::is_alphanumeric)))
    }

    /// A word back, by Unicode's word boundaries, as browsers and Word step: `src/util/format.ts` stops at `src`,
    /// `util` and `format.ts`, and the punctuation and spaces between words are stepped over. A password's words
    /// are hidden, so it goes to the start.
    fn prev_word(&self, i: usize) -> usize {
        if self.secure {
            return 0;
        }
        let mut j = i.min(self.text.len());
        loop {
            let Some((start, word)) = self.segment_before(j) else { return 0 };
            if word {
                return start;
            }
            j = start;
        }
    }

    /// A word forward: off macOS to the next word's start, past the spaces after it, as Chromium's and Windows'
    /// Ctrl+Right lands; on macOS to the word's end.
    fn next_word(&self, i: usize) -> usize {
        let len = self.text.len();
        if self.secure {
            return len;
        }
        let mut j = i.min(len);
        if !cfg!(target_os = "macos") {
            if let Some((end, true)) = self.segment_after(j) {
                j = end;
            }
            while let Some((end, false)) = self.segment_after(j) {
                j = end;
            }
            return j;
        }
        loop {
            let Some((end, word)) = self.segment_after(j) else { return len };
            if word {
                return end;
            }
            j = end;
        }
    }

    /// Where the line the focus is on begins or ends, as it's drawn.
    fn line_edge(&self, layout: Option<&dyn Layout>, end: bool) -> usize {
        if self.secure && !self.multiline {
            return if end { self.text.len() } else { 0 };
        }
        if let Some(layout) = layout {
            return self.to_model(layout.line_edge(self.to_display(self.focus), end));
        }
        if end {
            self.text[self.focus..].find('\n').map_or(self.text.len(), |i| self.focus + i)
        } else {
            self.text[..self.focus].rfind('\n').map_or(0, |i| i + 1)
        }
    }

    /// Moves the caret to `i`, or collapses a selection to the end the arrow points at.
    fn move_h(&mut self, i: usize, extend: bool, right: bool) {
        let (a, b) = self.range();
        if !extend && a != b {
            let to = if right { b } else { a };
            (self.anchor, self.focus) = (to, to);
        } else {
            self.set_focus(i, extend);
        }
    }

    /// Left or Right across text that runs both ways, as Rune's caret moves (`caret_motion.rs`, `visual_in`): across
    /// the line as it's drawn, from where the caret was put, to the nearest place that way, the edge of the glyph it
    /// crosses; at the line's edge it goes on to the next line the way its paragraph runs, so Left leaves a
    /// right-to-left line forward. A word goes by the text's order, forward where the arrow points the way the
    /// paragraph runs. A selection collapses to the end drawn further that way, as Notepad's and Word's do.
    fn move_visually(&mut self, layout: &dyn Layout, place: Option<(usize, usize, f32)>, right: bool, word: bool, extend: bool) {
        let (line, x) = place.filter(|place| place.0 == self.focus).map_or_else(|| layout.caret(self.to_display(self.focus)), |(_, line, x)| (line, x));
        let forward = right != layout.rtl(line);
        let (a, b) = self.range();
        if !extend && a != b {
            let (to_a, to_b) = (layout.caret(self.to_display(a)), layout.caret(self.to_display(b)));
            let to = if to_a.0 == to_b.0 { if (to_a.1 <= to_b.1) == right { b } else { a } } else if forward { b } else { a };
            (self.anchor, self.focus) = (to, to);
            return;
        }
        if word {
            let to = if forward { self.next_word(self.focus) } else { self.prev_word(self.focus) };
            return self.set_focus(to, extend);
        }
        let stops = layout.stops(line);
        let ahead = stops.iter().map(|stop| stop.x).filter(|at| if right { *at > x + 0.5 } else { *at < x - 0.5 });
        let nearest = if right { ahead.fold(f32::INFINITY, f32::min) } else { ahead.fold(f32::NEG_INFINITY, f32::max) };
        // Of the places there, the edge of the glyph just crossed: its right edge going right.
        match stops.iter().filter(|stop| (stop.x - nearest).abs() < 0.5).max_by_key(|stop| stop.right == right) {
            Some(stop) => {
                self.set_focus(self.to_model(stop.index), extend);
                self.place = Some((self.focus, line, stop.x));
            }
            // At the line's edge it goes on to the next line or the one before, the way its paragraph runs, and
            // stays on the first's or the last's.
            None => {
                let edge = self.to_model(layout.line_edge(self.to_display(self.focus), forward));
                let to = if forward { next_char(&self.text, edge) } else { prev_char(&self.text, edge) };
                if to != edge && layout.caret(self.to_display(to)).0 != line {
                    self.set_focus(to, extend);
                }
            }
        }
    }

    /// Applies one key, as the DOM's `KeyboardEvent.key` names it, with its modifiers, laid out as `layout` draws
    /// it. While an input method composes, its keys are its own.
    pub fn key(&mut self, key: &Key, layout: Option<&dyn Layout>) -> Outcome {
        let mut out = Outcome::default();
        if self.preedit.is_some() {
            return out;
        }
        let mac = cfg!(target_os = "macos");
        // The edit chords' modifier: ⌘ on macOS, Ctrl elsewhere. A word: ⌥ on macOS, Ctrl elsewhere. A line's edge:
        // ⌘ on macOS.
        let command = if mac { key.meta && !key.ctrl } else { key.ctrl && !key.meta };
        let word = if mac { key.alt } else { key.ctrl };
        let line = mac && key.meta;
        let shift = key.shift;
        // Any key but Up and Down lets the column go, and any key the caret's place.
        let column = self.column.take();
        let place = self.place.take();
        // Typing, deleting and line breaks join the open undo step; any other key closes it.
        let typed = key.key.chars().count() == 1 && !command && !key.ctrl && !key.meta;
        if !typed && !matches!(key.key.as_str(), "Backspace" | "Delete" | "Enter") {
            self.history.run = Run::Closed;
        }
        match key.key.as_str() {
            text if typed => {
                let text = self.sanitize(text);
                if !text.is_empty() {
                    self.replace(self.range(), &text, EditKind::Text);
                }
            }
            "Enter" if self.multiline && (self.newline || shift) && !command => {
                self.replace(self.range(), "\n", EditKind::LineBreak)
            }
            "Enter" => out.action = true,
            "Backspace" => {
                let (a, b) = self.range();
                if a != b {
                    self.replace((a, b), "", EditKind::Delete);
                } else if a > 0 {
                    let from = if line {
                        if self.secure { 0 } else { self.text[..a].rfind('\n').map_or(0, |i| i + 1) }
                    } else if word {
                        self.prev_word(a)
                    } else {
                        prev_backspace(&self.text, a)
                    };
                    self.replace((from, a), "", if line || word { EditKind::DeleteRun } else { EditKind::Delete });
                }
            }
            // Shift+Delete cuts, as GTK's and Windows' fields do.
            "Delete" if shift && !mac && !word => {
                if !self.secure {
                    out.copy = Some(self.selected().to_owned());
                    self.replace(self.range(), "", EditKind::Other);
                }
            }
            "Delete" => {
                let (a, b) = self.range();
                if a != b {
                    self.replace((a, b), "", EditKind::Delete);
                } else if a < self.text.len() {
                    let to = if word { self.next_word(a) } else { next_char(&self.text, a) };
                    self.replace((a, to), "", if word { EditKind::DeleteRun } else { EditKind::Delete });
                }
            }
            "Insert" if shift && !key.ctrl => out.paste = !self.readonly,
            "Insert" if key.ctrl && !self.secure => out.copy = Some(self.selected().to_owned()),
            "ArrowLeft" | "ArrowRight" => {
                let right = key.key == "ArrowRight";
                match layout.filter(|layout| !line && !self.secure && layout.bidi()) {
                    Some(layout) => self.move_visually(layout, place, right, word, shift),
                    None => {
                        let i = match (line, word, right) {
                            (true, _, _) => self.line_edge(layout, right),
                            (false, true, true) => self.next_word(self.focus),
                            (false, true, false) => self.prev_word(self.focus),
                            (false, false, true) => next_char(&self.text, self.focus),
                            (false, false, false) => prev_char(&self.text, self.focus),
                        };
                        self.move_h(i, shift, right);
                    }
                }
            }
            // Home and End go to the text's ends on macOS. Elsewhere they go to the line's edge, and with Ctrl to
            // the text's ends.
            "Home" | "End" => {
                let end = key.key == "End";
                let i =
                    if mac || key.ctrl { if end { self.text.len() } else { 0 } } else { self.line_edge(layout, end) };
                self.set_focus(i, shift);
            }
            "ArrowUp" | "ArrowDown" | "PageUp" | "PageDown" => {
                let down = matches!(key.key.as_str(), "ArrowDown" | "PageDown");
                let n = self.text.len();
                let page = key.key.starts_with("Page");
                let mut kept = None;
                let i = match layout {
                    Some(layout) if !line && !page && self.multiline && !self.secure => {
                        let (at, x) = layout.caret(self.to_display(self.focus));
                        let goal = column.unwrap_or(x);
                        match at.checked_add_signed(if down { 1 } else { -1 }).and_then(|to| layout.index_on(to, goal))
                        {
                            Some(j) => {
                                kept = Some(goal);
                                self.to_model(j)
                            }
                            None if down => n,
                            None => 0,
                        }
                    }
                    _ if down => n,
                    _ => 0,
                };
                self.set_focus(i.min(n), shift);
                self.column = kept;
            }
            _ if command => match key.key.to_lowercase().as_str() {
                "z" if shift => self.redo(),
                "z" => self.undo(),
                "y" if !mac && !shift => self.redo(),
                "a" if !shift => self.select_all(),
                "c" if !shift && !self.secure => out.copy = Some(self.selected().to_owned()),
                "x" if !shift && !self.secure && !self.readonly => {
                    out.copy = Some(self.selected().to_owned());
                    self.replace(self.range(), "", EditKind::Other);
                }
                "v" if !shift => out.paste = !self.readonly,
                _ => {}
            },
            _ => {}
        }
        out
    }

    /// Undoes one typing run or edit. A composition is cancelled first.
    pub fn undo(&mut self) {
        self.history.run = Run::Closed;
        if self.preedit.take().is_some() || self.readonly {
            return;
        }
        if let Some((before, after)) = self.history.undo.pop_back() {
            self.restore(&before);
            self.history.redo.push((before, after));
        }
    }

    pub fn redo(&mut self) {
        self.history.run = Run::Closed;
        if self.preedit.take().is_some() || self.readonly {
            return;
        }
        if let Some((before, after)) = self.history.redo.pop() {
            self.restore(&after);
            self.history.undo.push_back((before, after));
        }
    }
}

/// A document, either side of an undo step.
#[derive(Clone)]
struct Snapshot {
    text: String,
    anchor: usize,
    focus: usize,
}

const HISTORY_STEPS: usize = 128;
const HISTORY_BYTES: usize = 1024 * 1024;

/// Undo and redo: each step the document before and after it, typing runs joined as Chromium's editor joins them
/// (`step`), bounded in steps and bytes but always keeping its newest step.
#[derive(Default)]
struct History {
    undo: VecDeque<(Snapshot, Snapshot)>,
    redo: Vec<(Snapshot, Snapshot)>,
    run: Run,
}

impl History {
    fn record(&mut self, before: Snapshot, after: Snapshot, kind: EditKind, into_empty: bool) {
        self.redo.clear();
        match step(&mut self.run, kind, into_empty) {
            Record::Join if !self.undo.is_empty() => self.undo.back_mut().expect("not empty").1 = after,
            _ => self.undo.push_back((before, after)),
        }
        let bytes = |history: &VecDeque<(Snapshot, Snapshot)>| {
            history.iter().map(|(a, b)| a.text.len() + b.text.len()).sum::<usize>()
        };
        while self.undo.len() > 1 && (self.undo.len() > HISTORY_STEPS || bytes(&self.undo) > HISTORY_BYTES) {
            self.undo.pop_front();
        }
    }
}

/// The undo step the last edit leaves open for the next to join, as a browser's editor keeps its typing command
/// open. A caret move, a pointer selection, a focus change, undo, redo and any edit but typing close it. Pauses and
/// word boundaries don't: Chromium's textarea undoes a whole typing run in one step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Run {
    #[default]
    Closed,
    /// Typed text or line breaks: more join; a deletion starts a new step.
    Inserting,
    /// Backspace or Delete: more deletions and typed text join.
    Deleting,
}

/// What an edit is, for joining it to the open undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    /// Typed text without a line break.
    Text,
    /// A typed line break.
    LineBreak,
    /// Backspace or Delete over a character or a selection.
    Delete,
    /// A word or line deleted at once (Ctrl+Backspace): always a new step, which further deletions and typing join.
    DeleteRun,
    /// Paste, cut: a step of its own.
    Other,
}

/// Where an edit's change goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Record {
    Join,
    Push,
}

/// Steps the open run with an edit of `kind`; `into_empty` when typed text lands in a field that held nothing. The
/// first character typed into an empty field is a step of its own, so undoing a typed message leaves that character,
/// then empties the field.
fn step(run: &mut Run, kind: EditKind, into_empty: bool) -> Record {
    let (record, next) = match (*run, kind) {
        (_, EditKind::Other) => (Record::Push, Run::Closed),
        (_, EditKind::DeleteRun) | (Run::Closed | Run::Inserting, EditKind::Delete) => (Record::Push, Run::Deleting),
        (Run::Deleting, EditKind::Delete) => (Record::Join, Run::Deleting),
        (Run::Closed, EditKind::Text | EditKind::LineBreak) => (Record::Push, Run::Inserting),
        (Run::Inserting | Run::Deleting, EditKind::Text | EditKind::LineBreak) => (Record::Join, Run::Inserting),
    };
    *run = if kind == EditKind::Text && into_empty { Run::Closed } else { next };
    record
}
