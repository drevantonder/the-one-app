// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/windows/direct_write.rs) at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), and cut down for the strict host: DirectWrite's
// shaping and rasterizing, on Windows. Changed: the typeface is loaded from memory into a typographic collection, and a
// line is laid out at the weight asked for on the font's `wght` axis (`IDWriteTextFormat3`'s axis values), which the
// glyph runs' faces then draw by; letter spacing is DirectWrite's character spacing; there are no user fallback chains;
// and a colour glyph's layers are blended on the CPU rather than on the GPU.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use std::cell::RefCell;

use anyhow::{Context as _, Result};
use windows::Win32::Foundation::{E_NOTIMPL, RECT};
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::{BOOL, HSTRING, IUnknown, Interface, Ref, implement};
use windows_numerics::Vector2;

use super::{FontId, GlyphId, LineLayout, RenderGlyphParams, SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, ShapedGlyph, ShapedRun, TextStyle};
use crate::geometry::{Bounds, DevicePixels, Size, point, px, size};

pub struct DirectWriteTextSystem {
    factory: IDWriteFactory6,
    collection: IDWriteFontCollection2,
    /// No fallback beyond the collection's own fonts, where the system's aren't wanted.
    fallback: Option<IDWriteFontFallback>,
    family: HSTRING,
    locale: HSTRING,
    renderer: IDWriteTextRenderer,
    /// The faces glyphs are drawn from, each at its axes' values, by `FontId`.
    faces: RefCell<Vec<Face>>,
    // Kept for the in-memory font files, which DirectWrite reads as it needs them.
    _loader: IDWriteInMemoryFontFileLoader,
    _fonts: Vec<Vec<u8>>,
}

struct Face {
    face: IDWriteFontFace3,
    key: FaceKey,
    color: bool,
}

/// What tells two faces apart: the font's PostScript name, its axes' values and its simulations.
#[derive(PartialEq)]
struct FaceKey {
    postscript_name: String,
    axes: Vec<(u32, u32)>,
    simulations: i32,
}

impl DirectWriteTextSystem {
    /// Text in `family`, from `fonts`, and from the system's fonts where those lack a character, if `system_fonts`.
    pub fn new(family: &str, fonts: Vec<Vec<u8>>, system_fonts: bool) -> Self {
        Self::try_new(family, fonts, system_fonts).expect("DirectWrite, which Windows 10 1803 and later have")
    }

    fn try_new(family: &str, fonts: Vec<Vec<u8>>, system_fonts: bool) -> Result<Self> {
        unsafe {
            let factory: IDWriteFactory6 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            // The `IDWriteInMemoryFontFileLoader` here is supported starting from
            // Windows 10 Creators Update.
            let loader = factory.CreateInMemoryFontFileLoader()?;
            factory.RegisterFontFileLoader(&loader)?;
            let builder: IDWriteFontSetBuilder1 = factory.CreateFontSetBuilder()?.cast()?;
            for font in &fonts {
                let file = loader.CreateInMemoryFontFileReference(&factory, font.as_ptr() as _, font.len() as _, None)?;
                builder.AddFontFile(&file)?;
            }
            let set = builder.CreateFontSet()?;
            // Typographic families, as CSS has them, so a variable font is one family along its axes.
            let collection = factory.CreateFontCollectionFromFontSet(&set, DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC)?;
            let fallback = if system_fonts { None } else { Some(factory.CreateFontFallbackBuilder()?.CreateFontFallback()?) };
            let locale = HSTRING::from(sys_locale::get_locale().unwrap_or_else(|| "en-US".into()));
            let renderer: IDWriteTextRenderer = TextRenderer.into();
            Ok(Self {
                factory,
                collection,
                fallback,
                family: HSTRING::from(family),
                locale,
                renderer,
                faces: RefCell::default(),
                _loader: loader,
                _fonts: fonts,
            })
        }
    }

    pub fn layout_line(&mut self, text: &str, style: &TextStyle) -> LineLayout {
        self.try_layout_line(text, style).unwrap_or_else(|error| {
            log::error!("laying out {text:?}: {error:#}");
            LineLayout { font_size: px(style.size), len: text.len(), ..Default::default() }
        })
    }

