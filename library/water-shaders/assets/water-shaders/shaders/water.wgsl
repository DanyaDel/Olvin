// Stylised water: two scrolling procedural normal layers, a fresnel mix
// between a clear tint and an opaque reflective surface, foam bands on the
// wave crests and tinted transparency. Surface shader for the Olvin engine,
// see SURFACE_SHADER_CONTRACT.
//
// Use it with `alpha_blend: true` in the material. The PBR reflection of the
// sky does the shiny part, so keep `roughness` low. There is no scene depth in
// a surface shader, so the foam does not hug the shore: it sits on wave
// crests ("depth-free"). Waves use world XZ, so tiles line up.
//
// Material fields it reads:
//   base_color    water tint (rgb) and its alpha when seen head on
//   emissive      foam colour (black = white foam)
// Wave steepness, scale and foam amount are the constants below.

const WAT_STEEP: f32 = 0.35;
const WAT_SCALE: f32 = 0.55;
const WAT_FOAM: f32 = 0.84;

fn wat_hash(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn wat_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(wat_hash(i), wat_hash(i + vec2<f32>(1.0, 0.0)), u.x),
               mix(wat_hash(i + vec2<f32>(0.0, 1.0)), wat_hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

fn wat_height(p: vec2<f32>, t: f32) -> f32 {
    let a = wat_noise(p * WAT_SCALE + vec2<f32>(t * 0.21, t * 0.13));
    let b = wat_noise(p * WAT_SCALE * 2.3 + vec2<f32>(-t * 0.17, t * 0.29));
    return a * 0.65 + b * 0.35;
}

fn wat_view(p: vec3<f32>) -> vec3<f32> {
    let c = globals.camera;
    return normalize(select(c.xyz, c.xyz - p, c.w > 0.5));
}

fn surface(in: SurfaceInput) -> SurfaceOutput {
    var out = default_surface(in);
    let p = in.world_position.xz;
    let e = 0.08;
    let h = wat_height(p, in.time);
    let dx = wat_height(p + vec2<f32>(e, 0.0), in.time) - h;
    let dz = wat_height(p + vec2<f32>(0.0, e), in.time) - h;
    // Perturb the geometric normal; this works for any surface orientation
    // that is roughly horizontal.
    let n = normalize(in.normal + vec3<f32>(-dx, 0.0, -dz) * (WAT_STEEP / e));
    out.normal = n;

    let v = wat_view(in.world_position);
    let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 4.0);

    // Foam: bands where the height is high, broken up by a faster noise.
    let crest = wat_height(p * 1.7 + 11.0, in.time * 1.3);
    let foam_edge = max(fwidth(crest), 0.01);
    let foam = smoothstep(WAT_FOAM - foam_edge, WAT_FOAM + foam_edge, crest) * (0.6 + 0.4 * wat_noise(p * 9.0 - in.time));
    var foam_col = material.emissive.rgb;
    if (dot(foam_col, vec3<f32>(1.0)) < 0.001) {
        foam_col = vec3<f32>(1.0);
    }

    let tint = in.color.rgb;
    let colour = mix(tint, foam_col * 0.9, foam);
    out.color = vec4<f32>(colour, clamp(mix(in.color.a, 0.97, max(fres, foam)), 0.0, 1.0));
    out.emissive = vec3<f32>(0.0);
    out.metallic = 0.0;
    out.roughness = mix(out.roughness, 0.8, foam);
    return out;
}
