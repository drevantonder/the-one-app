//! How a screen's tree is styled, laid out and painted, a frame at a time: the painter (`Painter`), the layout it keeps
//! from frame to frame (`Retained`), and the layers what only moves is drawn from (`Layer`), as GPUI's and Flutter's
//! paint passes are their own.
use super::*;

/// What a leaf measures for layout.
#[derive(PartialEq)]
pub(super) enum Measure {
    /// A text's words, and its runs where texts in it set some of them another way.
    Text { words: String, runs: Vec<TextRun>, text: Text, lines: Option<usize> },
    Input { words: String, text: Text, multiline: bool },
    /// A frame, as tall as its page says its content is (`Screen::content_height`), or an iframe's 150 pixels.
    Frame { height: Option<f32> },
}

/// A tree node as laid out: which node, the text it inherits, how it looks, and what it measures; and a text's words
/// as they were last wrapped, to the width they were wrapped to (`None`, as wide as they are), kept until it's styled
/// again.
pub(super) struct Leaf {
    pub(super) id: Id,
    pub(super) text: Text,
    pub(super) look: Look,
    pub(super) line_clamp: Option<usize>,
    pub(super) positioned: bool,
    /// Placed in the window, not in its parent (`position: fixed`).
    pub(super) fixed: bool,
    pub(super) measure: Option<Measure>,
    pub(super) paragraph: Option<(Option<f32>, Arc<Paragraph>)>,
    /// Where each text and raw text inside a text is in its words, to be pressed, hovered, reached and read there.
    pub(super) spans: Vec<Span>,
}

/// A text or raw text inside a text, by where its words are in the text's (bytes `start..end`), with its pointer's
/// shape and whether it takes the pointer, as its style says.
#[derive(Clone)]
pub(super) struct Span {
    pub(super) id: Id,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) cursor: Option<Cursor>,
    pub(super) pointer: Option<bool>,
}

/// A picture's size in device pixels, as the atlas holds it: its pixels are the atlas's alone once it has them.
#[derive(Clone, Copy)]
pub(crate) struct Picture {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// A picture, decoded, in the atlas's byte order (BGRA).
pub(crate) struct Decoded {
    pub(crate) picture: Picture,
    pub(crate) bgra: Vec<u8>,
}

/// Where a node's layout is drawn in the window: moved by `offset` and grown by `scale`, as its translates and
/// scales and its parents' say.
#[derive(Clone, Copy)]
pub(super) struct Transform {
    pub(super) offset: Point<f32>,
    pub(super) scale: f32,
}

impl Transform {
    pub(super) const NONE: Transform = Transform { offset: Point { x: 0.0, y: 0.0 }, scale: 1.0 };

    pub(super) fn point(&self, at: Point<f32>) -> Point<f32> {
        point(self.offset.x + at.x * self.scale, self.offset.y + at.y * self.scale)
    }

    pub(super) fn bounds(&self, bounds: Bounds<f32>) -> Bounds<f32> {
        Bounds { origin: self.point(bounds.origin), size: Size { width: bounds.size.width * self.scale, height: bounds.size.height * self.scale } }
    }

    /// This, then a node's own move and growth around its middle, `centre`.
    pub(super) fn then(&self, translate: Point<f32>, scale: f32, centre: Point<f32>) -> Transform {
        // p ↦ centre + translate + (p − centre)·scale, then this.
        let moved = point(centre.x + translate.x - centre.x * scale, centre.y + translate.y - centre.y * scale);
        Transform { offset: self.point(moved), scale: self.scale * scale }
    }
}

/// A node's subtree as it was drawn at rest, kept while the node only moves, grows or fades, so each frame moves it
/// rather than drawing it again (`Painter::paint_layer`): its scene, and where it put what's in it.
pub(super) struct Layer {
    pub(super) key: LayerKey,
    /// Where what it's in had moved it as it was drawn: it's drawn where that's moved it since, as it is.
    pub(super) at: Point<f32>,
    /// It's been drawn, rather than only noted where it rests, as that was changing.
    pub(super) recorded: bool,
    /// How much what it's in had grown it as it was drawn.
    pub(super) scale: f32,
    /// It's kept for the next frame: what's in it doesn't move by itself.
    pub(super) kept: bool,
    /// What it draws, in order: its scenes, and between them what's in it that's a layer of its own (`Sublayer`).
    pub(super) parts: Vec<Part>,
    pub(super) placed: Vec<(Id, Placed)>,
    pub(super) rects: Vec<(Id, Rect)>,
    pub(super) tab_order: Vec<Id>,
    pub(super) intersections: Vec<(Id, bool)>,
    pub(super) drag_regions: Vec<(Id, Bounds<f32>)>,
    pub(super) kept_regions: Vec<Bounds<f32>>,
    pub(super) glass: Vec<crate::platform::Glass>,
    pub(super) wanted: Vec<String>,
    pub(super) modal: Option<Id>,
    pub(super) framed: Vec<(Id, Bounds<f32>)>,
}

impl Layer {
    /// One noted where it rests, not drawn, as that's changing.
    pub(super) fn unsettled(key: LayerKey) -> Layer {
        Layer {
            key,
            at: point(0.0, 0.0),
            scale: 1.0,
            recorded: false,
            kept: true,
            parts: Vec::new(),
            placed: Vec::new(),
            rects: Vec::new(),
            tab_order: Vec::new(),
            intersections: Vec::new(),
            drag_regions: Vec::new(),
            kept_regions: Vec::new(),
            glass: Vec::new(),
            wanted: Vec::new(),
            modal: None,
            framed: Vec::new(),
        }
    }
}

/// What a layer was drawn for: where its node rests and the window's scale. Any other, and it's drawn again; what it's
/// in moving, growing or fading it (a card it's in sliding as it grows) only moves, grows and fades it, as a compositor
/// does a sublayer.
#[derive(PartialEq)]
pub(super) struct LayerKey {
    pub(super) laid_out: Bounds<f32>,
    pub(super) device: f32,
}

/// Part of what a layer draws: a scene of it, drawn as an image of it (`Recording`, as a compositor draws a layer,
/// once, with `Scene::raster`) where it can be one, or what's in it that's a layer of its own.
#[derive(Clone)]
pub(super) enum Part {
    Scene(std::sync::Arc<Scene>, Option<Recording>),
    Sublayer(Sublayer),
}

/// What's in a layer that's a layer of its own, and what it's painted with as the layer it's in rests.
#[derive(Clone, Copy)]
pub(super) struct Sublayer {
    pub(super) taffy_node: NodeId,
    pub(super) offset: Point<f32>,
    pub(super) transform: Transform,
    pub(super) mask: Bounds<f32>,
    pub(super) opacity: f32,
    pub(super) parent: Option<Id>,
    pub(super) pointer: bool,
}

/// The tree as Taffy lays it out, kept from frame to frame, as GPUI's and Flutter's are: a frame styles again only what
/// changed (a mutation, a state, what it inherits, a transition), tells Taffy only what's different, and Taffy lays out
/// again only what that touches, from its cache; a change of style that doesn't move anything lays out nothing again.
pub(super) struct Retained {
    pub(super) taffy: TaffyTree<Leaf>,
    pub(super) kept: HashMap<Id, Kept>,
    /// What's placed in the window (`position: fixed`), laid out as the root's children after its own.
    pub(super) fixed: HashSet<Id>,
    /// What's about to move, grow or fade (`Look::will_change`), drawn as a layer while it holds still too.
    pub(super) will_change: HashSet<Id>,
    /// The conditions it was styled in: when they change, everything's styled again.
    pub(super) conditions: Option<Conditions>,
}

/// A node as it was last built: its Taffy node, what its style was resolved from, its children in the tree, those of
/// them laid out in it (all but what's fixed), and the frame it was last seen in.
pub(super) struct Kept {
    pub(super) node: NodeId,
    pub(super) inherited: Text,
    pub(super) state: State,
    pub(super) children: Vec<Id>,
    pub(super) inside: Vec<NodeId>,
    pub(super) frame: u64,
}

impl Default for Retained {
    fn default() -> Retained {
        // Laid out in fractions of a point, as GPUI's and Flutter's layouts are, not rounded to whole points: what grows
        // or moves by its layout (a card's width) moves its contents with it smoothly, a frame at a time.
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        Retained { taffy, kept: HashMap::new(), fixed: HashSet::new(), will_change: HashSet::new(), conditions: None }
    }
}

impl Retained {
    pub(super) fn leaf(&self, node: NodeId) -> &Leaf {
        self.taffy.get_node_context(node).expect("every node has its leaf")
    }

