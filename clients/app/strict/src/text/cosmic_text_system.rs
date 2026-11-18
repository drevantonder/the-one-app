// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_wgpu/src/cosmic_text_system.rs) at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, and cut down for the strict host: cosmic-text's shaping and swash's
// rasterizing, on Linux and Android. Changed: a face is loaded at the weight asked for, which a variable font draws by
// its `wght` axis in shaping and rasterizing alike, letter spacing is cosmic-text's, there are no user fallback chains,
// a glyph says where its characters end, how wide it is and which way it runs, for a caret in right-to-left text, a
// line's glyphs are placed left to right from its own left edge, which cosmic-text breaks text into where it asks,
// Android's own fonts are the fallback there, and a COLRv1 emoji is drawn by skrifa (`colr`).
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::collections::HashMap;

use anyhow::{Context as _, Result};
use cosmic_text::{
    Attrs, AttrsList, Ellipsize, Family, FontFeatures, FontSystem, ShapeBuffer, ShapeLine, Weight, fontdb,
};
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Vector};

use super::{FontId, GlyphId, LineLayout, RenderGlyphParams, SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, ShapedGlyph, ShapedRun, TextStyle};
use crate::geometry::{Bounds, DevicePixels, Size, point, px, size};

pub struct CosmicTextSystem {
    font_system: FontSystem,
    swash_scale_context: ScaleContext,
    scratch: ShapeBuffer,
    /// The faces glyphs are drawn from, each at a weight, by `FontId`.
    faces: Vec<(fontdb::ID, Weight, bool)>,
    family: String,
    /// A glyph's image, made to measure it, until it's put in the atlas.
    pending_glyph_images: HashMap<RenderGlyphParams, swash::scale::image::Image>,
}

impl CosmicTextSystem {
    /// Text in `family`, from `fonts`, and from the system's fonts where those lack a character, if `system_fonts`.
    pub fn new(family: &str, fonts: Vec<Vec<u8>>, system_fonts: bool) -> Self {
        let mut db = fontdb::Database::new();
        if system_fonts {
            db.load_system_fonts();
            // fontdb reads no system fonts on Android: those it falls back on for what Allo hasn't, memory-mapped, as
            // Android's own fonts.xml names them for every script it has a font for.
            #[cfg(target_os = "android")]
            for file in ANDROID_FONTS {
                let _ = db.load_font_file(format!("/system/fonts/{file}"));
            }
        }
        for font in fonts {
            db.load_font_data(font);
        }
        #[cfg(not(target_os = "android"))]
        let font_system = FontSystem::new_with_locale_and_db("en-US".into(), db);
        #[cfg(target_os = "android")]
        let font_system = FontSystem::new_with_locale_and_db_and_fallback("en-US".into(), db, Android);
        CosmicTextSystem {
            font_system,
            swash_scale_context: ScaleContext::new(),
            scratch: ShapeBuffer::default(),
            faces: Vec::new(),
            family: family.to_owned(),
            pending_glyph_images: HashMap::new(),
        }
    }

    fn font_id(&mut self, id: fontdb::ID, weight: Weight) -> FontId {
        if let Some(at) = self.faces.iter().position(|face| face.0 == id && face.1 == weight) {
            return FontId(at);
        }
        let emoji = self.font_system.db().face(id).is_some_and(|face| face.post_script_name.contains("Emoji"));
        self.faces.push((id, weight, emoji));
        FontId(self.faces.len() - 1)
    }

    pub fn layout_line(&mut self, text: &str, style: &TextStyle) -> LineLayout {
        let mut lines = self.layout_lines(text, style, None);
        if lines.is_empty() {
            return LineLayout { font_size: px(style.size), len: text.len(), ..Default::default() };
        }
        lines.swap_remove(0)
    }

