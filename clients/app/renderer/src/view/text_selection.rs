//! What a finger selects in an input, as a phone's own fields have it (Flutter's `TextSelectionOverlay` and its
//! `TextSelectionControls`, UIKit's `UITextInteraction`, Android's `Editor`): a finger held on a word, or a double
//! tap, selects it; handles at the selection's ends drag them; the system's own menu for it (`TextMenu`: Cut, Copy,
//! Paste, Select All) shows over it, or under it where the safe area leaves no room above; and the system's magnifier
//! shows what's under a finger moving it. A tap on the caret shows the menu too (Paste). Where each is comes from where
//! the input's text was last drawn (`Drawn`), the same for every host: each host shows its system's own menu and
//! magnifier (`Display::text_menu`, `Display::magnifier`), and the renderer draws the handles, as Flutter does.

use super::{Screen, TextMenu};
use crate::geometry::{Bounds, Corners, Point, Size, point};
use crate::tree::Id;

/// What a finger selected in input `id`, and whether the system's menu is shown for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Overlay {
    pub id: Id,
    pub menu: bool,
}

/// Which end of a selection a handle is at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Start,
    End,
}

/// Where the system's magnifier shows what a finger moving a selection is over: where the finger is, along the line
/// of the caret it moves, and that caret, in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Magnifier {
    pub at: Point<f32>,
    pub caret: Bounds<f32>,
}

/// How far a fingertip reaches for a handle: Apple's 44-point target.
const TARGET: f32 = 44.0;
/// iOS's handle: a bar the caret's height and 2 points wide, with a knob 10 across over the start and under the end.
const BAR: f32 = 2.0;
const KNOB: f32 = 10.0;
/// Android's: Material's teardrop (`text_select_handle_left`, `_right`), 22 dp across, hanging under each end with
/// its point there.
const DROP: f32 = 22.0;
/// The room the system's menu takes above what it's shown for, its height and the gap under it: UIEditMenuInteraction's
/// 44 points and Android's floating toolbar's 48 dp, each with 8 between.
pub const MENU_ROOM: f32 = 56.0;

/// A handle at end `end` of a selection, where the caret there is drawn (`caret`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handle {
    pub end: End,
    caret: Bounds<f32>,
}

/// The handles at a selection's ends, where the carets at its start and its end are drawn.
pub fn handles(start: Bounds<f32>, end: Bounds<f32>) -> [Handle; 2] {
    [Handle { end: End::Start, caret: start }, Handle { end: End::End, caret: end }]
}

impl Handle {
    /// What's drawn of it, each shape with its corners: iOS's bar and knob, Android's teardrop.
    pub fn shapes(&self) -> Vec<(Bounds<f32>, Corners<f32>)> {
        let caret = self.caret;
        let bottom = caret.origin.y + caret.size.height;
        if cfg!(target_os = "android") {
            let round = DROP / 2.0;
            let (x, corners) = match self.end {
                End::Start => (caret.origin.x - DROP, Corners { top_left: round, top_right: 0.0, bottom_right: round, bottom_left: round }),
                End::End => (caret.origin.x, Corners { top_left: 0.0, top_right: round, bottom_right: round, bottom_left: round }),
            };
            return vec![(Bounds { origin: point(x, bottom), size: Size { width: DROP, height: DROP } }, corners)];
        }
        let bar = Bounds { origin: point(caret.origin.x - BAR / 2.0, caret.origin.y), size: Size { width: BAR, height: caret.size.height } };
        let y = match self.end {
            End::Start => caret.origin.y - KNOB + 1.0,
            End::End => bottom - 1.0,
        };
        let knob = Bounds { origin: point(caret.origin.x - KNOB / 2.0, y), size: Size { width: KNOB, height: KNOB } };
        vec![(bar, Corners::default()), (knob, Corners { top_left: KNOB / 2.0, top_right: KNOB / 2.0, bottom_right: KNOB / 2.0, bottom_left: KNOB / 2.0 })]
    }

    /// Where it's drawn, all of it.
    pub fn bounds(&self) -> Bounds<f32> {
        let shapes = self.shapes();
        shapes.iter().skip(1).fold(shapes[0].0, |all, (shape, _)| all.union(shape))
    }

