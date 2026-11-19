// Our backdrop, as clients/app/strict/src/render/shaders.wgsl draws it (`fs_blur`, `fs_backdrop`), as a pass beside
// Vello's: the frame so far halved until the blur left is a few texels, blurred one way and the other, then drawn
// inside the backdrop's rounded box, saturated. And the blit that puts a Vello compute renderer's frame (straight
// alpha) over ours (premultiplied).

struct Params {
    // One texel along the way this pass blurs, in the source's coordinates (0 to 1).
    step: vec2<f32>,
    // The Gaussian's sigma, in the source's texels; 0 just samples (halving, four texels to one).
    sigma: f32,
    saturation: f32,
    // The backdrop's box and its clip, in the target's pixels.
    bounds: vec4<f32>,
    mask: vec4<f32>,
    radii: vec4<f32>,
    viewport: vec2<f32>,
    pad: vec2<f32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;
@group(0) @binding(2) var<uniform> params: Params;

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_full(@builtin(vertex_index) id: u32) -> Varying {
    let unit = vec2<f32>(f32(id & 1u), 0.5 * f32(id & 2u));
    var out: Varying;
    out.position = vec4<f32>(unit.x * 2.0 - 1.0, 1.0 - unit.y * 2.0, 0.0, 1.0);
    out.uv = unit;
    return out;
}

@fragment
fn fs_blur(input: Varying) -> @location(0) vec4<f32> {
    if (params.sigma <= 0.0) {
        return textureSample(source, linear, input.uv);
    }
    let sigma = max(params.sigma, 0.5);
    let reach = min(i32(ceil(sigma * 3.0)), 48);
    var sum = textureSample(source, linear, input.uv);
    var total = 1.0;
    for (var i = 1; i <= reach; i += 2) {
        let a = f32(i);
        let b = a + 1.0;
        let wa = exp(-0.5 * a * a / (sigma * sigma));
        let wb = exp(-0.5 * b * b / (sigma * sigma));
        let w = wa + wb;
        let offset = (a * wa + b * wb) / w;
        sum += (textureSample(source, linear, input.uv + params.step * offset) + textureSample(source, linear, input.uv - params.step * offset)) * w;
        total += 2.0 * w;
    }
    return sum / total;
}

// The distance from a rounded box's edge, as GPUI's `quad_sdf`.
fn rounded_sdf(point: vec2<f32>, bounds: vec4<f32>, radii: vec4<f32>) -> f32 {
    let half = bounds.zw / 2.0;
    let centre = bounds.xy + half;
    let to_point = point - centre;
    var radius: f32;
    if (to_point.x < 0.0) {
        if (to_point.y < 0.0) { radius = radii.x; } else { radius = radii.w; }
    } else {
        if (to_point.y < 0.0) { radius = radii.y; } else { radius = radii.z; }
    }
    let corner = abs(to_point) - half + radius;
    return length(max(vec2<f32>(0.0), corner)) + min(0.0, max(corner.x, corner.y)) - radius;
}

const GRAYSCALE: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// The blurred frame inside the backdrop's box, saturated as CSS's `saturate()`, premultiplied, over what's there.
@fragment
fn fs_backdrop(input: Varying) -> @location(0) vec4<f32> {
    let point = input.position.xy;
    let mask = params.mask;
    if (point.x < mask.x || point.y < mask.y || point.x > mask.x + mask.z || point.y > mask.y + mask.w) {
        return vec4<f32>(0.0);
    }
    let sample = textureSample(source, linear, point / params.viewport);
    let rgb = select(sample.rgb / sample.a, vec3<f32>(0.0), sample.a <= 0.0);
    let luminance = dot(rgb, GRAYSCALE);
    let colour = clamp(mix(vec3<f32>(luminance), rgb, params.saturation), vec3<f32>(0.0), vec3<f32>(1.0));
    let coverage = saturate(0.5 - rounded_sdf(point, params.bounds, params.radii));
    return vec4<f32>(colour * coverage, coverage);
}

// A straight-alpha frame, as is, for blending over a premultiplied one.
@fragment
fn fs_copy(input: Varying) -> @location(0) vec4<f32> {
    return textureSample(source, linear, input.uv);
}
