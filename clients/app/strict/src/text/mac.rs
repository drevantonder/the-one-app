// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/mac/text_system.rs and
// open_type.rs) at commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the
// strict host: Core Text's shaping and Core Graphics' rasterizing, on a Mac, with Core Text's own fallback for a
// character the typeface lacks. Changed: no font-kit; one typeface, from its bytes, at the weight asked for on its
// `wght` axis, with tabular figures by its `tnum` feature, and tracking as Core Text's kern.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::collections::HashMap;

use anyhow::Result;
use core_foundation::array::CFArray;
use core_foundation::attributed_string::CFMutableAttributedString;
use core_foundation::base::{CFRange, CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::base::{CGFloat, CGGlyph, kCGImageAlphaPremultipliedLast};
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::{CGContext, CGTextDrawingMode};
use core_graphics::geometry::{CGPoint, CGSize};
use core_text::font::CTFont;
use core_text::font_descriptor::{
    CTFontDescriptor, CTFontDescriptorCreateCopyWithVariation, kCTFontOrientationDefault, kCTFontFeatureSettingsAttribute,
};
use core_text::line::CTLine;
use core_text::string_attributes::{kCTFontAttributeName, kCTKernAttributeName};

use super::{FontId, GlyphId, LineLayout, RenderGlyphParams, SUBPIXEL_VARIANTS_X, ShapedGlyph, ShapedRun, TextStyle};
use crate::geometry::{Bounds, DevicePixels, Size, point, px, size};

#[allow(non_upper_case_globals)]
const kCGImageAlphaOnly: u32 = 7;

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    static kCTFontOpenTypeFeatureTag: CFStringRef;
    static kCTFontOpenTypeFeatureValue: CFStringRef;
    fn CTRunGetStatus(run: core_text::run::CTRunRef) -> u32;
    fn CTTypesetterCreateWithAttributedString(string: core_foundation::attributed_string::CFAttributedStringRef) -> core_foundation::base::CFTypeRef;
    fn CTTypesetterSuggestLineBreak(typesetter: core_foundation::base::CFTypeRef, start: isize, width: f64) -> isize;
    fn CTTypesetterCreateLine(typesetter: core_foundation::base::CFTypeRef, range: CFRange) -> core_text::line::CTLineRef;
}

/// A run's status says it runs right to left (`kCTRunStatusRightToLeft`).
const RUN_RIGHT_TO_LEFT: u32 = 1;

pub struct CoreTextSystem {
    /// The typeface, as its bytes load it.
    typeface: CTFont,
    /// The typeface at a weight, with tabular figures or not.
    styled: HashMap<(u16, bool), CTFont>,
    /// The faces glyphs are drawn from, by `FontId`, and whether each is emoji.
    faces: Vec<(CTFont, bool)>,
    /// Each face's `FontId`, by its PostScript name, and the weight and figures it was asked at where it's the
    /// typeface.
    ids: HashMap<String, FontId>,
}

impl CoreTextSystem {
    /// Text in `family`, from `fonts`, the first of which is the typeface; Core Text falls back to the system's
    /// fonts where it lacks a character, whatever `_system_fonts` says.
    pub fn new(_family: &str, fonts: Vec<Vec<u8>>, _system_fonts: bool) -> Self {
        let bytes = fonts.into_iter().next().expect("the typeface is built in");
        let typeface = core_text::font::new_from_buffer(&bytes).expect("the built-in typeface loads");
        CoreTextSystem { typeface, styled: HashMap::new(), faces: Vec::new(), ids: HashMap::new() }
    }

    /// The typeface at `style`'s weight and figures.
    fn styled(&mut self, style: &TextStyle) -> CTFont {
        let typeface = &self.typeface;
        self.styled
            .entry((style.weight, style.tabular))
            .or_insert_with(|| unsafe {
                let descriptor = typeface.copy_descriptor();
                let wght = CFNumber::from(i64::from(u32::from_be_bytes(*b"wght")));
                let weighted = CTFontDescriptorCreateCopyWithVariation(descriptor.as_concrete_TypeRef(), wght.as_concrete_TypeRef(), f64::from(style.weight));
                let mut descriptor = CTFontDescriptor::wrap_under_create_rule(weighted);
                if style.tabular {
                    let feature = CFDictionary::from_CFType_pairs(&[
                        (CFString::wrap_under_get_rule(kCTFontOpenTypeFeatureTag), CFString::new("tnum").as_CFType()),
                        (CFString::wrap_under_get_rule(kCTFontOpenTypeFeatureValue), CFNumber::from(1).as_CFType()),
                    ]);
                    let features = CFArray::from_CFTypes(&[feature]);
                    let attributes =
                        CFDictionary::from_CFType_pairs(&[(CFString::wrap_under_get_rule(kCTFontFeatureSettingsAttribute), features.as_CFType())]);
                    descriptor = descriptor.create_copy_with_attributes(attributes.to_untyped()).unwrap_or(descriptor);
                }
                core_text::font::new_from_descriptor(&descriptor, 16.0)
            })
            .clone()
    }

