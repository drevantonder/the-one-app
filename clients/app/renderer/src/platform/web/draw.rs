//! What the browser draws for the renderer: a page's paths and its pictures, and the text its own faces draw.

use wasm_bindgen::JsCast as _;

use crate::geometry::Size;
use crate::platform::Native;

use super::WINDOW;

/// A path, SVG's `d` in its `viewBox` stretched to `device` pixels, filled and stroked (`stroke_width` device pixels
/// wide however it's stretched), drawn by the browser's canvas, as its own SVG would draw it: in BGRA, straight.
pub(crate) fn path(d: &str, view_box: &str, device: Size<u32>, fill: Option<String>, stroke: Option<String>, stroke_width: f32) -> Option<crate::view::Decoded> {
    let [x, y, width, height] = <[f64; 4]>::try_from(view_box.split([' ', ',']).filter(|part| !part.is_empty()).map(|part| part.parse().unwrap_or(0.0)).collect::<Vec<f64>>()).ok()?;
    let canvas = web_sys::OffscreenCanvas::new(device.width, device.height).ok()?;
    let context: web_sys::OffscreenCanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let (sx, sy) = (f64::from(device.width) / width.max(f64::EPSILON), f64::from(device.height) / height.max(f64::EPSILON));
    let shape = web_sys::Path2d::new_with_path_string(d).ok()?;
    // In device pixels, so its stroke is as wide however it's stretched (`vector-effect: non-scaling-stroke`).
    let placed = web_sys::Path2d::new().ok()?;
    let matrix = web_sys::DomMatrix2dInit::new();
    (matrix.set_a(sx), matrix.set_d(sy), matrix.set_e(-x * sx), matrix.set_f(-y * sy));
    // `addPath` takes any DOMMatrix2DInit, where web-sys binds only an SVGMatrix.
    let add: js_sys::Function = js_sys::Reflect::get(&placed, &"addPath".into()).ok()?.dyn_into().ok()?;
    add.call2(&placed, &shape, &matrix).ok()?;
    if let Some(fill) = fill {
        context.set_fill_style_str(&fill);
        context.fill_with_path_2d(&placed);
    }
    if let Some(stroke) = stroke {
        context.set_stroke_style_str(&stroke);
        context.set_line_width(f64::from(stroke_width));
        context.set_line_cap("round");
        context.set_line_join("round");
        context.stroke_with_path(&placed);
    }
    let pixels = context.get_image_data(0.0, 0.0, f64::from(device.width), f64::from(device.height)).ok()?.data().0;
    Some(bgra(device, pixels))
}

/// The face the browser draws what no face of the renderer's has in: its UI's, which falls back to its emoji's.
const BROWSER_FONT: &str = "system-ui, sans-serif";

/// A 2D canvas of `width` by `height`, its text in the browser's face at `size` pixels.
fn canvas(width: u32, height: u32, size: f32) -> Option<web_sys::OffscreenCanvasRenderingContext2d> {
    let canvas = web_sys::OffscreenCanvas::new(width.max(1), height.max(1)).ok()?;
    let context: web_sys::OffscreenCanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    context.set_font(&format!("{size}px {BROWSER_FONT}"));
    Some(context)
}

/// How wide `text` is in the browser's face, in ems, and whether the browser draws it in colour (an emoji's) or in
/// one, as a symbol, which is then drawn in its text's colour (`text::browser`).
pub(crate) fn measure(text: &str) -> (f32, bool) {
    const SIZE: f32 = 100.0;
    let Some(context) = canvas(1, 1, SIZE) else { return (0.0, false) };
    let advance = context.measure_text(text).map_or(0.0, |metrics| metrics.width() as f32 / SIZE);
    // Drawn in black and in red: one that's the same both times has colours of its own, as an emoji (a grey one too).
    let colored = match (glyph_in(text, 32.0, "#000"), glyph_in(text, 32.0, "#f00")) {
        (Some(black), Some(red)) => !black.4.is_empty() && black.4 == red.4,
        _ => false,
    };
    (advance, colored)
}

/// `text` as the browser's canvas draws it at `size` pixels, in black where it has no colour of its own: where it is
/// from its pen (left, and top above the baseline), its size, and its pixels, RGBA, straight.
pub(crate) fn glyph(text: &str, size: f32) -> Option<(i32, i32, u32, u32, Vec<u8>)> {
    glyph_in(text, size, "#000")
}

/// `text` drawn as `glyph` says, in `colour` where it has none of its own.
fn glyph_in(text: &str, size: f32, colour: &str) -> Option<(i32, i32, u32, u32, Vec<u8>)> {
    let metrics = canvas(1, 1, size)?.measure_text(text).ok()?;
    let left = (-metrics.actual_bounding_box_left()).floor() as i32;
    let right = metrics.actual_bounding_box_right().ceil() as i32;
    let (ascent, descent) = (metrics.actual_bounding_box_ascent().ceil() as i32, metrics.actual_bounding_box_descent().ceil() as i32);
    let (width, height) = ((right - left).max(0) as u32, (ascent + descent).max(0) as u32);
    if width == 0 || height == 0 {
        return Some((0, 0, 0, 0, Vec::new()));
    }
    let context = canvas(width, height, size)?;
    context.set_fill_style_str(colour);
    context.fill_text(text, f64::from(-left), f64::from(ascent)).ok()?;
    let pixels = context.get_image_data(0.0, 0.0, f64::from(width), f64::from(height)).ok()?.data().0;
    Some((left, ascent, width, height, pixels))
}

/// The browser's straight RGBA as the atlas's BGRA.
fn bgra(size: Size<u32>, mut pixels: Vec<u8>) -> crate::view::Decoded {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    crate::view::Decoded { picture: crate::view::Picture { width: size.width, height: size.height }, bgra: pixels }
}

