// Copied from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/blade/shaders.wgsl) at commit
// 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), without its paths and video surfaces, which the
// native host doesn't draw, and with backdrops, inset shadows and transformed quads, shadows and images, which are
// ours; its bindings numbered as GPUI's wgpu shaders number them (crates/gpui_wgpu/src/shaders.wgsl): the frame's in
// group 0, a batch's instances in group 1 (every pipeline's at binding 0, as each binds only its own), and the texture
// it samples in group 2. Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

/* Functions useful for debugging:

// A heat map color for debugging (blue -> cyan -> green -> yellow -> red).
fn heat_map_color(value: f32, minValue: f32, maxValue: f32, position: vec2<f32>) -> vec4<f32> {
    // Normalize value to 0-1 range
    let t = clamp((value - minValue) / (maxValue - minValue), 0.0, 1.0);

    // Heat map color calculation
    let r = t * t;
    let g = 4.0 * t * (1.0 - t);
    let b = (1.0 - t) * (1.0 - t);
    let heat_color = vec3<f32>(r, g, b);

    // Create a checkerboard pattern (black and white)
    let sum = floor(position.x / 3) + floor(position.y / 3);
    let is_odd = fract(sum * 0.5); // 0.0 for even, 0.5 for odd
    let checker_value = is_odd * 2.0; // 0.0 for even, 1.0 for odd
    let checker_color = vec3<f32>(checker_value);

    // Determine if value is in range (1.0 if in range, 0.0 if out of range)
    let in_range = step(minValue, value) * step(value, maxValue);

    // Mix checkerboard and heat map based on whether value is in range
    let final_color = mix(checker_color, heat_color, in_range);

    return vec4<f32>(final_color, 1.0);
}

*/

fn color_brightness(color: vec3<f32>) -> f32 {
    // REC. 601 luminance coefficients for perceived brightness
    return dot(color, vec3<f32>(0.30, 0.59, 0.11));
}

fn light_on_dark_contrast(enhancedContrast: f32, color: vec3<f32>) -> f32 {
    let brightness = color_brightness(color);
    let multiplier = saturate(4.0 * (0.75 - brightness));
    return enhancedContrast * multiplier;
}

fn enhance_contrast(alpha: f32, k: f32) -> f32 {
    return alpha * (k + 1.0) / (alpha * k + 1.0);
}

fn apply_alpha_correction(a: f32, b: f32, g: vec4<f32>) -> f32 {
    let brightness_adjustment = g.x * b + g.y;
    let correction = brightness_adjustment * a + (g.z * b + g.w);
    return a + a * (1.0 - a) * correction;
}

fn apply_contrast_and_gamma_correction(sample: f32, color: vec3<f32>, enhanced_contrast_factor: f32, gamma_ratios: vec4<f32>) -> f32 {
    let enhanced_contrast = light_on_dark_contrast(enhanced_contrast_factor, color);
    let brightness = color_brightness(color);

    let contrasted = enhance_contrast(sample, enhanced_contrast);
    return apply_alpha_correction(contrasted, brightness, gamma_ratios);
}

// The target's size, and where in the scene its top left is: a window's part draws its own box of the scene.
struct GlobalParams {
    viewport_size: vec2<f32>,
    origin: vec2<f32>,
    premultiplied_alpha: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

struct GammaParams {
    gamma_ratios: vec4<f32>,
    grayscale_enhanced_contrast: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
}

@group(0) @binding(0) var<uniform> globals: GlobalParams;
@group(0) @binding(1) var<uniform> gamma_params: GammaParams;
// A 64×64 tile of blue noise, which `dither` reads.
@group(0) @binding(2) var t_noise: texture_2d<f32>;
@group(2) @binding(0) var t_sprite: texture_2d<f32>;
@group(2) @binding(1) var s_sprite: sampler;

const M_PI_F: f32 = 3.1415926;
const GRAYSCALE_FACTORS: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

struct Bounds {
    origin: vec2<f32>,
    size: vec2<f32>,
}

struct Corners {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32,
}

struct Edges {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

struct Hsla {
    h: f32,
    s: f32,
    l: f32,
    a: f32,
}

struct LinearColorStop {
    color: Hsla,
    percentage: f32,
}

struct Background {
    // 0u is Solid
    // 1u is LinearGradient
    // 2u is PatternSlash
    tag: u32,
    // 0u is sRGB linear color
    // 1u is Oklab color
    color_space: u32,
    solid: Hsla,
    gradient_angle_or_pattern_height: f32,
    colors: array<LinearColorStop, 2>,
    pad: u32,
}

struct AtlasTextureId {
    index: u32,
    kind: u32,
}

struct AtlasBounds {
    origin: vec2<i32>,
    size: vec2<i32>,
}

struct AtlasTile {
    texture_id: AtlasTextureId,
    tile_id: u32,
    padding: u32,
    bounds: AtlasBounds,
}

struct TransformationMatrix {
    rotation_scale: mat2x2<f32>,
    translation: vec2<f32>,
}

fn to_device_position_impl(position: vec2<f32>) -> vec4<f32> {
    let device_position = (position - globals.origin) / globals.viewport_size * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(device_position, 0.0, 1.0);
}

fn to_device_position(unit_vertex: vec2<f32>, bounds: Bounds) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    return to_device_position_impl(position);
}

fn to_device_position_transformed(unit_vertex: vec2<f32>, bounds: Bounds, transform: TransformationMatrix) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    //Note: Rust side stores it as row-major, so transposing here
    let transformed = transpose(transform.rotation_scale) * position + transform.translation;
    return to_device_position_impl(transformed);
}

fn to_tile_position(unit_vertex: vec2<f32>, tile: AtlasTile) -> vec2<f32> {
  let atlas_size = vec2<f32>(textureDimensions(t_sprite, 0));
  return (vec2<f32>(tile.bounds.origin) + unit_vertex * vec2<f32>(tile.bounds.size)) / atlas_size;
}

fn distance_from_clip_rect_impl(position: vec2<f32>, clip_bounds: Bounds) -> vec4<f32> {
    let tl = position - clip_bounds.origin;
    let br = clip_bounds.origin + clip_bounds.size - position;
    return vec4<f32>(tl.x, br.x, tl.y, br.y);
}

fn distance_from_clip_rect(unit_vertex: vec2<f32>, bounds: Bounds, clip_bounds: Bounds) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    return distance_from_clip_rect_impl(position, clip_bounds);
}

fn distance_from_clip_rect_transformed(unit_vertex: vec2<f32>, bounds: Bounds, clip_bounds: Bounds, transform: TransformationMatrix) -> vec4<f32> {
    let position = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    let transformed = transpose(transform.rotation_scale) * position + transform.translation;
    return distance_from_clip_rect_impl(transformed, clip_bounds);
}

// https://gamedev.stackexchange.com/questions/92015/optimized-linear-to-srgb-glsl
fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    let lower = srgb / vec3<f32>(12.92);
    return select(higher, lower, cutoff);
}

fn srgb_to_linear_component(a: f32) -> f32 {
    let cutoff = a < 0.04045;
    let higher = pow((a + 0.055) / 1.055, 2.4);
    let lower = a / 12.92;
    return select(higher, lower, cutoff);
}