    fn id_for_native_font(&mut self, font: CTFont, style: &TextStyle) -> FontId {
        let postscript_name = font.postscript_name();
        let key = if postscript_name == self.typeface.postscript_name() {
            format!("{postscript_name} {} {}", style.weight, style.tabular)
        } else {
            postscript_name.clone()
        };
        if let Some(font_id) = self.ids.get(&key) {
            return *font_id;
        }
        let font_id = FontId(self.faces.len());
        self.ids.insert(key, font_id);
        let emoji = postscript_name == "AppleColorEmoji" || postscript_name == ".AppleColorEmojiUI";
        self.faces.push((font, emoji));
        font_id
    }

    pub fn layout_line(&mut self, text: &str, style: &TextStyle) -> LineLayout {
        let string = self.attributed(text, style);
        let line = CTLine::new_with_attributed_string(string.as_concrete_TypeRef());
        self.shape(&line, text, style, text.len())
    }

    /// `text` broken into lines no wider than `width` by Core Text's typesetter, each line's glyphs placed from its
    /// own left edge, left to right, as Unicode's bidi algorithm reorders a line once it's broken (UAX #9, L1 and
    /// L2); with no width, one line however long. Each line's `len` is where it ends in `text`.
    pub fn layout_lines(&mut self, text: &str, style: &TextStyle, width: Option<f32>) -> Vec<LineLayout> {
        let string = self.attributed(text, style);
        let Some(width) = width else {
            let line = CTLine::new_with_attributed_string(string.as_concrete_TypeRef());
            return vec![self.shape(&line, text, style, text.len())];
        };
        let typesetter = unsafe { CFType::wrap_under_create_rule(CTTypesetterCreateWithAttributedString(string.as_concrete_TypeRef())) };
        let units = string.char_len();
        let mut lines = Vec::new();
        let mut start = 0;
        while start < units {
            let count = unsafe { CTTypesetterSuggestLineBreak(typesetter.as_CFTypeRef(), start, f64::from(width)) }.max(1);
            let line = unsafe { CTLine::wrap_under_create_rule(CTTypesetterCreateLine(typesetter.as_CFTypeRef(), CFRange::init(start, count))) };
            start += count;
            let mut end = StringIndexConverter::new(text);
            end.advance_to_utf16_ix(start as usize);
            lines.push(self.shape(&line, text, style, end.utf8_ix));
        }
        if lines.is_empty() {
            let line = CTLine::new_with_attributed_string(string.as_concrete_TypeRef());
            lines.push(self.shape(&line, text, style, text.len()));
        }
        lines
    }

    /// `text` in `style`'s face at its size, tracked: Core Text's attributed string.
    fn attributed(&mut self, text: &str, style: &TextStyle) -> CFMutableAttributedString {
        let font = self.styled(style).clone_with_font_size(style.size.into());
        let tracking = style.tracking * style.size;
        let mut string = CFMutableAttributedString::new();
        string.replace_str(&CFString::new(text), CFRange::init(0, 0));
        let range = CFRange::init(0, string.char_len());
        unsafe {
            string.set_attribute(range, kCTFontAttributeName, &font);
            if tracking != 0.0 {
                string.set_attribute(range, kCTKernAttributeName, &CFNumber::from(f64::from(tracking)));
            }
        }
        string
    }