    /// Where a finger takes it: what's drawn, grown to a fingertip's target about its middle.
    fn target(&self) -> Bounds<f32> {
        let drawn = self.bounds();
        let (width, height) = (drawn.size.width.max(TARGET), drawn.size.height.max(TARGET));
        let middle = point(drawn.origin.x + drawn.size.width / 2.0, drawn.origin.y + drawn.size.height / 2.0);
        Bounds { origin: point(middle.x - width / 2.0, middle.y - height / 2.0), size: Size { width, height } }
    }
}

impl Screen {
    /// The input whose selection shows its handles: one a finger selected in, while something's selected in it and it
    /// has the keyboard.
    pub(super) fn handles_on(&self) -> Option<Id> {
        let id = self.editing.overlay?.id;
        let field = self.editing.fields.get(&id)?;
        (self.focus.focused == Some(id) && !field.selected().is_empty() && !field.composing()).then_some(id)
    }

    /// The handles drawn now, at the ends of what a finger selected.
    pub fn handles(&self) -> Vec<Handle> {
        let Some(id) = self.handles_on() else { return Vec::new() };
        let (Some(field), Some(drawn)) = (self.editing.fields.get(&id), self.editing.drawn.get(&id)) else { return Vec::new() };
        let (start, end) = field.selection();
        handles(drawn.caret_at(start, false), drawn.caret_at(end, true)).to_vec()
    }

    /// The handle a finger landing at `at` takes, if one's there: the nearer, where their targets overlap.
    pub(super) fn handle_at(&self, at: Point<f32>) -> Option<End> {
        let distance = |handle: &Handle| {
            let drawn = handle.bounds();
            (drawn.origin.x + drawn.size.width / 2.0 - at.x).hypot(drawn.origin.y + drawn.size.height / 2.0 - at.y)
        };
        self.handles().into_iter().filter(|handle| handle.target().contains(&at)).min_by(|a, b| distance(a).total_cmp(&distance(b))).map(|handle| handle.end)
    }

    /// Handle `end` dragged to `at`: that end of the selection goes there by the character, the other staying.
    pub(super) fn drag_handle(&mut self, end: End, at: Point<f32>) {
        let Some(id) = self.editing.overlay.map(|overlay| overlay.id) else { return };
        let Some((index, upstream)) = self.editing.drawn.get(&id).map(|drawn| drawn.hit(at)) else { return };
        self.edit(id, |field| {
            let (start, finish) = field.selection();
            let kept = if end == End::Start { finish } else { start };
            field.press(kept, false, 1, false);
            field.drag(index.min(field.display().len()), upstream);
        });
        self.changed();
    }

    /// A finger held on input `id`, or tapped twice there, selected at `at`, by the word: its handles and the
    /// system's menu show for it.
    pub(super) fn select_word(&mut self, at: Point<f32>) -> Option<Id> {
        let id = self.under(at).into_iter().find(|id| self.editing.fields.contains_key(id))?;
        self.focus(Some(id));
        let index = self.editing.drawn.get(&id).map_or(usize::MAX, |drawn| drawn.index_at(at));
        self.edit(id, |field| field.press(index.min(field.display().len()), false, 2, false));
        self.editing.overlay = Some(Overlay { id, menu: true });
        self.changed();
        Some(id)
    }

    /// The finger held on a selection moved to `at`: it runs from the word it was held on to the word there.
    pub(super) fn drag_word(&mut self, at: Point<f32>) {
        let Some(id) = self.editing.overlay.map(|overlay| overlay.id) else { return };
        let Some((index, upstream)) = self.editing.drawn.get(&id).map(|drawn| drawn.hit(at)) else { return };
        self.edit(id, |field| field.drag(index.min(field.display().len()), upstream));
        self.changed();
    }

    /// The system's menu for what a finger selected, or at the caret it tapped, as it's to show now: none while a
    /// finger is down (moving it, scrolling, or about to tap elsewhere), as UIKit's and Android's hide theirs.
    pub fn text_menu_shown(&self) -> Option<TextMenu> {
        let id = self.editing.overlay.filter(|overlay| overlay.menu)?.id;
        if self.focus.focused != Some(id) || self.finger.is_some() {
            return None;
        }
        let (field, drawn) = (self.editing.fields.get(&id)?, self.editing.drawn.get(&id)?);
        let (start, end) = field.selection();
        let rect = match start < end {
            true => drawn.spans(start, end).into_iter().reduce(|all, line| all.union(&line))?,
            false => drawn.caret_at(field.caret(), field.upstream()),
        };
        // With its handles, as Android's Editor gives its toolbar, so it covers neither.
        let rect = self.handles().iter().fold(rect, |all, handle| all.union(&handle.bounds()));
        // Where the safe area leaves no room for it above, it's shown for what's above too, up to the window's top,
        // so the system puts it under: Android's floating toolbar is placed in the whole window, which an app drawn
        // edge to edge has under its status bar, and goes over the clock otherwise.
        let rect = match rect.origin.y - self.insets.top < MENU_ROOM {
            true => Bounds { origin: point(rect.origin.x, 0.0), size: Size { width: rect.size.width, height: rect.origin.y + rect.size.height } },
            false => rect,
        };
        Some(self.text_menu_of(id, rect))
    }

