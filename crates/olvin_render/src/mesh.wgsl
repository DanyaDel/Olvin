// Instanced mesh shader. Written to WebGL2 / GLES 3.0 limits: one uniform
// buffer (std140-safe: only vec4 / mat4 / arrays of vec4 structs), one depth
// texture with a comparison sampler, 7 vertex attributes, no storage buffers.
//
// Lighting, Unity built-in style: hemisphere ambient, a sun with PCF shadows,
// up to 16 point / spot / extra directional lights, normalized Blinn-Phong
// specular at a fixed roughness, linear fog, exposure + ACES, then sRGB.

const MAX_LIGHTS: u32 = 16u;
// Fixed "material" for now: dielectric F0 and a moderate gloss.
const SPEC_POWER: f32 = 48.0;
const F0: f32 = 0.04;

struct Light {
    // xyz: world position; w: range (0 = directional).
    position_range: vec4<f32>,
    // rgb: colour * intensity; w: spot cone scale.
    color_spot_scale: vec4<f32>,
    // xyz: direction the light travels; w: spot cone offset.
    direction_spot_offset: vec4<f32>,
};

struct Globals {
    view_proj: mat4x4<f32>,
    shadow_view_proj: mat4x4<f32>,
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
    // x: normal offset (world units); y: shadow fade start; z = 1: ACES.
    shadow_params: vec4<f32>,
    lights: array<Light, 16>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(6) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    let world = model * vec4<f32>(in.position, 1.0);
    var out: VertexOut;
    out.clip = globals.view_proj * world;
    // Exact for uniform scale; non-uniform scale would need the inverse transpose.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.color = in.color;
    out.world = world.xyz;
    return out;
}

// Diffuse + specular from one light; `l` points towards the light.
fn shade(albedo: vec3<f32>, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>) -> vec3<f32> {
    let ndl = max(dot(n, l), 0.0);
    let h = normalize(l + v + vec3<f32>(0.0, 1e-5, 0.0));
    let fresnel = F0 + (1.0 - F0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    // Normalized Blinn-Phong: energy stays put as the gloss changes.
    let spec = fresnel * (SPEC_POWER + 8.0) * 0.0397887 * pow(max(dot(n, h), 0.0), SPEC_POWER);
    return (albedo * (1.0 - F0) + spec) * radiance * ndl;
}

// 1 = lit, 0 = in the sun's shadow. 3x3 taps of a linear comparison sampler.
fn sun_shadow(p: vec3<f32>, n: vec3<f32>) -> f32 {
    if (globals.sun_dir.w < 0.5) {
        return 1.0;
    }
    // Normal offset: slide the lookup off the surface, more at grazing angles.
    let ndl = clamp(dot(n, -globals.sun_dir.xyz), 0.0, 1.0);
    let offset_p = p + n * globals.shadow_params.x * (1.0 - 0.5 * ndl);
    let c = globals.shadow_view_proj * vec4<f32>(offset_p, 1.0);
    let ndc = c.xyz / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let texel = globals.params.z;
    var sum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * texel, ndc.z);
        }
    }
    var lit = sum / 9.0;
    // Fade out towards the shadow distance instead of a hard edge.
    let depth = dot(globals.view_plane.xyz, p) + globals.view_plane.w;
    let fade = clamp((depth - globals.shadow_params.y) / max(globals.params.w - globals.shadow_params.y, 1e-3), 0.0, 1.0);
    lit = mix(lit, 1.0, fade);
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)) && ndc.z <= 1.0;
    return select(1.0, lit, inside);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let p = in.world;
    let v = normalize(globals.camera.xyz - p * globals.camera.w);
    let albedo = in.color.rgb;

    let hemi = mix(globals.ground.rgb, globals.sky.rgb, n.y * 0.5 + 0.5);
    var color = albedo * hemi;
    color += shade(albedo, n, v, -globals.sun_dir.xyz, globals.sun_color.rgb * sun_shadow(p, n));

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
        }
        if (atten > 0.0) {
            color += shade(albedo, n, v, l, light.color_spot_scale.rgb * atten);
        }
    }

    if (globals.fog_color.w > 0.5) {
        let depth = dot(globals.view_plane.xyz, p) + globals.view_plane.w;
        let fog = clamp((depth - globals.params.x) / max(globals.params.y - globals.params.x, 1e-3), 0.0, 1.0);
        color = mix(color, globals.fog_color.rgb, fog);
    }

    color *= globals.sky.w;
    if (globals.shadow_params.z > 0.5) {
        color = aces(color);
    }
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    if (globals.sun_color.w > 0.5) {
        color = linear_to_srgb(color);
    }
    return vec4<f32>(color, in.color.a);
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
