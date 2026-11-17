// Editor viewport: lit meshes, an infinite adaptive grid, lines.

struct Uniforms {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // xyz: direction the light travels.
    light: vec4<f32>,
    // x: minor step, y: lines per major, z: fade distance, w: grid height.
    grid: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// --- meshes ------------------------------------------------------------------

struct MeshIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) flags: u32,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) flags: u32,
};

@vertex
fn vs_mesh(in: MeshIn) -> MeshOut {
    var out: MeshOut;
    out.clip = u.view_proj * vec4<f32>(in.position, 1.0);
    out.world = in.position;
    out.normal = in.normal;
    out.color = in.color;
    out.flags = in.flags;
    return out;
}

fn line_mask(coord: vec2<f32>) -> f32 {
    let g = abs(fract(coord - 0.5) - 0.5) / max(fwidth(coord), vec2<f32>(1e-4));
    return 1.0 - clamp(min(g.x, g.y), 0.0, 1.0);
}

@fragment
fn fs_mesh(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let diffuse = max(dot(n, -u.light.xyz), 0.0);
    let sky = 0.5 + 0.5 * n.y;
    var color = in.color.rgb * (0.22 + 0.18 * sky + 0.6 * diffuse);

    if ((in.flags & 1u) != 0u) {
        // Triplanar 1 m grid on level geometry.
        let a = abs(n);
        var uv = in.world.xz;
        if (a.x > a.y && a.x > a.z) {
            uv = in.world.zy;
        } else if (a.z > a.y) {
            uv = in.world.xy;
        }
        color *= 1.0 - 0.22 * line_mask(uv) - 0.07 * line_mask(uv * 4.0);
    }
    if ((in.flags & 2u) != 0u) {
        color = mix(color, vec3<f32>(1.0, 0.63, 0.2), 0.18);
    }
    return vec4<f32>(color, 1.0);
}

// --- grid --------------------------------------------------------------------

struct GridOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_grid(@builtin(vertex_index) i: u32) -> GridOut {
    // One triangle covering the viewport.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: GridOut;
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

struct GridFrag {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_grid(in: GridOut) -> GridFrag {
    let near = u.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let far = u.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let a = near.xyz / near.w;
    let b = far.xyz / far.w;
    let dir = b - a;
    let t = (u.grid.w - a.y) / dir.y;
    if (t <= 0.0 || abs(dir.y) < 1e-6) {
        discard;
    }
    let p = a + dir * t;

    let step = u.grid.x;
    let major = step * u.grid.y;
    let minor_mask = line_mask(p.xz / step);
    let major_mask = line_mask(p.xz / major);
    // Minor lines fade out before they turn into moiré.
    let density = max(fwidth(p.x / step), fwidth(p.z / step));
    let minor_fade = 1.0 - smoothstep(0.25, 0.6, density);

    var color = vec3<f32>(0.62);
    var alpha = max(minor_mask * 0.22 * minor_fade, major_mask * 0.42);

    // World axes: X red along z = 0, Z blue along x = 0.
    let axis_w = fwidth(p.xz) * 1.5;
    if (abs(p.z) < axis_w.y) {
        color = vec3<f32>(0.91, 0.28, 0.33);
        alpha = 0.9;
    } else if (abs(p.x) < axis_w.x) {
        color = vec3<f32>(0.25, 0.52, 0.91);
        alpha = 0.9;
    }

    let dist = length(p - u.eye.xyz);
    alpha *= 1.0 - smoothstep(u.grid.z * 0.35, u.grid.z, dist);
    if (alpha < 0.003) {
        discard;
    }

    let clip = u.view_proj * vec4<f32>(p, 1.0);
    var out: GridFrag;
    out.color = vec4<f32>(color * alpha, alpha);
    out.depth = clip.z / clip.w;
    return out;
}

// --- lines -------------------------------------------------------------------

struct LineIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(in: LineIn) -> LineOut {
    var out: LineOut;
    out.clip = u.view_proj * vec4<f32>(in.position, 1.0);
    // Pull lines slightly towards the camera so outlines win over their faces.
    out.clip.z -= 0.0004 * out.clip.w;
    out.color = in.color;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb * in.color.a, in.color.a);
}