fn linear_to_srgb(linear: vec3<f32>) -> vec3<f32> {
    let cutoff = linear < vec3<f32>(0.0031308);
    let higher = vec3<f32>(1.055) * pow(linear, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    let lower = linear * vec3<f32>(12.92);
    return select(higher, lower, cutoff);
}

/// Convert a linear color to sRGBA space.
fn linear_to_srgba(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(linear_to_srgb(color.rgb), color.a);
}

/// Convert a sRGBA color to linear space.
fn srgba_to_linear(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(srgb_to_linear(color.rgb), color.a);
}

/// Hsla to linear RGBA conversion.
fn hsla_to_rgba(hsla: Hsla) -> vec4<f32> {
    let h = hsla.h * 6.0; // Now, it's an angle but scaled in [0, 6) range
    let s = hsla.s;
    let l = hsla.l;
    let a = hsla.a;

    let c = (1.0 - abs(2.0 * l - 1.0)) * s;
    let x = c * (1.0 - abs(h % 2.0 - 1.0));
    let m = l - c / 2.0;
    var color = vec3<f32>(m);

    if (h >= 0.0 && h < 1.0) {
        color.r += c;
        color.g += x;
    } else if (h >= 1.0 && h < 2.0) {
        color.r += x;
        color.g += c;
    } else if (h >= 2.0 && h < 3.0) {
        color.g += c;
        color.b += x;
    } else if (h >= 3.0 && h < 4.0) {
        color.g += x;
        color.b += c;
    } else if (h >= 4.0 && h < 5.0) {
        color.r += x;
        color.b += c;
    } else {
        color.r += c;
        color.b += x;
    }

    return vec4<f32>(color, a);
}

/// Convert a linear sRGB to Oklab space.
/// Reference: https://bottosson.github.io/posts/oklab/#converting-from-linear-srgb-to-oklab
fn linear_srgb_to_oklab(color: vec4<f32>) -> vec4<f32> {
	let l = 0.4122214708 * color.r + 0.5363325363 * color.g + 0.0514459929 * color.b;
	let m = 0.2119034982 * color.r + 0.6806995451 * color.g + 0.1073969566 * color.b;
	let s = 0.0883024619 * color.r + 0.2817188376 * color.g + 0.6299787005 * color.b;

	let l_ = pow(l, 1.0 / 3.0);
	let m_ = pow(m, 1.0 / 3.0);
	let s_ = pow(s, 1.0 / 3.0);

	return vec4<f32>(
		0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
		1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
		0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
		color.a
	);
}

/// Convert an Oklab color to linear sRGB space.
fn oklab_to_linear_srgb(color: vec4<f32>) -> vec4<f32> {
	let l_ = color.r + 0.3963377774 * color.g + 0.2158037573 * color.b;
	let m_ = color.r - 0.1055613458 * color.g - 0.0638541728 * color.b;
	let s_ = color.r - 0.0894841775 * color.g - 1.2914855480 * color.b;

	let l = l_ * l_ * l_;
	let m = m_ * m_ * m_;
	let s = s_ * s_ * s_;

	return vec4<f32>(
		4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
		-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
		-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
		color.a
	);
}

fn over(below: vec4<f32>, above: vec4<f32>) -> vec4<f32> {
    let alpha = above.a + below.a * (1.0 - above.a);
    let color = (above.rgb * above.a + below.rgb * below.a * (1.0 - above.a)) / alpha;
    return vec4<f32>(color, alpha);
}

// A standard gaussian function, used for weighting samples
fn gaussian(x: f32, sigma: f32) -> f32{
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(2.0 * M_PI_F) * sigma);
}

// This approximates the error function, needed for the gaussian integral
fn erf(v: vec2<f32>) -> vec2<f32> {
    let s = sign(v);
    let a = abs(v);
    let r1 = 1.0 + (0.278393 + (0.230389 + (0.000972 + 0.078108 * a) * a) * a) * a;
    let r2 = r1 * r1;
    return s - s / (r2 * r2);
}

fn blur_along_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2<f32>) -> f32 {
  let delta = min(half_size.y - corner - abs(y), 0.0);
  let curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
  let integral = 0.5 + 0.5 * erf((x + vec2<f32>(-curved, curved)) * (sqrt(0.5) / sigma));
  return integral.y - integral.x;
}

// Selects corner radius based on quadrant.
fn pick_corner_radius(center_to_point: vec2<f32>, radii: Corners) -> f32 {
    if (center_to_point.x < 0.0) {
        if (center_to_point.y < 0.0) {
            return radii.top_left;
        } else {
            return radii.bottom_left;
        }
    } else {
        if (center_to_point.y < 0.0) {
            return radii.top_right;
        } else {
            return radii.bottom_right;
        }
    }
}

// A corner of `radius` as Apple draws one, continuous (SwiftUI's `RoundedRectangle(style: .continuous)`, a layer's
// `cornerCurve`): its curve leaves each edge 1.528 radii from the corner, where a circle's leaves at one, and bends
// from straight, its curvature growing from nothing (G2), where a circle's jumps from none to all of it. Here it's a
// superellipse across that reach, whose exponent puts its middle where the circle's is, so a corner looks as round as
// its radius says. Where a box has no room for the whole reach, as a capsule hasn't, it's as much as fits, and a
// circle at the radius. `x` is its reach, `y` its exponent (2, a circle's, where there's no room to spare).
fn continuous_corner(radius: f32, half_size: vec2<f32>) -> vec2<f32> {
    if (radius <= 0.0) {
        return vec2<f32>(0.0, 2.0);
    }
    let reach = max(radius, min(1.528665 * radius, min(half_size.x, half_size.y)));
    // A circle's middle is radius·(1 - 1/√2) in from each edge; a superellipse's, reach·(1 - 2^(-1/exponent)).
    let exponent = -log(2.0) / log(1.0 - (1.0 - inverseSqrt(2.0)) * radius / reach);
    return vec2<f32>(reach, exponent);
}

// Signed distance of the point to the quad's border - positive outside the
// border, and negative inside.
//
// See comments on similar code using `quad_sdf_impl` in `fs_quad` for
// explanation.
fn quad_sdf(point: vec2<f32>, bounds: Bounds, corner_radii: Corners) -> f32 {
    let half_size = bounds.size / 2.0;
    let center = bounds.origin + half_size;
    let center_to_point = point - center;
    let corner = continuous_corner(pick_corner_radius(center_to_point, corner_radii), half_size);
    let corner_to_point = abs(center_to_point) - half_size;
    let corner_center_to_point = corner_to_point + corner.x;
    return quad_sdf_impl(corner_center_to_point, corner);
}

// `corner` is `continuous_corner`'s: its reach and its exponent.
fn quad_sdf_impl(corner_center_to_point: vec2<f32>, corner: vec2<f32>) -> f32 {
    let corner_radius = corner.x;
    if (corner_radius == 0.0) {
        // Fast path for unrounded corners.
        return max(corner_center_to_point.x, corner_center_to_point.y);
    } else if (corner_center_to_point.x > 0.0 && corner_center_to_point.y > 0.0) {
        // In the corner: how far it is from its superellipse, |p|ₙ = reach, to first order (the norm's excess over
        // its gradient's length), in terms of its larger side so that nothing overflows.
        let n = corner.y;
        let most = max(corner_center_to_point.x, corner_center_to_point.y);
        let unit = corner_center_to_point / most;
        let norm = pow(pow(unit.x, n) + pow(unit.y, n), 1.0 / n);
        let gradient = pow(unit / norm, vec2<f32>(n - 1.0));
        return (most * norm - corner_radius) / length(gradient);
    } else {
        // Signed distance of the point from a quad that is inset by corner_radius.
        // It is negative inside this quad, and positive outside.
        let signed_distance_to_inset_quad =
            // 0 inside the inset quad, and positive outside.
            length(max(vec2<f32>(0.0), corner_center_to_point)) +
            // 0 outside the inset quad, and negative inside.
            min(0.0, max(corner_center_to_point.x, corner_center_to_point.y));

        return signed_distance_to_inset_quad - corner_radius;
    }
}

