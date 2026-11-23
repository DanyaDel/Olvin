// ---- Engine: instanced PBR mesh shader template. The surface shader
// (`fn surface`) is pasted in front of this file and the globals.
//
// WebGL2 limits: 9 vertex attributes (4 vertex + 5 instance), 5 varyings,
// group 0 = view globals, group 1 = material (uniform + 4 textures + sampler),
// 7 sampled textures in the fragment stage (shadow atlases for the sun and
// for spot / point lights, sky, 4 material textures).
//
// Lighting, Unity built-in style: hemisphere ambient + roughness-faded sky
// reflection, a sun with cascaded PCF shadows, up to 16 point / spot / extra
// directional lights, GGX + height-correlated Smith + Schlick
// (metallic-roughness), linear and exponential height fog (towards the sky at
// the horizon when a sky is set), exposure + ACES, then sRGB.

struct SurfaceInput {
    world_position: vec3<f32>,
    // World space, normalized, normal map applied.
    normal: vec3<f32>,
    // After uv_scale / uv_offset.
    uv: vec2<f32>,
    // base_color * base_texture * instance colour.
    color: vec4<f32>,
    // Seconds since start.
    time: f32,
};

struct SurfaceOutput {
    // Albedo + alpha.
    color: vec4<f32>,
    // World space.
    normal: vec3<f32>,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
};

struct Material {
    base_color: vec4<f32>,
    // rgb: emissive; w: normal strength.
    emissive: vec4<f32>,
    // x: metallic; y: roughness; z: alpha cutoff.
    params: vec4<f32>,
    // xy: scale; zw: offset.
    uv: vec4<f32>,
    // x: unlit; y: triplanar; z: has a normal map; w: retro look (0: follow
    // the settings, 1: on, 2: off).
    flags: vec4<f32>,
};

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var normal_texture: texture_2d<f32>;
@group(1) @binding(3) var orm_texture: texture_2d<f32>;
@group(1) @binding(4) var emissive_texture: texture_2d<f32>;
@group(1) @binding(5) var material_sampler: sampler;

// The material's sampled ORM and emissive texels for this pixel (sampled in
// uniform control flow before `surface` runs).
var<private> sampled_orm: vec3<f32>;
var<private> sampled_emissive: vec3<f32>;

// The standard material.
fn default_surface(in: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    out.color = in.color;
    out.normal = in.normal;
    out.metallic = material.params.x * sampled_orm.b;
    out.roughness = material.params.y * sampled_orm.g;
    out.emissive = material.emissive.rgb * sampled_emissive;
    return out;
}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
    @location(8) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) tangent: vec4<f32>,
    // Retro only. xy: uv * clip.w (screen-linear uv = this * the fragment's
    // 1 / w); z: the sun's n.l at the vertex.
    @location(5) retro_uv: vec4<f32>,
    // Retro vertex lighting: ambient + local lights.
    @location(6) vlight: vec3<f32>,
};

// Whether the PS1 look applies to this material.
fn retro_on() -> bool {
    let m = material.flags.w;
    let inherited = globals.retro_a.x > 0.5;
    let forced_on = m < 1.5;
    return select(inherited, forced_on, m > 0.5);
}

// A point / spot light seen from `p`: xyz = direction towards it, w = the
// attenuation (range window, inverse square, spot cone; no shadow).
fn local_light(light: Light, p: vec3<f32>) -> vec4<f32> {
    let range = light.position_range.w;
    let dir = light.direction_spot_offset.xyz;
    if (range <= 0.0) {
        return vec4<f32>(-dir, 1.0);
    }
    let to_light = light.position_range.xyz - p;
    let d2 = max(dot(to_light, to_light), 1e-8);
    let l = to_light * inverseSqrt(d2);
    let f = d2 / (range * range);
    let window = clamp(1.0 - f * f, 0.0, 1.0);
    let cone = clamp(dot(-l, dir) * light.color_spot_scale.w + light.direction_spot_offset.w, 0.0, 1.0);
    return vec4<f32>(l, window * window / (d2 + 1.0) * cone * cone);
}