    /// `text` broken into lines no wider than `width`, where Unicode's line breaking (UAX #14) allows, each line set
    /// by DirectWrite on its own, so each is reordered as Unicode's bidi algorithm says once it's broken (UAX #9, L1
    /// and L2), its glyphs from its own left edge; with no width, one line however long. Each line's `len` is where it
    /// ends in `text`.
    pub fn layout_lines(&mut self, text: &str, style: &TextStyle, width: Option<f32>) -> Vec<LineLayout> {
        let Some(width) = width else { return vec![self.layout_line(text, style)] };
        let breaks: Vec<usize> = unicode_linebreak::linebreaks(text).map(|(at, _)| at).collect();
        let mut lines = Vec::new();
        let mut start = 0;
        while start < text.len() {
            let mut fits = breaks.iter().copied().filter(|at| *at > start);
            let mut end = fits.next().unwrap_or(text.len());
            for at in fits {
                if self.layout_line(text[start..at].trim_end(), style).width.0 > width {
                    break;
                }
                end = at;
            }
            let mut line = self.layout_line(&text[start..end], style);
            for glyph in line.runs.iter_mut().flat_map(|run| run.glyphs.iter_mut()) {
                glyph.index += start;
                glyph.end += start;
            }
            line.len = end;
            line.rtl = unicode_bidi::get_base_direction(text) == unicode_bidi::Direction::Rtl;
            lines.push(line);
            start = end;
        }
        if lines.is_empty() {
            lines.push(self.layout_line(text, style));
        }
        lines
    }