// Abstract away the final color transformation based on the
// target alpha compositing mode.
fn blend_color(color: vec4<f32>, alpha_factor: f32) -> vec4<f32> {
    let alpha = color.a * alpha_factor;
    let multiplier = select(1.0, alpha, globals.premultiplied_alpha != 0u);
    return vec4<f32>(color.rgb * multiplier, alpha);
}

// How far to move a colour at pixel `position` so that what changes slowly across pixels, as a gradient, a shadow's
// edge or a blur does, shows no bands where its 8 bits a channel step: less than half a step either way, by blue
// noise, whose dither the eye doesn't see as a pattern, as Skia dithers a gradient.
fn dither(position: vec2<f32>) -> f32 {
    return (textureLoad(t_noise, vec2<u32>(position) % 64u, 0).r - 0.5) / 255.0;
}

// `blend_color`, dithered at `position`: its colour, and its alpha where it's see-through, which is all of a
// shadow's change.
fn blend_color_dithered(color: vec4<f32>, alpha_factor: f32, position: vec2<f32>) -> vec4<f32> {
    let noise = dither(position);
    let alpha = color.a * alpha_factor;
    let dithered = select(alpha, saturate(alpha + noise), alpha > 0.0 && alpha < 1.0);
    return blend_color(vec4<f32>(color.rgb + noise, 1.0), dithered);
}

struct GradientColor {
    solid: vec4<f32>,
    color0: vec4<f32>,
    color1: vec4<f32>,
}

fn prepare_gradient_color(tag: u32, color_space: u32,
    solid: Hsla, colors: array<LinearColorStop, 2>) -> GradientColor {
    var result = GradientColor();

    if (tag == 0u || tag == 2u) {
        result.solid = hsla_to_rgba(solid);
    } else if (tag == 1u) {
        // The hsla_to_rgba is returns a linear sRGB color
        result.color0 = hsla_to_rgba(colors[0].color);
        result.color1 = hsla_to_rgba(colors[1].color);

        // Prepare color space in vertex for avoid conversion
        // in fragment shader for performance reasons
        if (color_space == 0u) {
            // sRGB
            result.color0 = linear_to_srgba(result.color0);
            result.color1 = linear_to_srgba(result.color1);
        } else if (color_space == 1u) {
            // Oklab
            result.color0 = linear_srgb_to_oklab(result.color0);
            result.color1 = linear_srgb_to_oklab(result.color1);
        }
    }

    return result;
}

fn gradient_color(background: Background, position: vec2<f32>, bounds: Bounds,
    solid_color: vec4<f32>, color0: vec4<f32>, color1: vec4<f32>) -> vec4<f32> {
    var background_color = vec4<f32>(0.0);

    switch (background.tag) {
        default: {
            return solid_color;
        }
        case 1u: {
            // Linear gradient background.
            // -90 degrees to match the CSS gradient angle.
            let angle = background.gradient_angle_or_pattern_height;
            let radians = (angle % 360.0 - 90.0) * M_PI_F / 180.0;
            var direction = vec2<f32>(cos(radians), sin(radians));
            let stop0_percentage = background.colors[0].percentage;
            let stop1_percentage = background.colors[1].percentage;

            // Expand the short side to be the same as the long side
            if (bounds.size.x > bounds.size.y) {
                direction.y *= bounds.size.y / bounds.size.x;
            } else {
                direction.x *= bounds.size.x / bounds.size.y;
            }

            // Get the t value for the linear gradient with the color stop percentages.
            let half_size = bounds.size / 2.0;
            let center = bounds.origin + half_size;
            let center_to_point = position - center;
            var t = dot(center_to_point, direction) / length(direction);
            // Check the direct to determine the use x or y
            if (abs(direction.x) > abs(direction.y)) {
                t = (t + half_size.x) / bounds.size.x;
            } else {
                t = (t + half_size.y) / bounds.size.y;
            }

            // Adjust t based on the stop percentages
            t = (t - stop0_percentage) / (stop1_percentage - stop0_percentage);
            t = clamp(t, 0.0, 1.0);

            switch (background.color_space) {
                default: {
                    background_color = srgba_to_linear(mix(color0, color1, t));
                }
                case 1u: {
                    let oklab_color = mix(color0, color1, t);
                    background_color = oklab_to_linear_srgb(oklab_color);
                }
            }
        }
        case 2u: {
            let gradient_angle_or_pattern_height = background.gradient_angle_or_pattern_height;
            let pattern_width = (gradient_angle_or_pattern_height / 65535.0f) / 255.0f;
            let pattern_interval = (gradient_angle_or_pattern_height % 65535.0f) / 255.0f;
            let pattern_height = pattern_width + pattern_interval;
            let stripe_angle = M_PI_F / 4.0;
            let pattern_period = pattern_height * sin(stripe_angle);
            let rotation = mat2x2<f32>(
                cos(stripe_angle), -sin(stripe_angle),
                sin(stripe_angle), cos(stripe_angle)
            );
            let relative_position = position - bounds.origin;
            let rotated_point = rotation * relative_position;
            let pattern = rotated_point.x % pattern_period;
            let distance = min(pattern, pattern_period - pattern) - pattern_period * (pattern_width / pattern_height) /  2.0f;
            background_color = solid_color;
            background_color.a *= saturate(0.5 - distance);
        }
    }

    return background_color;
}

// --- quads --- //

struct Quad {
    order: u32,
    border_style: u32,
    bounds: Bounds,
    content_mask: Bounds,
    background: Background,
    border_color: Hsla,
    corner_radii: Corners,
    border_widths: Edges,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_quads: array<Quad>;

struct QuadVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) border_color: vec4<f32>,
    @location(1) @interpolate(flat) quad_id: u32,
    // TODO: use `clip_distance` once Naga supports it
    @location(2) clip_distances: vec4<f32>,
    @location(3) @interpolate(flat) background_solid: vec4<f32>,
    @location(4) @interpolate(flat) background_color0: vec4<f32>,
    @location(5) @interpolate(flat) background_color1: vec4<f32>,
    // Where it is in the quad's own space, before its transformation, which its edges are measured in.
    @location(6) local: vec2<f32>,
}

@vertex
fn vs_quad(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> QuadVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let quad = b_quads[instance_id];

    var out = QuadVarying();
    out.position = to_device_position_transformed(unit_vertex, quad.bounds, quad.transformation);
    out.local = unit_vertex * vec2<f32>(quad.bounds.size) + quad.bounds.origin;

    let gradient = prepare_gradient_color(
        quad.background.tag,
        quad.background.color_space,
        quad.background.solid,
        quad.background.colors
    );
    out.background_solid = gradient.solid;
    out.background_color0 = gradient.color0;
    out.background_color1 = gradient.color1;
    out.border_color = hsla_to_rgba(quad.border_color);
    out.quad_id = instance_id;
    out.clip_distances = distance_from_clip_rect_transformed(unit_vertex, quad.bounds, quad.content_mask, quad.transformation);
    return out;
}

