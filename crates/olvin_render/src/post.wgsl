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

// ---- FXAA 3.11 (quality preset 12): edge detection on luma, a search along
// the edge, and a sub-pixel shift. Input: the tonemapped frame (params.output.x
// = 1: already sRGB-encoded; otherwise linear, so luma is taken after a
// square root, a cheap stand-in for the encode).

const FXAA_EDGE_THRESHOLD: f32 = 0.125;
const FXAA_EDGE_THRESHOLD_MIN: f32 = 0.0312;
const FXAA_SUBPIX: f32 = 0.75;
const FXAA_STEPS: i32 = 12;

fn fxaa_luma(uv: vec2<f32>) -> f32 {
    let c = tap(source, uv);
    let l = dot(c, vec3<f32>(0.299, 0.587, 0.114));
    return select(sqrt(max(l, 0.0)), l, params.output.x > 0.5);
}

fn fxaa_step(i: i32) -> f32 {
    // 1, 1, 1, 1, 1, 1.5, 2, 2, 2, 2, 4, 8
    if (i < 5) { return 1.0; }
    if (i == 5) { return 1.5; }
    if (i < 10) { return 2.0; }
    if (i == 10) { return 4.0; }
    return 8.0;
}

@fragment
fn fs_fxaa(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let rcp = 1.0 / vec2<f32>(textureDimensions(source));
    let m = tap(source, uv);
    let luma_m = fxaa_luma(uv);
    var luma_n = fxaa_luma(uv + vec2<f32>(0.0, -rcp.y));
    var luma_s = fxaa_luma(uv + vec2<f32>(0.0, rcp.y));
    let luma_w = fxaa_luma(uv + vec2<f32>(-rcp.x, 0.0));
    let luma_e = fxaa_luma(uv + vec2<f32>(rcp.x, 0.0));
    let range_max = max(max(luma_n, luma_s), max(max(luma_w, luma_e), luma_m));
    let range_min = min(min(luma_n, luma_s), min(min(luma_w, luma_e), luma_m));
    let range = range_max - range_min;
    if (range < max(FXAA_EDGE_THRESHOLD_MIN, range_max * FXAA_EDGE_THRESHOLD)) {
        return vec4<f32>(m, 1.0);
    }
    let luma_nw = fxaa_luma(uv + vec2<f32>(-rcp.x, -rcp.y));
    let luma_ne = fxaa_luma(uv + vec2<f32>(rcp.x, -rcp.y));
    let luma_sw = fxaa_luma(uv + vec2<f32>(-rcp.x, rcp.y));
    let luma_se = fxaa_luma(uv + vec2<f32>(rcp.x, rcp.y));

    let luma_ns = luma_n + luma_s;
    let luma_we = luma_w + luma_e;
    let luma_nwsw = luma_nw + luma_sw;
    let luma_nese = luma_ne + luma_se;
    let edge_horz = abs(-2.0 * luma_w + luma_nwsw)
        + abs(-2.0 * luma_m + luma_ns) * 2.0 + abs(-2.0 * luma_e + luma_nese);
    let edge_vert = abs(-2.0 * luma_s + (luma_sw + luma_se))
        + abs(-2.0 * luma_m + luma_we) * 2.0 + abs(-2.0 * luma_n + (luma_nw + luma_ne));
    let horz_span = edge_horz >= edge_vert;

    var length_sign = rcp.x;
    if (!horz_span) {
        luma_n = luma_w;
        luma_s = luma_e;
    } else {
        length_sign = rcp.y;
    }
    let gradient_n = luma_n - luma_m;
    let gradient_s = luma_s - luma_m;
    var luma_nn = luma_n + luma_m;
    let luma_ss = luma_s + luma_m;
    let pair_n = abs(gradient_n) >= abs(gradient_s);
    let gradient = max(abs(gradient_n), abs(gradient_s));
    if (pair_n) {
        length_sign = -length_sign;
    }
    // Sub-pixel aliasing: how far the pixel is from the average of its neighbourhood.
    var subpix = (luma_ns + luma_we) * 2.0 + (luma_nwsw + luma_nese);
    subpix = subpix * (1.0 / 12.0) - luma_m;
    var subpix_b = clamp(abs(subpix) / range, 0.0, 1.0);
    subpix_b = (-2.0 * subpix_b + 3.0) * subpix_b * subpix_b;
    let subpix_h = subpix_b * subpix_b * FXAA_SUBPIX;

    var pos_b = uv;
    var off_np = vec2<f32>(0.0, rcp.y);
    if (!horz_span) {
        pos_b.x += length_sign * 0.5;
        off_np = vec2<f32>(rcp.x, 0.0);
    } else {
        pos_b.y += length_sign * 0.5;
    }
    var pos_n = pos_b - off_np;
    var pos_p = pos_b + off_np;
    if (!pair_n) {
        luma_nn = luma_ss;
    }
    let gradient_scaled = gradient * 0.25;
    let luma_mm = luma_m - luma_nn * 0.5;
    let luma_mm_neg = luma_mm < 0.0;
    var luma_end_n = fxaa_luma(pos_n) - luma_nn * 0.5;
    var luma_end_p = fxaa_luma(pos_p) - luma_nn * 0.5;
    var done_n = abs(luma_end_n) >= gradient_scaled;
    var done_p = abs(luma_end_p) >= gradient_scaled;
    for (var i = 1; i < FXAA_STEPS; i++) {
        if (done_n && done_p) {
            break;
        }
        let step = fxaa_step(i);
        if (!done_n) {
            pos_n -= off_np * step;
            luma_end_n = fxaa_luma(pos_n) - luma_nn * 0.5;
            done_n = abs(luma_end_n) >= gradient_scaled;
        }
        if (!done_p) {
            pos_p += off_np * step;
            luma_end_p = fxaa_luma(pos_p) - luma_nn * 0.5;
            done_p = abs(luma_end_p) >= gradient_scaled;
        }
    }
    var dst_n = uv.y - pos_n.y;
    var dst_p = pos_p.y - uv.y;
    if (!horz_span) {
        dst_n = uv.x - pos_n.x;
        dst_p = pos_p.x - uv.x;
    }
    let direction_n = dst_n < dst_p;
    let luma_end = select(luma_end_p, luma_end_n, direction_n);
    let span = dst_p + dst_n;
    let dst_m = select(dst_p, dst_n, direction_n);
    let pixel_offset = -dst_m / span + 0.5;
    // The end of the span must lie on the other side of the edge than this pixel.
    let good_span = (luma_end < 0.0) != luma_mm_neg;
    let offset = max(select(0.0, pixel_offset, good_span), subpix_h);
    var pos_m = uv;
    if (!horz_span) {
        pos_m.x += offset * length_sign;
    } else {
        pos_m.y += offset * length_sign;
    }
    return vec4<f32>(tap(source, pos_m), 1.0);
}
