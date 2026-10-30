// Upscales the ray-cast image to the output target. Below native resolution a
// Catmull-Rom reconstruction plus clamped contrast-adaptive sharpening keeps
// small, distant block texels crisp where plain bilinear filtering smears them.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return o;
}

// Catmull-Rom with 5 bilinear taps (corner taps of the 4x4 kernel dropped).
fn catmull_rom(uv: vec2<f32>, size: vec2<f32>) -> vec3<f32> {
    let pos = uv * size;
    let center = floor(pos - 0.5) + 0.5;
    let f = pos - center;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (center - 1.0) / size;
    let t3 = (center + 2.0) / size;
    let t12 = (center + w2 / w12) / size;
    var c = textureSampleLevel(src, samp, vec2<f32>(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y)
        + textureSampleLevel(src, samp, vec2<f32>(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y)
        + textureSampleLevel(src, samp, t12, 0.0).rgb * (w12.x * w12.y)
        + textureSampleLevel(src, samp, vec2<f32>(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y)
        + textureSampleLevel(src, samp, vec2<f32>(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
    let weight = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return c / weight;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(src));
    // Native resolution (one source texel per output pixel): plain copy.
    if abs(abs(dpdx(in.uv.x)) * size.x - 1.0) < 0.01 {
        return textureSampleLevel(src, samp, in.uv, 0.0);
    }
    let base = catmull_rom(in.uv, size);
    // Clamped contrast-adaptive sharpening on the source texel neighbourhood.
    let texel = 1.0 / size;
    let n = textureSampleLevel(src, samp, in.uv - vec2<f32>(0.0, texel.y), 0.0).rgb;
    let s = textureSampleLevel(src, samp, in.uv + vec2<f32>(0.0, texel.y), 0.0).rgb;
    let e = textureSampleLevel(src, samp, in.uv + vec2<f32>(texel.x, 0.0), 0.0).rgb;
    let w = textureSampleLevel(src, samp, in.uv - vec2<f32>(texel.x, 0.0), 0.0).rgb;
    let lo = min(min(min(n, s), min(e, w)), base);
    let hi = max(max(max(n, s), max(e, w)), base);
    // Stronger where local contrast is low, weaker at hard edges (no halos).
    let amount = sqrt(clamp(min(lo, 1.0 - hi) / max(hi, vec3<f32>(1e-4)), vec3<f32>(0.0), vec3<f32>(1.0))) * 0.25;
    let sharpened = base + (base * 4.0 - (n + s + e + w)) * amount;
    return vec4<f32>(clamp(sharpened, lo, hi), 1.0);
}