// Gouraud lighting at a vertex: rgb = hemisphere ambient + the local lights'
// diffuse (Lambert, no specular, no shadows); w = the sun's n.l.
fn vertex_light(p: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    var sum = hemisphere(n);
    let count = min(u32(globals.ground.w), MAX_LIGHTS);
    for (var i = 0u; i < count; i++) {
        let light = globals.lights[i];
        let la = local_light(light, p);
        sum += light.color_spot_scale.rgb * la.w * max(dot(n, la.xyz), 0.0);
    }
    return vec4<f32>(sum, max(dot(n, -globals.sun_dir.xyz), 0.0));
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    var world = model * vec4<f32>(in.position, 1.0);
    let retro = retro_on();
    if (retro && globals.retro_a.w > 0.0) {
        // World-space grid: whole vertices jump between cells.
        world = vec4<f32>(floor(world.xyz / globals.retro_a.w + 0.5) * globals.retro_a.w, 1.0);
    }
    var out: VertexOut;
    out.clip = globals.view_proj * world;
    if (retro && globals.retro_a.y > 0.0 && out.clip.w > 1e-4) {
        // Snap x / y / w to the pixel corners of the virtual screen, then
        // multiply w back so depth and perspective stay exact.
        let res = globals.retro_a.yz;
        let px = (out.clip.xy / out.clip.w * 0.5 + 0.5) * res;
        out.clip = vec4<f32>(((round(px) / res) * 2.0 - 1.0) * out.clip.w, out.clip.zw);
    }
    // Exact for uniform scale; non-uniform scale would need the inverse transpose.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.color = in.color;
    out.world = world.xyz;
    out.uv = in.uv;
    out.tangent = vec4<f32>((model * vec4<f32>(in.tangent.xyz, 0.0)).xyz, in.tangent.w);
    out.retro_uv = vec4<f32>(0.0);
    out.vlight = vec3<f32>(0.0);
    if (retro) {
        out.retro_uv = vec4<f32>(in.uv * out.clip.w, 0.0, 0.0);
        if (globals.retro_c.x > 0.5 && material.flags.x < 0.5) {
            let vl = vertex_light(world.xyz, normalize(out.normal));
            out.vlight = vl.rgb;
            out.retro_uv.z = vl.w;
        }
    }
    return out;
}

// GGX / height-correlated Smith / Schlick, Unity's convention (the lambert
// term isn't divided by pi, so a white light of 1 gives the full albedo).
// `l` points towards the light.
fn brdf(albedo: vec3<f32>, metallic: f32, roughness: f32, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>) -> vec3<f32> {
    let nl = dot(n, l);
    if (nl <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(l + v + vec3<f32>(0.0, 1e-5, 0.0));
    let nv = max(dot(n, v), 1e-4);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let a = max(roughness * roughness, 2e-3);
    let a2 = a * a;
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * dd * dd);
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    let vis = 0.5 / max(gv + gl, 1e-5);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
    let spec = d * vis * f * PI;
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    return (kd * albedo + spec) * radiance * nl;
}