    fn try_layout_line(&mut self, text: &str, style: &TextStyle) -> Result<LineLayout> {
        if text.is_empty() {
            return Ok(LineLayout { font_size: px(style.size), ..Default::default() });
        }
        unsafe {
            let wide: Vec<u16> = text.encode_utf16().collect();
            // Its optical size is its size, as CSS's `font-optical-sizing: auto` sets a variable font's `opsz`.
            let axes = [
                DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT, value: f32::from(style.weight) },
                DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_OPTICAL_SIZE, value: style.size },
            ];
            let format: IDWriteTextFormat3 = self.factory.CreateTextFormat(&self.family, &self.collection, &axes, style.size, &self.locale)?;
            if let Some(fallback) = &self.fallback {
                format.SetFontFallback(fallback)?;
            }
            let layout: IDWriteTextLayout1 = self.factory.CreateTextLayout(&wide, &format, f32::INFINITY, f32::INFINITY)?.cast()?;
            let whole = DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 };
            if style.tabular {
                let typography = self.factory.CreateTypography()?;
                typography.AddFontFeature(DWRITE_FONT_FEATURE { nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, parameter: 1 })?;
                layout.SetTypography(&typography, whole)?;
            }
            if style.tracking != 0.0 {
                layout.SetCharacterSpacing(0.0, style.tracking * style.size, 0.0, whole)?;
            }
            // Each UTF-16 unit's byte in the text, and the text's end.
            let utf8_of: Vec<usize> = text.char_indices().flat_map(|(at, c)| std::iter::repeat_n(at, c.len_utf16())).chain([text.len()]).collect();
            let mut context = RendererContext { text_system: self, utf8_of, runs: Vec::new(), ascent: 0.0, descent: 0.0, size: style.size };
            layout.Draw(Some(&mut context as *mut _ as _), &self.renderer, 0.0, 0.0)?;
            let RendererContext { runs, mut ascent, mut descent, .. } = context;
            if runs.is_empty() {
                // Only spaces: their line's metrics, from its first line.
                let mut metrics = [DWRITE_LINE_METRICS::default(); 1];
                let mut count = 0;
                let _ = layout.GetLineMetrics(Some(&mut metrics), &mut count);
                ascent = metrics[0].baseline;
                descent = metrics[0].height - metrics[0].baseline;
            }
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            Ok(LineLayout {
                font_size: px(style.size),
                width: px(metrics.widthIncludingTrailingWhitespace),
                ascent: px(ascent),
                descent: px(descent),
                runs,
                len: text.len(),
                rtl: unicode_bidi::get_base_direction(text) == unicode_bidi::Direction::Rtl,
            })
        }
    }

    /// The `FontId` of `face`, kept the first time it's seen.
    fn font_id(&self, face: &IDWriteFontFace3) -> Option<FontId> {
        let key = face_key(face, &self.locale)?;
        let mut faces = self.faces.borrow_mut();
        if let Some(at) = faces.iter().position(|known| known.key == key) {
            return Some(FontId(at));
        }
        let color = unsafe { face.IsColorFont().as_bool() };
        faces.push(Face { face: face.clone(), key, color });
        Some(FontId(faces.len() - 1))
    }

    fn create_glyph_run_analysis(&self, params: &RenderGlyphParams) -> Result<IDWriteGlyphRunAnalysis> {
        let faces = self.faces.borrow();
        let font = &faces.get(params.font_id.0).context("no such face")?.face;
        let glyph_id = [params.glyph_id.0 as u16];
        let advance = [0.0];
        let offset = [DWRITE_GLYPH_OFFSET::default()];
        let glyph_run = DWRITE_GLYPH_RUN {
            fontFace: unsafe { std::mem::transmute_copy(font) },
            fontEmSize: params.font_size(),
            glyphCount: 1,
            glyphIndices: glyph_id.as_ptr(),
            glyphAdvances: advance.as_ptr(),
            glyphOffsets: offset.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        let scale_factor = params.scale_factor();
        let transform = DWRITE_MATRIX { m11: scale_factor, m12: 0.0, m21: 0.0, m22: scale_factor, dx: 0.0, dy: 0.0 };
        let baseline_origin_x = params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / scale_factor;
        let baseline_origin_y = params.subpixel_variant.y as f32 / SUBPIXEL_VARIANTS_Y as f32 / scale_factor;

        let mut rendering_mode = DWRITE_RENDERING_MODE1::default();
        let mut grid_fit_mode = DWRITE_GRID_FIT_MODE::default();
        unsafe {
            font.GetRecommendedRenderingMode(
                params.font_size(),
                // Using 96 as scale is applied by the transform
                96.0,
                96.0,
                Some(&transform),
                false,
                DWRITE_OUTLINE_THRESHOLD_ANTIALIASED,
                DWRITE_MEASURING_MODE_NATURAL,
                None,
                &mut rendering_mode,
                &mut grid_fit_mode,
            )?;
        }
        let rendering_mode = match rendering_mode {
            DWRITE_RENDERING_MODE1_OUTLINE => DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
            m => m,
        };

        let glyph_analysis = unsafe {
            self.factory.CreateGlyphRunAnalysis(
                &glyph_run,
                Some(&transform),
                rendering_mode,
                DWRITE_MEASURING_MODE_NATURAL,
                grid_fit_mode,
                DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                baseline_origin_x,
                baseline_origin_y,
            )
        }?;
        Ok(glyph_analysis)
    }

    pub fn raster_bounds(&mut self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let glyph_analysis = self.create_glyph_run_analysis(params)?;
        let bounds = unsafe { glyph_analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1)? };
        if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
            Ok(Bounds { origin: point(0.into(), 0.into()), size: size(0.into(), 0.into()) })
        } else {
            Ok(Bounds {
                origin: point(bounds.left.into(), bounds.top.into()),
                size: size((bounds.right - bounds.left).into(), (bounds.bottom - bounds.top).into()),
            })
        }
    }

    pub fn rasterize_glyph(&mut self, params: &RenderGlyphParams, glyph_bounds: Bounds<DevicePixels>) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        if glyph_bounds.size.width.0 == 0 || glyph_bounds.size.height.0 == 0 {
            anyhow::bail!("glyph bounds are empty");
        }
        let bitmap_data = if params.is_emoji {
            match self.rasterize_color(params, glyph_bounds) {
                Ok(color) => color,
                Err(_) => self.rasterize_monochrome(params, glyph_bounds)?.into_iter().flat_map(|pixel| [0, 0, 0, pixel]).collect(),
            }
        } else {
            self.rasterize_monochrome(params, glyph_bounds)?
        };
        Ok((glyph_bounds.size, bitmap_data))
    }

    fn rasterize_monochrome(&self, params: &RenderGlyphParams, glyph_bounds: Bounds<DevicePixels>) -> Result<Vec<u8>> {
        let mut bitmap_data = vec![0u8; glyph_bounds.size.width.0 as usize * glyph_bounds.size.height.0 as usize];
        let glyph_analysis = self.create_glyph_run_analysis(params)?;
        unsafe {
            glyph_analysis.CreateAlphaTexture(
                DWRITE_TEXTURE_ALIASED_1x1,
                &RECT {
                    left: glyph_bounds.origin.x.0,
                    top: glyph_bounds.origin.y.0,
                    right: glyph_bounds.size.width.0 + glyph_bounds.origin.x.0,
                    bottom: glyph_bounds.size.height.0 + glyph_bounds.origin.y.0,
                },
                &mut bitmap_data,
            )?;
        }
        Ok(bitmap_data)
    }

    /// A colour glyph (COLR) in BGRA: each of its layers rasterized as a monochrome glyph and blended over the ones
    /// before in its colour.
    fn rasterize_color(&self, params: &RenderGlyphParams, glyph_bounds: Bounds<DevicePixels>) -> Result<Vec<u8>> {
        let faces = self.faces.borrow();
        let font = &faces.get(params.font_id.0).context("no such face")?.face;
        let scale_factor = params.scale_factor();
        let baseline_origin_x = params.subpixel_variant.x as f32 / SUBPIXEL_VARIANTS_X as f32 / scale_factor;
        let baseline_origin_y = params.subpixel_variant.y as f32 / SUBPIXEL_VARIANTS_Y as f32 / scale_factor;
        let transform = DWRITE_MATRIX { m11: scale_factor, m12: 0.0, m21: 0.0, m22: scale_factor, dx: 0.0, dy: 0.0 };
        let glyph_id = [params.glyph_id.0 as u16];
        let advance = [0.0];
        let offset = [DWRITE_GLYPH_OFFSET::default()];
        let glyph_run = DWRITE_GLYPH_RUN {
            fontFace: unsafe { std::mem::transmute_copy(font) },
            fontEmSize: params.font_size(),
            glyphCount: 1,
            glyphIndices: glyph_id.as_ptr(),
            glyphAdvances: advance.as_ptr(),
            glyphOffsets: offset.as_ptr(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        // The run's face is borrowed (`transmute_copy` into a `ManuallyDrop`), so its count is left alone.
        let enumerator = unsafe {
            self.factory.TranslateColorGlyphRun(
                Vector2::new(baseline_origin_x, baseline_origin_y),
                &glyph_run,
                None,
                DWRITE_GLYPH_IMAGE_FORMATS_COLR,
                DWRITE_MEASURING_MODE_NATURAL,
                Some(&transform),
                0,
            )
        }?;
        let (width, height) = (glyph_bounds.size.width.0 as usize, glyph_bounds.size.height.0 as usize);
        let mut out = vec![0f32; width * height * 4];
        loop {
            let run = unsafe { &*enumerator.GetCurrentRun()? };
            let analysis = unsafe {
                self.factory.CreateGlyphRunAnalysis(
                    &run.Base.glyphRun,
                    Some(&transform),
                    DWRITE_RENDERING_MODE1_NATURAL_SYMMETRIC,
                    DWRITE_MEASURING_MODE_NATURAL,
                    DWRITE_GRID_FIT_MODE_DEFAULT,
                    DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                    run.Base.baselineOriginX,
                    run.Base.baselineOriginY,
                )
            }?;
            let bounds = unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1) }?;
            let (layer_width, layer_height) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
            if layer_width > 0 && layer_height > 0 {
                let mut alpha = vec![0u8; (layer_width * layer_height) as usize];
                unsafe { analysis.CreateAlphaTexture(DWRITE_TEXTURE_ALIASED_1x1, &bounds, &mut alpha) }?;
                let color = run.Base.runColor;
                for y in 0..layer_height {
                    for x in 0..layer_width {
                        let (out_x, out_y) = (bounds.left + x - glyph_bounds.origin.x.0, bounds.top + y - glyph_bounds.origin.y.0);
                        if out_x < 0 || out_y < 0 || out_x as usize >= width || out_y as usize >= height {
                            continue;
                        }
                        let a = alpha[(y * layer_width + x) as usize] as f32 / 255.0 * color.a;
                        let pixel = &mut out[(out_y as usize * width + out_x as usize) * 4..][..4];
                        // Premultiplied, blended over what's there.
                        for (channel, value) in pixel.iter_mut().zip([color.b, color.g, color.r, 1.0]) {
                            *channel = value * a + *channel * (1.0 - a);
                        }
                    }
                }
            }
            if !unsafe { enumerator.MoveNext() }?.as_bool() {
                break;
            }
        }
        // Back to straight alpha, as the atlas's polychrome tiles are.
        Ok(out
            .chunks_exact(4)
            .flat_map(|pixel| {
                let a = pixel[3];
                let straight = |channel: f32| if a > 0.0 { (channel / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
                [straight(pixel[0]), straight(pixel[1]), straight(pixel[2]), (a * 255.0).round() as u8]
            })
            .collect())
    }
}

fn face_key(face: &IDWriteFontFace3, locale: &HSTRING) -> Option<FaceKey> {
    let postscript_name = postscript_name(face, locale)?;
    let axes = face
        .cast::<IDWriteFontFace5>()
        .ok()
        .map(|face| unsafe {
            let mut values = vec![DWRITE_FONT_AXIS_VALUE::default(); face.GetFontAxisValueCount() as usize];
            let _ = face.GetFontAxisValues(&mut values);
            values.iter().map(|value| (value.axisTag.0, value.value.to_bits())).collect()
        })
        .unwrap_or_default();
    Some(FaceKey { postscript_name, axes, simulations: unsafe { face.GetSimulations().0 } })
}

fn postscript_name(face: &IDWriteFontFace3, locale: &HSTRING) -> Option<String> {
    let mut info = None;
    let mut exists = BOOL(0);
    unsafe { face.GetInformationalStrings(DWRITE_INFORMATIONAL_STRING_POSTSCRIPT_NAME, &mut info, &mut exists) }.ok()?;
    let info = info.filter(|_| exists.as_bool())?;
    let mut index = 0;
    let mut found = BOOL(0);
    unsafe { info.FindLocaleName(locale, &mut index, &mut found) }.ok()?;
    if !found.as_bool() {
        index = 0;
    }
    let length = unsafe { info.GetStringLength(index) }.ok()? as usize;
    let mut name = vec![0u16; length + 1];
    unsafe { info.GetString(index, &mut name) }.ok()?;
    Some(String::from_utf16_lossy(&name[..length]))
}

/// What `Draw` hands each glyph run to: where the line's runs go, as they're shaped.
struct RendererContext<'t> {
    text_system: &'t DirectWriteTextSystem,
    utf8_of: Vec<usize>,
    runs: Vec<ShapedRun>,
    ascent: f32,
    descent: f32,
    size: f32,
}

