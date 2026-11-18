//! A text input's editing state in the shape the platform's keyboard bridge speaks (Flutter's `TextEditingValue`:
//! text, a selection from its base to its extent, and the input method's composing range, which is part of the text),
//! with Rune's secure-field masking (from anomalyco/rune 78efd4a7876b5478dd72033ce6fd2b2f13ab9f9a,
//! crates/rune/src/edit/secure.rs, MIT, Copyright (c) 2026 Anomaly: `masked_display`, `display_index` and
//! `model_index` for a single-line field, here as functions of this state).
//!
//! Offsets are UTF-8 byte offsets on char boundaries here; Java's are UTF-16 code units, converted at the bridge.

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Edit {
    pub text: String,
    pub base: usize,
    pub extent: usize,
    pub composing: Option<(usize, usize)>,
    pub secure: bool,
}

impl Edit {
    /// A state from the bridge, in UTF-16 offsets (-1 for none), as Java's `Editable` has them.
    pub fn from_utf16(text: String, base: i32, extent: i32, composing_start: i32, composing_end: i32, secure: bool) -> Edit {
        let at = |offset: i32| utf16_to_byte(&text, offset.max(0) as usize);
        let (base, extent) = if base < 0 { (text.len(), text.len()) } else { (at(base), at(extent)) };
        let composing = (composing_start >= 0 && composing_end > composing_start).then(|| (at(composing_start), at(composing_end)));
        Edit { base, extent, composing, secure, text }
    }

    /// This state's offsets in UTF-16, for the bridge: (base, extent, composing start, composing end).
    pub fn utf16(&self) -> (usize, usize, i64, i64) {
        let at = |byte: usize| byte_to_utf16(&self.text, byte);
        let (start, end) = self.composing.map_or((-1, -1), |(s, e)| (at(s) as i64, at(e) as i64));
        (at(self.base), at(self.extent), start, end)
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.base.min(self.extent), self.base.max(self.extent))
    }

    pub fn collapsed(&self) -> bool {
        self.base == self.extent
    }

    pub fn set_caret(&mut self, byte: usize) {
        let byte = floor_char_boundary(&self.text, byte);
        self.base = byte;
        self.extent = byte;
        self.composing = None;
    }

    /// Selects the word around `byte` (UAX #29), as a long press does.
    pub fn select_word(&mut self, byte: usize) {
        use unicode_segmentation::UnicodeSegmentation;
        let word = self
            .text
            .split_word_bound_indices()
            .map(|(i, w)| i..i + w.len())
            .find(|r| r.start <= byte && byte <= r.end && !self.text[r.clone()].trim().is_empty());
        match word {
            Some(range) => {
                self.base = range.start;
                self.extent = range.end;
                self.composing = None;
            }
            None => self.set_caret(byte),
        }
    }

    /// What the field draws (Rune's `masked_display`): the text, or one bullet a character where it's secure.
    pub fn display(&self) -> String {
        if self.secure { "•".repeat(self.text.chars().count()) } else { self.text.clone() }
    }

    /// Rune's `display_index`: a byte of the text, as a byte of what's drawn.
    pub fn display_index(&self, byte: usize) -> usize {
        if self.secure { self.text[..byte.min(self.text.len())].chars().count() * '•'.len_utf8() } else { byte.min(self.text.len()) }
    }

    /// Rune's `model_index`: a byte of what's drawn, as a byte of the text.
    pub fn model_index(&self, byte: usize) -> usize {
        if self.secure {
            self.text.char_indices().nth(byte / '•'.len_utf8()).map_or(self.text.len(), |(i, _)| i)
        } else {
            floor_char_boundary(&self.text, byte)
        }
    }
}

fn floor_char_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

pub fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

pub fn byte_to_utf16(text: &str, byte: usize) -> usize {
    text[..floor_char_boundary(text, byte)].encode_utf16().count()
}
