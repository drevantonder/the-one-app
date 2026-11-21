//! Painting a node, and the layers what only moves is drawn from (`Layer`).

use super::*;

impl Painter<'_> {
    /// Draws node `id`, which is only moving, growing or fading, from its layer: what's in it drawn once as it rests,
    /// then each frame moved, grown and faded as a whole, with where it puts what's in it for the pointer, Tab and the
    /// inspector, as Core Animation moves a layer and Flutter a RepaintBoundary's picture. Nothing in it is laid out,
    /// shaped or painted again, and its words move exactly with their boxes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn paint_layer(&mut self, taffy_node: NodeId, id: Id, offset: Point<f32>, transform: Transform, look: &Look, mask: Bounds<f32>, opacity: f32, parent: Option<Id>, pointer: bool) {
        let Ok(laid) = self.layout.taffy.layout(taffy_node).copied() else { return };
        let laid_out = Bounds { origin: point(offset.x + laid.location.x, offset.y + laid.location.y), size: Size { width: laid.size.width, height: laid.size.height } };
        let key = LayerKey { laid_out, device: self.scale };
        match self.layers.get(&id) {
            Some(layer) if layer.key == key && layer.recorded => {}
            // Where it rests is changing (what it's in is growing): it's drawn as usual until it holds still a frame,
            // rather than drawn into a layer each frame.
            Some(layer) if layer.key != key => {
                self.layers.insert(id, Layer::unsettled(key));
                self.unsettled = Some(id);
                self.paint_node(taffy_node, offset, transform, mask, opacity, parent, pointer);
                self.unsettled = None;
                return;
            }
            // What's only about to move is drawn ahead only in a frame where nothing does, as usual till then.
            _ if !self.still && !self.moving.contains(&id) => {
                self.unsettled = Some(id);
                self.paint_node(taffy_node, offset, transform, mask, opacity, parent, pointer);
                self.unsettled = None;
                return;
            }
            _ => {
                let layer = self.record(taffy_node, id, offset, transform, parent, pointer, key);
                self.layers.insert(id, layer);
            }
        }
        // Its own move and growth around its middle, as `paint_node` draws it, in the window, from where it was drawn
        // (`Layer::at`, grown by `Layer::scale`): p ↦ p·grown + by.
        let centre = point(laid_out.origin.x + laid_out.size.width / 2.0, laid_out.origin.y + laid_out.size.height / 2.0);
        let translate = point(look.translate.0.of(laid_out.size.width), look.translate.1.of(laid_out.size.height));
        let (at, k, own) = (transform.offset, transform.scale, look.scale);
        let (was, grown) = (self.layers[&id].at, k * own / self.layers[&id].scale);
        let by = point(at.x + k * (centre.x + translate.x - own * centre.x) - grown * was.x, at.y + k * (centre.y + translate.y - own * centre.y) - grown * was.y);
        let moved = |bounds: Bounds<f32>| Bounds { origin: point(bounds.origin.x * grown + by.x, bounds.origin.y * grown + by.y), size: Size { width: bounds.size.width * grown, height: bounds.size.height * grown } };
        let opacity = opacity * look.opacity;
        for part in self.layers[&id].parts.clone() {
            match part {
                Part::Scene(_, Some(recording)) => self.scene.raster(&recording, grown, [by.x * self.scale, by.y * self.scale], opacity, mask.scale_to(self.scale)),
                Part::Scene(scene, None) => self.scene.splice(&scene, grown, [by.x * self.scale, by.y * self.scale], opacity, mask.scale_to(self.scale)),
                Part::Sublayer(sublayer) => {
                    let transform = Transform { offset: point(sublayer.transform.offset.x * grown + by.x, sublayer.transform.offset.y * grown + by.y), scale: sublayer.transform.scale * grown, rest: sublayer.transform.rest };
                    self.paint(sublayer.taffy_node, sublayer.offset, transform, moved(sublayer.mask).intersect(&mask), sublayer.opacity * opacity, sublayer.parent, sublayer.pointer);
                }
            }
        }
        let layer = &self.layers[&id];
        self.placed.extend(layer.placed.iter().map(|(id, placed)| (*id, Placed { shown: moved(placed.shown).intersect(&mask), opacity: placed.opacity * opacity, ..placed.clone() })));
        self.rects.extend(layer.rects.iter().map(|(id, rect)| {
            let drawn = moved(Bounds { origin: point(rect.x, rect.y), size: Size { width: rect.width, height: rect.height } });
            (*id, Rect { x: drawn.origin.x, y: drawn.origin.y, width: drawn.size.width, height: drawn.size.height })
        }));
        self.tab_order.extend(&layer.tab_order);
        self.intersections.extend(&layer.intersections);
        self.drag_regions.extend(layer.drag_regions.iter().map(|(id, bounds)| (*id, moved(*bounds).intersect(&mask))));
        self.kept_regions.extend(layer.kept_regions.iter().map(|bounds| moved(*bounds).intersect(&mask)));
        // A layer fading has no glass of the system's, which can't fade (`paint`).
        if let Some(glass) = self.glass.as_mut().filter(|_| opacity >= 1.0) {
            glass.extend(layer.glass.iter().map(|pane| crate::platform::Glass { bounds: moved(pane.bounds).intersect(&mask), radius: pane.radius * grown, opacity: pane.opacity * opacity }));
        }
        self.wanted.extend(layer.wanted.iter().cloned());
        self.framed.extend(layer.framed.iter().map(|(id, bounds)| (*id, moved(*bounds))));
        self.modal = layer.modal.or(self.modal);
        if !layer.kept {
            self.layers.remove(&id);
        }
    }

    /// What node `id` draws as it rests, everywhere, not cut to what it's in: its own scene, and what it says of where
    /// it puts what's in it, apart from this frame's.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn record(&mut self, taffy_node: NodeId, id: Id, offset: Point<f32>, transform: Transform, parent: Option<Id>, pointer: bool, key: LayerKey) -> Layer {
        let mut scene = Scene::default();
        std::mem::swap(&mut *self.scene, &mut scene);
        let outside = (
            std::mem::take(&mut self.placed),
            std::mem::take(&mut self.rects),
            std::mem::take(&mut self.tab_order),
            std::mem::take(&mut self.intersections),
            std::mem::take(&mut self.drag_regions),
            std::mem::take(&mut self.kept_regions),
            self.glass.as_mut().map(std::mem::take),
            std::mem::take(&mut self.wanted),
            self.modal.take(),
            std::mem::take(&mut self.framed),
            std::mem::take(&mut self.animating),
            std::mem::take(&mut self.parts),
        );
        self.recording = Some(id);
        let everywhere = Bounds { origin: point(-1.0e6, -1.0e6), size: Size { width: 2.0e6, height: 2.0e6 } };
        self.paint(taffy_node, offset, transform, everywhere, 1.0, parent, pointer);
        self.recording = None;
        std::mem::swap(&mut *self.scene, &mut scene);
        let (placed, rects, tab_order, intersections, drag_regions, kept_regions, glass, wanted, modal, framed, animating, parts) = outside;
        // What moves by itself or shows another frame's page changes as it's drawn: it's drawn afresh each frame, so
        // it's no image, which would be drawn afresh too.
        let kept = !self.animating && self.framed.is_empty();
        let mut drawn = std::mem::replace(&mut self.parts, parts);
        scene.sort();
        drawn.push(Part::Scene(std::sync::Arc::new(scene), None));
        for part in &mut drawn {
            if let Part::Scene(scene, image) = part {
                *image = kept.then(|| recording(scene)).flatten();
            }
        }
        let layer = Layer {
            key,
            at: transform.offset,
            scale: transform.scale,
            recorded: true,
            kept,
            parts: drawn,
            placed: std::mem::replace(&mut self.placed, placed),
            rects: std::mem::replace(&mut self.rects, rects).into_iter().collect(),
            tab_order: std::mem::replace(&mut self.tab_order, tab_order),
            intersections: std::mem::replace(&mut self.intersections, intersections),
            drag_regions: std::mem::replace(&mut self.drag_regions, drag_regions),
            kept_regions: std::mem::replace(&mut self.kept_regions, kept_regions),
            glass: match glass {
                Some(glass) => self.glass.replace(glass).unwrap_or_default(),
                None => Vec::new(),
            },
            wanted: std::mem::replace(&mut self.wanted, wanted),
            modal: std::mem::replace(&mut self.modal, modal),
            framed: std::mem::replace(&mut self.framed, framed),
        };
        self.animating |= animating;
        layer
    }

    /// Paints `node` with its parent's top left at `offset` in its layout, drawn through `transform` and turned as
    /// what it's in is, clipped to `mask` (in the window), `opacity` as opaque as it shows, taking the pointer if
    /// `pointer`.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn paint(&mut self, taffy_node: NodeId, offset: Point<f32>, transform: Transform, mask: Bounds<f32>, opacity: f32, parent: Option<Id>, pointer: bool) {
        let turn = self.turn;
        self.paint_node(taffy_node, offset, transform, mask, opacity, parent, pointer);
        self.turn = turn;
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn paint_node(&mut self, taffy_node: NodeId, offset: Point<f32>, transform: Transform, mask: Bounds<f32>, opacity: f32, parent: Option<Id>, pointer: bool) {
        let Ok(laid) = self.layout.taffy.layout(taffy_node).copied() else { return };
        // What isn't displayed isn't drawn, and nothing in it is.
        if self.layout.taffy.style(taffy_node).is_ok_and(|style| style.display == taffy::style::Display::None) {
            return;
        }
        let Some(leaf) = self.layout.taffy.get_node_context(taffy_node) else { return };
        let (id, text, mut look, lines) = (leaf.id, leaf.text, leaf.look.clone(), leaf.line_clamp);
        if self.recording == Some(id) {
            // Its layer is drawn as it rests, and moved, grown and faded as it's drawn from it.
            (look.translate, look.scale, look.rest_scale, look.opacity) = ((Offset::Px(0.0), Offset::Px(0.0)), 1.0, 1.0, 1.0);
        } else if self.recording.is_some() && self.layered.contains(&id) && self.turn != TransformationMatrix::unit() {
            // Turned in the layer being drawn, it's drawn in it as usual, which is drawn again each frame.
            self.animating = true;
        } else if self.recording.is_some() && self.layered.contains(&id) {
            // A layer of its own in the one being drawn: drawn into it each frame, where that's moved it.
            // In the order it was drawn, as its image draws its batches (`recording`).
            let mut scene = std::mem::take(&mut *self.scene);
            scene.sort();
            self.parts.push(Part::Scene(std::sync::Arc::new(scene), None));
            self.parts.push(Part::Sublayer(Sublayer { taffy_node, offset, transform, mask, opacity, parent, pointer }));
            return;
        } else if self.layered.contains(&id) && self.unsettled != Some(id) && self.turn == TransformationMatrix::unit() && look.animation.is_none() {
            return self.paint_layer(taffy_node, id, offset, transform, &look, mask, opacity, parent, pointer);
        }
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let laid_out = Bounds { origin: point(offset.x + laid.location.x, offset.y + laid.location.y), size: Size { width: laid.size.width, height: laid.size.height } };
        // On its way from where it was laid out, its box is drawn between, and what's in it moves with it.
        let laid_out = match self.transitions.flip(id, self.time) {
            Some([x, y, wider, taller]) => Bounds { origin: point(laid_out.origin.x + x, laid_out.origin.y + y), size: Size { width: (laid_out.size.width + wider).max(0.0), height: (laid_out.size.height + taller).max(0.0) } },
            None => laid_out,
        };
        // Its own move and growth, around its middle, which moves nothing around it.
        let centre = point(laid_out.origin.x + laid_out.size.width / 2.0, laid_out.origin.y + laid_out.size.height / 2.0);
        let translate = point(look.translate.0.of(laid_out.size.width), look.translate.1.of(laid_out.size.height));
        let transform = if translate == point(0.0, 0.0) && look.scale == 1.0 && look.rest_scale == 1.0 { transform } else { transform.then(translate, look.scale, look.rest_scale, centre) };
        let bounds = transform.bounds(laid_out);
        let opacity = opacity * look.opacity;
        let rotate = look.rotate;
        // Turned around its middle, with what's in it.
        if rotate.rem_euclid(360.0) != 0.0 {
            let middle = transform.point(centre);
            self.turn = TransformationMatrix::rotation(rotate, [middle.x, middle.y]).then(&self.turn);
        }
        let content = transform.bounds(Bounds {
            origin: point(laid_out.origin.x + laid.border.left + laid.padding.left, laid_out.origin.y + laid.border.top + laid.padding.top),
            size: Size { width: laid.content_box_width(), height: laid.content_box_height() },
        });
        let shown = self.turned(bounds).intersect(&mask);
        let root = id == 0;
        // What's inert takes no pointer, nor does what's in it.
        let pointer = !node.is("inert") && !tree.leaving(id) && look.pointer_events.unwrap_or(pointer);
        if node.str("aria-modal") == Some("true") && !tree.leaving_in(id) {
            self.modal = Some(id);
        }
        let drawn = self.turned(bounds);
        self.rects.insert(id, Rect { x: drawn.origin.x, y: drawn.origin.y, width: drawn.size.width, height: drawn.size.height });
        if node.listens("intersection") {
            self.intersections.push((id, drawn.intersects(&self.near)));
        }
        let radii = Corners {
            top_left: corner(look.radii.top_left, bounds.size, transform.scale),
            top_right: corner(look.radii.top_right, bounds.size, transform.scale),
            bottom_right: corner(look.radii.bottom_right, bounds.size, transform.scale),
            bottom_left: corner(look.radii.bottom_left, bounds.size, transform.scale),
        };
        let scaled = |edges: Edges<f32>| Edges { top: edges.top * transform.scale, right: edges.right * transform.scale, bottom: edges.bottom * transform.scale, left: edges.left * transform.scale };
        // Shadows under it, the first nearest, then what shows through it, its fill and border, then shadows inside.
        if !root {
            for shadow in look.shadows.iter().rev().filter(|shadow| !shadow.inset) {
                self.shadow(shadow, bounds, radii, transform.scale, mask, opacity);
            }
            match look.app_region {
                Some(true) => self.drag_regions.push((id, bounds.intersect(&mask))),
                Some(false) => self.kept_regions.push(bounds.intersect(&mask)),
                None => {}
            }
            // The system's own glass is a pane of its own under the page, which can't fade: what's fading is drawn
            // with the renderer's own, which does, as it goes (an owner pill fading off with its pages).
            if let (Some((_, _, true)), Some(glass)) = (look.material, self.glass.as_mut().filter(|_| opacity >= 1.0)) {
                glass.push(crate::platform::Glass { bounds: bounds.intersect(&mask), radius: radii.top_left, opacity });
                if let Some(fill) = self.fill {
                    self.quad(bounds, mask, fill, Edges::default(), Hsla::default(), radii, false, opacity);
                }
            } else if let Some((blur, saturate, _)) = look.material.filter(|_| opacity > 0.0) {
                // Faded all the way out, nothing of it shows, so nothing behind it is blurred.
                self.scene.insert_primitive(Backdrop {
                    order: 0,
                    opacity,
                    blur: ScaledPixels(blur * transform.scale * self.scale),
                    saturation: saturate,
                    bounds: bounds.scale_to(self.scale),
                    content_mask: ContentMask { bounds: mask.scale_to(self.scale) },
                    corner_radii: Corners {
                        top_left: ScaledPixels(radii.top_left * self.scale),
                        top_right: ScaledPixels(radii.top_right * self.scale),
                        bottom_right: ScaledPixels(radii.bottom_right * self.scale),
                        bottom_left: ScaledPixels(radii.bottom_left * self.scale),
                    },
                });
            }
            self.fill(bounds, mask, look.background.unwrap_or_default(), scaled(look.borders), look.border_color, radii, look.dashed, opacity);
            for shadow in look.shadows.iter().rev().filter(|shadow| shadow.inset) {
                self.shadow(shadow, bounds, radii, transform.scale, mask, opacity);
            }
        }
        // A picker is pressed to open the device's (`Screen::click`).
        let pressable = (node.listens("click") || matches!(node.kind, Kind::Input | Kind::Picker)) && !node.is("disabled") && !node.is("aria-disabled");
        // Tab reaches what the DOM's does: an input, what's pressed as a button is (a role the DOM draws as a
        // `<button>`, core/ui's drawing-primitives/primitives.tsx `pressed`), and what says its `tabindex`.
        let tabindex = node.prop("tabindex").and_then(|index| index.as_i64().or_else(|| index.as_str()?.parse().ok()));
        // A text is never a `<button>` on the DOM, whatever its role: it's reached only by its `tabindex`.
        let button = node.kind != Kind::Text && node.listens("click") && node.str("role").is_some_and(|role| BUTTON_ROLES.contains(&role));
        // And a frame, where it goes into the frame's page.
        if (pressable && tabindex.is_none_or(|index| index >= 0) && (node.kind == Kind::Input || button || tabindex.is_some())) || node.kind == Kind::Frame {
            self.tab_order.push(id);
        }
        let focus = match node.str("role") {
            Some("option") => Clicked::Keeps,
            _ if (node.kind == Kind::Input && !node.is("disabled")) || button || tabindex.is_some() || node.kind == Kind::Frame => Clicked::Takes,
            _ => Clicked::Passes,
        };
        let mut placed = Placed { parent, shown, opacity, pointer, cursor: look.cursor, pressable, input: node.kind == Kind::Input, focus, scroll: None, touch_action: look.touch_action };
        match node.kind {
            Kind::Text | Kind::RawText => {
                let width = laid.content_box_width().max(1.0);
                let wrap = text.wrap.then_some(width);
                let kept = self.layout.leaf(taffy_node).paragraph.as_ref().filter(|(wrapped, _)| *wrapped == wrap).map(|(_, paragraph)| paragraph.clone());
                let (runs, spans) = match &self.layout.leaf(taffy_node).measure {
                    Some(Measure::Text { runs, .. }) => (runs.clone(), self.layout.leaf(taffy_node).spans.clone()),
                    _ => (Vec::new(), Vec::new()),
                };
                let paragraph = kept.unwrap_or_else(|| {
                    let words = match &self.layout.leaf(taffy_node).measure {
                        Some(Measure::Text { words, .. }) => words.clone(),
                        _ => String::new(),
                    };
                    let paragraph = Arc::new(self.text.paragraph(&words, &text.style, &runs, wrap, lines));
                    if let Some(leaf) = self.layout.taffy.get_node_context_mut(taffy_node) {
                        leaf.paragraph = Some((wrap, paragraph.clone()));
                    }
                    paragraph
                });
                // Selectable, it's kept as it's drawn, and what's selected of it is tinted behind its words.
                if text.selectable {
                    let words = match &self.layout.leaf(taffy_node).measure {
                        Some(Measure::Text { words, .. }) => words.clone(),
                        _ => String::new(),
                    };
                    let shown = super::super::selection::Shown { paragraph: paragraph.clone(), words, content, width, grown: transform.scale, align: text.align };
                    if let Some((_, (start, end))) = self.selected.filter(|(on, (start, end))| *on == id && start < end) {
                        let (tint, line_height) = (colour_token("blue", self.conditions).opacity(0.3), paragraph.line_height * transform.scale);
                        for (line, x0, x1) in paragraph.spans(start, end) {
                            let bounds = Bounds { origin: point(shown.left(line) + x0 * transform.scale, content.origin.y + line as f32 * line_height), size: Size { width: (x1 - x0) * transform.scale, height: line_height } };
                            self.quad(bounds, mask, tint, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
                        }
                    }
                    self.texts.insert(id, shown);
                } else {
                    self.texts.remove(&id);
                }
                // Words that don't show aren't drawn: their lines start at its content's left and run on to the right.
                let size = paragraph.size();
                let covers = Bounds { origin: content.origin, size: Size { width: size.width.max(width) * transform.scale, height: size.height * transform.scale } };
                if self.turned(covers).intersects(&mask) {
                    self.paint_text(&paragraph, &runs, content, width, transform.scale, text, opacity, mask);
                }
                if !spans.is_empty() {
                    self.placed.push((id, placed));
                    self.place_spans(id, &spans, &paragraph, content, width, transform.scale, text.align, mask, pointer, opacity);
                    return;
                }
            }
            Kind::Input => {
                let padding = transform.bounds(Bounds {
                    origin: point(laid_out.origin.x + laid.border.left, laid_out.origin.y + laid.border.top),
                    size: Size { width: laid.size.width - laid.border.left - laid.border.right, height: laid.size.height - laid.border.top - laid.border.bottom },
                });
                // Several lines scroll by the wheel and a trackpad too, as a `<textarea>` does.
                placed.scroll = self.paint_input(id, node, content, padding, laid.content_box_width(), transform.scale, text, opacity, mask).map(|most| (most, 0.0, false));
            }
            Kind::Image => self.paint_image(node, content, transform, radii, opacity, mask),
            Kind::Path => self.paint_path(node, content, &look, text.color, opacity, mask),
            // Its latest frame, as the web's browser plays it, or as its connection decoded it natively.
            Kind::Video => self.paint_video(id, content, radii, opacity, mask),
            Kind::Picker => {}
            // A frame's page, drawn as it last drew itself, in its box, where the frame comes in this scene's order.
            Kind::Frame => {
                self.framed.push((id, content));
                if let Some(child) = self.frames.get(&id) {
                    self.scene.splice(&child.scene, 1.0, [content.origin.x * self.scale, content.origin.y * self.scale], 1.0, shown.intersect(&content).scale_to(self.scale));
                }
            }
            Kind::Box | Kind::Scroll | Kind::List => {
                let mut child_mask = mask;
                let mut child_offset = laid_out.origin;
                let around = self.near;
                let mut near = self.window;
                if look.clips || look.scrolls {
                    let padding_box = transform.bounds(Bounds {
                        origin: point(laid_out.origin.x + laid.border.left, laid_out.origin.y + laid.border.top),
                        size: Size { width: laid.size.width - laid.border.left - laid.border.right, height: laid.size.height - laid.border.top - laid.border.bottom },
                    });
                    child_mask = self.turned(padding_box).intersect(&mask);
                    near = self.turned(padding_box);
                    if look.scrolls {
                        near = self.grown(near);
                        let inner = Size { width: laid.size.width - laid.border.left - laid.border.right, height: laid.size.height - laid.border.top - laid.border.bottom };
                        let most = point((laid.scrollable_overflow_rect.right - inner.width).max(0.0), (laid.scrollable_overflow_rect.bottom - inner.height).max(0.0));
                        let now = self.scrolled.entry(id).or_insert(point(0.0, 0.0));
                        *now = point(now.x.clamp(0.0, most.x), now.y.clamp(0.0, most.y));
                        let stretched = self.stretched.get(&id).copied().unwrap_or(point(0.0, 0.0));
                        child_offset = point(child_offset.x - now.x - stretched.x, child_offset.y - now.y - stretched.y);
                        let along = node.kind == Kind::Scroll && most.x > 0.0 && most.y == 0.0;
                        placed.scroll = Some((most.y, most.x, along));
                        placed.shown = child_mask;
                    }
                }
                self.placed.push((id, placed));
                // Its children in order, those with a higher `zIndex` later, and positioned ones after the rest of theirs.
                let mut children: Vec<(usize, NodeId, i32, bool)> = self
                    .layout
                    .taffy
                    .children(taffy_node)
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                    .map(|(at, child)| {
                        let leaf = self.layout.leaf(child);
                        (at, child, leaf.look.z_index, leaf.positioned)
                    })
                    .collect();
                children.sort_by_key(|(at, _, z, positioned)| (*z, *positioned, *at));
                // What scrolls is near as far as it'd be scrolled into view, however tight what it's in clips: its box
                // and what it's in, each grown by the margin. What only clips cuts what's near to it.
                let inside = match (look.scrolls, look.clips) {
                    (true, _) => self.grown(around).intersect(&near),
                    (false, true) => around.intersect(&near),
                    (false, false) => around,
                };
                for (_, child, _, _) in children {
                    if self.layout.leaf(child).fixed {
                        let window = self.window;
                        self.near = self.grown(window);
                        self.paint(child, point(0.0, 0.0), Transform::NONE, window, 1.0, Some(id), pointer);
                    } else {
                        self.near = inside;
                        self.paint(child, child_offset, transform, child_mask, opacity, Some(id), pointer);
                    }
                }
                self.near = around;
                return;
            }
        }
        self.placed.push((id, placed));
    }
}