#[implement(IDWriteTextRenderer)]
struct TextRenderer;

#[derive(Debug)]
struct ClusterAnalyzer<'t> {
    utf16_idx: usize,
    glyph_idx: usize,
    glyph_count: usize,
    cluster_map: &'t [u16],
}

impl<'t> ClusterAnalyzer<'t> {
    pub fn new(cluster_map: &'t [u16], glyph_count: usize) -> Self {
        ClusterAnalyzer { utf16_idx: 0, glyph_idx: 0, glyph_count, cluster_map }
    }
}

impl Iterator for ClusterAnalyzer<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        if self.utf16_idx >= self.cluster_map.len() {
            return None; // No more clusters
        }
        let start_utf16_idx = self.utf16_idx;
        let current_glyph = self.cluster_map[start_utf16_idx] as usize;

        // Find the end of current cluster (where glyph index changes)
        let mut end_utf16_idx = start_utf16_idx + 1;
        while end_utf16_idx < self.cluster_map.len() && self.cluster_map[end_utf16_idx] as usize == current_glyph {
            end_utf16_idx += 1;
        }

        let utf16_len = end_utf16_idx - start_utf16_idx;

        // Calculate glyph count for this cluster
        let next_glyph = if end_utf16_idx < self.cluster_map.len() { self.cluster_map[end_utf16_idx] as usize } else { self.glyph_count };

        let glyph_count = next_glyph - current_glyph;

        // Update state for next call
        self.utf16_idx = end_utf16_idx;
        self.glyph_idx = next_glyph;

        Some((utf16_len, glyph_count))
    }
}

