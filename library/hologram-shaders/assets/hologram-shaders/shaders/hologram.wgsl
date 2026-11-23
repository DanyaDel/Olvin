// Hologram: scanlines running up the object, a fresnel glow, flicker and a
// translucent body. Surface shader for the Olvin engine, see
// SURFACE_SHADER_CONTRACT.
//
// Use it with `unlit: true` and `alpha_blend: true` in the material (alpha
// blending is what makes it see-through; blended meshes cast no shadow).
// Material fields it reads:
//   base_color    hologram tint (linear); alpha scales the whole effect
//   emissive      unused (the tint glows by itself)
// Constants below set line density, speed and flicker.

const HOLO_LINES: f32 = 28.0;
const HOLO_SPEED: f32 = 0.35;
const HOLO_GLOW: f32 = 1.6;

fn holo_hash(x: f32) -> f32 {
    return fract(sin(x * 127.1 + 311.7) * 43758.5453);
}

fn holo_view(p: vec3<f32>) -> vec3<f32> {
    let c = globals.camera;
    return normalize(select(c.xyz, c.xyz - p, c.w > 0.5));
}

fn surface(in: SurfaceInput) -> SurfaceOutput {
    var out = default_surface(in);
    let n = normalize(in.normal);
    let v = holo_view(in.world_position);
    let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 2.5);

    // Scanlines drift upwards in world space; a wider, slower band sweeps too.
    let y = in.world_position.y;
    let line = smoothstep(0.35, 0.65, abs(fract(y * HOLO_LINES - in.time * HOLO_SPEED * 6.0) - 0.5) * 2.0);
    let sweep = smoothstep(0.9, 1.0, sin(y * 3.0 - in.time * 2.0) * 0.5 + 0.5);

    // Flicker: a steady shimmer plus an occasional dropout per 1/12 s step.
    let step_t = floor(in.time * 12.0);
    let drop = step(0.93, holo_hash(step_t));
    let flick = 0.92 + 0.08 * sin(in.time * 40.0) - 0.5 * drop;

    let strength = (0.25 + 0.5 * line + 0.9 * fres + 0.6 * sweep) * flick;
    out.color = vec4<f32>(in.color.rgb, clamp(strength, 0.0, 1.0) * in.color.a);
    out.emissive = in.color.rgb * (0.4 + fres * 2.0 + sweep + line * 0.3) * HOLO_GLOW * flick;
    return out;
}