    /// `text` broken into lines no wider than `width` by cosmic-text, each line's glyphs placed from its own left
    /// edge, left to right, as Unicode's bidi algorithm reorders a line once it's broken (UAX #9, L1 and L2); with no
    /// width, one line however long. Each line's `len` is where it ends in `text`.
    pub fn layout_lines(&mut self, text: &str, style: &TextStyle, width: Option<f32>) -> Vec<LineLayout> {
        let mut features = FontFeatures::new();
        if style.tabular {
            features.enable(cosmic_text::FeatureTag::new(b"tnum"));
        }
        let attrs = Attrs::new()
            .family(Family::Name(&self.family))
            .weight(Weight(style.weight))
            .letter_spacing(style.tracking)
            .font_features(features);
        let attrs_list = AttrsList::new(&attrs);
        let line = ShapeLine::new(&mut self.font_system, text, &attrs_list, cosmic_text::Shaping::Advanced, 4);
        let mut layout_lines = Vec::with_capacity(1);
        line.layout_to_buffer(
            &mut self.scratch,
            style.size,
            width,
            if width.is_some() { cosmic_text::Wrap::WordOrGlyph } else { cosmic_text::Wrap::None },
            Ellipsize::None,
            // From the left, whichever way it runs: where a line goes along its box is the paragraph's to say.
            Some(cosmic_text::Align::Left),
            &mut layout_lines,
            None,
            cosmic_text::Hinting::Disabled,
        );

        let mut out = Vec::with_capacity(layout_lines.len());
        for layout in &layout_lines {
            let left = layout.glyphs.iter().map(|glyph| glyph.x).fold(f32::INFINITY, f32::min);
            let left = if left.is_finite() { left } else { 0.0 };
            let mut glyphs: Vec<&cosmic_text::LayoutGlyph> = layout.glyphs.iter().collect();
            glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
            let mut runs: Vec<ShapedRun> = Vec::new();
            for glyph in glyphs {
                let font_id = self.font_id(glyph.font_id, glyph.font_weight);
                let is_emoji = self.faces[font_id.0].2;
                // HACK: Prevent crash caused by variation selectors.
                if glyph.glyph_id == 3 && is_emoji {
                    continue;
                }
                let shaped_glyph = ShapedGlyph {
                    id: GlyphId(glyph.glyph_id as u32),
                    position: point(
                        (glyph.x - left + glyph.x_offset * style.size).into(),
                        (glyph.y - glyph.y_offset * style.size).into(),
                    ),
                    index: glyph.start,
                    end: glyph.end,
                    advance: glyph.w,
                    rtl: glyph.level.is_rtl(),
                    is_emoji,
                };
                if let Some(last_run) = runs.last_mut().filter(|last_run| last_run.font_id == font_id) {
                    last_run.glyphs.push(shaped_glyph);
                } else {
                    runs.push(ShapedRun { font_id, glyphs: vec![shaped_glyph] });
                }
            }
            let len = layout.glyphs.iter().map(|glyph| glyph.end).max().unwrap_or(0);
            out.push(LineLayout {
                font_size: px(style.size),
                width: layout.w.into(),
                ascent: layout.max_ascent.into(),
                descent: layout.max_descent.into(),
                runs,
                len,
                rtl: line.rtl,
            });
        }
        if let Some(last) = out.last_mut() {
            last.len = text.len();
        }
        out
    }

    pub fn raster_bounds(&mut self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let image = self.render_glyph_image(params)?;
        let bounds = Bounds {
            origin: point(image.placement.left.into(), (-image.placement.top).into()),
            size: size(image.placement.width.into(), image.placement.height.into()),
        };
        if !bounds.is_zero() {
            self.pending_glyph_images.insert(params.clone(), image);
        }
        Ok(bounds)
    }