@fragment
fn fs_quad(input: QuadVarying) -> @location(0) vec4<f32> {
    // Alpha clip first, since we don't have `clip_distance`.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let quad = b_quads[input.quad_id];

    var background_color = gradient_color(quad.background, input.local, quad.bounds,
        input.background_solid, input.background_color0, input.background_color1);
    if (quad.background.tag == 1u) {
        background_color = vec4<f32>(background_color.rgb + dither(input.position.xy), background_color.a);
    }

    let unrounded = quad.corner_radii.top_left == 0.0 &&
        quad.corner_radii.bottom_left == 0.0 &&
        quad.corner_radii.top_right == 0.0 &&
        quad.corner_radii.bottom_right == 0.0;

    // Fast path when the quad is not rounded and doesn't have any border
    if (quad.border_widths.top == 0.0 &&
            quad.border_widths.left == 0.0 &&
            quad.border_widths.right == 0.0 &&
            quad.border_widths.bottom == 0.0 &&
            unrounded) {
        return blend_color(background_color, 1.0);
    }

    let size = quad.bounds.size;
    let half_size = size / 2.0;
    let point = input.local - quad.bounds.origin;
    let center_to_point = point - half_size;

    // Signed distance field threshold for inclusion of pixels. 0.5 is the
    // minimum distance between the center of the pixel and the edge.
    let antialias_threshold = 0.5;

    // The nearest corner, continuous: how far in from the corner it bends (`corner_radius`), and how.
    let corner = continuous_corner(pick_corner_radius(center_to_point, quad.corner_radii), half_size);
    let corner_radius = corner.x;

    // Width of the nearest borders
    let border = vec2<f32>(
        select(
            quad.border_widths.right,
            quad.border_widths.left,
            center_to_point.x < 0.0),
        select(
            quad.border_widths.bottom,
            quad.border_widths.top,
            center_to_point.y < 0.0));

    // 0-width borders are reduced so that `inner_sdf >= antialias_threshold`.
    // The purpose of this is to not draw antialiasing pixels in this case.
    let reduced_border =
        vec2<f32>(select(border.x, -antialias_threshold, border.x == 0.0),
                  select(border.y, -antialias_threshold, border.y == 0.0));

    // Vector from the corner of the quad bounds to the point, after mirroring
    // the point into the bottom right quadrant. Both components are <= 0.
    let corner_to_point = abs(center_to_point) - half_size;

    // Vector from the point to the center of the rounded corner's circle, also
    // mirrored into bottom right quadrant.
    let corner_center_to_point = corner_to_point + corner_radius;

    // Whether the nearest point on the border is rounded
    let is_near_rounded_corner =
            corner_center_to_point.x >= 0 &&
            corner_center_to_point.y >= 0;

    // Vector from straight border inner corner to point.
    let straight_border_inner_corner_to_point = corner_to_point + reduced_border;

    // Whether the point is beyond the inner edge of the straight border.
    let is_beyond_inner_straight_border =
            straight_border_inner_corner_to_point.x > 0 ||
            straight_border_inner_corner_to_point.y > 0;

    // Whether the point is far enough inside the quad, such that the pixels are
    // not affected by the straight border.
    let is_within_inner_straight_border =
        straight_border_inner_corner_to_point.x < -antialias_threshold &&
        straight_border_inner_corner_to_point.y < -antialias_threshold;

    // Fast path for points that must be part of the background.
    //
    // This could be optimized further for large rounded corners by including
    // points in an inscribed rectangle, or some other quick linear check.
    // However, that might negatively impact performance in the case of
    // reasonable sizes for rounded corners.
    if (is_within_inner_straight_border && !is_near_rounded_corner) {
        return blend_color(background_color, 1.0);
    }

    // Signed distance of the point to the outside edge of the quad's border. It
    // is positive outside this edge, and negative inside.
    let outer_sdf = quad_sdf_impl(corner_center_to_point, corner);

    // Approximate signed distance of the point to the inside edge of the quad's
    // border. It is negative outside this edge (within the border), and
    // positive inside.
    //
    // This is not always an accurate signed distance:
    // * The rounded portions with varying border width use an approximation of
    //   nearest-point-on-ellipse.
    // * When it is quickly known to be outside the edge, -1.0 is used.
    var inner_sdf = 0.0;
    if (corner_center_to_point.x <= 0 || corner_center_to_point.y <= 0) {
        // Fast paths for straight borders.
        inner_sdf = -max(straight_border_inner_corner_to_point.x,
                         straight_border_inner_corner_to_point.y);
    } else if (is_beyond_inner_straight_border) {
        // Fast path for points that must be outside the inner edge.
        inner_sdf = -1.0;
    } else if (reduced_border.x == reduced_border.y) {
        // Fast path for circular inner edge.
        inner_sdf = -(outer_sdf + reduced_border.x);
    } else {
        let ellipse_radii = max(vec2<f32>(0.0), corner_radius - reduced_border);
        inner_sdf = quarter_ellipse_sdf(corner_center_to_point, ellipse_radii);
    }

    // Negative when inside the border
    let border_sdf = max(inner_sdf, outer_sdf);

    var color = background_color;
    if (border_sdf < antialias_threshold) {
        var border_color = input.border_color;

        // Dashed border logic when border_style == 1
        if (quad.border_style == 1) {
            // Position along the perimeter in "dash space", where each dash
            // period has length 1
            var t = 0.0;

            // Total number of dash periods, so that the dash spacing can be
            // adjusted to evenly divide it
            var max_t = 0.0;

            // Border width is proportional to dash size. This is the behavior
            // used by browsers, but also avoids dashes from different segments
            // overlapping when dash size is smaller than the border width.
            //
            // Dash pattern: (2 * border width) dash, (1 * border width) gap
            let dash_length_per_width = 2.0;
            let dash_gap_per_width = 1.0;
            let dash_period_per_width = dash_length_per_width + dash_gap_per_width;

            // Since the dash size is determined by border width, the density of
            // dashes varies. Multiplying a pixel distance by this returns a
            // position in dash space - it has units (dash period / pixels). So
            // a dash velocity of (1 / 10) is 1 dash every 10 pixels.
            var dash_velocity = 0.0;

            // Dividing this by the border width gives the dash velocity
            let dv_numerator = 1.0 / dash_period_per_width;

            if (unrounded) {
                // When corners aren't rounded, the dashes are separately laid
                // out on each straight line, rather than around the whole
                // perimeter. This way each line starts and ends with a dash.
                let is_horizontal =
                        corner_center_to_point.x <
                        corner_center_to_point.y;

                // When applying dashed borders to just some, not all, the sides.
                // The way we chose border widths above sometimes comes with a 0 width value.
                // So we choose again to avoid division by zero.
                // TODO: A better solution exists taking a look at the whole file.
                // this does not fix single dashed borders at the corners
                let dashed_border = vec2<f32>(
                        max(
                            quad.border_widths.bottom,
                            quad.border_widths.top,
                        ),
                        max(
                            quad.border_widths.right,
                            quad.border_widths.left,
                        )
                   );

                let border_width = select(dashed_border.y, dashed_border.x, is_horizontal);
                dash_velocity = dv_numerator / border_width;
                t = select(point.y, point.x, is_horizontal) * dash_velocity;
                max_t = select(size.y, size.x, is_horizontal) * dash_velocity;
            } else {
                // When corners are rounded, the dashes are laid out clockwise
                // around the whole perimeter.

                let r_tr = continuous_corner(quad.corner_radii.top_right, half_size).x;
                let r_br = continuous_corner(quad.corner_radii.bottom_right, half_size).x;
                let r_bl = continuous_corner(quad.corner_radii.bottom_left, half_size).x;
                let r_tl = continuous_corner(quad.corner_radii.top_left, half_size).x;

                let w_t = quad.border_widths.top;
                let w_r = quad.border_widths.right;
                let w_b = quad.border_widths.bottom;
                let w_l = quad.border_widths.left;

                // Straight side dash velocities
                let dv_t = select(dv_numerator / w_t, 0.0, w_t <= 0.0);
                let dv_r = select(dv_numerator / w_r, 0.0, w_r <= 0.0);
                let dv_b = select(dv_numerator / w_b, 0.0, w_b <= 0.0);
                let dv_l = select(dv_numerator / w_l, 0.0, w_l <= 0.0);

                // Straight side lengths in dash space
                let s_t = (size.x - r_tl - r_tr) * dv_t;
                let s_r = (size.y - r_tr - r_br) * dv_r;
                let s_b = (size.x - r_br - r_bl) * dv_b;
                let s_l = (size.y - r_bl - r_tl) * dv_l;

                let corner_dash_velocity_tr = corner_dash_velocity(dv_t, dv_r);
                let corner_dash_velocity_br = corner_dash_velocity(dv_b, dv_r);
                let corner_dash_velocity_bl = corner_dash_velocity(dv_b, dv_l);
                let corner_dash_velocity_tl = corner_dash_velocity(dv_t, dv_l);

                // Corner lengths in dash space
                let c_tr = r_tr * (M_PI_F / 2.0) * corner_dash_velocity_tr;
                let c_br = r_br * (M_PI_F / 2.0) * corner_dash_velocity_br;
                let c_bl = r_bl * (M_PI_F / 2.0) * corner_dash_velocity_bl;
                let c_tl = r_tl * (M_PI_F / 2.0) * corner_dash_velocity_tl;

                // Cumulative dash space upto each segment
                let upto_tr = s_t;
                let upto_r = upto_tr + c_tr;
                let upto_br = upto_r + s_r;
                let upto_b = upto_br + c_br;
                let upto_bl = upto_b + s_b;
                let upto_l = upto_bl + c_bl;
                let upto_tl = upto_l + s_l;
                max_t = upto_tl + c_tl;

                if (is_near_rounded_corner) {
                    let radians = atan2(corner_center_to_point.y,
                                        corner_center_to_point.x);
                    let corner_t = radians * corner_radius;

                    if (center_to_point.x >= 0.0) {
                        if (center_to_point.y < 0.0) {
                            dash_velocity = corner_dash_velocity_tr;
                            // Subtracted because radians is pi/2 to 0 when
                            // going clockwise around the top right corner,
                            // since the y axis has been flipped
                            t = upto_r - corner_t * dash_velocity;
                        } else {
                            dash_velocity = corner_dash_velocity_br;
                            // Added because radians is 0 to pi/2 when going
                            // clockwise around the bottom-right corner
                            t = upto_br + corner_t * dash_velocity;
                        }
                    } else {
                        if (center_to_point.y >= 0.0) {
                            dash_velocity = corner_dash_velocity_bl;
                            // Subtracted because radians is pi/2 to 0 when
                            // going clockwise around the bottom-left corner,
                            // since the x axis has been flipped
                            t = upto_l - corner_t * dash_velocity;
                        } else {
                            dash_velocity = corner_dash_velocity_tl;
                            // Added because radians is 0 to pi/2 when going
                            // clockwise around the top-left corner, since both
                            // axis were flipped
                            t = upto_tl + corner_t * dash_velocity;
                        }
                    }
                } else {
                    // Straight borders
                    let is_horizontal =
                            corner_center_to_point.x <
                            corner_center_to_point.y;
                    if (is_horizontal) {
                        if (center_to_point.y < 0.0) {
                            dash_velocity = dv_t;
                            t = (point.x - r_tl) * dash_velocity;
                        } else {
                            dash_velocity = dv_b;
                            t = upto_bl - (point.x - r_bl) * dash_velocity;
                        }
                    } else {
                        if (center_to_point.x < 0.0) {
                            dash_velocity = dv_l;
                            t = upto_tl - (point.y - r_tl) * dash_velocity;
                        } else {
                            dash_velocity = dv_r;
                            t = upto_r + (point.y - r_tr) * dash_velocity;
                        }
                    }
                }
            }

            let dash_length = dash_length_per_width / dash_period_per_width;
            let desired_dash_gap = dash_gap_per_width / dash_period_per_width;

            // Straight borders should start and end with a dash, so max_t is
            // reduced to cause this.
            max_t -= select(0.0, dash_length, unrounded);
            if (max_t >= 1.0) {
                // Adjust dash gap to evenly divide max_t.
                let dash_count = floor(max_t);
                let dash_period = max_t / dash_count;
                border_color.a *= dash_alpha(
                    t,
                    dash_period,
                    dash_length,
                    dash_velocity,
                    antialias_threshold);
            } else if (unrounded) {
                // When there isn't enough space for the full gap between the
                // two start / end dashes of a straight border, reduce gap to
                // make them fit.
                let dash_gap = max_t - dash_length;
                if (dash_gap > 0.0) {
                    let dash_period = dash_length + dash_gap;
                    border_color.a *= dash_alpha(
                        t,
                        dash_period,
                        dash_length,
                        dash_velocity,
                        antialias_threshold);
                }
            }
        }

        // Blend the border on top of the background and then linearly interpolate
        // between the two as we slide inside the background.
        let blended_border = over(background_color, border_color);
        color = mix(background_color, blended_border,
                    saturate(antialias_threshold - inner_sdf));
    }

    return blend_color(color, saturate(antialias_threshold - outer_sdf));
}

