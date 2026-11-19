// ---- Engine: post-processing chain on the HDR frame. Fullscreen triangle
// passes: bright-pass + downsample, downsample, separable blur, composite
// (bloom, exposure, tonemapping, sRGB when the target doesn't do it).

struct Params {
    // x: bloom threshold; y: bloom intensity; z: exposure; w = 1: ACES.
    bloom_exposure: vec4<f32>,
    // x = 1: encode sRGB here; y = 1: tonemapping is None (clamp).
    output: vec4<f32>,
    // xy: blur step in uv (one texel along the blur axis).
    blur: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_post(@builtin(vertex_index) i: u32) -> VertexOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VertexOut;
    out.clip = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn tap(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(t, linear_sampler, uv, 0.0).rgb;
}

// Four bilinear taps at the corners of the texel block: a 4x4 box.
fn box4(t: texture_2d<f32>, uv: vec2<f32>, texel: vec2<f32>) -> vec3<f32> {
    return 0.25 * (tap(t, uv + texel * vec2<f32>(-1.0, -1.0))
        + tap(t, uv + texel * vec2<f32>(1.0, -1.0))
        + tap(t, uv + texel * vec2<f32>(-1.0, 1.0))
        + tap(t, uv + texel * vec2<f32>(1.0, 1.0)));
}

// Keeps what is brighter than the threshold (soft knee of 0.5).
@fragment
fn fs_bright(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    var c = box4(source, in.uv, texel);
    c = min(c, vec3<f32>(64.0));
    let l = max(c.r, max(c.g, c.b));
    let knee = 0.5 * params.bloom_exposure.x + 1e-4;
    let soft = clamp(l - params.bloom_exposure.x + knee, 0.0, 2.0 * knee);
    let contribution = max(soft * soft / (4.0 * knee), l - params.bloom_exposure.x) / max(l, 1e-4);
    return vec4<f32>(c * contribution, 1.0);
}

@fragment
fn fs_down(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(source));
    return vec4<f32>(box4(source, in.uv, texel), 1.0);
}

// 9-tap Gaussian (5 bilinear taps) along params.blur.xy.
@fragment
fn fs_blur(in: VertexOut) -> @location(0) vec4<f32> {
    let delta = params.blur.xy;
    var c = tap(source, in.uv) * 0.2270270270;
    c += (tap(source, in.uv + delta * 1.3846153846) + tap(source, in.uv - delta * 1.3846153846)) * 0.3162162162;
    c += (tap(source, in.uv + delta * 3.2307692308) + tap(source, in.uv - delta * 3.2307692308)) * 0.0702702703;
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_composite(in: VertexOut) -> @location(0) vec4<f32> {
    var color = tap(source, in.uv);
    color += tap(bloom, in.uv) * params.bloom_exposure.y;
    color *= params.bloom_exposure.z;
    if (params.bloom_exposure.w > 0.5) {
        color = aces(color);
    }
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    if (params.output.x > 0.5) {
        color = linear_to_srgb(color);
    }
    return vec4<f32>(color, 1.0);
}
