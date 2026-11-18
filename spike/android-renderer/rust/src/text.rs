//! Text, as GPUI's Linux text system has it (crates/gpui/src/platform/linux/text_system.rs: cosmic-text shapes and
//! lays out, swash rasterizes into the glyph atlas), with the UI library's own typeface, Allo, as the only font: what
//! a strict screen draws is in the design's type styles, and Android's own fonts aren't read at start (no font scan).

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight};

use crate::atlas::{AtlasKey, Kind};
use crate::renderer::Renderer;
use crate::scene::{Bounds, Hsla, MonochromeSprite, PolychromeSprite, Scene, TransformationMatrix, Corners};

/// The fonts every screen sets text in, read once per process: Allo, the UI library's typeface, as sans-serif, then
/// Android's own fonts for what Allo hasn't (CJK, emoji, other scripts), memory-mapped, so only what's drawn is read.
pub fn fonts() -> anyhow::Result<&'static cosmic_text::fontdb::Database> {
    static FONTS: std::sync::Mutex<Option<&'static cosmic_text::fontdb::Database>> = std::sync::Mutex::new(None);
    let mut fonts = FONTS.lock().unwrap();
    if let Some(db) = *fonts {
        return Ok(db);
    }
    let started = std::time::Instant::now();
    let ttf = wuff::decompress_woff2(include_bytes!("../../../../core/ui/src/Allo-Variable.woff2"))
        .map_err(|error| anyhow::anyhow!("Allo didn't decode: {error:?}"))?;
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_source(cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(ttf)));
    let family = db.faces().next().and_then(|face| face.families.first()).map(|(name, _)| name.clone()).unwrap_or_default();
    db.set_sans_serif_family(family);
    for file in ["NotoSansCJK-Regular.ttc", "NotoColorEmoji.ttf", "Roboto-Regular.ttf"] {
        let _ = db.load_font_file(format!("/system/fonts/{file}"));
    }
    log::info!("fonts read in {:?}: {} faces", started.elapsed(), db.len());
    let db: &'static _ = Box::leak(Box::new(db));
    *fonts = Some(db);
    Ok(db)
}

/// Android's fallback fonts, by script, as its fonts.xml has them for the fonts read here.
struct Android;

impl cosmic_text::Fallback for Android {
    fn common_fallback(&self) -> &[&'static str] {
        &["Roboto", "Noto Color Emoji"]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, script: unicode_script::Script, locale: &str) -> &[&'static str] {
        use unicode_script::Script;
        match script {
            Script::Hangul => &["Noto Sans CJK KR"],
            Script::Hiragana | Script::Katakana => &["Noto Sans CJK JP"],
            Script::Han if locale.starts_with("zh") => &["Noto Sans CJK SC"],
            Script::Han => &["Noto Sans CJK JP"],
            _ => &[],
        }
    }
}

pub struct Text {
    pub fonts: FontSystem,
    swash: SwashCache,
}

/// What a text is set in, in device pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Font {
    pub size: f32,
    pub line_height: f32,
    pub weight: f32,
    pub tabular: bool,
}

impl Text {
    pub fn new() -> anyhow::Result<Text> {
        Ok(Text { fonts: FontSystem::new_with_locale_and_db_and_fallback("en-US".into(), fonts()?.clone(), Android), swash: SwashCache::new() })
    }