// Returns the dash velocity of a corner given the dash velocity of the two
// sides, by returning the slower velocity (larger dashes).
//
// Since 0 is used for dash velocity when the border width is 0 (instead of
// +inf), this returns the other dash velocity in that case.
//
// An alternative to this might be to appropriately interpolate the dash
// velocity around the corner, but that seems overcomplicated.
fn corner_dash_velocity(dv1: f32, dv2: f32) -> f32 {
    if (dv1 == 0.0) {
        return dv2;
    } else if (dv2 == 0.0) {
        return dv1;
    } else {
        return min(dv1, dv2);
    }
}

// Returns alpha used to render antialiased dashes.
// `t` is within the dash when `fmod(t, period) < length`.
fn dash_alpha(t: f32, period: f32, length: f32, dash_velocity: f32, antialias_threshold: f32) -> f32 {
    let half_period = period / 2;
    let half_length = length / 2;
    // Value in [-half_period, half_period].
    // The dash is in [-half_length, half_length].
    let centered = fmod(t + half_period - half_length, period) - half_period;
    // Signed distance for the dash, negative values are inside the dash.
    let signed_distance = abs(centered) - half_length;
    // Antialiased alpha based on the signed distance.
    return saturate(antialias_threshold - signed_distance / dash_velocity);
}

