// ---- Engine: post-processing chain on the HDR frame. Fullscreen triangle
// passes: ambient occlusion (GTAO + blur), bright-pass + downsample,
// downsample, separable blur, composite (AO, bloom, exposure, tonemapping,
// sRGB when the target doesn't do it), FXAA.

struct Params {
    // x: bloom threshold; y: bloom intensity; z: exposure; w = 1: ACES.
    bloom_exposure: vec4<f32>,
    // x = 1: encode sRGB here; y = 1: tonemapping is None (clamp).
    output: vec4<f32>,
    // xy: blur step in uv (one texel along the blur axis).
    blur: vec4<f32>,
    // The camera the scene was drawn with (world <-> clip, depth 0..1).
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // x: AO radius (m); y: strength (exponent); z: bias; w = 1: AO on.
    ao: vec4<f32>,
    // x: LUT strength; y: saturation; z: contrast; w = 1: grading on.
    grade: vec4<f32>,
    // rgb: lift; rgb: gamma, w = 1: a LUT is bound; rgb: gain.
    lift: vec4<f32>,
    gamma: vec4<f32>,
    gain: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
// The scene's depth, and the blurred AO (a stand-in while it is being made).
@group(1) @binding(0) var depth_tex: texture_2d<f32>; // a depth texture read as unfilterable float (r)
@group(1) @binding(1) var ao_tex: texture_2d<f32>;
// The colour grading LUT strip (size slices of size x size, blue picks the slice).
@group(1) @binding(2) var lut_tex: texture_2d<f32>;

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

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
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

// ---- Colour grading, on sRGB-encoded colour.

// Trilinear lookup in the LUT strip: blue picks (and blends) two slices, red
// and green are filtered by the sampler inside a slice.
fn lut_lookup(c: vec3<f32>) -> vec3<f32> {
    let dim = vec2<f32>(textureDimensions(lut_tex));
    let n = dim.y;
    let b = clamp(c.b, 0.0, 1.0) * (n - 1.0);
    let b0 = floor(b);
    let b1 = min(b0 + 1.0, n - 1.0);
    let v = (clamp(c.g, 0.0, 1.0) * (n - 1.0) + 0.5) / n;
    let u = clamp(c.r, 0.0, 1.0) * (n - 1.0) + 0.5;
    let s0 = textureSampleLevel(lut_tex, linear_sampler, vec2<f32>((b0 * n + u) / dim.x, v), 0.0).rgb;
    let s1 = textureSampleLevel(lut_tex, linear_sampler, vec2<f32>((b1 * n + u) / dim.x, v), 0.0).rgb;
    return mix(s0, s1, b - b0);
}

fn grade(encoded: vec3<f32>) -> vec3<f32> {
    var c = max(encoded * params.gain.rgb + params.lift.rgb, vec3<f32>(0.0));
    c = pow(c, 1.0 / params.gamma.rgb);
    c = (c - 0.5) * params.grade.z + 0.5;
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = mix(vec3<f32>(luma), c, params.grade.y);
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    if (params.gamma.w > 0.5) {
        c = mix(c, lut_lookup(c), params.grade.x);
    }
    return c;
}

// ---- Ambient occlusion: GTAO (Jimenez et al. 2016) in world space. Two
// slices per pixel, five steps per side, a 4x4 ordered-dither pattern for the
// angles and step offsets, removed by a depth-aware 4x4 blur.

const AO_SLICES: i32 = 2;
const AO_STEPS: i32 = 5;
const PI: f32 = 3.14159265;

fn load_depth(p: vec2<i32>) -> f32 {
    let dim = vec2<i32>(textureDimensions(depth_tex));
    let q = clamp(p, vec2<i32>(0), dim - vec2<i32>(1));
    return textureLoad(depth_tex, q, 0).r;
}

// World position of the pixel centre `pix` (pixels, y down) at `depth`.
fn unproject(pix: vec2<f32>, depth: f32, size: vec2<f32>) -> vec3<f32> {
    let ndc = vec4<f32>(pix.x / size.x * 2.0 - 1.0, 1.0 - pix.y / size.y * 2.0, depth, 1.0);
    let w = params.inv_view_proj * ndc;
    return w.xyz / w.w;
}

fn to_pixels(p: vec3<f32>, size: vec2<f32>) -> vec2<f32> {
    let c = params.view_proj * vec4<f32>(p, 1.0);
    let ndc = c.xy / max(abs(c.w), 1e-5);
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * size;
}

fn bayer(p: vec2<i32>) -> f32 {
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[(p.y & 3) * 4 + (p.x & 3)] + 0.5) / 16.0;
}