    /// A buffer laid out `width` wide (or as wide as its text), keeping `lines` lines at most, the last ending in an
    /// ellipsis where it's cut.
    pub fn buffer(&mut self, text: &str, font: Font, width: Option<f32>, lines: Option<usize>) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(font.size, font.line_height));
        self.set(&mut buffer, text, font, width, lines);
        buffer
    }

    pub fn set(&mut self, buffer: &mut Buffer, text: &str, font: Font, width: Option<f32>, lines: Option<usize>) {
        buffer.set_metrics(Metrics::new(font.size, font.line_height));
        let mut attrs = Attrs::new().family(Family::SansSerif).weight(Weight(font.weight.round() as u16));
        if font.tabular {
            let mut features = cosmic_text::FontFeatures::new();
            features.enable(cosmic_text::FeatureTag::new(b"tnum"));
            attrs = attrs.font_features(features);
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        self.wrap(buffer, width, lines);
    }

    /// Lays a buffer's shaped text out again at another width, without shaping it again.
    pub fn wrap(&mut self, buffer: &mut Buffer, width: Option<f32>, lines: Option<usize>) {
        buffer.set_ellipsize(cosmic_text::Ellipsize::None);
        buffer.set_size(width, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        // Ellipsized only where it has more lines than it may show.
        if let Some(lines) = lines.filter(|lines| buffer.layout_runs().count() > *lines) {
            buffer.set_ellipsize(cosmic_text::Ellipsize::End(cosmic_text::EllipsizeHeightLimit::Lines(lines)));
            buffer.shape_until_scroll(&mut self.fonts, false);
        }
    }

    /// The size a buffer's laid-out lines take.
    pub fn size(buffer: &Buffer) -> (f32, f32) {
        let mut width: f32 = 0.0;
        let mut height: f32 = 0.0;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height = run.line_top + run.line_height;
        }
        (width, height)
    }

    /// Draws a buffer's glyphs at `origin`, each a sprite in the glyph atlas (GPUI's `paint_glyph`).
    pub fn paint(&mut self, buffer: &Buffer, origin: [f32; 2], color: Hsla, clip: Bounds, renderer: &mut Renderer, scene: &mut Scene) {
        for run in buffer.layout_runs() {
            // cosmic-text 0.19, ellipsizing, starts a wrapped line with the space it wrapped at: it's drawn from the
            // first glyph after it.
            let indent = match run.glyphs.first() {
                Some(first) if first.start > 0 && run.text[first.start..first.end].trim().is_empty() && !run.rtl => first.w,
                _ => 0.0,
            };
            for glyph in run.glyphs {
                let physical = glyph.physical((origin[0] - indent, origin[1] + run.line_y), 1.0);
                let key = physical.cache_key;
                let mut placement = None;
                let mut colour = false;
                let swash = &mut self.swash;
                let fonts = &mut self.fonts;
                // A glyph's image is kept in the atlas by its key; its placement is asked of swash's cache once.
                let image = swash.get_image(fonts, key).as_ref().map(|image| {
                    placement = Some(image.placement);
                    colour = matches!(image.content, SwashContent::Color);
                    image.clone()
                });
                let Some(image) = image else { continue };
                let placement = placement.expect("set with the image");
                let kind = if colour { Kind::Polychrome } else { Kind::Monochrome };
                let tile = renderer.atlas.borrow_mut().get_or_insert_with(&AtlasKey::Glyph(key), kind, || {
                    let size = [placement.width as i32, placement.height as i32];
                    Some(match image.content {
                        SwashContent::Mask => (size, image.data),
                        SwashContent::Color => (size, rgba_to_bgra(image.data)),
                        // Android's subpixel order isn't known; grey antialiasing, as GPUI's mono sprites.
                        SwashContent::SubpixelMask => (size, image.data.chunks(3).map(|rgb| rgb[1]).collect()),
                    })
                });
                let Some(tile) = tile else { continue };
                let bounds = Bounds::new(
                    (physical.x + placement.left) as f32,
                    (physical.y - placement.top) as f32,
                    placement.width as f32,
                    placement.height as f32,
                );
                if colour {
                    scene.polychrome(PolychromeSprite { order: 0, pad: 0, grayscale: 0, opacity: 1.0, bounds, content_mask: clip, corner_radii: Corners::default(), tile });
                } else {
                    let color = glyph.color_opt.map(|c| Hsla::from_rgba(c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, c.a() as f32 / 255.0)).unwrap_or(color);
                    scene.monochrome(MonochromeSprite { order: 0, pad: 0, bounds, content_mask: clip, color, tile, transformation: TransformationMatrix::UNIT });
                }
            }
        }
    }
}

pub fn rgba_to_bgra(mut data: Vec<u8>) -> Vec<u8> {
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    data
}