// This approximates distance to the nearest point to a quarter ellipse in a way
// that is sufficient for anti-aliasing when the ellipse is not very eccentric.
// The components of `point` are expected to be positive.
//
// Negative on the outside and positive on the inside.
fn quarter_ellipse_sdf(point: vec2<f32>, radii: vec2<f32>) -> f32 {
    // Scale the space to treat the ellipse like a unit circle.
    let circle_vec = point / radii;
    let unit_circle_sdf = length(circle_vec) - 1.0;
    // Approximate up-scaling of the length by using the average of the radii.
    //
    // TODO: A better solution would be to use the gradient of the implicit
    // function for an ellipse to approximate a scaling factor.
    return unit_circle_sdf * (radii.x + radii.y) * -0.5;
}

// Modulus that has the same sign as `a`.
fn fmod(a: f32, b: f32) -> f32 {
    return a - b * trunc(a / b);
}

// --- shadows --- //

struct Shadow {
    order: u32,
    blur_radius: f32,
    bounds: Bounds,
    corner_radii: Corners,
    content_mask: Bounds,
    color: Hsla,
    transformation: TransformationMatrix,
    // Ours: an inset shadow (`inset` 1) is drawn inside the box it's of (`outer`, with `outer_radii`), where the
    // shadow's own box (`bounds`, moved and shrunk) doesn't cover it.
    inset: u32,
    pad: u32,
    outer: Bounds,
    outer_radii: Corners,
}
@group(1) @binding(0) var<storage, read> b_shadows: array<Shadow>;

struct ShadowVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) shadow_id: u32,
    //TODO: use `clip_distance` once Naga supports it
    @location(3) clip_distances: vec4<f32>,
    @location(4) local: vec2<f32>,
}

// A shadow, drawn as a frame (a triangle strip of ten vertices, round from its top left, each corner outside then
// inside) around where it's surely clear: the box under a shadow outside it, or the middle of an inset one past its
// blur and corners. Most of a large box's shadow is the box, which isn't drawn at all, rather than passed over.
@vertex
fn vs_shadow(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> ShadowVarying {
    let shadow = b_shadows[instance_id];
    let bounds = shadow_bounds(shadow);
    let corner = (vertex_id / 2u) % 4u;
    let outer = vec2<f32>(f32(corner == 1u || corner == 2u), f32(corner >= 2u));
    var hole: Bounds;
    if (shadow.inset != 0u) {
        let radii = shadow.corner_radii;
        let clear = 3.0 * shadow.blur_radius + max(max(radii.top_left, radii.top_right), max(radii.bottom_right, radii.bottom_left)) + 0.5;
        hole = Bounds(shadow.bounds.origin + clear, shadow.bounds.size - 2.0 * clear);
    } else {
        let radii = shadow.outer_radii;
        let clear = max(max(radii.top_left, radii.top_right), max(radii.bottom_right, radii.bottom_left)) + 0.5;
        hole = Bounds(shadow.outer.origin + clear, shadow.outer.size - 2.0 * clear);
    }
    // Where there's no hole inside what's drawn, the inside corners meet in its middle, and the frame is all of it.
    let low = max(hole.origin, bounds.origin);
    let high = min(hole.origin + hole.size, bounds.origin + bounds.size);
    var inner = vec2<f32>(0.5);
    if (all(high > low)) {
        inner = (mix(low, high, outer) - bounds.origin) / bounds.size;
    }
    let unit_vertex = select(inner, outer, vertex_id % 2u == 0u);
    return shadow_varying(unit_vertex, bounds, shadow, instance_id);
}

// A shadow drawn whole, a quad, as its tile is (`fs_shadow_coverage`).
@vertex
fn vs_shadow_whole(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> ShadowVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let shadow = b_shadows[instance_id];
    return shadow_varying(unit_vertex, shadow_bounds(shadow), shadow, instance_id);
}

// Where a shadow's drawn: its box grown by its blur, or an inset one's box it's inside.
fn shadow_bounds(shadow: Shadow) -> Bounds {
    if (shadow.inset != 0u) {
        return shadow.outer;
    }
    let margin = 3.0 * shadow.blur_radius;
    return Bounds(shadow.bounds.origin - vec2<f32>(margin), shadow.bounds.size + 2.0 * vec2<f32>(margin));
}

fn shadow_varying(unit_vertex: vec2<f32>, bounds: Bounds, shadow: Shadow, instance_id: u32) -> ShadowVarying {
    var out = ShadowVarying();
    out.position = to_device_position_transformed(unit_vertex, bounds, shadow.transformation);
    out.local = unit_vertex * vec2<f32>(bounds.size) + bounds.origin;
    out.color = hsla_to_rgba(shadow.color);
    out.shadow_id = instance_id;
    out.clip_distances = distance_from_clip_rect_transformed(unit_vertex, bounds, shadow.content_mask, shadow.transformation);
    return out;
}

@fragment
fn fs_shadow(input: ShadowVarying) -> @location(0) vec4<f32> {
    // Alpha clip first, since we don't have `clip_distance`.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let shadow = b_shadows[input.shadow_id];
    let outside = quad_sdf(input.local, shadow.outer, shadow.outer_radii);
    if (shadow.inset != 0u) {
        // Inside its box, where its own box isn't.
        if (outside >= 0.5) {
            return vec4<f32>(0.0);
        }
        return blend_color_dithered(input.color, saturate(0.5 - outside) * (1.0 - shadow_coverage(input.local, shadow)), input.position.xy);
    }
    // Outside its box, as CSS draws a shadow: a box with no fill of its own, a field's, shows none of it. Most of
    // what a large box's shadow covers is the box, which is passed over before its blur is worked out.
    if (outside <= -0.5) {
        return vec4<f32>(0.0);
    }
    return blend_color_dithered(input.color, shadow_coverage(input.local, shadow) * saturate(0.5 + outside), input.position.xy);
}

// A shadow's tile: how much of each point its box covers, blurred, alone (`tiled` in renderer.rs).
@fragment
fn fs_shadow_coverage(input: ShadowVarying) -> @location(0) vec4<f32> {
    return vec4<f32>(shadow_coverage(input.local, b_shadows[input.shadow_id]));
}

// A shadow drawn from its tile: each corner as the tile has it, and between them the tile's middle, which along an
// edge is the same however far the other edge is. Along an axis no longer than two corners, its tile's box is as long
// as its own, and drawn as it is. Outside its box only, as `fs_shadow` draws one.
@fragment
fn fs_shadow_tile(input: ShadowVarying) -> @location(0) vec4<f32> {
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }
    let shadow = b_shadows[input.shadow_id];
    let outside = quad_sdf(input.local, shadow.outer, shadow.outer_radii);
    if (outside <= -0.5) {
        return vec4<f32>(0.0);
    }
    let radii = shadow.corner_radii;
    let margin = 3.0 * shadow.blur_radius;
    let size = shadow.bounds.size;
    let tile = min(size, vec2<f32>(2.0 * (max(max(radii.top_left, radii.top_right), max(radii.bottom_right, radii.bottom_left)) + margin)));
    let corner = tile / 2.0;
    let along = input.local - shadow.bounds.origin;
    let point = select(select(corner, along - (size - tile), along > size - corner), along, along < corner);
    let uv = (point + margin) / vec2<f32>(textureDimensions(t_sprite));
    let alpha = textureSampleLevel(t_sprite, s_sprite, uv, 0.0).r;
    return blend_color_dithered(input.color, alpha * saturate(0.5 + outside), input.position.xy);
}