// Normal from the closer neighbour on each axis (no smearing over edges),
// facing the camera.
fn surface_normal(pi: vec2<i32>, pix: vec2<f32>, size: vec2<f32>, p: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let pl = unproject(pix + vec2<f32>(-1.0, 0.0), load_depth(pi + vec2<i32>(-1, 0)), size) - p;
    let pr = unproject(pix + vec2<f32>(1.0, 0.0), load_depth(pi + vec2<i32>(1, 0)), size) - p;
    let pu = unproject(pix + vec2<f32>(0.0, -1.0), load_depth(pi + vec2<i32>(0, -1)), size) - p;
    let pd = unproject(pix + vec2<f32>(0.0, 1.0), load_depth(pi + vec2<i32>(0, 1)), size) - p;
    let dx = select(-pl, pr, dot(pr, pr) < dot(pl, pl));
    let dy = select(-pu, pd, dot(pd, pd) < dot(pu, pu));
    let n = normalize(cross(dy, dx));
    return select(-n, n, dot(n, v) >= 0.0);
}

@fragment
fn fs_ao(in: VertexOut) -> @location(0) vec4<f32> {
    let pi = vec2<i32>(in.clip.xy);
    let size = vec2<f32>(textureDimensions(depth_tex));
    let d = load_depth(pi);
    if (d >= 1.0) {
        return vec4<f32>(1.0);
    }
    let radius = params.ao.x;
    let pix = vec2<f32>(pi) + 0.5;
    let p = unproject(pix, d, size);
    let v = normalize(unproject(pix, 0.0, size) - p);

    let n = surface_normal(pi, pix, size, p, v);

    // Screen-space size of `radius` at this depth.
    let side = normalize(cross(v, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(v.y) > 0.95)));
    let r_pix = clamp(distance(to_pixels(p + side * radius, size), pix), 2.0, size.y * 0.3);

    let noise_slice = bayer(pi);
    let noise_step = bayer(pi + vec2<i32>(2, 1));
    let falloff_from = radius * 0.385;
    let falloff_len = radius * 0.615;
    var visibility = 0.0;
    for (var s = 0; s < AO_SLICES; s++) {
        let phi = (f32(s) + noise_slice) / f32(AO_SLICES) * PI;
        let dir = vec2<f32>(cos(phi), sin(phi));
        // The slice's direction in the plane facing the camera, and its normal.
        let a = unproject(pix + dir, d, size) - p;
        let ortho = normalize(a - v * dot(a, v));
        let axis = normalize(cross(ortho, v));
        let proj_n = n - axis * dot(n, axis);
        let proj_len = length(proj_n);
        if (proj_len < 1e-4) {
            visibility += 1.0;
            continue;
        }
        let sign_n = sign(dot(ortho, proj_n));
        let cos_n = clamp(dot(proj_n, v) / proj_len, -1.0, 1.0);
        let ang_n = sign_n * acos(cos_n);
        // Horizon cosines on the +dir side (0) and the -dir side (1); the
        // start is the horizon of the unoccluded hemisphere.
        let low0 = cos(ang_n + 0.5 * PI);
        let low1 = cos(ang_n - 0.5 * PI);
        var h0 = low0;
        var h1 = low1;
        for (var i = 0; i < AO_STEPS; i++) {
            var t = (f32(i) + noise_step) / f32(AO_STEPS);
            t = t * t;
            let off = dir * (t * r_pix + 1.0);
            for (var side_i = 0; side_i < 2; side_i++) {
                let o = select(-off, off, side_i == 0);
                let sp = vec2<i32>(floor(pix + o));
                let sd = load_depth(sp);
                if (sd >= 1.0) {
                    continue;
                }
                let delta = unproject(vec2<f32>(sp) + 0.5, sd, size) - p;
                let dist = max(length(delta), 1e-4);
                var c = dot(delta, v) / dist - params.ao.z;
                let low = select(low1, low0, side_i == 0);
                c = mix(c, low, clamp((dist - falloff_from) / falloff_len, 0.0, 1.0));
                if (side_i == 0) {
                    h0 = max(h0, c);
                } else {
                    h1 = max(h1, c);
                }
            }
        }
        let a0 = ang_n + clamp(acos(clamp(h0, -1.0, 1.0)) - ang_n, -0.5 * PI, 0.5 * PI);
        let a1 = ang_n + clamp(-acos(clamp(h1, -1.0, 1.0)) - ang_n, -0.5 * PI, 0.5 * PI);
        let arc0 = (cos_n + 2.0 * a0 * sin(ang_n) - cos(2.0 * a0 - ang_n)) * 0.25;
        let arc1 = (cos_n + 2.0 * a1 * sin(ang_n) - cos(2.0 * a1 - ang_n)) * 0.25;
        visibility += proj_len * (arc0 + arc1);
    }
    return vec4<f32>(clamp(visibility / f32(AO_SLICES), 0.0, 1.0));
}