/// A `data:` address's picture, decoded by the browser for a box of `device` pixels, as a native host's are
/// (`view::paint::decode`): an SVG drawn at that size, and any other at the smallest of its size halved again and again
/// that still covers what of the box it fills.
pub(crate) async fn decode(src: &str, device: Size<u32>) -> Option<crate::view::Decoded> {
    let image = web_sys::HtmlImageElement::new().ok()?;
    image.set_src(&allo(src).unwrap_or_else(|| src.to_owned()));
    wasm_bindgen_futures::JsFuture::from(image.decode()).await.ok()?;
    let svg = src.starts_with("data:image/svg+xml");
    let size = if svg {
        device
    } else {
        let (mut width, mut height) = (image.natural_width().max(1), image.natural_height().max(1));
        let fit = (device.width as f32 / width as f32).min(device.height as f32 / height as f32);
        let shown = (width as f32 * fit, height as f32 * fit);
        while (width / 2) as f32 >= shown.0 && (height / 2) as f32 >= shown.1 && width > 1 && height > 1 {
            (width, height) = (width / 2, height / 2);
        }
        Size { width, height }
    };
    let canvas = web_sys::OffscreenCanvas::new(size.width, size.height).ok()?;
    let context: web_sys::OffscreenCanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let (width, height) = (f64::from(size.width), f64::from(size.height));
    if svg {
        // Fitted in its box, keeping its shape, as resvg draws one natively.
        let (natural_width, natural_height) = (f64::from(image.natural_width().max(1)), f64::from(image.natural_height().max(1)));
        let fit = (width / natural_width).min(height / natural_height);
        context.draw_image_with_html_image_element_and_dw_and_dh(&image, 0.0, 0.0, natural_width * fit, natural_height * fit).ok()?;
    } else {
        context.draw_image_with_html_image_element_and_dw_and_dh(&image, 0.0, 0.0, width, height).ok()?;
    }
    Some(bgra(size, context.get_image_data(0.0, 0.0, width, height).ok()?.data().0))
}

/// An SVG's text in Allo, as a native host draws it (`view::paint::svg`), where the browser would draw its
/// `system-ui` and `sans-serif` in its own face: the typeface given it as a `data:` address, the only kind an SVG
/// drawn as a picture may load. → the SVG again as a `data:` address, if it has text.
fn allo(src: &str) -> Option<String> {
    let (head, body) = src.strip_prefix("data:image/svg+xml")?.split_once(',')?;
    let svg = if head.ends_with(";base64") { String::from_utf8(data_encoding::BASE64.decode(body.as_bytes()).ok()?).ok()? } else { String::from(js_sys::decode_uri_component(body).ok()?) };
    if !svg.contains("<text") {
        return None;
    }
    let opened = svg.find("<svg")? + svg[svg.find("<svg")?..].find('>')? + 1;
    thread_local! {
        static FACE: String = format!("<style>@font-face{{font-family:Allo;font-weight:1 1000;src:url(data:font/woff2;base64,{})}}</style>", data_encoding::BASE64.encode(crate::view::ALLO));
    }
    let svg = FACE.with(|face| format!("{}{face}{}", &svg[..opened], &svg[opened..])).replace("system-ui", "Allo").replace("sans-serif", "Allo");
    Some(format!("data:image/svg+xml;base64,{}", data_encoding::BASE64.encode(svg.as_bytes())))
}

/// A picture a page names by an address, fetched as one of its app's files by the browser, which the screen then has
/// as a `data:` address (`Screen::fetched`): the frame's own page's, or that of the page in frame `frame`. A fetch that
/// fails on the network is tried again as a decode is (`RETRIES_MS`), and the picture is pending (`Screen::fetching`)
/// until it's come or failed every time; one the server doesn't have (a 404) isn't.
pub(super) fn fetch(page: &url::Url, src: &str, frame: Option<crate::tree::Id>) {
    let (src, address) = (src.to_owned(), page.join(src).map(String::from).unwrap_or_default());
    wasm_bindgen_futures::spawn_local(async move {
        let data = fetched(&address).await;
        let _ = Native::get().map(|native| native.run(move |windows| {
            windows.with(WINDOW, |screen| match frame {
                Some(node) => screen.frame_mut(node).map_or((), |screen| screen.fetched(&src, data)),
                None => screen.fetched(&src, data),
            });
        }));
    });
}

/// What `address` holds as a `data:` address: none where it isn't found (a 404), or the network failed every time.
async fn fetched(address: &str) -> Option<String> {
    for after in crate::view::RETRIES_MS.iter().map(|after| Some(*after)).chain([None]) {
        match fetch_once(address).await {
            Ok(found) => return found,
            Err(()) => match after {
                Some(after) => gloo_timers::future::TimeoutFuture::new(after as u32).await,
                None => return None,
            },
        }
    }
    None
}

/// One try at `address`: its `data:` address, none where the server hasn't it, or an error where the network failed.
async fn fetch_once(address: &str) -> Result<Option<String>, ()> {
    let window = web_sys::window().ok_or(())?;
    let response: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(address)).await.map_err(drop)?.dyn_into().map_err(drop)?;
    if !response.ok() {
        return Ok(None);
    }
    let kind = response.headers().get("content-type").ok().flatten().unwrap_or_default();
    let bytes = wasm_bindgen_futures::JsFuture::from(response.array_buffer().map_err(drop)?).await.map_err(drop)?;
    Ok(Some(format!("data:{kind};base64,{}", data_encoding::BASE64.encode(&js_sys::Uint8Array::new(&bytes).to_vec()))))
}