    pub fn rasterize_glyph(&mut self, params: &RenderGlyphParams, glyph_bounds: Bounds<DevicePixels>) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            anyhow::bail!("glyph bounds are empty");
        }
        let mut image = match self.pending_glyph_images.remove(params) {
            Some(image) => image,
            None => self.render_glyph_image(params)?,
        };
        if params.is_emoji {
            // Convert from RGBA to BGRA.
            for pixel in image.data.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        Ok((glyph_bounds.size, image.data))
    }

    fn render_glyph_image(&mut self, params: &RenderGlyphParams) -> Result<swash::scale::image::Image> {
        let (id, weight, _) = self.faces[params.font_id.0];
        let font = self.font_system.get_font(id, weight).context("the font went")?;
        if params.is_emoji {
            let index = self.font_system.db().face(id).map_or(0, |face| face.index);
            let colr = skrifa::FontRef::from_index(font.data(), index)
                .ok()
                .and_then(|face| super::colr::render(&face, params.glyph_id.0.try_into().ok()?, params.font_size() * params.scale_factor(), &[]));
            if let Some((left, top, width, height, rgba)) = colr {
                let mut image = swash::scale::image::Image::new();
                image.content = swash::scale::image::Content::Color;
                image.placement = swash::zeno::Placement { left, top, width, height };
                image.data = rgba;
                return Ok(image);
            }
        }
        let font_ref = font.as_swash();
        let scale_factor = params.scale_factor();
        let subpixel_offset = Vector::new(
            params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / scale_factor,
            params.subpixel_variant.y as f32 / SUBPIXEL_VARIANTS_Y as f32 / scale_factor,
        );
        let mut scaler = self.swash_scale_context.builder(font_ref).size(params.font_size() * scale_factor).hint(true);
        // A variable font drawn at the weight asked for, as it was shaped (cosmic-text's `swash_image`).
        let wght = swash::Tag::from_be_bytes(*b"wght");
        if let Some(axis) = font_ref.variations().find_by_tag(wght) {
            let value = f32::from(weight.0).clamp(axis.min_value(), axis.max_value());
            scaler = scaler.normalized_coords(font_ref.variations().normalized_coords([(wght, value)]));
        }
        let mut scaler = scaler.build();

        let sources: &[Source] = if params.is_emoji {
            &[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline]
        } else {
            &[Source::Bitmap(StrikeWith::ExactSize), Source::Outline]
        };
        let mut renderer = Render::new(sources);
        renderer.format(Format::Alpha).offset(subpixel_offset);
        let glyph_id: u16 = params.glyph_id.0.try_into()?;
        renderer.render(&mut scaler, glyph_id).with_context(|| format!("unable to render glyph via swash for {params:?}"))
    }
}

/// The fonts Android falls back on for what Allo hasn't, by their files in /system/fonts, as its fonts.xml lists
/// them: Roboto for Latin, Greek and Cyrillic, the emoji, CJK, and the scripts most read.
#[cfg(target_os = "android")]
const ANDROID_FONTS: &[&str] = &[
    "Roboto-Regular.ttf",
    "NotoColorEmoji.ttf",
    "NotoSansCJK-Regular.ttc",
    "NotoNaskhArabic-Regular.ttf",
    "NotoSansHebrew-Regular.ttf",
    "NotoSansDevanagari-VF.ttf",
    "NotoSansThai-Regular.ttf",
    "NotoSansBengali-VF.ttf",
    "NotoSansTamil-VF.ttf",
    "NotoSansArmenian-VF.ttf",
    "NotoSansGeorgian-VF.ttf",
    "NotoSansEthiopic-VF.ttf",
    "NotoSansSymbols-Regular-Subsetted.ttf",
    "NotoSansSymbols-Regular-Subsetted2.ttf",
];

/// Android's fallbacks, by script, as its fonts.xml orders them for the fonts read here.
#[cfg(target_os = "android")]
struct Android;

#[cfg(target_os = "android")]
impl cosmic_text::Fallback for Android {
    fn common_fallback(&self) -> &[&'static str] {
        &["Roboto", "Noto Color Emoji", "Noto Sans Symbols"]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, script: unicode_script::Script, locale: &str) -> &[&'static str] {
        use unicode_script::Script;
        match script {
            Script::Hangul => &["Noto Sans CJK KR"],
            Script::Hiragana | Script::Katakana => &["Noto Sans CJK JP"],
            Script::Han if locale.starts_with("zh-TW") || locale.starts_with("zh-HK") => &["Noto Sans CJK TC"],
            Script::Han if locale.starts_with("zh") => &["Noto Sans CJK SC"],
            Script::Han if locale.starts_with("ko") => &["Noto Sans CJK KR"],
            Script::Han => &["Noto Sans CJK JP"],
            Script::Arabic => &["Noto Naskh Arabic"],
            Script::Hebrew => &["Noto Sans Hebrew"],
            Script::Devanagari => &["Noto Sans Devanagari"],
            Script::Thai => &["Noto Sans Thai"],
            Script::Bengali => &["Noto Sans Bengali"],
            Script::Tamil => &["Noto Sans Tamil"],
            Script::Armenian => &["Noto Sans Armenian"],
            Script::Georgian => &["Noto Sans Georgian"],
            Script::Ethiopic => &["Noto Sans Ethiopic"],
            _ => &[],
        }
    }
}