    /// A line Core Text set from `text`, its glyphs from its own left edge, ending at `len` in `text`.
    fn shape(&mut self, line: &CTLine, text: &str, style: &TextStyle, len: usize) -> LineLayout {
        let font = self.styled(style).clone_with_font_size(style.size.into());
        let tracking = style.tracking * style.size;
        // Retrieve the glyphs from the shaped line, converting UTF16 offsets to UTF8 offsets.
        let glyph_runs = line.glyph_runs();
        let mut runs = <Vec<ShapedRun>>::with_capacity(glyph_runs.len() as usize);
        let mut ix_converter = StringIndexConverter::new(text);
        let (mut ascent, mut descent) = (font.ascent() as f32, font.descent() as f32);
        for run in glyph_runs.into_iter() {
            let Some(attributes) = run.attributes() else { continue };
            let Some(run_font) = attributes.find(unsafe { CFString::wrap_under_get_rule(kCTFontAttributeName) }).and_then(|font| font.downcast::<CTFont>()) else {
                continue;
            };
            ascent = ascent.max(run_font.ascent() as f32);
            descent = descent.max(run_font.descent() as f32);
            let faced = run_font.clone();
            let font_id = self.id_for_native_font(run_font, style);
            let is_emoji = self.faces[font_id.0].1;
            let glyphs = match runs.last_mut() {
                Some(run) if run.font_id == font_id => &mut run.glyphs,
                _ => {
                    runs.push(ShapedRun { font_id, glyphs: Vec::with_capacity(run.glyph_count().try_into().unwrap_or(0)) });
                    &mut runs.last_mut().expect("just pushed").glyphs
                }
            };
            let rtl = unsafe { CTRunGetStatus(run.as_concrete_TypeRef()) } & RUN_RIGHT_TO_LEFT != 0;
            let ids = run.glyphs();
            let mut advances = vec![CGSize::new(0.0, 0.0); ids.len()];
            unsafe { faced.get_advances_for_glyphs(kCTFontOrientationDefault, ids.as_ptr(), advances.as_mut_ptr(), ids.len() as isize) };
            for (((&glyph_id, position), &glyph_utf16_ix), advance) in ids.iter().zip(run.positions().iter()).zip(run.string_indices().iter()).zip(&advances) {
                let glyph_utf16_ix = usize::try_from(glyph_utf16_ix).unwrap_or_default();
                if ix_converter.utf16_ix > glyph_utf16_ix {
                    // We cannot reuse current index converter, as it can only seek forward. Restart the search.
                    ix_converter = StringIndexConverter::new(text);
                }
                ix_converter.advance_to_utf16_ix(glyph_utf16_ix);
                glyphs.push(ShapedGlyph {
                    id: GlyphId(u32::from(glyph_id)),
                    position: point(position.x as f32, position.y as f32).map(px),
                    index: ix_converter.utf8_ix,
                    end: text.len(),
                    advance: advance.width as f32 + tracking,
                    rtl,
                    is_emoji,
                });
            }
        }
        // Where each glyph's characters end: where the next glyph's, in the text's order, start.
        let mut starts: Vec<usize> = runs.iter().flat_map(|run| run.glyphs.iter().map(|glyph| glyph.index)).collect();
        starts.sort_unstable();
        starts.dedup();
        for glyph in runs.iter_mut().flat_map(|run| run.glyphs.iter_mut()) {
            glyph.end = starts.iter().copied().find(|start| *start > glyph.index).unwrap_or(len);
        }
        let typographic_bounds = line.get_typographic_bounds();
        LineLayout {
            runs,
            font_size: px(style.size),
            width: px(typographic_bounds.width as f32),
            ascent: px(ascent),
            descent: px(descent),
            len,
            // It runs right to left by its first strong character, as Unicode's bidi algorithm says (P2 and P3).
            rtl: unicode_bidi::get_base_direction(text) == unicode_bidi::Direction::Rtl,
        }
    }

    pub fn raster_bounds(&mut self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let font = self.faces[params.font_id.0].0.clone_with_font_size(params.font_size().into());
        let rect = font.get_bounding_rects_for_glyphs(kCTFontOrientationDefault, &[params.glyph_id.0 as CGGlyph]);
        let scale = f64::from(params.scale_factor());
        // Its bounds in device pixels, rounded out, y down from the baseline, as font-kit's `raster_bounds`.
        let left = (rect.origin.x * scale).floor();
        let right = ((rect.origin.x + rect.size.width) * scale).ceil();
        let top = (-(rect.origin.y + rect.size.height) * scale).floor();
        let bottom = (-rect.origin.y * scale).ceil();
        Ok(Bounds {
            origin: point(DevicePixels(left as i32), DevicePixels(top as i32)),
            size: size(DevicePixels((right - left) as i32), DevicePixels((bottom - top) as i32)),
        })
    }

