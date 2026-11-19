//! A COLRv1 colour glyph (Android's emoji font since Android 13, Noto Color Emoji's), which swash doesn't draw: skrifa
//! walks its paint graph and calls a `ColorPainter`, which this one rasterizes with tiny-skia, as Chrome's Skia does
//! for it (skia/src/ports/SkFontHost_FreeType_common.cpp, `colrv1_*`): each clip a mask, each layer a pixmap blended
//! down by its composite mode, each brush a shader in the glyph's own units.

use resvg::tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Point, RadialGradient,
    Rect, Shader, SpreadMode, SweepGradient, Transform,
};
use skrifa::color::{Brush, ColorPainter, ColorStop, CompositeMode, Extend};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::types::BoundingBox;
use skrifa::raw::TableProvider;
use skrifa::{GlyphId, MetadataProvider};

/// Glyph `glyph` of `font` drawn at `ppem` pixels to the em, if it's a COLRv1 glyph: where it goes from the pen (its
/// left and top) and its straight RGBA pixels, `width` by `height`.
pub fn render(font: &skrifa::FontRef, glyph: u16, ppem: f32, coords: &[skrifa::instance::NormalizedCoord]) -> Option<(i32, i32, u32, u32, Vec<u8>)> {
    let location = LocationRef::new(coords);
    let color = font.color_glyphs().get_with_format(GlyphId::new(glyph.into()), skrifa::color::ColorGlyphFormat::ColrV1)?;
    let upem = font.head().ok()?.units_per_em() as f32;
    let scale = ppem / upem;
    // Its clip box, or the em square from the descender up, as a glyph with no clip box is drawn in.
    let bounds = color.bounding_box(location, Size::new(ppem)).unwrap_or_else(|| {
        let metrics = font.metrics(Size::new(ppem), location);
        let advance = font.glyph_metrics(Size::new(ppem), location).advance_width(GlyphId::new(glyph.into())).unwrap_or(ppem);
        BoundingBox { x_min: 0.0, y_min: metrics.descent, x_max: advance, y_max: metrics.ascent }
    });
    let (left, top) = (bounds.x_min.floor(), bounds.y_max.ceil());
    let (width, height) = ((bounds.x_max.ceil() - left) as u32, (top - bounds.y_min.floor()) as u32);
    let base = Transform::from_row(scale, 0.0, 0.0, -scale, -left, top);
    let mut painter = Painter {
        font,
        location,
        palette: font.color_palettes().get(0).map(|palette| palette.colors().to_vec()).unwrap_or_default(),
        layers: vec![Pixmap::new(width.max(1), height.max(1))?],
        clips: Vec::new(),
        transforms: vec![base],
    };
    color.paint(location, &mut painter).ok()?;
    let mut rgba = painter.layers.pop()?.take();
    // tiny-skia's pixels are premultiplied; the atlas takes them straight.
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = pixel[3] as f32 / 255.0;
        if alpha > 0.0 {
            for channel in &mut pixel[..3] {
                *channel = (*channel as f32 / alpha).round().min(255.0) as u8;
            }
        }
    }
    Some((left as i32, top as i32, width, height, rgba))
}

struct Painter<'a> {
    font: &'a skrifa::FontRef<'a>,
    location: LocationRef<'a>,
    palette: Vec<skrifa::color::Color>,
    layers: Vec<Pixmap>,
    clips: Vec<Mask>,
    transforms: Vec<Transform>,
}

/// A glyph's outline, in its own units, as a path.
struct Pen(PathBuilder);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, cy0, x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

impl Painter<'_> {
    fn transform(&self) -> Transform {
        *self.transforms.last().expect("the base is never popped")
    }

    fn size(&self) -> (u32, u32) {
        let layer = &self.layers[0];
        (layer.width(), layer.height())
    }

    fn outline(&self, glyph: GlyphId) -> Option<Path> {
        let outline = self.font.outline_glyphs().get(glyph)?;
        let mut pen = Pen(PathBuilder::new());
        outline.draw(DrawSettings::unhinted(Size::unscaled(), self.location), &mut pen).ok()?;
        pen.0.finish()
    }

    /// The current clip narrowed to `path`, in the glyph's units.
    fn clip(&mut self, path: Option<Path>) {
        let (width, height) = self.size();
        let mut mask = self.clips.last().cloned().unwrap_or_else(|| {
            let mut all = Mask::new(width, height).expect("its size isn't zero");
            all.data_mut().fill(255);
            all
        });
        match path {
            Some(path) => mask.intersect_path(&path, FillRule::Winding, true, self.transform()),
            None => mask.data_mut().fill(0),
        }
        self.clips.push(mask);
    }

    fn colour(&self, palette_index: u16, alpha: f32) -> Color {
        // 0xFFFF is the text's own colour; an emoji's, drawn as an image, is black.
        let record = self.palette.get(usize::from(palette_index)).copied().unwrap_or(skrifa::color::Color { red: 0, green: 0, blue: 0, alpha: 255 });
        Color::from_rgba8(record.red, record.green, record.blue, (f32::from(record.alpha) * alpha).round().clamp(0.0, 255.0) as u8)
    }

    fn stops(&self, stops: &[ColorStop]) -> Vec<GradientStop> {
        stops.iter().map(|stop| GradientStop::new(stop.offset, self.colour(stop.palette_index, stop.alpha))).collect()
    }
}