#[allow(non_snake_case)]
impl IDWritePixelSnapping_Impl for TextRenderer_Impl {
    fn IsPixelSnappingDisabled(&self, _clientdrawingcontext: *const ::core::ffi::c_void) -> windows::core::Result<BOOL> {
        Ok(BOOL(0))
    }

    fn GetCurrentTransform(&self, _clientdrawingcontext: *const ::core::ffi::c_void, transform: *mut DWRITE_MATRIX) -> windows::core::Result<()> {
        unsafe {
            *transform = DWRITE_MATRIX { m11: 1.0, m12: 0.0, m21: 0.0, m22: 1.0, dx: 0.0, dy: 0.0 };
        }
        Ok(())
    }

    fn GetPixelsPerDip(&self, _clientdrawingcontext: *const ::core::ffi::c_void) -> windows::core::Result<f32> {
        Ok(1.0)
    }
}

#[allow(non_snake_case)]
impl IDWriteTextRenderer_Impl for TextRenderer_Impl {
    fn DrawGlyphRun(
        &self,
        clientdrawingcontext: *const ::core::ffi::c_void,
        baselineoriginx: f32,
        _baselineoriginy: f32,
        _measuringmode: DWRITE_MEASURING_MODE,
        glyphrun: *const DWRITE_GLYPH_RUN,
        glyphrundescription: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _clientdrawingeffect: Ref<IUnknown>,
    ) -> windows::core::Result<()> {
        let glyphrun = unsafe { &*glyphrun };
        let glyph_count = glyphrun.glyphCount as usize;
        let Some(font_face) = glyphrun.fontFace.as_ref().filter(|_| glyph_count > 0) else {
            return Ok(());
        };
        let desc = unsafe { &*glyphrundescription };
        let context = unsafe { &mut *(clientdrawingcontext as *mut RendererContext) };
        // This `cast()` action here should never fail since we are running on Win10+, and
        // `IDWriteFontFace3` requires Win10
        let font_face = &font_face.cast::<IDWriteFontFace3>()?;
        let Some(font_id) = context.text_system.font_id(font_face) else {
            return Ok(());
        };
        let color_font = context.text_system.faces.borrow()[font_id.0].color;
        let mut metrics = DWRITE_FONT_METRICS1::default();
        unsafe { font_face.GetMetrics(&mut metrics) };
        let em = context.size / f32::from(metrics.Base.designUnitsPerEm.max(1));
        context.ascent = context.ascent.max(f32::from(metrics.Base.ascent) * em);
        context.descent = context.descent.max(f32::from(metrics.Base.descent) * em);

        let glyph_ids = unsafe { std::slice::from_raw_parts(glyphrun.glyphIndices, glyph_count) };
        let glyph_advances = unsafe { std::slice::from_raw_parts(glyphrun.glyphAdvances, glyph_count) };
        let glyph_offsets = unsafe { std::slice::from_raw_parts(glyphrun.glyphOffsets, glyph_count) };
        let cluster_map = unsafe { std::slice::from_raw_parts(desc.clusterMap, desc.stringLength as usize) };

        // A right-to-left run (an odd bidi level) is drawn leftwards from its origin, its glyphs in the text's order.
        let rtl = glyphrun.bidiLevel % 2 == 1;
        let cluster_analyzer = ClusterAnalyzer::new(cluster_map, glyph_count);
        let mut utf16_idx = desc.textPosition as usize;
        let mut glyph_idx = 0;
        let mut along = 0.0;
        let mut glyphs = Vec::with_capacity(glyph_count);
        let utf8 = |utf16: usize| context.utf8_of.get(utf16).copied().unwrap_or_else(|| *context.utf8_of.last().unwrap_or(&0));
        for (cluster_utf16_len, cluster_glyph_count) in cluster_analyzer {
            let (index, end) = (utf8(utf16_idx), utf8(utf16_idx + cluster_utf16_len));
            utf16_idx += cluster_utf16_len;
            for (cluster_glyph_idx, glyph_id) in glyph_ids[glyph_idx..(glyph_idx + cluster_glyph_count)].iter().enumerate() {
                let id = GlyphId(*glyph_id as u32);
                let is_emoji = color_font && is_color_glyph(font_face, id, &context.text_system.factory);
                let this_glyph_idx = glyph_idx + cluster_glyph_idx;
                let (advance, offset) = (glyph_advances[this_glyph_idx], glyph_offsets[this_glyph_idx]);
                let x = if rtl { baselineoriginx - along - advance - offset.advanceOffset } else { baselineoriginx + along + offset.advanceOffset };
                glyphs.push(ShapedGlyph { id, position: point(px(x), px(-offset.ascenderOffset)), index, end, advance, rtl, is_emoji });
                along += advance;
            }
            glyph_idx += cluster_glyph_count;
        }
        context.runs.push(ShapedRun { font_id, glyphs });
        Ok(())
    }

