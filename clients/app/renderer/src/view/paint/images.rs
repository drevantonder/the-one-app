//! Painting a picture, a sprite, an image, a video and a path.

use super::*;

impl Painter<'_> {
    /// A picture by its `data:` address at `bounds`, its corners rounded `radius`.
    pub(in crate::view) fn picture(&mut self, src: &str, bounds: Bounds<f32>, radius: f32) {
        let device = Size { width: (bounds.size.width * self.scale).round() as u32, height: (bounds.size.height * self.scale).round() as u32 };
        if let Some((_, tile)) = self.pictures.get(self.atlas, src, device) {
            let window = self.window;
            self.sprite(tile, bounds, Corners::all(radius), 1.0, window);
        }
    }

    /// A picture's sprite, `width`×`height` device pixels, at `bounds` in the window.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn sprite(&mut self, tile: AtlasTile, bounds: Bounds<f32>, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
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

    /// An image, scaled to fit its box keeping its shape, as GPUI's `img` is by default, decoded for its box as it
    /// rests (`Transform::rest`), and not at all where it doesn't show. One that's broken (not found, or that failed to
    /// decode every time it was tried) fills its box in a quiet grey, as a browser marks a broken image, never nothing.
    pub(in crate::view) fn paint_image(&mut self, node: &Node, content: Bounds<f32>, transform: Transform, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
        let Some(src) = node.str("src").filter(|_| opacity > 0.0) else { return };
        let broken = |painter: &mut Self| painter.quad(content, mask, colour_token("quinary-label", painter.conditions), Edges::default(), Hsla::default(), radii, false, opacity);
        // One by an address is drawn once its window has fetched it, as a `data:` address.
        let src = match self.fetched.get(src) {
            _ if src.starts_with("data:") => src,
            Some(Some(data)) => data.as_str(),
            Some(None) => return broken(self),
            None => {
                self.pictures.drew_blank();
                return self.wanted.push(src.to_owned());
            }
        };
        let rest = if transform.scale > 0.0 { transform.rest / transform.scale } else { 1.0 };
        let device = Size { width: (content.size.width * self.scale * rest).round() as u32, height: (content.size.height * self.scale * rest).round() as u32 };
        if device.width == 0 || device.height == 0 || content.size.width <= 0.0 || content.size.height <= 0.0 {
            return;
        }
        if self.pictures.broken(src) {
            return broken(self);
        }
        let Some((picture, tile)) = self.pictures.get(self.atlas, src, device) else { return self.pictures.drew_blank() };
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
    pub(in crate::view) fn paint_video(&mut self, id: Id, content: Bounds<f32>, radii: Corners<f32>, opacity: f32, mask: Bounds<f32>) {
        #[cfg(target_arch = "wasm32")]
        let device = Size { width: (content.size.width * self.scale).round() as u32, height: (content.size.height * self.scale).round() as u32 };
        #[cfg(target_arch = "wasm32")]
        let Some((tile, frame)) = crate::platform::web::video(id, content, device, self.atlas) else { return };
        #[cfg(not(target_arch = "wasm32"))]
        let Some((tile, frame)) = self.videos.get_mut(&id).and_then(|video| video.tile(id, content, self.atlas)) else { return };
        let fit = (content.size.width / frame.width as f32).min(content.size.height / frame.height as f32);
        let size = Size { width: frame.width as f32 * fit, height: frame.height as f32 * fit };
        let bounds = Bounds { origin: point(content.origin.x + (content.size.width - size.width) / 2.0, content.origin.y + (content.size.height - size.height) / 2.0), size };
        self.sprite(tile, bounds, radii, opacity, mask);
    }

    /// A path, SVG's: its `d` in its `viewBox`, stretched to its box, filled and stroked as its style says, drawn by
    /// resvg at the window's scale.
    pub(in crate::view) fn paint_path(&mut self, node: &Node, content: Bounds<f32>, look: &Look, current: Hsla, opacity: f32, mask: Bounds<f32>) {
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
