// Toon (cel) shading: banded diffuse, a hard specular blob and a rim light.
// Surface shader for the Olvin engine, see SURFACE_SHADER_CONTRACT.
//
// Use it with `unlit: true` in the material: the shader lights the surface
// itself (the sun and up to 16 point / spot lights, no shadows) and the
// engine adds only fog on top.
//
// Material fields it reads:
//   base_color / base_texture  albedo
//   roughness                  size of the specular blob (low = bigger blob)
//   metallic                   rim light strength (0 = none)
//   emissive                   rim colour (black = white rim)

const TOON_SHADOW: f32 = 0.38;
const TOON_MID: f32 = 0.72;

fn toon_view(p: vec3<f32>) -> vec3<f32> {
    // Perspective: (eye, 1); orthographic: (direction towards the camera, 0).
    let c = globals.camera;
    return normalize(select(c.xyz, c.xyz - p, c.w > 0.5));
}

// Three flat bands out of a smooth 0..1 value, edges anti-aliased by the
// screen-space derivative.
fn toon_band(x: f32) -> f32 {
    let w = max(fwidth(x), 0.004);
    let a = smoothstep(0.02 - w, 0.02 + w, x);
    let b = smoothstep(0.5 - w, 0.5 + w, x);
    return mix(mix(TOON_SHADOW, TOON_MID, a), 1.0, b * a);
}

fn toon_blob(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, size: f32) -> f32 {
    let h = normalize(l + v + vec3<f32>(0.0, 1e-5, 0.0));
    let nh = dot(n, h);
    let w = max(fwidth(nh), 0.004);
    let edge = 1.0 - size;
    return smoothstep(edge - w, edge + w, nh) * step(0.0, dot(n, l));
}

fn surface(in: SurfaceInput) -> SurfaceOutput {
    var out = default_surface(in);
    let n = normalize(in.normal);
    let v = toon_view(in.world_position);
    let albedo = in.color.rgb;
    let size = 0.004 + 0.07 * (1.0 - clamp(material.params.y, 0.0, 1.0));

    var lit = vec3<f32>(0.0);
    var spec = 0.0;
    var rim_light = 0.0;

    // The sun.
    let sun_l = -globals.sun_dir.xyz;
    let sun_c = globals.sun_color.rgb;
    let sun_nl = dot(n, sun_l);
    lit += sun_c * toon_band(sun_nl);
    spec += toon_blob(n, v, sun_l, size) * max(max(sun_c.r, sun_c.g), sun_c.b);
    rim_light += max(max(sun_c.r, sun_c.g), sun_c.b) * smoothstep(-0.3, 0.3, sun_nl);

    // Point, spot and extra directional lights.
    let count = min(u32(globals.ground.w), MAX_LIGHTS);
    for (var i = 0u; i < count; i++) {
        let light = globals.lights[i];
        let range = light.position_range.w;
        let dir = light.direction_spot_offset.xyz;
        var l = -dir;
        var atten = 1.0;
        if (range > 0.0) {
            let to_light = light.position_range.xyz - in.world_position;
            let d2 = max(dot(to_light, to_light), 1e-8);
            l = to_light * inverseSqrt(d2);
            let f = d2 / (range * range);
            let window = clamp(1.0 - f * f, 0.0, 1.0);
            atten = window * window / (d2 + 1.0);
            let cone = clamp(dot(-l, dir) * light.color_spot_scale.w + light.direction_spot_offset.w, 0.0, 1.0);
            atten *= cone * cone;
        }
        let radiance = light.color_spot_scale.rgb * atten;
        lit += radiance * toon_band(dot(n, l));
        spec += toon_blob(n, v, l, size) * atten * max(max(light.color_spot_scale.r, light.color_spot_scale.g), light.color_spot_scale.b);
        rim_light += atten;
    }

    let ambient = hemisphere(n);
    var color = albedo * (ambient + lit);

    // Specular blob: pushes the colour towards white.
    color = mix(color, vec3<f32>(1.0), clamp(spec, 0.0, 1.0) * 0.55);

    // Rim light: a hard band at grazing angles, on the lit side.
    let rim_colour = select(material.emissive.rgb, vec3<f32>(1.0), dot(material.emissive.rgb, vec3<f32>(1.0)) < 0.001);
    let facing = 1.0 - clamp(dot(n, v), 0.0, 1.0);
    let rw = max(fwidth(facing), 0.01);
    let rim = smoothstep(0.62 - rw, 0.62 + rw, facing) * clamp(rim_light, 0.0, 1.0) * clamp(material.params.x, 0.0, 1.0);
    color = mix(color, color + rim_colour * 0.6, rim);

    out.color = vec4<f32>(color, in.color.a);
    out.emissive = vec3<f32>(0.0);
    return out;
}
