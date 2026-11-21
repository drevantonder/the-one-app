//! How a screen's tree is styled, laid out and painted, a frame at a time: the painter (`Painter`), the layout it keeps
//! from frame to frame (`Retained`), and the layers what only moves is drawn from (`Layer`), as GPUI's and Flutter's
//! paint passes are their own.
use super::*;

mod build;
mod chrome;
mod images;
mod layers;
mod shapes;
mod text;

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
/// scales and its parents' say, and grown by `rest` once they're where their styles take them (`Look::rest_scale`).
#[derive(Clone, Copy)]
pub(super) struct Transform {
    pub(super) offset: Point<f32>,
    pub(super) scale: f32,
    pub(super) rest: f32,
}

impl Transform {
    pub(super) const NONE: Transform = Transform { offset: Point { x: 0.0, y: 0.0 }, scale: 1.0, rest: 1.0 };

    pub(super) fn point(&self, at: Point<f32>) -> Point<f32> {
        point(self.offset.x + at.x * self.scale, self.offset.y + at.y * self.scale)
    }

    pub(super) fn bounds(&self, bounds: Bounds<f32>) -> Bounds<f32> {
        Bounds { origin: self.point(bounds.origin), size: Size { width: bounds.size.width * self.scale, height: bounds.size.height * self.scale } }
    }

    /// This, then a node's own move and growth around its middle, `centre`, and the growth it rests at, `rest`.
    pub(super) fn then(&self, translate: Point<f32>, scale: f32, rest: f32, centre: Point<f32>) -> Transform {
        // p ↦ centre + translate + (p − centre)·scale, then this.
        let moved = point(centre.x + translate.x - centre.x * scale, centre.y + translate.y - centre.y * scale);
        Transform { offset: self.point(moved), scale: self.scale * scale, rest: self.rest * rest }
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
    /// Each list's rows, as it laid them out (`list::Listed`).
    pub(super) lists: HashMap<Id, super::list::Listed>,
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
        Retained { taffy, kept: HashMap::new(), fixed: HashSet::new(), will_change: HashSet::new(), conditions: None, lists: HashMap::new() }
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
        self.lists.remove(&id);
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
    /// The window isn't shown (a panel put away, an app in the background): what moves is where it's going at once.
    pub(super) unseen: bool,
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
    /// How far past its end each scroll is stretched, which moves its content as far again (`Screen::stretched`).
    pub(super) stretched: HashMap<Id, Point<f32>>,
    pub(super) fields: &'a HashMap<Id, Field>,
    pub(super) drawn: &'a mut HashMap<Id, Drawn>,
    /// Each selectable text as it's drawn, and what's selected of one: its node, start and end (`selection`).
    pub(super) texts: &'a mut HashMap<Id, super::selection::Shown>,
    pub(super) selected: Option<(Id, (usize, usize))>,
    /// The caret shows in this frame, as it blinks.
    pub(super) caret: bool,
    /// The input whose selection a finger made, which shows its handles (`text_selection`).
    pub(super) handles: Option<Id>,
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
    /// Each video's connection natively, whose latest frame it draws (`paint_video`).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) videos: &'a mut HashMap<Id, crate::video::Video>,
    pub(super) framed: Vec<(Id, Bounds<f32>)>,
    /// Where what's drawn says a press moves the window, and where what's in that says it doesn't (`appRegion`).
    pub(super) drag_regions: Vec<(Id, Bounds<f32>)>,
    pub(super) kept_regions: Vec<Bounds<f32>>,
}

pub(super) trait Scale {
    fn scale_to(&self, factor: f32) -> Bounds<ScaledPixels>;
    fn snap_to(&self, factor: f32) -> Bounds<ScaledPixels>;
}
impl Scale for Bounds<f32> {
    fn scale_to(&self, factor: f32) -> Bounds<ScaledPixels> {
        Bounds {
            origin: Point { x: ScaledPixels(self.origin.x * factor), y: ScaledPixels(self.origin.y * factor) },
            size: Size { width: ScaledPixels(self.size.width * factor), height: ScaledPixels(self.size.height * factor) },
        }
    }

    /// In device pixels, its edges on whole ones, as Chromium and Flutter snap a box's: a GPU fills a pixel whose
    /// middle is on an edge as its own rule says, so a hairline on half a pixel was one pixel on one GPU and two on
    /// another. What has any size keeps at least a pixel of it.
    fn snap_to(&self, factor: f32) -> Bounds<ScaledPixels> {
        let edges = |start: f32, length: f32| {
            let from = (start * factor).round();
            let to = ((start + length) * factor).round();
            (from, if length > 0.0 { to.max(from + 1.0) } else { to })
        };
        let ((left, right), (top, bottom)) = (edges(self.origin.x, self.size.width), edges(self.origin.y, self.size.height));
        Bounds { origin: Point { x: ScaledPixels(left), y: ScaledPixels(top) }, size: Size { width: ScaledPixels(right - left), height: ScaledPixels(bottom - top) } }
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

/// A colour token of the table, where there is one.
/// A colour token of the table: none, where it has none by that id.
pub(super) fn colour_token(id: &str, conditions: Conditions) -> Hsla {
    crate::tokens::table().colour(id, conditions).map(Hsla::from).unwrap_or_default()
}

/// What the window draws of its own over its page (its title, a notice) is set in: the medium type, as words,
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

/// A path, SVG's: its `d` in its `viewBox`, stretched to `device` pixels, filled and stroked `width` device pixels wide,
/// with round ends. → its key among the pictures, and what draws it: resvg, or on the web its canvas, as the browser's
/// own SVG would.
pub(in crate::view) fn path(d: &str, view_box: &str, device: Size<u32>, fill: Option<Hsla>, stroke: Option<Hsla>, width: f32) -> (String, impl Fn() -> Option<Decoded>) {
    let css = |colour: Hsla| {
        let rgba = crate::geometry::Rgba::from(colour);
        format!("rgba({},{},{},{})", (rgba.r * 255.0).round(), (rgba.g * 255.0).round(), (rgba.b * 255.0).round(), rgba.a)
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{view_box}" preserveAspectRatio="none"><path d="{d}" fill="{fill}" stroke="{stroke}" stroke-width="{width}" vector-effect="non-scaling-stroke" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
        w = device.width,
        h = device.height,
        d = d.replace('"', ""),
        fill = fill.map_or_else(|| "none".to_owned(), css),
        stroke = stroke.map_or_else(|| "none".to_owned(), css),
    );
    #[cfg(target_arch = "wasm32")]
    let draw = {
        let (d, view_box) = (d.to_owned(), view_box.to_owned());
        move || crate::platform::web::path(&d, &view_box, device, fill.map(css), stroke.map(css), width)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let draw = {
        let src = format!("data:image/svg+xml;base64,{}", data_encoding::BASE64.encode(svg.as_bytes()));
        move || decode(&src, device)
    };
    (format!("path{svg}"), draw)
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