    /// `id` was taken out of what it was in: unless it was built elsewhere in `frame`, it and what's in it go.
    pub(super) fn forget(&mut self, id: Id, frame: u64, transitions: &mut transition::Transitions) {
        if self.kept.get(&id).is_none_or(|kept| kept.frame == frame) {
            return;
        }
        let kept = self.kept.remove(&id).expect("it's kept");
        let _ = self.taffy.remove(kept.node);
        self.fixed.remove(&id);
        self.will_change.remove(&id);
        transitions.forget(id);
        for child in kept.children {
            self.forget(child, frame, transitions);
        }
    }

    /// Lays out what's fixed in the window, as the root's children after its own, each after what's in it and in the
    /// tree's order, as they're found.
    pub(super) fn place_fixed(&mut self, tree: &Tree) {
        let Some(root) = self.kept.get(&0) else { return };
        let mut children = root.inside.clone();
        if !self.fixed.is_empty() {
            // Where each is in the tree, as the index of each node on the way to it.
            let path = |id: Id| {
                let mut path = Vec::new();
                let mut at = id;
                while let Some(parent) = tree.parent(at) {
                    path.push(tree.nodes[&parent].children.iter().position(|child| *child == at).unwrap_or(0));
                    at = parent;
                }
                path.reverse();
                path
            };
            let mut fixed: Vec<(Vec<usize>, NodeId)> = self.fixed.iter().filter_map(|id| Some((path(*id), self.kept.get(id)?.node))).collect();
            // What's in a node comes before it.
            fixed.sort_by(|(a, _), (b, _)| match a.iter().zip(b).find(|(a, b)| a != b) {
                Some((a, b)) => a.cmp(b),
                None => b.len().cmp(&a.len()),
            });
            children.extend(fixed.into_iter().map(|(_, node)| node));
        }
        if self.taffy.children(root.node).is_ok_and(|now| now != children) {
            self.taffy.set_children(root.node, &children).expect("its children are Taffy's");
        }
    }
}

pub(super) struct Painter<'a> {
    /// What's drawn from a layer kept from an earlier frame (`Layer`), and the node whose is being drawn now.
    pub(super) layers: &'a mut HashMap<Id, Layer>,
    pub(super) layered: &'a HashSet<Id>,
    /// Nothing moves this frame, so what's about to (`will-change`) is drawn into its layer ahead.
    pub(super) still: bool,
    pub(super) recording: Option<Id>,
    /// The parts of the layer being drawn so far, before its scene now.
    pub(super) parts: Vec<Part>,
    /// A layered node drawn as usual, as where it rests is changing.
    pub(super) unsettled: Option<Id>,
    pub(super) tree: &'a Tree,
    pub(super) layout: &'a mut Retained,
    /// What's styled again this frame (`Screen::paint`): what changed, what's moving, what's in or around either
    /// (`below`), or all of it, where the conditions changed; the frame; and what was taken out of what it was in.
    pub(super) changed: &'a HashSet<Id>,
    pub(super) moving: &'a HashSet<Id>,
    pub(super) below: &'a HashSet<Id>,
    pub(super) everything: bool,
    pub(super) frame: u64,
    pub(super) gone: Vec<Id>,
    pub(super) text: &'a mut TextSystem,
    pub(super) atlas: &'a Atlas,
    pub(super) scene: &'a mut Scene,
    pub(super) scale: f32,
    pub(super) conditions: Conditions,
    pub(super) states: &'a States,
    pub(super) placed: Vec<(Id, Placed)>,
    pub(super) tab_order: Vec<Id>,
    pub(super) scrolled: &'a mut HashMap<Id, Point<f32>>,
    pub(super) fields: &'a HashMap<Id, Field>,
    pub(super) drawn: &'a mut HashMap<Id, Drawn>,
    /// Each selectable text as it's drawn, and what's selected of one: its node, start and end (`selection`).
    pub(super) texts: &'a mut HashMap<Id, super::selection::Shown>,
    pub(super) selected: Option<(Id, (usize, usize))>,
    /// The caret shows in this frame, as it blinks.
    pub(super) caret: bool,
    pub(super) pictures: &'a mut pictures::Pictures,
    /// Pictures by address, fetched, and those this frame wanted that weren't yet.
    pub(super) fetched: &'a HashMap<String, Option<String>>,
    pub(super) wanted: Vec<String>,
    pub(super) rects: Rects,
    /// The time on the screen's clock, in milliseconds, for what moves by itself (`animationName`) and what's on its
    /// way somewhere (`transitions`), and whether anything did.
    pub(super) time: f64,
    /// When what changed in this frame changed, which what it sets moving sets off from.
    pub(super) since: f64,
    pub(super) transitions: &'a mut transition::Transitions,
    pub(super) animating: bool,
    /// Whether each node listening for `intersection` is near the window: within half the window's size of what shows
    /// of it, as an `IntersectionObserver` with a `rootMargin` and `scrollMargin` of 50% says; and where what's painted
    /// now is near the window, what clips it cut to it and what scrolls it grown as the window is.
    pub(super) intersections: Vec<(Id, bool)>,
    pub(super) near: Bounds<f32>,
    /// The window, where what's fixed is placed.
    pub(super) window: Bounds<f32>,
    /// The topmost `aria-modal` dialog drawn, which holds the keyboard.
    pub(super) modal: Option<Id>,
    /// How what's painted now is turned, in the window: the `rotate`s of what it's in.
    pub(super) turn: TransformationMatrix,
    /// A panel's: where its page asks for the system's own material, each a window of its own over the system's glass
    /// (`Glass`), rather than blurring what the page drew; and what the host fills them with where the system has no
    /// glass (Linux).
    pub(super) glass: Option<Vec<crate::platform::Glass>>,
    pub(super) fill: Option<Hsla>,
    /// Each frame's page, as it last drew itself, and where each frame was drawn now, in the window.
    pub(super) frames: &'a HashMap<Id, frames::Child>,
    pub(super) framed: Vec<(Id, Bounds<f32>)>,
    /// Where what's drawn says a press moves the window, and where what's in that says it doesn't (`appRegion`).
    pub(super) drag_regions: Vec<(Id, Bounds<f32>)>,
    pub(super) kept_regions: Vec<Bounds<f32>>,
}