// Karis' analytic fit of the split-sum environment BRDF.
fn env_brdf(f0: vec3<f32>, roughness: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// Interleaved gradient noise (Jimenez): a cheap per-pixel rotation.
fn gradient_noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

// The one raw depth read (PCSS's blocker search). `shader::for_device` stubs
// the line out where GLSL can't do it.
fn shadow_depth_at(texel: vec2<i32>) -> f32 {
    return textureLoad(shadow_map, texel, 0);
}

// Reach of PCSS in atlas texels: the blocker search, and the widest filter.
const PCSS_SEARCH: f32 = 16.0;
const PCSS_MAX_RADIUS: f32 = 24.0;

// PCSS for one cascade: a rotated disc finds the average blocker depth, the
// distance to it sets the penumbra (light size x distance), then a disc of
// that radius filters the comparison. `z` is the receiver's depth (0..1);
// returns 1 lit .. 0 shadowed.
fn pcss_filter(i: u32, uv: vec2<f32>, z: f32, pixel: vec2<f32>) -> f32 {
    let texel = globals.params.z;
    let angle = gradient_noise(pixel) * 6.2831853;
    let dims = vec2<f32>(textureDimensions(shadow_map));
    let world_texel = globals.shadow_texel[i];
    let range = globals.shadow_depth[i];
    // A blocker must sit at least a texel in front of the receiver.
    let slack = world_texel / range;
    var blocker = 0.0;
    var hits = 0.0;
    for (var k = 0; k < 16; k++) {
        let r = sqrt((f32(k) + 0.5) / 16.0);
        let a = f32(k) * 2.3999632 + angle;
        let at = uv + vec2<f32>(cos(a), sin(a)) * r * PCSS_SEARCH * texel;
        let d = shadow_depth_at(vec2<i32>(at * dims));
        if (d < z - slack) {
            blocker += d;
            hits += 1.0;
        }
    }
    if (hits < 0.5) {
        return 1.0;
    }
    // Distance to the blockers in metres; penumbra half-width from the light's angular radius.
    let gap = (z - blocker / hits) * range;
    let radius = clamp(gap * globals.shadow_cfg.w / world_texel, 1.0, PCSS_MAX_RADIUS);
    var sum = 0.0;
    for (var k = 0; k < 16; k++) {
        let r = sqrt((f32(k) + 0.5) / 16.0);
        let a = f32(k) * 2.3999632 + angle;
        sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(cos(a), sin(a)) * r * radius * texel, z);
    }
    return sum / 16.0;
}

// One cascade's lookup: x = 1 lit .. 0 shadowed, y = 1 when the point falls
// inside the cascade's tile (with room for the filter), else 0.
fn cascade_shadow(i: u32, p: vec3<f32>, n: vec3<f32>, ndl: f32, pixel: vec2<f32>) -> vec2<f32> {
    // Normal offset: slide the lookup off the surface, more at grazing angles.
    let offset_p = p + n * globals.shadow_offsets[i] * (1.0 - 0.5 * ndl);
    let c = globals.shadow_view_proj[i] * vec4<f32>(offset_p, 1.0);
    let ndc = c.xyz / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let texel = globals.params.z;
    let filter_mode = globals.shadow_cfg.y;
    let radius = max(globals.shadow_cfg.z, 0.0) * texel;
    // Keeps the filter inside the tile (the disc reaches 1.5 radii).
    var margin = texel + radius * select(1.0, 1.5, filter_mode > 1.5) * select(0.0, 1.0, filter_mode > 0.5);
    if (filter_mode > 2.5) {
        margin = texel * (PCSS_SEARCH + PCSS_MAX_RADIUS + 1.0);
    }
    // Tile bounds in the atlas.
    let tiled = globals.shadow_cfg.x > 1.5;
    let size = select(1.0, 0.5, tiled);
    let origin = select(vec2<f32>(0.0), vec2<f32>(f32(i % 2u), f32(i / 2u)) * 0.5, tiled);
    let inside = all(uv >= origin + vec2<f32>(margin)) && all(uv <= origin + vec2<f32>(size - margin))
        && ndc.z <= 1.0 && ndc.z >= 0.0;
    var lit = 1.0;
    if (inside) {
        if (filter_mode < 0.5) {
            lit = textureSampleCompareLevel(shadow_map, shadow_sampler, uv, ndc.z);
        } else if (filter_mode < 1.5) {
            var sum = 0.0;
            for (var y = -1; y <= 1; y++) {
                for (var x = -1; x <= 1; x++) {
                    sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * radius, ndc.z);
                }
            }
            lit = sum / 9.0;
        } else if (filter_mode > 2.5) {
            lit = pcss_filter(i, uv, ndc.z, pixel);
        } else {
            // Vogel disc, rotated per pixel so the banding turns into noise.
            let angle = gradient_noise(pixel) * 6.2831853;
            var sum = 0.0;
            for (var k = 0; k < 16; k++) {
                let r = sqrt((f32(k) + 0.5) / 16.0);
                let a = f32(k) * 2.3999632 + angle;
                sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(cos(a), sin(a)) * r * radius * 1.5, ndc.z);
            }
            lit = sum / 16.0;
        }
    }
    return vec2<f32>(lit, select(0.0, 1.0, inside));
}

