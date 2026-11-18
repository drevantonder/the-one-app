//! A strict screen: its tree (the ops its JS sent, `tree`), laid out with taffy, drawn as GPUI's scene in device
//! pixels, and what a finger and the keyboard do to it.

use std::collections::HashMap;

use cosmic_text::Buffer;
use taffy::prelude::*;

use crate::edit::Edit;
use crate::renderer::Renderer;
use crate::scene::{Background, Bounds, Corners, Edges, Hsla, PolychromeSprite, Quad, Scene, Underline};
use crate::text::{Font, Text};
use crate::tokens::{TOKENS, type_style};
use crate::tree::{Id, Kind, Length, Node, Style, Tree};

/// What a leaf of the layout is measured by.
#[derive(Clone, Copy)]
enum Leaf {
    Text(Id),
    Input(Id),
}

/// A drawn node's place on screen (clipped by what scrolls it), for hit testing and the semantic tree.
#[derive(Clone, Copy)]
pub struct Hit {
    pub id: Id,
    pub bounds: Bounds,
}

pub struct View {
    pub tree: Tree,
    pub density: f32,
    pub dark: bool,
    pub size: (f32, f32),
    /// The window's insets (status bar, and navigation bar or keyboard), in device pixels.
    pub insets: (f32, f32),
    pub text: Text,
    pub scene: Scene,
    pub dirty: bool,
    taffy: TaffyTree<Leaf>,
    root: Option<NodeId>,
    nodes: HashMap<Id, NodeId>,
    buffers: HashMap<Id, (String, Font, Option<f32>, Option<usize>, Buffer)>,
    pub hits: Vec<Hit>,
    pub scroll: HashMap<Id, f32>,
    pub content: HashMap<Id, f32>,
    pub edits: HashMap<Id, Edit>,
    pub focus: Option<Id>,
    /// Each input's text's horizontal scroll, so the caret stays in view.
    input_scroll: HashMap<Id, f32>,
    /// Where each input's text starts on screen, for placing the caret by a finger.
    input_origin: HashMap<Id, [f32; 2]>,
    images: Box<dyn FnMut(&str) -> Option<Vec<u8>>>,
}

impl View {
    pub fn new(tree: Tree, density: f32, dark: bool, images: Box<dyn FnMut(&str) -> Option<Vec<u8>>>) -> anyhow::Result<View> {
        Ok(View {
            tree,
            density,
            dark,
            size: (0.0, 0.0),
            insets: (0.0, 0.0),
            text: Text::new()?,
            scene: Scene::default(),
            dirty: true,
            taffy: TaffyTree::new(),
            root: None,
            nodes: HashMap::new(),
            buffers: HashMap::new(),
            hits: Vec::new(),
            scroll: HashMap::new(),
            content: HashMap::new(),
            edits: HashMap::new(),
            focus: None,
            input_scroll: HashMap::new(),
            input_origin: HashMap::new(),
            images,
        })
    }

    fn colour(&self, name: &str) -> Hsla {
        TOKENS.colour(name, self.dark).map(|c| Hsla::from_rgba(c.r, c.g, c.b, c.a)).unwrap_or_default()
    }

    /// A space or a size: pixels, or a token (`gap`, `gutter`, `control`), under a finger as tokens.css has it.
    fn token(&self, word: &str) -> Option<f32> {
        touch(word).or_else(|| TOKENS.length(word)).map(|px| px * self.density)
    }

    fn font(&self, style: &Style) -> Font {
        let name = style.font.as_deref().unwrap_or("body");
        let ts = type_style(name, self.dark);
        let size = match name {
            "body" | "label" => touch("text-base").unwrap_or(ts.size),
            _ => ts.size,
        } * self.density;
        Font { size, line_height: (size * ts.line_height).round(), weight: ts.weight, tabular: ts.tabular || style.font_variant_numeric.as_deref() == Some("tabular-nums") }
    }

    fn lp(&self, length: &Length) -> LengthPercentage {
        match length {
            Length::Px(px) => LengthPercentage::length(px * self.density),
            Length::Word(word) if word.ends_with('%') => LengthPercentage::percent(word.trim_end_matches('%').parse::<f32>().unwrap_or(0.0) / 100.0),
            Length::Word(word) => LengthPercentage::length(self.token(word).unwrap_or(0.0)),
        }
    }