// 4x4 blur of the raw AO (`source`) that stays on its own surface: samples
// off this pixel's tangent plane are skipped. Applies the
// strength.
@fragment
fn fs_ao_blur(in: VertexOut) -> @location(0) vec4<f32> {
    let pi = vec2<i32>(in.clip.xy);
    let size = vec2<f32>(textureDimensions(depth_tex));
    let dim = vec2<i32>(textureDimensions(depth_tex));
    let d = load_depth(pi);
    if (d >= 1.0) {
        return vec4<f32>(1.0);
    }
    let pix = vec2<f32>(pi) + 0.5;
    let p = unproject(pix, d, size);
    let n = surface_normal(pi, pix, size, p, normalize(unproject(pix, 0.0, size) - p));
    let thr = max(params.ao.x * 0.15, 0.03);
    var sum = 0.0;
    var weight = 0.0;
    for (var y = -2; y < 2; y++) {
        for (var x = -2; x < 2; x++) {
            let q = clamp(pi + vec2<i32>(x, y), vec2<i32>(0), dim - vec2<i32>(1));
            let sd = load_depth(q);
            if (sd >= 1.0) {
                continue;
            }
            let delta = unproject(vec2<f32>(q) + 0.5, sd, size) - p;
            // Distance from this pixel's tangent plane: slanted surfaces blur.
            let off_plane = dot(delta, n);
            let w = exp(-off_plane * off_plane / (thr * thr));
            sum += w * textureLoad(source, q, 0).r;
            weight += w;
        }
    }
    return vec4<f32>(pow(sum / max(weight, 1e-4), params.ao.y));
}

@fragment
fn fs_composite(in: VertexOut) -> @location(0) vec4<f32> {
    var color = tap(source, in.uv);
    if (params.ao.w > 0.5) {
        // AO is a property of the ambient light: bright (sunlit, emissive)
        // pixels keep less of it.
        let l = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
        let ao = textureSampleLevel(ao_tex, linear_sampler, in.uv, 0.0).r;
        color *= mix(ao, 1.0, smoothstep(0.8, 3.0, l));
    }
    color += tap(bloom, in.uv) * params.bloom_exposure.y;
    color *= params.bloom_exposure.z;
    if (params.bloom_exposure.w > 0.5) {
        color = aces(color);
    }
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    if (params.grade.w > 0.5) {
        // Graded in display colour; hardware sRGB targets get linear back.
        let graded = grade(linear_to_srgb(color));
        if (params.output.x > 0.5) {
            return vec4<f32>(graded, 1.0);
        }
        return vec4<f32>(srgb_to_linear(graded), 1.0);
    }
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