// 1 = lit, 0 = in the sun's shadow. Picks the cascade by view depth, blends
// into the next one near its far edge, fades out towards the shadow distance.
fn sun_shadow(p: vec3<f32>, n: vec3<f32>, pixel: vec2<f32>) -> f32 {
    if (globals.sun_dir.w < 0.5) {
        return 1.0;
    }
    let depth = dot(globals.view_plane.xyz, p) + globals.view_plane.w;
    let count = u32(globals.shadow_cfg.x);
    let ndl = clamp(dot(n, -globals.sun_dir.xyz), 0.0, 1.0);
    var i = 0u;
    while (i + 1u < count && depth > globals.shadow_splits[i]) {
        i++;
    }
    var s = cascade_shadow(i, p, n, ndl, pixel);
    // A point that doesn't fit its tile (a filter at the edge) tries the next one.
    if (s.y < 0.5 && i + 1u < count) {
        i++;
        s = cascade_shadow(i, p, n, ndl, pixel);
    }
    var lit = s.x;
    if (s.y > 0.5 && i + 1u < count) {
        let start = select(0.0, globals.shadow_splits[i - 1u], i > 0u);
        let band = max((globals.shadow_splits[i] - start) * 0.1, 1e-3);
        let t = clamp((depth - (globals.shadow_splits[i] - band)) / band, 0.0, 1.0);
        if (t > 0.0) {
            let next = cascade_shadow(i + 1u, p, n, ndl, pixel);
            if (next.y > 0.5) {
                lit = mix(lit, next.x, t);
            }
        }
    }
    // Fade out towards the shadow distance instead of a hard edge.
    let fade = clamp((depth - globals.shadow_params.y) / max(globals.params.w - globals.shadow_params.y, 1e-3), 0.0, 1.0);
    lit = mix(lit, 1.0, fade);
    return select(1.0, lit, s.y > 0.5);
}