    fn lpa(&self, length: &Length) -> LengthPercentageAuto {
        match length {
            Length::Word(word) if word == "auto" => LengthPercentageAuto::auto(),
            Length::Px(px) => LengthPercentageAuto::length(px * self.density),
            Length::Word(word) if word.ends_with('%') => LengthPercentageAuto::percent(word.trim_end_matches('%').parse::<f32>().unwrap_or(0.0) / 100.0),
            Length::Word(word) => LengthPercentageAuto::length(self.token(word).unwrap_or(0.0)),
        }
    }

    fn dim(&self, length: &Length) -> Dimension {
        match length {
            Length::Word(word) if word == "auto" => Dimension::auto(),
            Length::Px(px) => Dimension::length(px * self.density),
            Length::Word(word) if word.ends_with('%') => Dimension::percent(word.trim_end_matches('%').parse::<f32>().unwrap_or(0.0) / 100.0),
            Length::Word(word) => Dimension::length(self.token(word).unwrap_or(0.0)),
        }
    }

    /// The protocol's style as taffy's: a box is a flex column that doesn't shrink, as the DOM host draws one.
    fn layout_style(&self, node: &Node) -> taffy::Style {
        let s = &node.style;
        let mut out = taffy::Style { display: Display::Flex, flex_direction: FlexDirection::Column, scrollbar_width: 0.0, ..Default::default() };
        if s.display.as_deref() == Some("none") {
            out.display = Display::None;
        }
        // A box doesn't shrink, as the DOM host draws one; an image is a replaced element, which keeps its size.
        out.flex_shrink = if matches!(node.kind, Kind::Box | Kind::Image) { 0.0 } else { 1.0 };
        out.item_is_replaced = node.kind == Kind::Image;
        if let Some(d) = s.flex_direction.as_deref() {
            out.flex_direction = if d == "row" { FlexDirection::Row } else { FlexDirection::Column };
        }
        if s.flex_wrap.as_deref() == Some("wrap") {
            out.flex_wrap = FlexWrap::Wrap;
        }
        if let Some(v) = s.flex_grow { out.flex_grow = v; }
        if let Some(v) = s.flex_shrink { out.flex_shrink = v; }
        if let Some(v) = &s.flex_basis { out.flex_basis = self.dim(v); }
        let align = |v: &str| match v {
            "center" => Some(AlignItems::CENTER),
            "start" | "flex-start" => Some(AlignItems::FLEX_START),
            "end" | "flex-end" => Some(AlignItems::FLEX_END),
            "baseline" => Some(AlignItems::BASELINE),
            "stretch" => Some(AlignItems::STRETCH),
            _ => None,
        };
        if let Some(v) = s.align_items.as_deref().and_then(align) { out.align_items = v; }
        if let Some(v) = s.align_self.as_deref().and_then(align) { out.align_self = Some(v); }
        if let Some(v) = s.justify_content.as_deref() {
            out.justify_content = match v {
                "center" => JustifyContent::CENTER,
                "space-between" => JustifyContent::SPACE_BETWEEN,
                "space-around" => JustifyContent::SPACE_AROUND,
                "end" | "flex-end" => JustifyContent::FLEX_END,
                _ => JustifyContent::FLEX_START,
            };
        }
        let gap = s.gap.as_ref().map(|g| self.lp(g));
        if let Some(g) = gap.or(s.row_gap.as_ref().map(|g| self.lp(g))) { out.gap.height = g; }
        if let Some(g) = gap.or(s.column_gap.as_ref().map(|g| self.lp(g))) { out.gap.width = g; }
        let all = s.padding.as_ref().map(|p| self.lp(p));
        let inline = s.padding_inline.as_ref().map(|p| self.lp(p)).or(all);
        let block = s.padding_block.as_ref().map(|p| self.lp(p)).or(all);
        let zero = LengthPercentage::length(0.0);
        out.padding = Rect {
            top: s.padding_top.as_ref().map(|p| self.lp(p)).or(block).unwrap_or(zero),
            bottom: s.padding_bottom.as_ref().map(|p| self.lp(p)).or(block).unwrap_or(zero),
            left: s.padding_left.as_ref().map(|p| self.lp(p)).or(inline).unwrap_or(zero),
            right: s.padding_right.as_ref().map(|p| self.lp(p)).or(inline).unwrap_or(zero),
        };
        let m_all = s.margin.as_ref().map(|m| self.lpa(m));
        let m_inline = s.margin_inline.as_ref().map(|m| self.lpa(m)).or(m_all);
        let m_block = s.margin_block.as_ref().map(|m| self.lpa(m)).or(m_all);
        let mzero = LengthPercentageAuto::length(0.0);
        out.margin = Rect { top: m_block.unwrap_or(mzero), bottom: m_block.unwrap_or(mzero), left: m_inline.unwrap_or(mzero), right: m_inline.unwrap_or(mzero) };
        if let Some(v) = &s.width { out.size.width = self.dim(v); }
        if let Some(v) = &s.height { out.size.height = self.dim(v); }
        if let Some(v) = &s.min_width { out.min_size.width = self.lpa(v); }
        if let Some(v) = &s.min_height { out.min_size.height = self.lpa(v); }
        if let Some(v) = &s.max_width { out.max_size.width = self.lpa(v); }
        if let Some(v) = &s.max_height { out.max_size.height = self.lpa(v); }
        out.aspect_ratio = s.aspect_ratio;
        if matches!(s.overflow.as_deref(), Some("scroll" | "auto" | "hidden")) {
            out.overflow = taffy::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll };
        }
        let (top, right, bottom, left) = borders(s);
        out.border = Rect {
            top: LengthPercentage::length(top * self.density),
            right: LengthPercentage::length(right * self.density),
            bottom: LengthPercentage::length(bottom * self.density),
            left: LengthPercentage::length(left * self.density),
        };
        out
    }

    fn build(&mut self, id: Id) -> Option<NodeId> {
        let node = self.tree.nodes.get(&id)?;
        let style = self.layout_style(node);
        let taffy_node = match node.kind {
            Kind::Text | Kind::RawText => self.taffy.new_leaf_with_context(style, Leaf::Text(id)).ok()?,
            Kind::Input => self.taffy.new_leaf_with_context(style, Leaf::Input(id)).ok()?,
            Kind::Image => self.taffy.new_leaf(style).ok()?,
            Kind::Box => {
                let children: Vec<Id> = node.children.clone();
                let children: Vec<NodeId> = children.into_iter().filter_map(|child| self.build(child)).collect();
                self.taffy.new_with_children(style, &children).ok()?
            }
        };
        self.nodes.insert(id, taffy_node);
        Some(taffy_node)
    }

    /// Lays the tree out again, as its ops or the window changed it.
    pub fn layout(&mut self) {
        self.taffy = TaffyTree::new();
        self.nodes.clear();
        let children: Vec<Id> = self.tree.root().children.clone();
        let children: Vec<NodeId> = children.into_iter().filter_map(|child| self.build(child)).collect();
        let root = taffy::Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            size: Size { width: Dimension::length(self.size.0), height: Dimension::length(self.size.1) },
            padding: Rect { top: LengthPercentage::length(self.insets.0), bottom: LengthPercentage::length(self.insets.1), left: LengthPercentage::length(0.0), right: LengthPercentage::length(0.0) },
            ..Default::default()
        };
        let root = self.taffy.new_with_children(root, &children).expect("a root");
        self.nodes.insert(0, root);
        self.root = Some(root);

        let fonts: HashMap<Id, Font> = self.tree.nodes.iter().filter(|(_, n)| matches!(n.kind, Kind::Text | Kind::RawText | Kind::Input)).map(|(id, n)| (*id, self.font(&n.style))).collect();
        let tree = &self.tree;
        let text = &mut self.text;
        let buffers = &mut self.buffers;
        let _ = self.taffy.compute_layout_with_measure(root, Size::MAX_CONTENT, |input, _node, context, style| {
            taffy::compute_leaf_layout(input, style, |_, _| 0.0, |known, available| {
                let Some(leaf) = context else { return Size::ZERO };
                match *leaf {
                    Leaf::Text(id) => {
                        let node = &tree.nodes[&id];
                        let width = known.width.or(match available.width {
                            AvailableSpace::Definite(w) => Some(w),
                            AvailableSpace::MinContent => Some(0.0),
                            AvailableSpace::MaxContent => None,
                        });
                        let content = tree.text(id);
                        let font = fonts[&id];
                        let lines = node.style.line_clamp;
                        let entry = buffers.entry(id).or_insert_with(|| (String::new(), font, None, None, text.buffer("", font, None, None)));
                        if entry.0 != content || entry.1 != font || entry.2 != width || entry.3 != lines {
                            text.set(&mut entry.4, &content, font, width, lines);
                            (entry.0, entry.1, entry.2, entry.3) = (content, font, width, lines);
                        }
                        let (w, h) = Text::size(&entry.4);
                        Size { width: known.width.unwrap_or(w.ceil()), height: known.height.unwrap_or(h.ceil()) }
                    }
                    Leaf::Input(id) => {
                        let font = fonts[&id];
                        Size { width: known.width.unwrap_or(0.0), height: known.height.unwrap_or(font.line_height) }
                    }
                }
            })
        });
        self.dirty = true;
    }

    /// Draws the tree into the scene, and notes where each node landed.
    pub fn paint(&mut self, renderer: &mut Renderer) {
        self.scene.clear();
        self.hits.clear();
        let screen = Bounds::new(0.0, 0.0, self.size.0, self.size.1);
        let background = self.colour("background");
        self.scene.quad(Quad {
            order: 0,
            border_style: 0,
            bounds: screen,
            content_mask: screen,
            background: Background::solid(background),
            border_color: Hsla::default(),
            corner_radii: Corners::default(),
            border_widths: Edges::default(),
        });
        let children = self.tree.root().children.clone();
        let origin = [0.0, 0.0];
        let fg = self.colour("foreground");
        for child in children {
            self.paint_node(child, origin, screen, fg, renderer);
        }
        self.dirty = false;
    }

    fn paint_node(&mut self, id: Id, parent: [f32; 2], clip: Bounds, inherited: Hsla, renderer: &mut Renderer) {
        let Some(&taffy_node) = self.nodes.get(&id) else { return };
        let Ok(layout) = self.taffy.layout(taffy_node).cloned() else { return };
        let Some(node) = self.tree.nodes.get(&id) else { return };
        if node.style.display.as_deref() == Some("none") {
            return;
        }
        let kind = node.kind;
        let style = node.style.clone();
        let bounds = Bounds::new(parent[0] + layout.location.x, parent[1] + layout.location.y, layout.size.width, layout.size.height);
        if bounds.intersect(&clip).size[1] <= 0.0 && kind != Kind::RawText {
            return;
        }
        self.hits.push(Hit { id, bounds: bounds.intersect(&clip) });
        let colour = style.color.as_deref().map(|c| self.colour(c)).unwrap_or(inherited);
        let opacity = style.opacity.unwrap_or(1.0);
        let radius = style.border_radius.as_ref().map_or(0.0, |r| match r {
            Length::Px(px) => px * self.density,
            Length::Word(word) if word == "full" => 9999.0,
            Length::Word(word) => TOKENS.length(&format!("radius-{word}")).unwrap_or(0.0) * self.density,
        });
        let (bt, br, bb, bl) = borders(&style);
        let background = style.background_color.as_deref().map(|c| self.colour(c).alpha(opacity));
        if background.is_some() || bt + br + bb + bl > 0.0 {
            self.scene.quad(Quad {
                order: 0,
                border_style: 0,
                bounds,
                content_mask: clip,
                background: Background::solid(background.unwrap_or_default()),
                border_color: style.border_color.as_deref().map(|c| self.colour(c)).unwrap_or_else(|| self.colour("border")),
                corner_radii: Corners::all(radius),
                border_widths: Edges { top: bt * self.density, right: br * self.density, bottom: bb * self.density, left: bl * self.density },
            });
        }
        match kind {
            Kind::Text | Kind::RawText => {
                if let Some((_, font, width, lines, _)) = self.buffers.get(&id).map(|(a, b, c, d, _)| (a.clone(), *b, *c, *d, ())) {
                    let content_width = layout.size.width - layout.padding.left - layout.padding.right;
                    if width != Some(content_width) {
                        let content = self.tree.text(id);
                        let entry = self.buffers.get_mut(&id).expect("there");
                        self.text.set(&mut entry.4, &content, font, Some(content_width), lines);
                        entry.2 = Some(content_width);
                    }
                    let buffer = &self.buffers[&id].4;
                    let origin = [bounds.origin[0] + layout.padding.left, bounds.origin[1] + layout.padding.top];
                    self.text.paint(buffer, origin, colour.alpha(opacity), clip, renderer, &mut self.scene);
                }
            }
            Kind::Image => {
                if let Some(src) = node.str("src").map(str::to_owned) {
                    let (w, h) = (bounds.size[0].round() as u32, bounds.size[1].round() as u32);
                    let images = &mut self.images;
                    let key = crate::atlas::AtlasKey::Image(format!("{src}@{w}x{h}"));
                    let tile = renderer.atlas.get_or_insert_with(&key, crate::atlas::Kind::Polychrome, || {
                        let png = images(&src)?;
                        let image = image::load_from_memory(&png).ok()?.resize_exact(w.max(1), h.max(1), image::imageops::FilterType::Lanczos3);
                        Some(([w as i32, h as i32], crate::text::rgba_to_bgra(image.to_rgba8().into_raw())))
                    });
                    if let Some(tile) = tile {
                        self.scene.polychrome(PolychromeSprite { order: 0, pad: 0, grayscale: 0, opacity, bounds, content_mask: clip, corner_radii: Corners::all(radius), tile });
                    }
                }
            }
            Kind::Input => self.paint_input(id, bounds, &layout, clip, colour, renderer),
            Kind::Box => {
                let scrolls = matches!(style.overflow.as_deref(), Some("scroll" | "auto" | "hidden"));
                let offset = if scrolls { self.scroll.get(&id).copied().unwrap_or(0.0) } else { 0.0 };
                let inner = if scrolls { bounds.intersect(&clip) } else { clip };
                let children = node.children.clone();
                let mut bottom: f32 = 0.0;
                for child in children {
                    if let Some(l) = self.nodes.get(&child).and_then(|n| self.taffy.layout(*n).ok()) {
                        bottom = bottom.max(l.location.y + l.size.height);
                    }
                    self.paint_node(child, [bounds.origin[0], bounds.origin[1] - offset], inner, colour, renderer);
                }
                if scrolls {
                    self.content.insert(id, (bottom + layout.padding.bottom - layout.size.height).max(0.0));
                }
            }
        }
    }

    fn paint_input(&mut self, id: Id, bounds: Bounds, layout: &taffy::Layout, clip: Bounds, colour: Hsla, renderer: &mut Renderer) {
        let node = &self.tree.nodes[&id];
        let font = self.font(&node.style);
        let secure = node.str("type") == Some("password");
        let placeholder = node.str("placeholder").unwrap_or("").to_owned();
        let edit = self.edits.entry(id).or_insert_with(|| Edit { secure, ..Default::default() }).clone();
        let focused = self.focus == Some(id);
        let shown = edit.display();
        let (content, colour) = if shown.is_empty() { (placeholder, self.colour("muted-foreground")) } else { (shown, colour) };
        let buffer = self.text.buffer(&content, font, None, None);
        let inner = Bounds::new(
            bounds.origin[0] + layout.padding.left + layout.border.left,
            bounds.origin[1] + layout.padding.top,
            (bounds.size[0] - layout.padding.left - layout.padding.right - layout.border.left - layout.border.right).max(1.0),
            bounds.size[1] - layout.padding.top - layout.padding.bottom,
        );
        let x_at = |buffer: &Buffer, byte: usize| -> f32 {
            for run in buffer.layout_runs() {
                for glyph in run.glyphs {
                    if glyph.start >= byte {
                        return glyph.x;
                    }
                }
                return run.line_w;
            }
            0.0
        };
        let has_text = !edit.text.is_empty();
        let caret = if has_text { x_at(&buffer, edit.display_index(edit.extent)) } else { 0.0 };
        let mut scroll = self.input_scroll.get(&id).copied().unwrap_or(0.0);
        let caret_width = (1.5 * self.density).round();
        if focused {
            if caret - scroll > inner.size[0] - caret_width {
                scroll = caret - inner.size[0] + caret_width;
            }
            if caret < scroll {
                scroll = caret;
            }
        }
        if !has_text {
            scroll = 0.0;
        }
        self.input_scroll.insert(id, scroll);
        let field_clip = inner.intersect(&clip);
        let top = inner.origin[1] + ((inner.size[1] - font.line_height) / 2.0).round();
        let origin = [inner.origin[0] - scroll, top];
        self.input_origin.insert(id, origin);
        if focused && has_text && !edit.collapsed() {
            let (start, end) = edit.selection();
            let (x0, x1) = (x_at(&buffer, edit.display_index(start)), x_at(&buffer, edit.display_index(end)));
            let selection = self.colour("primary").alpha(0.25);
            self.scene.quad(Quad {
                order: 0,
                border_style: 0,
                bounds: Bounds::new(origin[0] + x0, top, x1 - x0, font.line_height),
                content_mask: field_clip,
                background: Background::solid(selection),
                border_color: Hsla::default(),
                corner_radii: Corners::default(),
                border_widths: Edges::default(),
            });
        }
                self.text.paint(&buffer, origin, colour, field_clip, renderer, &mut self.scene);
        if let Some((start, end)) = edit.composing.filter(|_| has_text) {
            let (x0, x1) = (x_at(&buffer, edit.display_index(start)), x_at(&buffer, edit.display_index(end)));
            let thickness = self.density.round().max(1.0);
            self.scene.underline(Underline {
                order: 0,
                pad: 0,
                bounds: Bounds::new(origin[0] + x0, top + font.line_height - 2.0 * thickness, x1 - x0, thickness),
                content_mask: field_clip,
                color: colour,
                thickness,
                wavy: 0,
            });
        }
        if focused && edit.collapsed() {
            self.scene.quad(Quad {
                order: 0,
                border_style: 0,
                bounds: Bounds::new(origin[0] + caret, top + 0.1 * font.line_height, caret_width, 0.8 * font.line_height),
                content_mask: field_clip,
                background: Background::solid(self.colour("primary")),
                border_color: Hsla::default(),
                corner_radii: Corners::default(),
                border_widths: Edges::default(),
            });
        }
        if focused {
            // A focused field's ring, as the DOM host's focus-visible.
            self.scene.quad(Quad {
                order: 0,
                border_style: 0,
                bounds,
                content_mask: clip,
                background: Background::default(),
                border_color: self.colour("ring"),
                corner_radii: Corners::all(TOKENS.length("radius-md").unwrap_or(8.0) * self.density),
                border_widths: Edges { top: 2.0 * self.density, right: 2.0 * self.density, bottom: 2.0 * self.density, left: 2.0 * self.density },
            });
        }
    }

    /// The text byte of an input under `x` on screen.
    pub fn input_byte(&mut self, id: Id, x: f32) -> usize {
        let Some(edit) = self.edits.get(&id).cloned() else { return 0 };
        let node = &self.tree.nodes[&id];
        let font = self.font(&node.style);
        let buffer = self.text.buffer(&edit.display(), font, None, None);
        let origin = self.input_origin.get(&id).copied().unwrap_or([0.0, 0.0]);
        let x = x - origin[0];
        let mut byte = edit.display().len();
        'runs: for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                if x < glyph.x + glyph.w / 2.0 {
                    byte = glyph.start;
                    break 'runs;
                }
            }
        }
        edit.model_index(byte)
    }

    /// The topmost node under a point that does something with a tap: listens for a click, or takes text.
    pub fn target(&self, x: f32, y: f32) -> Option<Id> {
        self.hits.iter().rev().filter(|hit| hit.bounds.contains(x, y)).map(|hit| hit.id).find(|id| {
            let node = &self.tree.nodes[id];
            node.kind == Kind::Input || node.listens("click")
        })
    }

    /// The innermost box under a point that scrolls.
    pub fn scroller(&self, x: f32, y: f32) -> Option<Id> {
        self.hits.iter().rev().filter(|hit| hit.bounds.contains(x, y)).map(|hit| hit.id).find(|id| self.content.contains_key(id))
    }

    pub fn bounds(&self, id: Id) -> Option<Bounds> {
        self.hits.iter().find(|hit| hit.id == id).map(|hit| hit.bounds)
    }

    /// The inputs in the tree's order.
    pub fn inputs(&self) -> Vec<Id> {
        fn walk(tree: &Tree, id: Id, out: &mut Vec<Id>) {
            let Some(node) = tree.nodes.get(&id) else { return };
            if node.kind == Kind::Input {
                out.push(id);
            }
            for child in &node.children {
                walk(tree, *child, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.tree, 0, &mut out);
        out
    }
}

fn borders(s: &Style) -> (f32, f32, f32, f32) {
    let all = s.border_width.unwrap_or(0) as f32;
    (
        s.border_top_width.map_or(all, f32::from),
        s.border_right_width.map_or(all, f32::from),
        s.border_bottom_width.map_or(all, f32::from),
        s.border_left_width.map_or(all, f32::from),
    )
}

/// A length token as tokens.css has it under a finger (`@media (pointer: coarse)`), in CSS pixels: the shared
/// `tokens` reads only a computer's rule.
fn touch(name: &str) -> Option<f32> {
    let css = include_str!("../../../../core/ui/src/tokens.css");
    let block = &css[css.find("(pointer: coarse)")?..];
    let block = &block[..block.find("\n}")?];
    let at = block.find(&format!("--{name}:"))?;
    let value = block[at..].split_once(':')?.1.split(';').next()?.trim();
    value.strip_suffix("rem").and_then(|rem| rem.parse::<f32>().ok()).map(|rem| rem * 16.0).or_else(|| value.strip_suffix("px")?.parse().ok())
}
