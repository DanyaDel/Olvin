// ---- Engine: sky. A fullscreen triangle on the far plane, drawn after the
// opaque meshes with depth test LessEqual (only where nothing was drawn).

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: SkyOut;
    out.clip = vec4<f32>(p, 1.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    // View ray through this pixel: near point to far point (homogeneous, so
    // an infinite far plane or an orthographic camera both work).
    let near = globals.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = globals.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    var ray = far.xyz * near.w - near.xyz * far.w;
    ray *= select(1.0, -1.0, near.w * far.w < 0.0);
    if (abs(far.w) < 1e-12) {
        ray = far.xyz * sign(near.w);
    }
    let d = normalize(ray);

    var color: vec3<f32>;
    if (globals.sky_params.x > 1.5) {
        color = sky_texture_at(d, 0.0);
    } else {
        color = sky_gradient(d);
        if (globals.sky_params.w > 0.5) {
            // Sun disc with a soft rim and a faint glow.
            let c = dot(d, -globals.sun_dir.xyz);
            let disc = smoothstep(globals.sky_horizon.w, globals.sky_zenith.w, c);
            let glow = pow(max(c, 0.0), 256.0) * 0.5;
            color += globals.sun_color.rgb * (disc * 20.0 + glow);
        }
    }
    return vec4<f32>(finish(color, in.clip.xy, globals.retro_a.x > 0.5), 1.0);
}