// A point or spot light's shadow: 1 lit .. 0 shadowed. A point light picks
// the face (a tile of the atlas) by the dominant axis of the direction to the
// fragment; the lookup slides along the normal and towards the light by
// `bias` texels of that tile.
fn local_shadow(light: Light, p: vec3<f32>, n: vec3<f32>, l: vec3<f32>, dist: f32, pixel: vec2<f32>) -> f32 {
    let bias = light.shadow.y * light.shadow.z * dist;
    let ndl = clamp(dot(n, l), 0.0, 1.0);
    let q = p + n * bias * (1.0 - 0.5 * ndl) + l * bias * 0.5;
    var tile = u32(light.shadow.x) - 1u;
    if (light.shadow.w > 0.5) {
        let a = q - light.position_range.xyz;
        let m = abs(a);
        if (m.x >= m.y && m.x >= m.z) {
            tile += select(1u, 0u, a.x > 0.0);
        } else if (m.y >= m.z) {
            tile += select(3u, 2u, a.y > 0.0);
        } else {
            tile += select(5u, 4u, a.z > 0.0);
        }
    }
    let c = globals.local_view_proj[tile] * vec4<f32>(q, 1.0);
    if (c.w <= 0.0) {
        return 1.0;
    }
    let ndc = c.xyz / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let texel = globals.local_cfg.x;
    let radius = max(globals.shadow_cfg.z, 0.0) * texel;
    let filter_mode = globals.shadow_cfg.y;
    let origin = vec2<f32>(f32(tile % 4u), f32(tile / 4u)) * 0.25;
    let margin = texel + radius * select(1.0, 1.5, filter_mode > 1.5) * select(0.0, 1.0, filter_mode > 0.5);
    if (any(uv < origin + vec2<f32>(margin)) || any(uv > origin + vec2<f32>(0.25 - margin)) || ndc.z > 1.0 || ndc.z < 0.0) {
        return 1.0;
    }
    if (filter_mode < 0.5) {
        return textureSampleCompareLevel(local_shadow_map, shadow_sampler, uv, ndc.z);
    }
    if (filter_mode < 1.5) {
        var sum = 0.0;
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                sum += textureSampleCompareLevel(local_shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * radius, ndc.z);
            }
        }
        return sum / 9.0;
    }
    let angle = gradient_noise(pixel) * 6.2831853;
    var sum = 0.0;
    for (var k = 0; k < 16; k++) {
        let r = sqrt((f32(k) + 0.5) / 16.0);
        let a = f32(k) * 2.3999632 + angle;
        sum += textureSampleCompareLevel(local_shadow_map, shadow_sampler, uv + vec2<f32>(cos(a), sin(a)) * r * radius * 1.5, ndc.z);
    }
    return sum / 16.0;
}

// Tangent-space normal (green up) to world space.
fn perturb(n: vec3<f32>, tangent: vec4<f32>, tn: vec3<f32>) -> vec3<f32> {
    let t0 = tangent.xyz - n * dot(n, tangent.xyz);
    if (dot(t0, t0) < 1e-12) {
        return n;
    }
    let t = normalize(t0);
    let b = cross(n, t) * select(1.0, -1.0, tangent.w < 0.0);
    return normalize(t * tn.x + b * tn.y + n * tn.z);
}

fn unpack_normal(texel: vec4<f32>) -> vec3<f32> {
    let n = texel.xyz * 2.0 - 1.0;
    return vec3<f32>(n.xy * material.emissive.w, n.z);
}

// What fog fades into: its own colour, or the sky at the horizon behind the
// fragment (`v` points at the camera).
fn fog_tint(own: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    if (globals.sky_params.x > 0.5) {
        let flat_dir = vec3<f32>(-v.x, 0.0, -v.z);
        let d = select(vec3<f32>(0.0, 0.0, -1.0), normalize(flat_dir), dot(flat_dir, flat_dir) > 1e-8);
        return sky_radiance(d, 0.5);
    }
    return own;
}

