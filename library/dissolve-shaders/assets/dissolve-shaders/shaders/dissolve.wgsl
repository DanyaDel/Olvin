// Noise dissolve with a glowing burning edge, looping with time: the object
// eats itself away over about DIS_PERIOD seconds, rests, then comes back.
// Surface shader for the Olvin engine, see SURFACE_SHADER_CONTRACT.
//
// Use it with `alpha_cutoff: 0.5` in the material: the shader writes alpha 0
// or 1 and the engine discards the dissolved pixels (opaque, shadows follow
// the intact part). Material fields it reads:
//   base_color / base_texture  the surface colour
//   emissive                   edge colour (black = orange fire); use values
//                              above 1 to make the edge bloom
// `dissolve_driven.wgsl` is the same look driven by the colour's alpha
// instead of time, for gameplay to animate.

const DIS_PERIOD: f32 = 6.0;
const DIS_EDGE: f32 = 0.1;
const DIS_SCALE: f32 = 5.0;

fn dis_hash(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.3183099 + vec3<f32>(0.1, 0.2, 0.3));
    q = q * 17.0;
    return fract(q.x * q.y * q.z * (q.x + q.y + q.z));
}

fn dis_noise(x: vec3<f32>) -> f32 {
    let i = floor(x);
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(dis_hash(i), dis_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
                mix(dis_hash(i + vec3<f32>(0.0, 1.0, 0.0)), dis_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(dis_hash(i + vec3<f32>(0.0, 0.0, 1.0)), dis_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
                mix(dis_hash(i + vec3<f32>(0.0, 1.0, 1.0)), dis_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

fn dis_fbm(p: vec3<f32>) -> f32 {
    return dis_noise(p) * 0.6 + dis_noise(p * 2.03 + 7.1) * 0.3 + dis_noise(p * 4.1 + 3.7) * 0.1;
}

// `progress`: 0 = whole, 1 = gone.
fn dis_apply(in: SurfaceInput, progress: f32) -> SurfaceOutput {
    var out = default_surface(in);
    let n = dis_fbm(in.world_position * DIS_SCALE);
    // Remap so progress 0 shows everything and 1 nothing.
    let t = progress * (1.0 + DIS_EDGE * 2.0) - DIS_EDGE;
    let alive = n - t;
    let keep = step(0.0, alive);
    let burn = (1.0 - smoothstep(0.0, DIS_EDGE, alive)) * keep * step(0.001, progress);
    var edge = material.emissive.rgb;
    if (dot(edge, vec3<f32>(1.0)) < 0.001) {
        edge = vec3<f32>(3.0, 0.9, 0.12);
    }
    out.color = vec4<f32>(mix(in.color.rgb, vec3<f32>(0.02), burn), keep);
    out.emissive = edge * burn;
    return out;
}

fn surface(in: SurfaceInput) -> SurfaceOutput {
    // Dissolve for 60% of the period, rest gone for 10%, reform, rest whole.
    let k = fract(in.time / DIS_PERIOD);
    let progress = smoothstep(0.0, 0.6, k) * (1.0 - smoothstep(0.7, 0.95, k));
    return dis_apply(in, progress);
}