    fn DrawUnderline(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
        _baselineoriginx: f32,
        _baselineoriginy: f32,
        _underline: *const DWRITE_UNDERLINE,
        _clientdrawingeffect: Ref<IUnknown>,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::new(E_NOTIMPL, "DrawUnderline unimplemented"))
    }

    fn DrawStrikethrough(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
        _baselineoriginx: f32,
        _baselineoriginy: f32,
        _strikethrough: *const DWRITE_STRIKETHROUGH,
        _clientdrawingeffect: Ref<IUnknown>,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::new(E_NOTIMPL, "DrawStrikethrough unimplemented"))
    }

    fn DrawInlineObject(
        &self,
        _clientdrawingcontext: *const ::core::ffi::c_void,
        _originx: f32,
        _originy: f32,
        _inlineobject: Ref<IDWriteInlineObject>,
        _issideways: BOOL,
        _isrighttoleft: BOOL,
        _clientdrawingeffect: Ref<IUnknown>,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::new(E_NOTIMPL, "DrawInlineObject unimplemented"))
    }
}

fn is_color_glyph(font_face: &IDWriteFontFace3, glyph_id: GlyphId, factory: &IDWriteFactory6) -> bool {
    let glyph_run = DWRITE_GLYPH_RUN {
        fontFace: unsafe { std::mem::transmute_copy(font_face) },
        fontEmSize: 14.0,
        glyphCount: 1,
        glyphIndices: &(glyph_id.0 as u16),
        glyphAdvances: &0.0,
        glyphOffsets: &DWRITE_GLYPH_OFFSET { advanceOffset: 0.0, ascenderOffset: 0.0 },
        isSideways: BOOL(0),
        bidiLevel: 0,
    };
    unsafe {
        factory.TranslateColorGlyphRun(
            Vector2::default(),
            &glyph_run as _,
            None,
            DWRITE_GLYPH_IMAGE_FORMATS_COLR
                | DWRITE_GLYPH_IMAGE_FORMATS_SVG
                | DWRITE_GLYPH_IMAGE_FORMATS_PNG
                | DWRITE_GLYPH_IMAGE_FORMATS_JPEG
                | DWRITE_GLYPH_IMAGE_FORMATS_PREMULTIPLIED_B8G8R8A8,
            DWRITE_MEASURING_MODE_NATURAL,
            None,
            0,
        )
    }
    .is_ok()
}
