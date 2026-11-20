//! What the painter draws a box from: its quad, fill and shadow.

use super::*;

impl Painter<'_> {
    /// A quad, its bounds and mask in the window.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn quad(&mut self, bounds: Bounds<f32>, mask: Bounds<f32>, background: Hsla, borders: Edges<f32>, border_color: Hsla, radii: Corners<f32>, dashed: bool, opacity: f32) {
        let scale = self.scale;
        self.scene.insert_primitive(Quad {
            order: 0,
            border_style: if dashed { BorderStyle::Dashed } else { BorderStyle::Solid },
            bounds: bounds.scale_to(scale),
            content_mask: ContentMask { bounds: mask.scale_to(scale) },
            background: Background::from(background.opacity(opacity)),
            border_color: border_color.opacity(opacity),
            corner_radii: Corners {
                top_left: ScaledPixels(radii.top_left * scale),
                top_right: ScaledPixels(radii.top_right * scale),
                bottom_right: ScaledPixels(radii.bottom_right * scale),
                bottom_left: ScaledPixels(radii.bottom_left * scale),
            },
            border_widths: Edges {
                top: ScaledPixels(borders.top * scale),
                right: ScaledPixels(borders.right * scale),
                bottom: ScaledPixels(borders.bottom * scale),
                left: ScaledPixels(borders.left * scale),
            },
            transformation: self.turn.scaled(scale),
        });
    }

    /// A box's fill and border, drawn only where they show, as a GPU's fill rate is what a long list spends while it
    /// scrolls: nothing for a box that's see-through with no border, and a square one's border alone as its edges, not
    /// its whole box.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn fill(&mut self, bounds: Bounds<f32>, mask: Bounds<f32>, background: Hsla, borders: Edges<f32>, border_color: Hsla, radii: Corners<f32>, dashed: bool, opacity: f32) {
        let bordered = borders != Edges::default() && border_color.a > 0.0;
        if background.a > 0.0 || (bordered && (dashed || radii != Corners::default())) {
            return self.quad(bounds, mask, background, borders, border_color, radii, dashed, opacity);
        }
        if !bordered {
            return;
        }
        let Bounds { origin, size } = bounds;
        let edges = [
            (borders.top, Bounds { origin, size: Size { width: size.width, height: borders.top } }),
            (borders.bottom, Bounds { origin: point(origin.x, origin.y + size.height - borders.bottom), size: Size { width: size.width, height: borders.bottom } }),
            (borders.left, Bounds { origin: point(origin.x, origin.y + borders.top), size: Size { width: borders.left, height: size.height - borders.top - borders.bottom } }),
            (borders.right, Bounds { origin: point(origin.x + size.width - borders.right, origin.y + borders.top), size: Size { width: borders.right, height: size.height - borders.top - borders.bottom } }),
        ];
        for (width, edge) in edges {
            if width > 0.0 {
                self.quad(edge, mask, border_color, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
            }
        }
    }

    /// A box's shadow, as CSS's `box-shadow` draws it: its box moved by its offset and grown by its spread, blurred,
    /// under the box; or an inset one inside it, where the box moved and shrunk by its spread doesn't cover it. Either
    /// shows only near the box's edge, so it's drawn only outside the box's middle, in up to four bands, as a
    /// nine-slice shadow is: a long list's card's edge costs its edges, not its whole box.
    pub(in crate::view) fn shadow(&mut self, shadow: &crate::tokens::Shadow, bounds: Bounds<f32>, radii: Corners<f32>, grown: f32, mask: Bounds<f32>, opacity: f32) {
        let spread = if shadow.inset { -shadow.spread } else { shadow.spread } * grown;
        let shadow_bounds = Bounds {
            origin: point(bounds.origin.x + shadow.x * grown - spread, bounds.origin.y + shadow.y * grown - spread),
            size: Size { width: (bounds.size.width + 2.0 * spread).max(0.0), height: (bounds.size.height + 2.0 * spread).max(0.0) },
        };
        let scale = self.scale;
        let corners = |radii: Corners<f32>, by: f32| Corners {
            top_left: ScaledPixels((radii.top_left + by).max(0.0) * scale),
            top_right: ScaledPixels((radii.top_right + by).max(0.0) * scale),
            bottom_right: ScaledPixels((radii.bottom_right + by).max(0.0) * scale),
            bottom_left: ScaledPixels((radii.bottom_left + by).max(0.0) * scale),
        };
        let drawn = Shadow {
            order: 0,
            blur_radius: ScaledPixels(shadow.blur * grown * scale / 2.0),
            bounds: shadow_bounds.scale_to(scale),
            corner_radii: corners(radii, spread),
            content_mask: ContentMask { bounds: mask.scale_to(scale) },
            color: Hsla::from(shadow.colour).opacity(opacity),
            transformation: self.turn.scaled(scale),
            inset: u32::from(shadow.inset),
            pad: 0,
            outer: bounds.scale_to(scale),
            outer_radii: corners(radii, 0.0),
        };
        // The box's middle, which its shadow never shows in: inside it by its largest corner, and an inset one's by as
        // far in as it reaches (its blur's three deviations, its spread and its offset) too, with a pixel to spare.
        let corner = radii.top_left.max(radii.top_right).max(radii.bottom_left).max(radii.bottom_right);
        let reach = if shadow.inset { 0.75 * shadow.blur * grown + spread.abs() + shadow.x.abs().max(shadow.y.abs()) * grown + 1.0 } else { 0.0 };
        let inside = corner + reach;
        let (left, top, right, bottom) = (bounds.origin.x + inside, bounds.origin.y + inside, bounds.origin.x + bounds.size.width - inside, bounds.origin.y + bounds.size.height - inside);
        if self.turn != TransformationMatrix::unit() || right <= left || bottom <= top {
            return self.scene.insert_primitive(drawn);
        }
        let far = 1.0e6;
        let band = |x0: f32, y0: f32, x1: f32, y1: f32| Bounds { origin: point(x0, y0), size: Size { width: x1 - x0, height: y1 - y0 } };
        for around in [band(-far, -far, far, top), band(-far, bottom, far, far), band(-far, top, left, bottom), band(right, top, far, bottom)] {
            let cut = around.intersect(&mask);
            if cut.size.width > 0.0 && cut.size.height > 0.0 {
                self.scene.insert_primitive(Shadow { content_mask: ContentMask { bounds: cut.scale_to(scale) }, ..drawn.clone() });
            }
        }
    }

    /// `bounds` grown by half the window's size on each side, as near it as an `IntersectionObserver`'s margin of 50%.
    pub(in crate::view) fn grown(&self, bounds: Bounds<f32>) -> Bounds<f32> {
        let (x, y) = (self.window.size.width / 2.0, self.window.size.height / 2.0);
        Bounds { origin: point(bounds.origin.x - x, bounds.origin.y - y), size: Size { width: bounds.size.width + 2.0 * x, height: bounds.size.height + 2.0 * y } }
    }

    /// The box `bounds` covers in the window once it's turned as what's painted now is: itself, where nothing's turned.
    pub(in crate::view) fn turned(&self, bounds: Bounds<f32>) -> Bounds<f32> {
        if self.turn == TransformationMatrix::unit() {
            return bounds;
        }
        let (left, top, right, bottom) = (bounds.origin.x, bounds.origin.y, bounds.origin.x + bounds.size.width, bounds.origin.y + bounds.size.height);
        let corners = [[left, top], [right, top], [left, bottom], [right, bottom]].map(|corner| self.turn.apply(corner));
        let (mut low, mut high) = (corners[0], corners[0]);
        for [x, y] in corners {
            low = [low[0].min(x), low[1].min(y)];
            high = [high[0].max(x), high[1].max(y)];
        }
        Bounds { origin: point(low[0], low[1]), size: Size { width: high[0] - low[0], height: high[1] - low[1] } }
    }
}