pub(super) trait Scale {
    fn scale_to(&self, factor: f32) -> Bounds<ScaledPixels>;
}
impl Scale for Bounds<f32> {
    fn scale_to(&self, factor: f32) -> Bounds<ScaledPixels> {
        Bounds {
            origin: Point { x: ScaledPixels(self.origin.x * factor), y: ScaledPixels(self.origin.y * factor) },
            size: Size { width: ScaledPixels(self.size.width * factor), height: ScaledPixels(self.size.height * factor) },
        }
    }
}

/// A corner's radius in pixels, for a box of `size`, no more than half its shorter side, as CSS draws it.
pub(super) fn corner(radius: Radius, size: Size<f32>) -> f32 {
    let px = match radius {
        Radius::Px(px) => px,
        Radius::Percent(percent) => size.width.min(size.height) * percent / 100.0,
    };
    px.min(size.width / 2.0).min(size.height / 2.0).max(0.0)
}

impl Painter<'_> {
    /// `id` and what's in it as Taffy's nodes, styled again only where it or what it inherits changed, and gone into
    /// only where something in it did. The root fills the window and scrolls, as a page's body.
    pub(super) fn build(&mut self, id: Id, inherited: Text, root: bool) -> NodeId {
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let state = self.states.of(id, node.kind == Kind::Input);
        let restyle = match self.layout.kept.get_mut(&id) {
            None => true,
            Some(kept) => {
                kept.frame = self.frame;
                let restyle = self.everything
                    || kept.inherited != inherited
                    || kept.state != state
                    || node.kind == Kind::Input
                    || self.changed.contains(&id)
                    || self.moving.contains(&id);
                if !restyle && !self.below.contains(&id) {
                    return kept.node;
                }
                restyle
            }
        };
        let text = match restyle {
            true => self.restyle(id, inherited, state, root),
            false => self.layout.leaf(self.layout.kept[&id].node).text,
        };
        if matches!(node.kind, Kind::Box | Kind::Scroll) {
            let children: Vec<Id> = node.children.iter().copied().filter(|child| tree.nodes.contains_key(child)).collect();
            let built: Vec<NodeId> = children.iter().map(|child| self.build(*child, text, false)).collect();
            // What's fixed is laid out in the window, as the root's (`Retained::place_fixed`).
            let inside: Vec<NodeId> = built.into_iter().filter(|child| !self.layout.leaf(*child).fixed).collect();
            let kept = self.layout.kept.get_mut(&id).expect("it's built");
            if kept.children != children {
                let now: HashSet<&Id> = children.iter().collect();
                self.gone.extend(kept.children.iter().filter(|child| !now.contains(child)));
                kept.children = children;
            }
            if kept.inside != inside {
                if !root {
                    self.layout.taffy.set_children(kept.node, &inside).expect("its children are Taffy's");
                }
                kept.inside = inside;
            }
        }
        self.layout.kept[&id].node
    }

    /// Styles `id` again, in `state`, inheriting `inherited`, telling Taffy only what's different. → the text it
    /// passes on.
    pub(super) fn restyle(&mut self, id: Id, inherited: Text, state: State, root: bool) -> Text {
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let mut computed = style::of(node, state, self.conditions);
        // What's just been added starts from its `entering` state, to move from.
        let entering = (computed.transition.is_some() && self.transitions.entering(id)).then(|| style::of(node, State { entering: true, ..state }, self.conditions));
        self.transitions.apply(id, &mut computed, entering.as_ref(), self.time, self.since, self.conditions.reduced_motion);
        let mut layout = computed.layout.clone();
        let mut look = computed.look.clone();
        // A `Text` inherits what it's in, as CSS does.
        let text = inherited.with(&computed);
        let level = node.prop("aria-level").and_then(|level| level.as_u64().or_else(|| level.as_str()?.parse().ok())).unwrap_or(2);
        let text = match node.kind == Kind::Text && node.styles.is_none() && node.str("role") == Some("heading") && level <= 3 {
            true => text.heading(self.conditions),
            false => text,
        };
        if root {
            // The window, which places what's fixed, and what's absolute with nothing positioned around it.
            layout.position = taffy::style::Position::Relative;
            layout.size = taffy::geometry::Size { width: taffy::style::Dimension::percent(1.0), height: taffy::style::Dimension::percent(1.0) };
            layout.overflow = taffy::geometry::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll };
            look.scrolls = true;
        }
        if node.kind == Kind::Scroll {
            let along = matches!(layout.flex_direction, taffy::style::FlexDirection::Row | taffy::style::FlexDirection::RowReverse);
            layout.overflow = if along {
                taffy::geometry::Point { x: taffy::style::Overflow::Scroll, y: taffy::style::Overflow::Hidden }
            } else {
                taffy::geometry::Point { x: taffy::style::Overflow::Hidden, y: taffy::style::Overflow::Scroll }
            };
            look.clips = true;
            look.scrolls = true;
        }
        layout.scrollbar_width = 0.0;
        let lines = computed.line_clamp;
        let mut spans = Vec::new();
        let measure = match node.kind {
            Kind::Text => {
                let (mut words, mut runs) = (String::new(), Vec::new());
                self.runs(id, text, None, &mut words, &mut runs, &mut spans);
                // One run is the text's own style, which needs none.
                if runs.len() == 1 && runs[0].style == text.style && runs[0].color == text.color && runs[0].background_color.is_none() && runs[0].underline == text.underline {
                    runs.clear();
                }
                Some(Measure::Text { words, runs, text, lines })
            }
            Kind::RawText => Some(Measure::Text { words: node.value.clone().unwrap_or_default(), runs: Vec::new(), text, lines: None }),
            Kind::Input => {
                let words = self.fields.get(&id).map(Field::display).unwrap_or_default();
                let words = if words.is_empty() { node.str("placeholder").unwrap_or_default().to_owned() } else { words };
                Some(Measure::Input { words: if words.is_empty() { " ".into() } else { words }, text, multiline: node.is("multiline") })
            }
            Kind::Frame => Some(Measure::Frame { height: self.frames.get(&id).and_then(|child| child.screen.content_height) }),
            Kind::Image | Kind::Path | Kind::Picker | Kind::Box | Kind::Scroll | Kind::Video => None,
        };
        let positioned = layout.position == taffy::style::Position::Absolute;
        if look.will_change {
            self.layout.will_change.insert(id);
        } else {
            self.layout.will_change.remove(&id);
        }
        let leaf = Leaf { id, text, look, line_clamp: lines, positioned, fixed: computed.fixed, measure, paragraph: None, spans };
        let taffy = &mut self.layout.taffy;
        match self.layout.kept.get_mut(&id) {
            Some(kept) => {
                if taffy.style(kept.node).is_ok_and(|was| *was != layout) {
                    taffy.set_style(kept.node, layout).expect("taffy takes any style");
                }
                let was = taffy.get_node_context_mut(kept.node).expect("every node has its leaf");
                let measured = was.measure != leaf.measure;
                *was = leaf;
                if measured {
                    taffy.mark_dirty(kept.node).expect("it's Taffy's");
                }
                (kept.inherited, kept.state) = (inherited, state);
            }
            None => {
                let node = taffy.new_leaf_with_context(layout, leaf).expect("taffy takes any style");
                let kept = Kept { node, inherited, state, children: Vec::new(), inside: Vec::new(), frame: self.frame };
                self.layout.kept.insert(id, kept);
            }
        }
        if computed.fixed {
            self.layout.fixed.insert(id);
        } else {
            self.layout.fixed.remove(&id);
        }
        text
    }

    /// What's in text `id`, set as each text in it says, as CSS inherits text properties into an inline: its words, its
    /// runs, and its texts' and raw texts' spans (outer before inner), with `behind` what's behind its words.
    pub(super) fn runs(&self, id: Id, text: Text, behind: Option<Hsla>, words: &mut String, runs: &mut Vec<TextRun>, spans: &mut Vec<Span>) {
        let tree = self.tree;
        let Some(node) = tree.nodes.get(&id) else { return };
        for child in &node.children {
            let Some(inner) = tree.nodes.get(child).filter(|inner| !inner.hidden()) else { continue };
            let start = words.len();
            match inner.kind {
                Kind::RawText => {
                    words.push_str(inner.value.as_deref().unwrap_or_default());
                    let run = TextRun { len: words.len() - start, style: text.style, color: text.color, background_color: behind, underline: text.underline };
                    match runs.last_mut() {
                        Some(last) if TextRun { len: last.len, ..run } == *last => last.len += run.len,
                        _ if run.len > 0 => runs.push(run),
                        _ => {}
                    }
                    spans.push(Span { id: *child, start, end: words.len(), cursor: None, pointer: None });
                }
                Kind::Text => {
                    let computed = style::of(inner, self.states.of(*child, false), self.conditions);
                    let set = text.with(&computed);
                    let at = spans.len();
                    spans.push(Span { id: *child, start, end: start, cursor: computed.look.cursor, pointer: computed.look.pointer_events });
                    self.runs(*child, set, computed.look.background.or(behind), words, runs, spans);
                    spans[at].end = words.len();
                }
                _ => self.runs(*child, text, behind, words, runs, spans),
            }
        }
    }

    /// A quad, its bounds and mask in the window.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn quad(&mut self, bounds: Bounds<f32>, mask: Bounds<f32>, background: Hsla, borders: Edges<f32>, border_color: Hsla, radii: Corners<f32>, dashed: bool, opacity: f32) {
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

    /// A box's shadow, as CSS's `box-shadow` draws it: its box moved by its offset and grown by its spread, blurred,
    /// under the box; or an inset one inside it, where the box moved and shrunk by its spread doesn't cover it.
    pub(super) fn shadow(&mut self, shadow: &crate::tokens::Shadow, bounds: Bounds<f32>, radii: Corners<f32>, grown: f32, mask: Bounds<f32>, opacity: f32) {
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
        self.scene.insert_primitive(Shadow {
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
        });
    }

    /// `bounds` grown by half the window's size on each side, as near it as an `IntersectionObserver`'s margin of 50%.
    pub(super) fn grown(&self, bounds: Bounds<f32>) -> Bounds<f32> {
        let (x, y) = (self.window.size.width / 2.0, self.window.size.height / 2.0);
        Bounds { origin: point(bounds.origin.x - x, bounds.origin.y - y), size: Size { width: bounds.size.width + 2.0 * x, height: bounds.size.height + 2.0 * y } }
    }

    /// The box `bounds` covers in the window once it's turned as what's painted now is: itself, where nothing's turned.
    pub(super) fn turned(&self, bounds: Bounds<f32>) -> Bounds<f32> {
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

    /// Draws node `id`, which is only moving, growing or fading, from its layer: what's in it drawn once as it rests,
    /// then each frame moved, grown and faded as a whole, with where it puts what's in it for the pointer, Tab and the
    /// inspector, as Core Animation moves a layer and Flutter a RepaintBoundary's picture. Nothing in it is laid out,
    /// shaped or painted again, and its words move exactly with their boxes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_layer(&mut self, taffy_node: NodeId, id: Id, offset: Point<f32>, transform: Transform, look: &Look, mask: Bounds<f32>, opacity: f32, parent: Option<Id>, pointer: bool) {
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
                    let transform = Transform { offset: point(sublayer.transform.offset.x * grown + by.x, sublayer.transform.offset.y * grown + by.y), scale: sublayer.transform.scale * grown };
                    self.paint(sublayer.taffy_node, sublayer.offset, transform, moved(sublayer.mask).intersect(&mask), sublayer.opacity * opacity, sublayer.parent, sublayer.pointer);
                }
            }
        }
        let layer = &self.layers[&id];
        self.placed.extend(layer.placed.iter().map(|(id, placed)| (*id, Placed { shown: moved(placed.shown).intersect(&mask), ..placed.clone() })));
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
    pub(super) fn record(&mut self, taffy_node: NodeId, id: Id, offset: Point<f32>, transform: Transform, parent: Option<Id>, pointer: bool, key: LayerKey) -> Layer {
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
    pub(super) fn paint(&mut self, taffy_node: NodeId, offset: Point<f32>, transform: Transform, mask: Bounds<f32>, opacity: f32, parent: Option<Id>, pointer: bool) {
        let turn = self.turn;
        self.paint_node(taffy_node, offset, transform, mask, opacity, parent, pointer);
        self.turn = turn;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_node(&mut self, taffy_node: NodeId, offset: Point<f32>, transform: Transform, mask: Bounds<f32>, opacity: f32, parent: Option<Id>, pointer: bool) {
        let Ok(laid) = self.layout.taffy.layout(taffy_node).copied() else { return };
        // What isn't displayed isn't drawn, and nothing in it is.
        if self.layout.taffy.style(taffy_node).is_ok_and(|style| style.display == taffy::style::Display::None) {
            return;
        }
        let Some(leaf) = self.layout.taffy.get_node_context(taffy_node) else { return };
        let (id, text, mut look, lines) = (leaf.id, leaf.text, leaf.look.clone(), leaf.line_clamp);
        if self.recording == Some(id) {
            // Its layer is drawn as it rests, and moved, grown and faded as it's drawn from it.
            (look.translate, look.scale, look.opacity) = ((Offset::Px(0.0), Offset::Px(0.0)), 1.0, 1.0);
        } else if self.recording.is_some() && self.layered.contains(&id) && self.turn != TransformationMatrix::unit() {
            // Turned in the layer being drawn, it's drawn in it as usual, which is drawn again each frame.
            self.animating = true;
        } else if self.recording.is_some() && self.layered.contains(&id) {
            // A layer of its own in the one being drawn: drawn into it each frame, where that's moved it.
            let scene = std::mem::take(&mut *self.scene);
            self.parts.push(Part::Scene(std::sync::Arc::new(scene), None));
            self.parts.push(Part::Sublayer(Sublayer { taffy_node, offset, transform, mask, opacity, parent, pointer }));
            return;
        } else if self.layered.contains(&id) && self.unsettled != Some(id) && self.turn == TransformationMatrix::unit() && look.animation.is_none() {
            return self.paint_layer(taffy_node, id, offset, transform, &look, mask, opacity, parent, pointer);
        }
        let tree = self.tree;
        let node = &tree.nodes[&id];
        let laid_out = Bounds { origin: point(offset.x + laid.location.x, offset.y + laid.location.y), size: Size { width: laid.size.width, height: laid.size.height } };
        // Its own move and growth, around its middle, which moves nothing around it.
        let centre = point(laid_out.origin.x + laid_out.size.width / 2.0, laid_out.origin.y + laid_out.size.height / 2.0);
        let translate = point(look.translate.0.of(laid_out.size.width), look.translate.1.of(laid_out.size.height));
        let transform = if translate == point(0.0, 0.0) && look.scale == 1.0 { transform } else { transform.then(translate, look.scale, centre) };
        let bounds = transform.bounds(laid_out);
        let mut opacity = opacity * look.opacity;
        let mut rotate = look.rotate;
        if let Some((name, duration)) = &look.animation {
            self.animating = true;
            let duration = f64::from(duration.max(1.0));
            let turn = ((self.time % duration) / duration) as f32;
            match name.as_str() {
                "spin" => rotate += turn * 360.0,
                _ => opacity *= 1.0 - 0.25 * (1.0 - (turn * std::f32::consts::TAU).cos()),
            }
        }
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
        let pointer = !node.is("inert") && look.pointer_events.unwrap_or(pointer);
        if node.str("aria-modal") == Some("true") {
            self.modal = Some(id);
        }
        let drawn = self.turned(bounds);
        self.rects.insert(id, Rect { x: drawn.origin.x, y: drawn.origin.y, width: drawn.size.width, height: drawn.size.height });
        if node.listens("intersection") {
            self.intersections.push((id, drawn.intersects(&self.near)));
        }
        let radii = Corners {
            top_left: corner(look.radii.top_left, bounds.size),
            top_right: corner(look.radii.top_right, bounds.size),
            bottom_right: corner(look.radii.bottom_right, bounds.size),
            bottom_left: corner(look.radii.bottom_left, bounds.size),
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
            } else if let Some((blur, saturate, _)) = look.material {
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
            if look.background.is_some() || look.borders != Edges::default() {
                self.quad(bounds, mask, look.background.unwrap_or_default(), scaled(look.borders), look.border_color, radii, look.dashed, opacity);
            }
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
        let mut placed = Placed { parent, shown, pointer, cursor: look.cursor, pressable, input: node.kind == Kind::Input, focus, scroll: None, touch_action: look.touch_action };
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
                    let shown = super::selection::Shown { paragraph: paragraph.clone(), words, content, width, grown: transform.scale, align: text.align };
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
                    self.place_spans(id, &spans, &paragraph, content, width, transform.scale, text.align, mask, pointer);
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
            Kind::Image => self.paint_image(node, content, radii, opacity, mask),
            Kind::Path => self.paint_path(node, content, &look, text.color, opacity, mask),
            // Its latest frame, as the web's browser plays it; a native host draws none (its pages aren't given one).
            #[cfg(target_arch = "wasm32")]
            Kind::Video => self.paint_video(id, content, radii, opacity, mask),
            #[cfg(not(target_arch = "wasm32"))]
            Kind::Video => {}
            Kind::Picker => {}
            // A frame's page, drawn as it last drew itself, in its box, where the frame comes in this scene's order.
            Kind::Frame => {
                self.framed.push((id, content));
                if let Some(child) = self.frames.get(&id) {
                    self.scene.splice(&child.scene, 1.0, [content.origin.x * self.scale, content.origin.y * self.scale], 1.0, shown.intersect(&content).scale_to(self.scale));
                }
            }
            Kind::Box | Kind::Scroll => {
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
                        child_offset = point(child_offset.x - now.x, child_offset.y - now.y);
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

    /// An input: its text, or its placeholder while it's empty, clipped to its box; with the keyboard, what's selected
    /// behind it, what an input method composes underlined, and the caret, as it blinks; scrolled to keep the caret
    /// in view as it moves. Where it's drawn is kept for the pointer (`Drawn`). → how far several lines can scroll.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_input(&mut self, id: Id, node: &Node, content: Bounds<f32>, padding: Bounds<f32>, width: f32, grown: f32, text: Text, opacity: f32, mask: Bounds<f32>) -> Option<f32> {
        let field = self.fields.get(&id);
        let shown = field.map(Field::display).unwrap_or_default();
        let multiline = node.is("multiline");
        let focused = self.states.focused == Some(id) && field.is_some();
        let empty = shown.is_empty();
        let placeholder = colour_token("tertiary-label", self.conditions);
        let (words, color) = if empty { (node.str("placeholder").unwrap_or_default().to_owned(), placeholder) } else { (shown.clone(), text.color) };
        let paragraph = self.text.paragraph(&words, &text.style, &[], multiline.then_some(width), None);
        let line_height = text.style.line_height;
        // A one-line input's text in the middle of its box, as it's drawn in a row centred on its items.
        let top = if multiline { content.origin.y } else { content.origin.y + (content.size.height - paragraph.size().height * grown) / 2.0 };
        // Its text starts where its first strong character's direction says, as an input with `dir="auto"`: at the
        // right where it's Hebrew or Arabic.
        let rtl = !empty && paragraph.lines.first().is_some_and(|line| line.rtl);
        let text = if rtl { Text { align: 1.0 - text.align, ..text } } else { text };
        let aligned = |line: &crate::text::Line| (width - line.width).max(0.0) * text.align;
        let offsets: Vec<f32> = paragraph.lines.iter().map(aligned).collect();
        let offset = |line: usize| offsets.get(line).copied().unwrap_or(0.0);
        // Where the caret is drawn: where an arrow across text running both ways left it, or before its byte.
        let (caret_line, caret_x) = match field.and_then(Field::placed) {
            Some(placed) if !empty => placed,
            _ => {
                let (line, x) = paragraph.caret(field.map_or(0, Field::caret), field.is_some_and(Field::upstream));
                (line, x + offset(line))
            }
        };
        // One line scrolls along as the caret goes past its ends, as an `<input>` does, and back to its start once
        // it hasn't the keyboard; several scroll down and up to keep the caret's line in its box as it moves, as a
        // `<textarea>`, and by the wheel (`Screen::scroll`, which keeps how far in `scrolled`) without its moving.
        let mut scroll = self.drawn.get(&id).map_or(point(0.0, 0.0), |drawn| drawn.scroll);
        let seen = (focused, field.map_or(0, Field::caret), field.is_some_and(Field::upstream), shown.len());
        let moved = self.drawn.get(&id).is_none_or(|drawn| drawn.seen != seen);
        let mut most = None;
        if !multiline {
            scroll.x = if focused && !empty { scroll.x.max(caret_x - width + 1.0).min(caret_x) } else { 0.0 };
            scroll.x = scroll.x.clamp(0.0, (paragraph.size().width + 1.0 - width).max(0.0));
        } else {
            let height = content.size.height / grown;
            let top = caret_line as f32 * line_height;
            scroll.y = self.scrolled.get(&id).map_or(0.0, |scrolled| scrolled.y);
            if focused && moved {
                scroll.y = scroll.y.max(top + line_height - height).min(top);
            }
            let can = (paragraph.size().height - height).max(0.0);
            scroll.y = scroll.y.clamp(0.0, can);
            self.scrolled.insert(id, point(0.0, scroll.y));
            most = Some(can);
        }
        let origin = point(content.origin.x, top - scroll.y * grown);
        // Several lines scrolled run on into its padding, as a `<textarea>`'s do; one line stops at its content.
        let clip = if multiline { padding } else { content }.intersect(&mask);
        // A point along line `line`, `x` from where its paragraph starts, in the window.
        let at = |line: usize, x: f32| point(origin.x + (x - scroll.x) * grown, origin.y + line as f32 * line_height * grown);
        if let (true, Some(field), false) = (focused, field, empty) {
            let (start, end) = field.selection();
            if start < end {
                let tint = colour_token("blue", self.conditions).opacity(0.3);
                for (line, x0, x1) in paragraph.spans(start, end) {
                    let bounds = Bounds { origin: at(line, x0 + offset(line)), size: Size { width: (x1 - x0) * grown, height: line_height * grown } };
                    self.quad(bounds, clip, tint, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
                }
            }
        }
        let area = Bounds { origin: point(origin.x - scroll.x * grown, origin.y), size: content.size };
        self.paint_text(&paragraph, &[], area, width, grown, Text { color, ..text }, opacity, clip);
        if let (true, Some(field)) = (focused, field) {
            // What an input method is composing is underlined, as every platform's own fields draw it.
            if let Some((start, end)) = field.composed() {
                for (line, x0, x1) in paragraph.spans(start, end) {
                    let below = at(line, x0 + offset(line));
                    let bounds = Bounds { origin: point(below.x, below.y + (line_height - 2.0) * grown), size: Size { width: (x1 - x0) * grown, height: grown } };
                    self.quad(bounds, clip, text.color, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
                }
            }
            if self.caret && !field.readonly {
                let place = if empty { point(origin.x + width * text.align * grown, origin.y) } else { at(caret_line, caret_x) };
                let bounds = Bounds { origin: place, size: Size { width: grown, height: line_height * grown } };
                self.quad(bounds, clip, text.color, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
            }
        }
        self.drawn.insert(id, Drawn { paragraph, empty, origin, offsets, grown, scroll, seen });
        most
    }

    /// A paragraph's words, set in `text`, or as `runs` say where it's given them: what's behind each run first, then
    /// its glyphs, then each underline, along each line at its baseline's foot.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_text(&mut self, paragraph: &Paragraph, runs: &[TextRun], content: Bounds<f32>, width: f32, grown: f32, text: Text, opacity: f32, mask: Bounds<f32>) {
        let own = [TextRun { len: usize::MAX, style: text.style, color: text.color, background_color: None, underline: text.underline }];
        let runs = if runs.is_empty() { &own[..] } else { runs };
        let mut at = 0usize;
        let ranges: Vec<(usize, usize, &TextRun)> = runs
            .iter()
            .map(|run| {
                let start = at;
                at = at.saturating_add(run.len);
                (start, at, run)
            })
            .collect();
        let line_height = paragraph.line_height;
        let left = |line: usize| content.origin.x + paragraph.lines.get(line).map_or(0.0, |drawn| (width - drawn.width).max(0.0) * text.align) * grown;
        for (start, end, run) in &ranges {
            let Some(behind) = run.background_color else { continue };
            for (line, x0, x1) in paragraph.inked(*start, *end) {
                // As CSS draws an inline's background: over its content area, its font's ascent and descent.
                let (ascent, descent) = paragraph.lines.get(line).map_or((line_height, 0.0), |drawn| (drawn.ascent, drawn.descent));
                let top = content.origin.y + (line as f32 * line_height + (line_height - ascent - descent) / 2.0) * grown;
                let bounds = Bounds { origin: point(left(line) + x0 * grown, top), size: Size { width: (x1 - x0) * grown, height: (ascent + descent) * grown } };
                self.quad(bounds, mask, behind, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
            }
        }
        let scaled_mask = ContentMask { bounds: mask.scale_to(self.scale) };
        // Glyphs grown with their box are rasterized at that size, so they stay sharp.
        let origin = point(px(content.origin.x / grown), px(content.origin.y / grown));
        let color = |index: usize| ranges.iter().find(|(start, end, _)| (*start..*end).contains(&index)).map_or(text.color, |(_, _, run)| run.color).opacity(opacity);
        if let Err(error) = self.text.paint(paragraph, origin, width, text.align, &color, self.scale * grown, scaled_mask, self.turn.scaled(self.scale), self.atlas, self.scene) {
            log::warn!("text didn't draw: {error:#}");
        }
        for (start, end, run) in &ranges {
            if !run.underline {
                continue;
            }
            for (line, x0, x1) in paragraph.inked(*start, *end) {
                let y = content.origin.y + (line as f32 + 1.0) * line_height * grown - line_height * grown * 0.18;
                let bounds = Bounds { origin: point(left(line) + x0 * grown, y), size: Size { width: (x1 - x0) * grown, height: grown.max(1.0) } };
                self.quad(bounds, mask, run.color, Edges::default(), Hsla::default(), Corners::default(), false, opacity);
            }
        }
    }

    /// The texts and raw texts inside text `id` where their words are drawn: each text placed for the pointer over each
    /// stretch of its words, after what it's in, so a press, hover and cursor find the innermost; each reached by Tab
    /// where the DOM's would be, a span with its `tabindex`; and each where an agent or a screen reader finds it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_spans(&mut self, id: Id, spans: &[Span], paragraph: &Paragraph, content: Bounds<f32>, width: f32, grown: f32, align: f32, mask: Bounds<f32>, pointer: bool) {
        let tree = self.tree;
        let line_height = paragraph.line_height;
        let mut pointers: HashMap<Id, bool> = HashMap::from([(id, pointer)]);
        for span in spans {
            let Some(node) = tree.nodes.get(&span.id) else { continue };
            let parent = tree.parent(span.id).unwrap_or(id);
            let pointer = pointers.get(&parent).copied().unwrap_or(pointer) && span.pointer.unwrap_or(true) && !node.is("inert");
            pointers.insert(span.id, pointer);
            let mut drawn: Option<Bounds<f32>> = None;
            let rects: Vec<Bounds<f32>> = paragraph
                .inked(span.start, span.end)
                .into_iter()
                .map(|(line, x0, x1)| {
                    let left = content.origin.x + paragraph.lines.get(line).map_or(0.0, |drawn| (width - drawn.width).max(0.0) * align) * grown;
                    self.turned(Bounds { origin: point(left + x0 * grown, content.origin.y + line as f32 * line_height * grown), size: Size { width: (x1 - x0) * grown, height: line_height * grown } })
                })
                .collect();
            for rect in &rects {
                drawn = Some(drawn.map_or(*rect, |all| all.union(rect)));
            }
            if let Some(all) = drawn {
                self.rects.insert(span.id, Rect { x: all.origin.x, y: all.origin.y, width: all.size.width, height: all.size.height });
            }
            if node.kind != Kind::Text {
                continue;
            }
            let pressable = node.listens("click") && !node.is("disabled") && !node.is("aria-disabled");
            let tabindex = node.prop("tabindex").and_then(|index| index.as_i64().or_else(|| index.as_str()?.parse().ok()));
            if pressable && tabindex.is_some_and(|index| index >= 0) {
                self.tab_order.push(span.id);
            }
            for rect in rects {
                let focus = if tabindex.is_some() { Clicked::Takes } else { Clicked::Passes };
                let placed = Placed { parent: Some(parent), shown: rect.intersect(&mask), pointer, cursor: span.cursor, pressable, input: false, focus, scroll: None, touch_action: None };
                self.placed.push((span.id, placed));
            }
        }
    }

    /// A window's breadcrumb over the top of its page (`Breadcrumb`), in a computer's own size, as shadcn's: Home light,
    /// highlighted under the pointer (`hovered`); a muted /; then the app's icon and its name in semibold. → where
    /// Home is, and where the breadcrumb ends.
    pub(super) fn breadcrumb(&mut self, crumb: &Breadcrumb, start: f32, bar: f32, hovered: bool) -> (Bounds<f32>, f32) {
        let conditions = self.conditions;
        let window = self.window;
        let colour = |id: &str| colour_token(id, conditions);
        let (home_style, app_style) = chrome_type(conditions);
        let middle = bar / 2.0;
        let (icon, gap, pad) = (16.0, 6.0, 6.0);
        let mut x = start;
        // Home, with the ring.
        let home_words = self.text.paragraph("Home", &home_style, &[], None, None);
        let home = Bounds { origin: point(x, middle - 11.0), size: Size { width: pad + icon + gap + home_words.size().width.ceil() + pad, height: 22.0 } };
        if hovered {
            let radius = crate::tokens::table().radius("radius-md", conditions).map_or(8.0, |radius| corner(radius, home.size));
            self.quad(home, window, colour("quinary-label"), Edges::default(), Hsla::default(), Corners::all(radius), false, 1.0);
        }
        self.picture(&crumb.ring, Bounds { origin: point(x + pad, middle - icon / 2.0), size: Size { width: icon, height: icon } }, 0.0);
        let home_colour = if hovered { colour("label") } else { colour("secondary-label") };
        let words_at = |paragraph: &Paragraph, x: f32| Bounds { origin: point(x, middle - paragraph.size().height / 2.0), size: paragraph.size() };
        let text = Text { style: home_style, color: home_colour, align: 0.0, wrap: false, underline: false, selectable: false };
        self.paint_text(&home_words, &[], words_at(&home_words, x + pad + icon + gap), home_words.size().width.ceil(), 1.0, text, 1.0, window);
        x = home.origin.x + home.size.width + 4.0;
        // A muted /.
        let slash = self.text.paragraph("/", &home_style, &[], None, None);
        let muted = colour("secondary-label");
        self.paint_text(&slash, &[], words_at(&slash, x), slash.size().width.ceil(), 1.0, Text { color: muted, ..text }, 0.6, window);
        x += slash.size().width.ceil() + 4.0 + pad;
        // The app.
        if let Some(src) = &crumb.icon {
            self.picture(src, Bounds { origin: point(x, middle - icon / 2.0), size: Size { width: icon, height: icon } }, icon * 0.225);
            x += icon + gap;
        }
        let name = self.text.paragraph(&crumb.name, &app_style, &[], None, Some(1));
        self.paint_text(&name, &[], words_at(&name, x), name.size().width.ceil(), 1.0, Text { style: app_style, color: colour("label"), ..text }, 1.0, window);
        (home, x + name.size().width.ceil())
    }

    /// The window's notice over the top of its page (`Notice`), `top` points down: a pill of the foreground's colour
    /// with its words in the background's, as a toast, centred, and its action after them, highlighted under the
    /// pointer. → where its action is.
    pub(super) fn notice(&mut self, notice: &Notice, top: f32, hovered: bool) -> Option<Bounds<f32>> {
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

    /// A picture by its `data:` address at `bounds`, its corners rounded `radius`.
    pub(super) fn picture(&mut self, src: &str, bounds: Bounds<f32>, radius: f32) {
        let device = Size { width: (bounds.size.width * self.scale).round() as u32, height: (bounds.size.height * self.scale).round() as u32 };
        if let Some((_, tile)) = self.pictures.get(self.atlas, src, device) {
            let window = self.window;
            self.sprite(tile, bounds, Corners::all(radius), 1.0, window);
        }
    }

    /// A picture's sprite, `width`×`height` device pixels, at `bounds` in the window.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sprite(&mut self, tile: AtlasTile, bounds: Bounds<f32>, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
        let scale = self.scale;
        // Its tile's clear pixel all round (`pictures::tiled`), drawn as wide as one of its pixels is here, outside
        // `bounds`.
        let texel = |drawn: f32, pixels: i32| drawn / (pixels - 2).max(1) as f32;
        let (x, y) = (texel(bounds.size.width, tile.bounds.size.width.0), texel(bounds.size.height, tile.bounds.size.height.0));
        let bounds = Bounds { origin: point(bounds.origin.x - x, bounds.origin.y - y), size: Size { width: bounds.size.width + 2.0 * x, height: bounds.size.height + 2.0 * y } };
        self.scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: 0,
            opacity,
            bounds: bounds.scale_to(scale),
            content_mask: ContentMask { bounds: mask.scale_to(scale) },
            corner_radii: Corners {
                top_left: ScaledPixels(radii.top_left * scale),
                top_right: ScaledPixels(radii.top_right * scale),
                bottom_right: ScaledPixels(radii.bottom_right * scale),
                bottom_left: ScaledPixels(radii.bottom_left * scale),
            },
            tile,
            transformation: self.turn.scaled(scale),
        });
    }

    /// An image, scaled to fit its box keeping its shape, as GPUI's `img` is by default.
    pub(super) fn paint_image(&mut self, node: &Node, content: Bounds<f32>, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
        let Some(src) = node.str("src") else { return };
        // One by an address is drawn once its window has fetched it, as a `data:` address.
        let src = match self.fetched.get(src) {
            _ if src.starts_with("data:") => src,
            Some(Some(data)) => data.as_str(),
            Some(None) => return,
            None => return self.wanted.push(src.to_owned()),
        };
        let device = Size { width: (content.size.width * self.scale).round() as u32, height: (content.size.height * self.scale).round() as u32 };
        if device.width == 0 || device.height == 0 {
            return;
        }
        let Some((picture, tile)) = self.pictures.get(self.atlas, src, device) else { return };
        let fit = (content.size.width / picture.width as f32).min(content.size.height / picture.height as f32);
        let size = Size { width: picture.width as f32 * fit, height: picture.height as f32 * fit };
        let bounds = Bounds {
            origin: point(content.origin.x + (content.size.width - size.width) / 2.0, content.origin.y + (content.size.height - size.height) / 2.0),
            size,
        };
        let radii = Corners { top_left: radii.top_left.min(size.width / 2.0), top_right: radii.top_right.min(size.width / 2.0), bottom_right: radii.bottom_right.min(size.width / 2.0), bottom_left: radii.bottom_left.min(size.width / 2.0) };
        self.sprite(tile, bounds, radii, opacity, mask);
    }

    /// A video's latest frame, as big as fits in `content`, in the middle, as a `<video>` draws it.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn paint_video(&mut self, id: Id, content: Bounds<f32>, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
        let device = Size { width: (content.size.width * self.scale).round() as u32, height: (content.size.height * self.scale).round() as u32 };
        let Some((tile, frame)) = crate::platform::web::video(id, content, device, self.atlas) else { return };
        let fit = (content.size.width / frame.width as f32).min(content.size.height / frame.height as f32);
        let size = Size { width: frame.width as f32 * fit, height: frame.height as f32 * fit };
        let bounds = Bounds { origin: point(content.origin.x + (content.size.width - size.width) / 2.0, content.origin.y + (content.size.height - size.height) / 2.0), size };
        self.sprite(tile, bounds, radii, opacity, mask);
    }

    /// A path, SVG's: its `d` in its `viewBox`, stretched to its box, filled and stroked as its style says, drawn by
    /// resvg at the window's scale.
    pub(super) fn paint_path(&mut self, node: &Node, content: Bounds<f32>, look: &Look, current: Hsla, opacity: f32, mask: Bounds<f32>) {
        let Some(d) = node.str("d") else { return };
        let view_box = node.str("viewBox").unwrap_or("0 0 24 24");
        let device = pictures::fitting(Size { width: (content.size.width * self.scale).round() as u32, height: (content.size.height * self.scale).round() as u32 }, self.atlas);
        if device.width == 0 || device.height == 0 {
            return;
        }
        let paint = |paint: Paint| match paint {
            Paint::Colour(colour) => Some(colour),
            Paint::Current => Some(current),
            Paint::None => None,
        };
        let css = |colour: Hsla| {
            let rgba = crate::geometry::Rgba::from(colour);
            format!("rgba({},{},{},{})", (rgba.r * 255.0).round(), (rgba.g * 255.0).round(), (rgba.b * 255.0).round(), rgba.a)
        };
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{view_box}" preserveAspectRatio="none"><path d="{d}" fill="{fill}" stroke="{stroke}" stroke-width="{width}" vector-effect="non-scaling-stroke" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
            w = device.width,
            h = device.height,
            d = d.replace('"', ""),
            fill = paint(look.fill).map_or_else(|| "none".to_owned(), css),
            stroke = paint(look.stroke).map_or_else(|| "none".to_owned(), css),
            width = look.stroke_width * self.scale,
        );
        // The web's canvas draws it, from its `d`, as the browser's own SVG would.
        #[cfg(target_arch = "wasm32")]
        let draw = || crate::platform::web::path(d, view_box, device, paint(look.fill).map(css), paint(look.stroke).map(css), look.stroke_width * self.scale);
        #[cfg(not(target_arch = "wasm32"))]
        let draw = || decode(&format!("data:image/svg+xml;base64,{}", data_encoding::BASE64.encode(svg.as_bytes())), device);
        let Some(tile) = self.pictures.now(self.atlas, &format!("path{svg}"), draw) else { return };
        self.sprite(tile, content, Corners::default(), opacity, mask);
    }
}

/// A colour token of the table, where there is one.
/// A colour token of the table: none, where it has none by that id.
pub(super) fn colour_token(id: &str, conditions: Conditions) -> Hsla {
    crate::tokens::table().colour(id, conditions).map(Hsla::from).unwrap_or_default()
}

/// What the window draws of its own over its page (its breadcrumb, a notice) is set in: the medium type, as words,
/// and in its medium weight, as a name or an action, each line its size.
pub(super) fn chrome_type(conditions: Conditions) -> (crate::text::TextStyle, crate::text::TextStyle) {
    let table = crate::tokens::table();
    let medium = table.type_style("type-medium", conditions).unwrap_or_default();
    let words = crate::text::TextStyle { size: medium.size, weight: medium.weight.round() as u16, tracking: medium.tracking, line_height: medium.size.round(), tabular: false, mono: false };
    let weight = table.weight("label-weight", conditions).unwrap_or(medium.weight);
    (words, crate::text::TextStyle { weight: weight.round() as u16, ..words })
}

#[cfg(not(target_arch = "wasm32"))]
/// An SVG, parsed as a browser reads it: its `oklch()` colours as sRGB, which resvg's parser (svgtypes) doesn't read
/// and draws black, and its text in Allo, the typeface a host has everywhere, for `sans-serif` and `system-ui`.
pub(super) fn svg(bytes: &[u8]) -> Option<resvg::usvg::Tree> {
    static OPTIONS: std::sync::OnceLock<resvg::usvg::Options<'static>> = std::sync::OnceLock::new();
    let options = OPTIONS.get_or_init(|| {
        let mut fonts = resvg::usvg::fontdb::Database::new();
        for font in crate::fonts().unwrap_or_default() {
            fonts.load_font_data(font);
        }
        fonts.set_sans_serif_family(crate::FONT);
        resvg::usvg::Options { fontdb: Arc::new(fonts), ..resvg::usvg::Options::default() }
    });
    let text = std::str::from_utf8(bytes).ok()?;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("oklch(") {
        let end = rest[at..].find(')').map_or(rest.len(), |end| at + end + 1);
        out.push_str(&rest[..at]);
        match crate::tokens::oklch(&rest[at..end]) {
            Some(rgba) => {
                let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
                out.push_str(&format!("rgba({}, {}, {}, {})", byte(rgba.r), byte(rgba.g), byte(rgba.b), rgba.a));
            }
            None => out.push_str(&rest[at..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    resvg::usvg::Tree::from_str(&out, options).ok()
}

/// A `data:` address's image, decoded for a box of `device` pixels: an SVG drawn at that size, and any other at the
/// smallest of its size halved again and again that still covers what of the box it fills, as a mipmap's levels are, so its pixels
/// beyond what the box shows aren't kept, and a box that grows and shrinks (a window resized) decodes it at a few
/// sizes, not one for each.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn decode(src: &str, device: Size<u32>) -> Option<Decoded> {
    let (head, body) = src.strip_prefix("data:")?.split_once(',')?;
    let bytes = if head.ends_with(";base64") {
        data_encoding::BASE64.decode(body.as_bytes()).ok()?
    } else {
        url::form_urlencoded::parse(format!("x={body}").as_bytes()).next().map(|(_, value)| value.into_owned().into_bytes())?
    };
    let (width, height, mut rgba) = if head.starts_with("image/svg+xml") {
        let tree = svg(&bytes)?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(device.width, device.height)?;
        let size = tree.size();
        let fit = (device.width as f32 / size.width()).min(device.height as f32 / size.height());
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(fit, fit), &mut pixmap.as_mut());
        // tiny-skia's pixels are premultiplied; the atlas takes them straight.
        let mut data = pixmap.take();
        for pixel in data.chunks_exact_mut(4) {
            let alpha = pixel[3] as f32 / 255.0;
            if alpha > 0.0 {
                for channel in &mut pixel[..3] {
                    *channel = (*channel as f32 / alpha).round().min(255.0) as u8;
                }
            }
        }
        (device.width, device.height, data)
    } else {
        let mut image = image::load_from_memory(&bytes).ok()?;
        let (mut width, mut height) = (image.width(), image.height());
        // What it's drawn at, fitted in its box keeping its shape (`paint_image`).
        let fit = (device.width as f32 / width as f32).min(device.height as f32 / height as f32);
        let shown = (width as f32 * fit, height as f32 * fit);
        while (width / 2) as f32 >= shown.0 && (height / 2) as f32 >= shown.1 {
            (width, height) = (width / 2, height / 2);
        }
        if width < image.width() {
            image = image.thumbnail_exact(width, height);
        }
        let image = image.into_rgba8();
        (image.width(), image.height(), image.into_raw())
    };
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(Decoded { picture: Picture { width, height }, bgra: rgba })
}

/// The largest a layer's image is, in device pixels, along either side, as every GPU the renderer runs on allows.
pub(super) const RASTER_MOST: f32 = 8192.0;

/// A layer's scene as an image of it (`Recording`), with an id of its own, where it can be one: what it draws covers
/// whole device pixels from its origin, and it blurs nothing behind it.
pub(super) fn recording(scene: &std::sync::Arc<Scene>) -> Option<Recording> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    if !scene.backdrops.is_empty() {
        return None;
    }
    let extent = scene.extent()?;
    let (left, top) = (extent.origin.x.0.floor(), extent.origin.y.0.floor());
    let (right, bottom) = ((extent.origin.x.0 + extent.size.width.0).ceil(), (extent.origin.y.0 + extent.size.height.0).ceil());
    let size = [right - left, bottom - top];
    if size[0] < 1.0 || size[1] < 1.0 || size[0] > RASTER_MOST || size[1] > RASTER_MOST {
        return None;
    }
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Some(Recording { id, scene: scene.clone(), origin: [left, top], size: [size[0] as u32, size[1] as u32] })
}