// Exponential height fog: density * exp(-falloff * (y - base)) integrated
// along the ray from the eye to `p`, as the fog opacity 1 - exp(-optical depth).
fn height_fog_amount(p: vec3<f32>, v: vec3<f32>, view_depth: f32) -> f32 {
    let density = globals.height_fog.x;
    let falloff = globals.height_fog.y;
    // Orthographic: the eye plane is `view_depth` back along the view direction.
    let eye = select(p + v * view_depth, globals.camera.xyz, globals.camera.w > 0.5);
    let len = length(p - eye);
    let y0 = eye.y - globals.height_fog.z;
    let y1 = p.y - globals.height_fog.z;
    let a0 = exp(clamp(-falloff * y0, -60.0, 60.0));
    let a1 = exp(clamp(-falloff * y1, -60.0, 60.0));
    let k = falloff * (y1 - y0);
    // The mean of exp(-falloff * y) along the ray.
    let mean = select(a0, (a0 - a1) / k, abs(k) > 1e-4);
    return 1.0 - exp(-density * len * mean);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let geo_n = normalize(in.normal);
    let p = in.world;
    let v = normalize(globals.camera.xyz - p * globals.camera.w);
    let retro = retro_on();
    var mesh_uv = in.uv;
    if (retro && globals.retro_b.x > 0.0) {
        // PS1 affine mapping: uv * w interpolated perspective-correctly, times
        // the fragment's 1 / w (`in.clip.w`), is the screen-linear uv. Fades
        // back to perspective-correct for fragments nearer than retro_b.y.
        let depth = 1.0 / max(in.clip.w, 1e-6);
        let k = globals.retro_b.x * smoothstep(0.0, max(globals.retro_b.y, 1e-3), depth);
        mesh_uv = mix(in.uv, in.retro_uv.xy * in.clip.w, k);
    }
    let uv = mesh_uv * material.uv.xy + material.uv.zw;

    // Textures, all in uniform control flow (branches on the material uniform).
    var n = geo_n;
    var base: vec4<f32>;
    var orm: vec4<f32>;
    var emissive: vec4<f32>;
    if (material.flags.y > 0.5) {
        // Triplanar: world projections blended by the normal.
        let w4 = pow(abs(geo_n), vec3<f32>(4.0));
        let w = w4 / max(w4.x + w4.y + w4.z, 1e-5);
        let ux = vec2<f32>(p.z, -p.y) * material.uv.xy + material.uv.zw;
        let uy = vec2<f32>(p.x, p.z) * material.uv.xy + material.uv.zw;
        let uz = vec2<f32>(p.x, -p.y) * material.uv.xy + material.uv.zw;
        base = textureSample(base_texture, material_sampler, ux) * w.x
            + textureSample(base_texture, material_sampler, uy) * w.y
            + textureSample(base_texture, material_sampler, uz) * w.z;
        orm = textureSample(orm_texture, material_sampler, ux) * w.x
            + textureSample(orm_texture, material_sampler, uy) * w.y
            + textureSample(orm_texture, material_sampler, uz) * w.z;
        emissive = textureSample(emissive_texture, material_sampler, ux) * w.x
            + textureSample(emissive_texture, material_sampler, uy) * w.y
            + textureSample(emissive_texture, material_sampler, uz) * w.z;
        if (material.flags.z > 0.5) {
            // Whiteout blend (Golus): each projection's tangent normal swizzled to world.
            let nx = unpack_normal(textureSample(normal_texture, material_sampler, ux));
            let ny = unpack_normal(textureSample(normal_texture, material_sampler, uy));
            let nz = unpack_normal(textureSample(normal_texture, material_sampler, uz));
            let tx = vec3<f32>(nx.xy + geo_n.zy, abs(nx.z) * geo_n.x);
            let ty = vec3<f32>(ny.xy + geo_n.xz, abs(ny.z) * geo_n.y);
            let tz = vec3<f32>(nz.xy + geo_n.xy, abs(nz.z) * geo_n.z);
            n = normalize(tx.zyx * w.x + ty.xzy * w.y + tz.xyz * w.z);
        }
    } else {
        base = textureSample(base_texture, material_sampler, uv);
        orm = textureSample(orm_texture, material_sampler, uv);
        emissive = textureSample(emissive_texture, material_sampler, uv);
        if (material.flags.z > 0.5) {
            n = perturb(geo_n, in.tangent, unpack_normal(textureSample(normal_texture, material_sampler, uv)));
        }
    }
    sampled_orm = orm.rgb;
    sampled_emissive = emissive.rgb;

    var si: SurfaceInput;
    si.world_position = p;
    si.normal = n;
    si.uv = uv;
    si.color = material.base_color * base * in.color;
    si.time = globals.shadow_params.w;
    let s = surface(si);
    if (s.color.a < material.params.z) {
        discard;
    }
    if (retro && globals.retro_c.z > 0.0) {
        // Draw distance: a hard cut (the fog is meant to hide it), optionally
        // dissolving the last tenth with the Bayer pattern.
        let depth = dot(globals.view_plane.xyz, p) + globals.view_plane.w;
        let cut = globals.retro_c.z;
        if (depth > cut || (globals.retro_c.w > 0.5 && (depth - cut * 0.9) / (cut * 0.1) > bayer4(in.clip.xy))) {
            discard;
        }
    }

    var color: vec3<f32>;
    if (material.flags.x > 0.5) {
        color = s.color.rgb + s.emissive;
    } else if (retro && globals.retro_c.x > 0.5) {
        // Gouraud: the vertex colours interpolated, times the albedo.
        var sun = in.retro_uv.z;
        if (globals.retro_c.y > 0.5) {
            sun *= sun_shadow(p, geo_n, in.clip.xy);
        }
        color = s.color.rgb * (in.vlight + globals.sun_color.rgb * sun) + s.emissive;
    } else {
        let sn = normalize(s.normal);
        let albedo = s.color.rgb;
        let metallic = clamp(s.metallic, 0.0, 1.0);
        let roughness = clamp(s.roughness, 0.03, 1.0);
        let ao = sampled_orm.r;
        let nv = max(dot(sn, v), 1e-4);
        let f0 = mix(vec3<f32>(0.04), albedo, metallic);

        // Ambient: hemisphere diffuse + the sky along the reflection, faded
        // towards the normal (a cheap blur) as roughness grows.
        color = albedo * (1.0 - metallic) * hemisphere(sn) * ao;
        let r = reflect(-v, sn);
        let spec_dir = normalize(mix(r, sn, roughness * roughness));
        color += env_brdf(f0, roughness, nv) * sky_radiance(spec_dir, roughness) * ao;

        color += brdf(albedo, metallic, roughness, sn, v, -globals.sun_dir.xyz, globals.sun_color.rgb * sun_shadow(p, geo_n, in.clip.xy));

        let count = min(u32(globals.ground.w), MAX_LIGHTS);
        for (var i = 0u; i < count; i++) {
            let light = globals.lights[i];
            let range = light.position_range.w;
            let dir = light.direction_spot_offset.xyz;
            var l = -dir;
            var atten = 1.0;
            if (range > 0.0) {
                let to_light = light.position_range.xyz - p;
                let d2 = max(dot(to_light, to_light), 1e-8);
                l = to_light * inverseSqrt(d2);
                // Unity-style: inverse square with a smooth window to zero at `range`.
                let f = d2 / (range * range);
                let window = clamp(1.0 - f * f, 0.0, 1.0);
                atten = window * window / (d2 + 1.0);
                // Spot cone (scale 0, offset 1 for point lights).
                let cone = clamp(dot(-l, dir) * light.color_spot_scale.w + light.direction_spot_offset.w, 0.0, 1.0);
                atten *= cone * cone;
                if (atten > 0.0 && light.shadow.x > 0.5) {
                    atten *= local_shadow(light, p, geo_n, l, sqrt(d2), in.clip.xy);
                }
            }
            if (atten > 0.0) {
                color += brdf(albedo, metallic, roughness, sn, v, l, light.color_spot_scale.rgb * atten);
            }
        }
        color += s.emissive;
    }

    if (globals.fog_color.w > 0.5) {
        let depth = dot(globals.view_plane.xyz, p) + globals.view_plane.w;
        let fog = clamp((depth - globals.params.x) / max(globals.params.y - globals.params.x, 1e-3), 0.0, 1.0);
        // Into the sky at the horizon behind the fragment, when there is one.
        color = mix(color, fog_tint(globals.fog_color.rgb, v), fog);
    }
    if (globals.height_fog_color.w > 0.5) {
        let depth = dot(globals.view_plane.xyz, p) + globals.view_plane.w;
        let fog = height_fog_amount(p, v, depth);
        color = mix(color, fog_tint(globals.height_fog_color.rgb, v), fog);
    }

    return vec4<f32>(finish(color, in.clip.xy, retro), s.color.a);
}
