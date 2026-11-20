//! What the window draws of its own over its page: its title, its buttons and its notice.

use super::*;

impl Painter<'_> {
    /// A window's title over the top of its page (`Title`), in a computer's own size: the app's icon and its name in
    /// semibold, in the middle of the `bar`, which nothing presses. → where it ends.
    pub(in crate::view) fn title(&mut self, title: &Title, start: f32, bar: f32) -> f32 {
        let conditions = self.conditions;
        let window = self.window;
        let (_, style) = chrome_type(conditions);
        let middle = bar / 2.0;
        let (icon, gap) = (16.0, 6.0);
        let mut x = start;
        if let Some(src) = &title.icon {
            self.picture(src, Bounds { origin: point(x, middle - icon / 2.0), size: Size { width: icon, height: icon } }, icon * 0.225);
            x += icon + gap;
        }
        let name = self.text.paragraph(&title.name, &style, &[], None, Some(1));
        let at = Bounds { origin: point(x, middle - name.size().height / 2.0), size: name.size() };
        let text = Text { style, color: colour_token("label", conditions), align: 0.0, wrap: false, underline: false, selectable: false };
        self.paint_text(&name, &[], at, name.size().width.ceil(), 1.0, text, 1.0, window);
        x + name.size().width.ceil()
    }

    /// One of the window's own buttons (`WindowButton`): on Windows as its caption buttons are, as wide as they are
    /// and as tall as the row, lit under the pointer (the close one in red) with a thin symbol; elsewhere as a GNOME
    /// header bar's, a round button with its symbol in the label's colour.
    pub(in crate::view) fn window_button(&mut self, kind: super::super::WindowButton, bounds: Bounds<f32>, hovered: bool) {
        let conditions = self.conditions;
        let window = self.window;
        let colour = |id: &str| colour_token(id, conditions);
        let windows = cfg!(windows);
        let closing = windows && hovered && kind == super::super::WindowButton::Close;
        let fill = if closing { colour("destructive") } else if hovered { colour("quaternary-label") } else if windows { Hsla::default() } else { colour("quinary-label") };
        if windows {
            self.quad(bounds, window, fill, Edges::default(), Hsla::default(), Corners::default(), false, 1.0);
        } else {
            self.quad(bounds, window, fill, Edges::default(), Hsla::default(), Corners::all(bounds.size.width / 2.0), false, 1.0);
        }
        let (middle, ink) = (point(bounds.origin.x + bounds.size.width / 2.0, bounds.origin.y + bounds.size.height / 2.0), if closing { colour("on-accent") } else { colour("label") });
        let (size, thick, rise) = if windows { (10.0, 1.0, 0.0) } else { (8.0, 1.5, 3.0) };
        let line = |width: f32, height: f32, y: f32| Bounds { origin: point(middle.x - width / 2.0, middle.y + y - height / 2.0), size: Size { width, height } };
        match kind {
            super::super::WindowButton::Minimize => self.quad(line(size, thick, rise), window, ink, Edges::default(), Hsla::default(), Corners::default(), false, 1.0),
            super::super::WindowButton::Maximize => self.quad(line(size, size, 0.0), window, Hsla::default(), Edges { top: thick, right: thick, bottom: thick, left: thick }, ink, Corners::all(if windows { 0.0 } else { 1.0 }), false, 1.0),
            // A cross of two diagonals, a row of small squares each: a line turned by `turn` that thin isn't drawn whole.
            super::super::WindowButton::Close => {
                let half = if windows { 5.0 } else { 4.0 };
                let (steps, dot) = (((2.0 * half) / 0.5) as i32, thick.max(1.0));
                for step in 0..=steps {
                    let along = -half + step as f32 * 0.5;
                    for across in [along, -along] {
                        let square = Bounds { origin: point(middle.x + along - dot / 2.0, middle.y + across - dot / 2.0), size: Size { width: dot, height: dot } };
                        self.quad(square, window, ink, Edges::default(), Hsla::default(), Corners::default(), false, 1.0);
                    }
                }
            }
        }
    }

    /// The window's notice over the top of its page (`Notice`), `top` points down: a pill of the foreground's colour
    /// with its words in the background's, as a toast, centred, and its action after them, highlighted under the
    /// pointer. → where its action is.
    pub(in crate::view) fn notice(&mut self, notice: &Notice, top: f32, hovered: bool) -> Option<Bounds<f32>> {
        let conditions = self.conditions;
        let window = self.window;
        let colour = |id: &str| colour_token(id, conditions);
        let (words_style, action_style) = chrome_type(conditions);
        let words = self.text.paragraph(&notice.text, &words_style, &[], None, Some(1));
        let action = notice.action.as_ref().map(|(name, _)| self.text.paragraph(name, &action_style, &[], None, Some(1)));
        let (pad, gap, height) = (12.0, 4.0, 30.0);
        let action_width = action.as_ref().map_or(0.0, |action| gap + pad + action.size().width.ceil() + pad - 4.0);
        let width = pad + words.size().width.ceil() + action_width + if action.is_some() { 4.0 } else { pad };
        let pill = Bounds { origin: point((window.size.width - width) / 2.0, top + 8.0), size: Size { width, height } };
        self.quad(pill, window, colour("label"), Edges::default(), Hsla::default(), Corners::all(height / 2.0), false, 1.0);
        let middle = pill.origin.y + height / 2.0;
        let words_at = |paragraph: &Paragraph, x: f32| Bounds { origin: point(x, middle - paragraph.size().height / 2.0), size: paragraph.size() };
        let text = Text { style: words_style, color: colour("background"), align: 0.0, wrap: false, underline: false, selectable: false };
        self.paint_text(&words, &[], words_at(&words, pill.origin.x + pad), words.size().width.ceil(), 1.0, text, 1.0, window);
        let action = action?;
        let x = pill.origin.x + pad + words.size().width.ceil() + gap;
        let bounds = Bounds { origin: point(x, pill.origin.y + 4.0), size: Size { width: pad + action.size().width.ceil() + pad, height: height - 8.0 } };
        if hovered {
            let mut highlight = colour("background");
            highlight.a *= 0.2;
            self.quad(bounds, window, highlight, Edges::default(), Hsla::default(), Corners::all(bounds.size.height / 2.0), false, 1.0);
        }
        self.paint_text(&action, &[], words_at(&action, x + pad), action.size().width.ceil(), 1.0, Text { style: action_style, ..text }, 1.0, window);
        Some(bounds)
    }
}
