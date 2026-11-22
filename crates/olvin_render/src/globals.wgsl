// ---- Engine: per-view globals (group 0), shared by the mesh and sky shaders.
// WebGL2 / GLES 3.0 limits: one std140-safe uniform (only vec4 / mat4 /
// arrays of vec4 structs), a shadow map with a comparison sampler, the sky
// texture, no storage buffers.

const MAX_LIGHTS: u32 = 16u;
const PI: f32 = 3.14159265;

struct Light {
    // xyz: world position; w: range (0 = directional).
    position_range: vec4<f32>,
    // rgb: colour * intensity; w: spot cone scale.
    color_spot_scale: vec4<f32>,
    // xyz: direction the light travels; w: spot cone offset.
    direction_spot_offset: vec4<f32>,
    // x: first local atlas tile + 1 (0: no shadow); y: bias in texels;
    // z: texel size per metre of distance; w = 1: point light (six tiles).
    shadow: vec4<f32>,
};

struct Globals {
    view_proj: mat4x4<f32>,
    // World to the sun's shadow atlas (clip space of the whole map), per cascade.
    shadow_view_proj: array<mat4x4<f32>, 4>,
    // World to the spot / point light atlas (4x4 tiles), per tile.
    local_view_proj: array<mat4x4<f32>, 16>,
    inv_view_proj: mat4x4<f32>,
    // Perspective: (eye, 1); orthographic: (direction towards the camera, 0).
    camera: vec4<f32>,
    // View depth of p = dot(xyz, p) + w.
    view_plane: vec4<f32>,
    // xyz: direction the sun travels, normalized; w = 1: sample the shadow map.
    sun_dir: vec4<f32>,
    // rgb: sun colour * intensity; w = 1: encode sRGB here (the target isn't
    // an sRGB format, e.g. the editor's egui pass).
    sun_color: vec4<f32>,
    // rgb: sky ambient; w: exposure.
    sky: vec4<f32>,
    // rgb: ground ambient; w: number of lights.
    ground: vec4<f32>,
    // rgb: fog colour; w = 1: fog on.
    fog_color: vec4<f32>,
    // x: fog start; y: fog end; z: 1 / shadow map size; w: shadow distance.
    params: vec4<f32>,
    // x = 1: HDR output (no clamping); y: shadow fade start; z = 1: ACES; w: time (s).
    shadow_params: vec4<f32>,
    // View depth where each shadow cascade ends.
    shadow_splits: vec4<f32>,
    // Per cascade: world-space normal offset for the shadow lookup.
    shadow_offsets: vec4<f32>,
    // x: cascade count (> 1: a 2x2 atlas of tiles); y: filter (0 hard, 1 PCF
    // 3x3, 2 rotated disc, 3 PCSS); z: filter radius in texels.
    // w: tan of the sun's angular radius (PCSS).
    shadow_cfg: vec4<f32>,
    // Per cascade: world size of a shadow texel.
    shadow_texel: vec4<f32>,
    // Per cascade: world length of the depth range 0..1.
    shadow_depth: vec4<f32>,
    // x: 1 / local atlas size.
    local_cfg: vec4<f32>,
    // rgb: procedural zenith; w: cos of the sun disc's radius.
    sky_zenith: vec4<f32>,
    // rgb: procedural horizon; w: cos of the disc's soft outer edge.
    sky_horizon: vec4<f32>,
    // rgb: procedural ground; w: sky texture rotation (radians).
    sky_ground: vec4<f32>,
    // x: mode (0 clear colour, 1 procedural, 2 texture); y: texture intensity;
    // z: texture mip count; w = 1: draw the sun disc.
    sky_params: vec4<f32>,
    // rgb: height fog colour; w = 1: height fog on.
    height_fog_color: vec4<f32>,
    // x: density at the base altitude (per metre); y: falloff per metre of
    // altitude; z: base altitude (world Y).
    height_fog: vec4<f32>,
    lights: array<Light, 16>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var sky_texture: texture_2d<f32>;
@group(0) @binding(4) var sky_sampler: sampler;
@group(0) @binding(5) var local_shadow_map: texture_depth_2d;

// Hemisphere ambient (the settings' sky / ground ambient) along `d`.
fn hemisphere(d: vec3<f32>) -> vec3<f32> {
    return mix(globals.ground.rgb, globals.sky.rgb, d.y * 0.5 + 0.5);
}

fn sky_gradient(d: vec3<f32>) -> vec3<f32> {
    if (d.y >= 0.0) {
        let t = 1.0 - pow(1.0 - d.y, 3.0);
        return mix(globals.sky_horizon.rgb, globals.sky_zenith.rgb, t);
    }
    return mix(globals.sky_horizon.rgb, globals.sky_ground.rgb, smoothstep(0.0, 0.12, -d.y));
}

// Equirectangular panorama: u = 0.5 looks down -Z, v = 0 is straight up.
fn equirect_uv(d: vec3<f32>) -> vec2<f32> {
    let u = atan2(d.x, -d.z) * (0.5 / PI) + 0.5 + globals.sky_ground.w * (0.5 / PI);
    let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
    return vec2<f32>(u, v);
}

fn sky_texture_at(d: vec3<f32>, lod: f32) -> vec3<f32> {
    return textureSampleLevel(sky_texture, sky_sampler, equirect_uv(d), lod).rgb * globals.sky_params.y;
}

// Light arriving along `d` for reflections, blurred by roughness (a lower
// mip of a texture sky). Without a sky: the hemisphere ambient.
fn sky_radiance(d: vec3<f32>, roughness: f32) -> vec3<f32> {
    let mode = globals.sky_params.x;
    if (mode > 1.5) {
        let lod = roughness * max(globals.sky_params.z - 1.0, 0.0);
        return sky_texture_at(d, lod);
    }
    if (mode > 0.5) {
        return sky_gradient(d);
    }
    return hemisphere(d);
}

// Narkowicz's ACES filmic fit.
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Exposure, tonemapping, then sRGB when the target doesn't do it (HDR
// output skips all of it).
fn finish(c: vec3<f32>) -> vec3<f32> {
    var color = c * globals.sky.w;
    if (globals.shadow_params.z > 0.5) {
        color = aces(color);
    }
    if (globals.shadow_params.x > 0.5) {
        // Linear HDR for the post-processing chain (below f16's maximum).
        return clamp(color, vec3<f32>(0.0), vec3<f32>(60000.0));
    }
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    if (globals.sun_color.w > 0.5) {
        color = linear_to_srgb(color);
    }
    return color;
}