    pub fn rasterize_glyph(&mut self, params: &RenderGlyphParams, glyph_bounds: Bounds<DevicePixels>) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            anyhow::bail!("glyph bounds are empty");
        }
        // Add an extra pixel when the subpixel variant isn't zero to make room for anti-aliasing.
        let mut bitmap_size = glyph_bounds.size;
        if params.subpixel_variant.x > 0 {
            bitmap_size.width += DevicePixels(1);
        }
        if params.subpixel_variant.y > 0 {
            bitmap_size.height += DevicePixels(1);
        }
        let (width, height) = (bitmap_size.width.0 as usize, bitmap_size.height.0 as usize);

        let mut bytes;
        let cx;
        if params.is_emoji {
            bytes = vec![0; width * 4 * height];
            cx = CGContext::create_bitmap_context(
                Some(bytes.as_mut_ptr() as *mut _),
                width,
                height,
                8,
                width * 4,
                &CGColorSpace::create_device_rgb(),
                kCGImageAlphaPremultipliedLast,
            );
        } else {
            bytes = vec![0; width * height];
            cx = CGContext::create_bitmap_context(Some(bytes.as_mut_ptr() as *mut _), width, height, 8, width, &CGColorSpace::create_device_gray(), kCGImageAlphaOnly);
        }

        // Move the origin to bottom left and account for scaling, this
        // makes drawing text consistent with the font-kit's raster_bounds.
        cx.translate(-glyph_bounds.origin.x.0 as CGFloat, (glyph_bounds.origin.y.0 + glyph_bounds.size.height.0) as CGFloat);
        cx.scale(params.scale_factor() as CGFloat, params.scale_factor() as CGFloat);

        let subpixel_shift = params.subpixel_variant.map(|v| v as f32 / SUBPIXEL_VARIANTS_X as f32);
        cx.set_text_drawing_mode(CGTextDrawingMode::CGTextFill);
        cx.set_gray_fill_color(0.0, 1.0);
        cx.set_allows_antialiasing(true);
        cx.set_should_antialias(true);
        cx.set_allows_font_subpixel_positioning(true);
        cx.set_should_subpixel_position_fonts(true);
        cx.set_allows_font_subpixel_quantization(false);
        cx.set_should_subpixel_quantize_fonts(false);
        self.faces[params.font_id.0].0.clone_with_font_size(params.font_size().into()).draw_glyphs(
            &[params.glyph_id.0 as CGGlyph],
            &[CGPoint::new((subpixel_shift.x / params.scale_factor()) as CGFloat, (subpixel_shift.y / params.scale_factor()) as CGFloat)],
            cx,
        );

        if params.is_emoji {
            // Convert from RGBA with premultiplied alpha to BGRA with straight alpha.
            for pixel in bytes.chunks_exact_mut(4) {
                swap_rgba_pa_to_bgra(pixel);
            }
        }
        Ok((bitmap_size, bytes))
    }
}

/// From GPUI's `swap_rgba_pa_to_bgra` (crates/gpui/src/platform.rs).
fn swap_rgba_pa_to_bgra(color: &mut [u8]) {
    color.swap(0, 2);
    if color[3] > 0 {
        let a = color[3] as f32 / 255.;
        color[0] = (color[0] as f32 / a) as u8;
        color[1] = (color[1] as f32 / a) as u8;
        color[2] = (color[2] as f32 / a) as u8;
    }
}

#[derive(Debug, Clone)]
struct StringIndexConverter<'a> {
    text: &'a str,
    /// Index in UTF-8 bytes
    utf8_ix: usize,
    /// Index in UTF-16 code units
    utf16_ix: usize,
}

impl<'a> StringIndexConverter<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, utf8_ix: 0, utf16_ix: 0 }
    }

    fn advance_to_utf16_ix(&mut self, utf16_target: usize) {
        for (ix, c) in self.text[self.utf8_ix..].char_indices() {
            if self.utf16_ix >= utf16_target {
                self.utf8_ix += ix;
                return;
            }
            self.utf16_ix += c.len_utf16();
        }
        self.utf8_ix = self.text.len();
    }
}