// How much of `point` a shadow's box covers, blurred.
fn shadow_coverage(point: vec2<f32>, shadow: Shadow) -> f32 {
    // Not blurred, it's its box, its edge smoothed as a quad's.
    if (shadow.blur_radius <= 0.0) {
        return saturate(0.5 - quad_sdf(point, shadow.bounds, shadow.corner_radii));
    }
    let half_size = shadow.bounds.size / 2.0;
    let center = shadow.bounds.origin + half_size;
    let center_to_point = point - center;

    let corner_radius = pick_corner_radius(center_to_point, shadow.corner_radii);

    // The signal is only non-zero in a limited range, so don't waste samples
    let low = center_to_point.y - half_size.y;
    let high = center_to_point.y + half_size.y;
    let start = clamp(-3.0 * shadow.blur_radius, low, high);
    let end = clamp(3.0 * shadow.blur_radius, low, high);

    // Accumulate samples (we can get away with surprisingly few samples)
    let step = (end - start) / 4.0;
    var y = start + step * 0.5;
    var alpha = 0.0;
    for (var i = 0; i < 4; i += 1) {
        let blur = blur_along_x(center_to_point.x, center_to_point.y - y,
            shadow.blur_radius, corner_radius, half_size);
        alpha +=  blur * gaussian(y, shadow.blur_radius) * step;
        y += step;
    }
    return alpha;
}

// --- underlines --- //

struct Underline {
    order: u32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    color: Hsla,
    thickness: f32,
    wavy: u32,
}
@group(1) @binding(0) var<storage, read> b_underlines: array<Underline>;

struct UnderlineVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) underline_id: u32,
    //TODO: use `clip_distance` once Naga supports it
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_underline(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> UnderlineVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let underline = b_underlines[instance_id];

    var out = UnderlineVarying();
    out.position = to_device_position(unit_vertex, underline.bounds);
    out.color = hsla_to_rgba(underline.color);
    out.underline_id = instance_id;
    out.clip_distances = distance_from_clip_rect(unit_vertex, underline.bounds, underline.content_mask);
    return out;
}

@fragment
fn fs_underline(input: UnderlineVarying) -> @location(0) vec4<f32> {
    const WAVE_FREQUENCY: f32 = 2.0;
    const WAVE_HEIGHT_RATIO: f32 = 0.8;

    // Alpha clip first, since we don't have `clip_distance`.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let underline = b_underlines[input.underline_id];
    if ((underline.wavy & 0xFFu) == 0u)
    {
        return blend_color(input.color, input.color.a);
    }

    let half_thickness = underline.thickness * 0.5;

    let st = (input.position.xy + globals.origin - underline.bounds.origin) / underline.bounds.size.y - vec2<f32>(0.0, 0.5);
    let frequency = M_PI_F * WAVE_FREQUENCY * underline.thickness / underline.bounds.size.y;
    let amplitude = (underline.thickness * WAVE_HEIGHT_RATIO) / underline.bounds.size.y;

    let sine = sin(st.x * frequency) * amplitude;
    let dSine = cos(st.x * frequency) * amplitude * frequency;
    let distance = (st.y - sine) / sqrt(1.0 + dSine * dSine);
    let distance_in_pixels = distance * underline.bounds.size.y;
    let distance_from_top_border = distance_in_pixels - half_thickness;
    let distance_from_bottom_border = distance_in_pixels + half_thickness;
    let alpha = saturate(0.5 - max(-distance_from_bottom_border, distance_from_top_border));
    return blend_color(input.color, alpha * input.color.a);
}

// --- monochrome sprites --- //

struct MonochromeSprite {
    order: u32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    color: Hsla,
    tile: AtlasTile,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_mono_sprites: array<MonochromeSprite>;

struct MonoSpriteVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) tile_position: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(3) clip_distances: vec4<f32>,
}

@vertex
fn vs_mono_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> MonoSpriteVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_mono_sprites[instance_id];

    var out = MonoSpriteVarying();
    out.position = to_device_position_transformed(unit_vertex, sprite.bounds, sprite.transformation);

    out.tile_position = to_tile_position(unit_vertex, sprite.tile);
    out.color = hsla_to_rgba(sprite.color);
    out.clip_distances = distance_from_clip_rect_transformed(unit_vertex, sprite.bounds, sprite.content_mask, sprite.transformation);
    return out;
}

@fragment
fn fs_mono_sprite(input: MonoSpriteVarying) -> @location(0) vec4<f32> {
    let sample = textureSample(t_sprite, s_sprite, input.tile_position).r;
    let alpha_corrected = apply_contrast_and_gamma_correction(sample, input.color.rgb, gamma_params.grayscale_enhanced_contrast, gamma_params.gamma_ratios);

    // Alpha clip after using the derivatives.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    // convert to srgb space as the rest of the code (output swapchain) expects that
    return blend_color(input.color, alpha_corrected);
}

// --- polychrome sprites --- //

struct PolychromeSprite {
    order: u32,
    pad: u32,
    grayscale: u32,
    opacity: f32,
    bounds: Bounds,
    content_mask: Bounds,
    corner_radii: Corners,
    tile: AtlasTile,
    transformation: TransformationMatrix,
}
@group(1) @binding(0) var<storage, read> b_poly_sprites: array<PolychromeSprite>;

struct PolySpriteVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) tile_position: vec2<f32>,
    @location(1) @interpolate(flat) sprite_id: u32,
    @location(3) clip_distances: vec4<f32>,
    @location(4) local: vec2<f32>,
}

@vertex
fn vs_poly_sprite(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> PolySpriteVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let sprite = b_poly_sprites[instance_id];

    var out = PolySpriteVarying();
    out.position = to_device_position_transformed(unit_vertex, sprite.bounds, sprite.transformation);
    out.local = unit_vertex * vec2<f32>(sprite.bounds.size) + sprite.bounds.origin;
    out.tile_position = to_tile_position(unit_vertex, sprite.tile);
    out.sprite_id = instance_id;
    out.clip_distances = distance_from_clip_rect_transformed(unit_vertex, sprite.bounds, sprite.content_mask, sprite.transformation);
    return out;
}

// What tile `tile` shows at `uv` (in its atlas texture, 0 to 1), filtered as a mipmapped texture is, trilinear: its
// texels as they are where it's drawn as large as they are or larger, and where it's drawn smaller, blended toward its
// next mip level, each texel of which is the average of four of its own, which one bilinear tap where the four meet
// gives. A picture is decoded at most twice as large as it's shown (`view::paint::decode`), so that level is all it
// needs; with one tap of its own, it'd skip texels, and a picture drawn smaller would shimmer as it moved.
fn sample_mipmapped(uv: vec2<f32>, tile: AtlasTile) -> vec4<f32> {
    let atlas_size = vec2<f32>(textureDimensions(t_sprite, 0));
    let texel = uv * atlas_size;
    let toward = saturate(log2(max(length(dpdx(texel)), length(dpdy(texel)))));
    let level0 = textureSampleLevel(t_sprite, s_sprite, uv, 0.0);
    // Drawn at its size, give or take the float's rounding, it's its texels.
    if (toward < 1.0 / 256.0) {
        return level0;
    }
    // Its texels are paired from the tile's first inside its clear pixel all round (`pictures::tiled`), and each tap
    // stays where its four texels are the tile's.
    let first = vec2<f32>(tile.bounds.origin) + 1.0;
    let last = vec2<f32>(tile.bounds.origin + tile.bounds.size) - 1.0;
    let at = (texel - first) / 2.0 - 0.5;
    let base = floor(at);
    let part = at - base;
    let corner = first + 2.0 * base + 1.0;
    let t00 = textureSampleLevel(t_sprite, s_sprite, clamp(corner, first, last) / atlas_size, 0.0);
    let t10 = textureSampleLevel(t_sprite, s_sprite, clamp(corner + vec2<f32>(2.0, 0.0), first, last) / atlas_size, 0.0);
    let t01 = textureSampleLevel(t_sprite, s_sprite, clamp(corner + vec2<f32>(0.0, 2.0), first, last) / atlas_size, 0.0);
    let t11 = textureSampleLevel(t_sprite, s_sprite, clamp(corner + 2.0, first, last) / atlas_size, 0.0);
    let level1 = mix(mix(t00, t10, part.x), mix(t01, t11, part.x), part.y);
    return mix(level0, level1, toward);
}