fn spread(extend: Extend) -> SpreadMode {
    match extend {
        Extend::Repeat => SpreadMode::Repeat,
        Extend::Reflect => SpreadMode::Reflect,
        _ => SpreadMode::Pad,
    }
}

fn blend(mode: CompositeMode) -> BlendMode {
    match mode {
        CompositeMode::Clear => BlendMode::Clear,
        CompositeMode::Src => BlendMode::Source,
        CompositeMode::Dest => BlendMode::Destination,
        CompositeMode::DestOver => BlendMode::DestinationOver,
        CompositeMode::SrcIn => BlendMode::SourceIn,
        CompositeMode::DestIn => BlendMode::DestinationIn,
        CompositeMode::SrcOut => BlendMode::SourceOut,
        CompositeMode::DestOut => BlendMode::DestinationOut,
        CompositeMode::SrcAtop => BlendMode::SourceAtop,
        CompositeMode::DestAtop => BlendMode::DestinationAtop,
        CompositeMode::Xor => BlendMode::Xor,
        CompositeMode::Plus => BlendMode::Plus,
        CompositeMode::Screen => BlendMode::Screen,
        CompositeMode::Overlay => BlendMode::Overlay,
        CompositeMode::Darken => BlendMode::Darken,
        CompositeMode::Lighten => BlendMode::Lighten,
        CompositeMode::ColorDodge => BlendMode::ColorDodge,
        CompositeMode::ColorBurn => BlendMode::ColorBurn,
        CompositeMode::HardLight => BlendMode::HardLight,
        CompositeMode::SoftLight => BlendMode::SoftLight,
        CompositeMode::Difference => BlendMode::Difference,
        CompositeMode::Exclusion => BlendMode::Exclusion,
        CompositeMode::Multiply => BlendMode::Multiply,
        CompositeMode::HslHue => BlendMode::Hue,
        CompositeMode::HslSaturation => BlendMode::Saturation,
        CompositeMode::HslColor => BlendMode::Color,
        CompositeMode::HslLuminosity => BlendMode::Luminosity,
        _ => BlendMode::SourceOver,
    }
}

impl ColorPainter for Painter<'_> {
    fn push_transform(&mut self, transform: skrifa::color::Transform) {
        let ours = Transform::from_row(transform.xx, transform.yx, transform.xy, transform.yy, transform.dx, transform.dy);
        self.transforms.push(self.transform().pre_concat(ours));
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph: GlyphId) {
        let path = self.outline(glyph);
        self.clip(path);
    }

    fn push_clip_box(&mut self, clip: BoundingBox<f32>) {
        let path = Rect::from_ltrb(clip.x_min, clip.y_min, clip.x_max, clip.y_max).map(PathBuilder::from_rect);
        self.clip(path);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        let transform = self.transform();
        let shader = match brush {
            Brush::Solid { palette_index, alpha } => Some(Shader::SolidColor(self.colour(palette_index, alpha))),
            Brush::LinearGradient { p0, p1, color_stops, extend } => {
                LinearGradient::new(Point::from_xy(p0.x, p0.y), Point::from_xy(p1.x, p1.y), self.stops(color_stops), spread(extend), transform)
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => {
                RadialGradient::new(Point::from_xy(c0.x, c0.y), r0, Point::from_xy(c1.x, c1.y), r1, self.stops(color_stops), spread(extend), transform)
            }
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend } => {
                SweepGradient::new(Point::from_xy(c0.x, c0.y), start_angle, end_angle, self.stops(color_stops), spread(extend), transform)
            }
        };
        let Some(shader) = shader else { return };
        let (width, height) = self.size();
        let paint = Paint { shader, anti_alias: true, ..Default::default() };
        let all = Rect::from_xywh(0.0, 0.0, width as f32, height as f32).expect("its size isn't zero");
        let clip = self.clips.last();
        if let Some(layer) = self.layers.last_mut() {
            layer.fill_rect(all, &paint, Transform::identity(), clip);
        }
    }

    fn push_layer(&mut self, _mode: CompositeMode) {
        let (width, height) = self.size();
        self.layers.push(Pixmap::new(width, height).expect("its size isn't zero"));
    }

    fn pop_layer_with_mode(&mut self, mode: CompositeMode) {
        if self.layers.len() < 2 {
            return;
        }
        let layer = self.layers.pop().expect("there are two");
        let below = self.layers.last_mut().expect("there are two");
        below.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint { blend_mode: blend(mode), ..Default::default() }, Transform::identity(), None);
    }
}