    /// The system's menu for input `id`'s text, shown beside `rect`: what of it can be done there.
    pub(super) fn text_menu_of(&self, id: Id, rect: Bounds<f32>) -> TextMenu {
        let field = &self.editing.fields[&id];
        let selected = !field.selected().is_empty();
        let empty = field.text.is_empty();
        TextMenu {
            rect,
            cut: selected && !field.secure && !field.readonly,
            copy: selected && !field.secure,
            paste: !field.readonly,
            select_all: !empty && field.selected().len() < field.text.len(),
            select: self.touch && !selected && !empty,
        }
    }

    /// The system's menu for what a finger selected shows, or not, once no finger is down.
    pub(super) fn show_text_menu(&mut self, menu: bool) {
        if let Some(overlay) = &mut self.editing.overlay {
            overlay.menu = menu;
        }
    }

    /// A finger tapped an input: tapped twice or three times, what it selected has its handles and the system's menu;
    /// once on the caret of the input that had the keyboard (`had`, its caret `caret`, where nothing was selected), the
    /// menu shows, or goes where it showed (`menu`), as iOS's Paste does; once anywhere else, what a finger selected is
    /// let go.
    pub(super) fn tapped_input(&mut self, had: Option<Id>, caret: Option<usize>, menu: bool) {
        let Some(id) = self.focus.focused.filter(|id| self.editing.fields.contains_key(id)) else { return };
        let taps = self.editing.last_press.map_or(1, |(.., taps)| taps);
        let field = &self.editing.fields[&id];
        let selected = !field.selected().is_empty();
        self.editing.overlay = match (taps > 1 && selected, had == Some(id) && !selected && caret == Some(field.caret())) {
            (true, _) => Some(Overlay { id, menu: true }),
            (false, true) => Some(Overlay { id, menu: !menu }),
            (false, false) => None,
        };
        self.changed();
    }

    /// The system's menu closed by itself: a press outside it, or one of its items chosen.
    pub fn text_menu_closed(&mut self) {
        if let Some(overlay) = &mut self.editing.overlay {
            overlay.menu = false;
        }
    }

    /// The menu's Select (iOS's): the word at the caret of the input with the keyboard is selected, with its handles
    /// and the menu again.
    pub(super) fn select_at_caret(&mut self) {
        let Some(id) = self.focus.focused.filter(|id| self.editing.fields.contains_key(id)) else { return };
        self.edit(id, |field| field.press(field.caret(), false, 2, false));
        self.editing.overlay = Some(Overlay { id, menu: true });
        self.changed();
    }

    /// The keyboard said input `id`'s editing state is now this (one of the menu's items, or typing): typing lets go of
    /// what a finger selected; Select All selects with handles and the menu again, as UIKit's and Android's fields do.
    pub(super) fn overlay_keyboard(&mut self, id: Id, before: &crate::editing::Value) {
        let Some(after) = self.editing.fields.get(&id).map(crate::editing::Field::value) else { return };
        if after.text != before.text {
            self.editing.overlay = None;
        } else if after.selection != before.selection && after.selection.base != after.selection.extent && self.touch {
            self.editing.overlay = Some(Overlay { id, menu: true });
        }
    }

    /// Where the system's magnifier shows, while a finger held on a selection or on a handle moves it.
    pub fn magnifier(&self) -> Option<Magnifier> {
        let finger = self.finger.as_ref().filter(|finger| finger.held || finger.handle.is_some())?;
        let id = self.editing.overlay?.id;
        let (field, drawn) = (self.editing.fields.get(&id)?, self.editing.drawn.get(&id)?);
        let caret = drawn.caret_at(field.caret(), field.upstream());
        Some(Magnifier { at: point(finger.last.x, caret.origin.y + caret.size.height / 2.0), caret })
    }
}