@fragment
fn fs_poly_sprite(input: PolySpriteVarying) -> @location(0) vec4<f32> {
    let sprite = b_poly_sprites[input.sprite_id];
    let sample = sample_mipmapped(input.tile_position, sprite.tile);
    // Alpha clip after using the derivatives.
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }

    let distance = quad_sdf(input.local, sprite.bounds, sprite.corner_radii);

    var color = sample;
    if (sprite.grayscale != 0u) {
        let grayscale = dot(color.rgb, GRAYSCALE_FACTORS);
        color = vec4<f32>(vec3<f32>(grayscale), sample.a);
    }
    return blend_color(color, sprite.opacity * saturate(0.5 - distance));
}


// --- backdrops (ours: what's behind a node, blurred and saturated, as Apple's materials draw it) --- //

struct Backdrop {
    order: u32,
    opacity: f32,
    blur: f32,
    saturation: f32,
    bounds: Bounds,
    content_mask: Bounds,
    corner_radii: Corners,
}
@group(1) @binding(0) var<storage, read> b_backdrops: array<Backdrop>;

struct BlurParams {
    // One texel along the way this pass blurs, in the source's coordinates (0 to 1); shrinking, one texel each way.
    step: vec2<f32>,
    // The Gaussian's sigma, in the source's texels.
    sigma: f32,
    // With no `sigma`, how many of the source's texels each way one of the target's is.
    shrink: f32,
}
@group(1) @binding(0) var<uniform> blur: BlurParams;

struct BlurVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// A triangle strip over the whole target.
@vertex
fn vs_blur(@builtin(vertex_index) vertex_id: u32) -> BlurVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    var out = BlurVarying();
    out.position = vec4<f32>(unit_vertex.x * 2.0 - 1.0, 1.0 - unit_vertex.y * 2.0, 0.0, 1.0);
    out.uv = unit_vertex;
    return out;
}

// One way of a separable Gaussian: two taps a sample, between texels, for linear filtering to weigh. With no
// `sigma`, it's the average of the `shrink` × `shrink` texels under each: a tap at the middle of each 2 × 2 of them,
// which linear filtering averages, as halving the source that many times over did.
@fragment
fn fs_blur(input: BlurVarying) -> @location(0) vec4<f32> {
    if (blur.sigma <= 0.0) {
        let taps = max(i32(blur.shrink) / 2, 1);
        var sum = vec4<f32>(0.0);
        for (var y = 0; y < taps; y++) {
            for (var x = 0; x < taps; x++) {
                let offset = vec2<f32>(f32(2 * x + 1 - taps), f32(2 * y + 1 - taps));
                sum += textureSample(t_sprite, s_sprite, input.uv + blur.step * offset);
            }
        }
        return sum / f32(taps * taps);
    }
    let sigma = max(blur.sigma, 0.5);
    let reach = min(i32(ceil(sigma * 3.0)), 48);
    var sum = textureSample(t_sprite, s_sprite, input.uv);
    var total = 1.0;
    for (var i = 1; i <= reach; i += 2) {
        let a = f32(i);
        let b = a + 1.0;
        let wa = exp(-0.5 * a * a / (sigma * sigma));
        let wb = exp(-0.5 * b * b / (sigma * sigma));
        let w = wa + wb;
        let offset = (a * wa + b * wb) / w;
        sum += (textureSample(t_sprite, s_sprite, input.uv + blur.step * offset) + textureSample(t_sprite, s_sprite, input.uv - blur.step * offset)) * w;
        total += 2.0 * w;
    }
    return sum / total;
}

struct BackdropVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) backdrop_id: u32,
    @location(1) clip_distances: vec4<f32>,
}

@vertex
fn vs_backdrop(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> BackdropVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let backdrop = b_backdrops[instance_id];
    var out = BackdropVarying();
    out.position = to_device_position(unit_vertex, backdrop.bounds);
    out.backdrop_id = instance_id;
    out.clip_distances = distance_from_clip_rect(unit_vertex, backdrop.bounds, backdrop.content_mask);
    return out;
}

@fragment
fn fs_backdrop(input: BackdropVarying) -> @location(0) vec4<f32> {
    let uv = input.position.xy / globals.viewport_size;
    let sample = textureSample(t_sprite, s_sprite, uv);
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }
    let backdrop = b_backdrops[input.backdrop_id];
    let distance = quad_sdf(input.position.xy + globals.origin, backdrop.bounds, backdrop.corner_radii);
    // Saturated as CSS's `saturate()`: away from its luminance, by the factor.
    let luminance = dot(sample.rgb, GRAYSCALE_FACTORS);
    let color = vec4<f32>(clamp(mix(vec3<f32>(luminance), sample.rgb, backdrop.saturation), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
    return blend_color_dithered(color, saturate(0.5 - distance) * backdrop.opacity, input.position.xy);
}

// --- rasters (ours: a layer drawn once into a texture of its own, and drawn as that, as a compositor draws one) --- //

struct Raster {
    order: u32,
    raster: u32,
    opacity: f32,
    pad: u32,
    bounds: Bounds,
    content_mask: Bounds,
    transformation: TransformationMatrix,
    slot: Bounds,
}
@group(1) @binding(0) var<storage, read> b_rasters: array<Raster>;

struct RasterVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) raster_id: u32,
    @location(2) clip_distances: vec4<f32>,
}

@vertex
fn vs_raster(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> RasterVarying {
    let unit_vertex = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
    let raster = b_rasters[instance_id];
    var out = RasterVarying();
    out.position = to_device_position_transformed(unit_vertex, raster.bounds, raster.transformation);
    out.uv = unit_vertex;
    out.raster_id = instance_id;
    out.clip_distances = distance_from_clip_rect_transformed(unit_vertex, raster.bounds, raster.content_mask, raster.transformation);
    return out;
}

// Its texture holds what was drawn over nothing, so each texel's colour is already multiplied by its alpha: it's drawn
// over what's there as it is, as opaque as the raster is (the pipeline blends premultiplied, whatever the target's).
// It's read from its slot in its texture, held to its slot's edge texels as a texture of its own is by its sampler's
// clamp to its edge, so what's beside it in the texture never bleeds in.
@fragment
fn fs_raster(input: RasterVarying) -> @location(0) vec4<f32> {
    let slot = b_rasters[input.raster_id].slot;
    let at = slot.origin + clamp(input.uv * slot.size, vec2<f32>(0.5), slot.size - vec2<f32>(0.5));
    let texel = textureSample(t_sprite, s_sprite, at / vec2<f32>(textureDimensions(t_sprite)));
    if (any(input.clip_distances < vec4<f32>(0.0))) {
        return vec4<f32>(0.0);
    }
    return texel * b_rasters[input.raster_id].opacity;
}
