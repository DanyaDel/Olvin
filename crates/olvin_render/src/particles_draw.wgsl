// GPU particles as camera-facing quads: one instance per slot of the
// simulation buffer, six vertices from the vertex index. Dead slots collapse
// to a point outside the clip volume.

struct Particle {
    pos: vec3<f32>,
    age: f32,
    vel: vec3<f32>,
    life: f32,
    rot: f32,
    spin: f32,
    size: f32,
    pad: f32,
    tint: vec4<f32>,
};

struct Draw {
    view_proj: mat4x4<f32>,
    // The emitter's world matrix (local-space particles).
    matrix: mat4x4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    // x = encode sRGB here, y = 1: local space, z / w = flip-book columns / rows (0: none).
    flags: vec4<f32>,
    // Colour over life and size over life, 32 samples each (size packed 4 per vec4).
    color_lut: array<vec4<f32>, 32>,
    size_lut: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> draw: Draw;
@group(0) @binding(1) var<storage, read> particles: array<Particle>;
@group(1) @binding(0) var particle_texture: texture_2d<f32>;
@group(1) @binding(1) var particle_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

fn size_at(t: f32) -> f32 {
    let x = clamp(t, 0.0, 1.0) * 31.0;
    let i = u32(x);
    let j = min(i + 1u, 31u);
    return mix(draw.size_lut[i / 4u][i % 4u], draw.size_lut[j / 4u][j % 4u], x - f32(i));
}

fn color_at(t: f32) -> vec4<f32> {
    let x = clamp(t, 0.0, 1.0) * 31.0;
    let i = u32(x);
    let j = min(i + 1u, 31u);
    return mix(draw.color_lut[i], draw.color_lut[j], x - f32(i));
}

@vertex
fn vs_particle(@builtin(vertex_index) index: u32, @builtin(instance_index) slot: u32) -> VsOut {
    var out: VsOut;
    let p = particles[slot];
    if (p.age >= p.life) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.uv = vec2<f32>(0.0);
        out.color = vec4<f32>(0.0);
        return out;
    }
    let corner = vec2<f32>(
        select(0.0, 1.0, index == 1u || index == 4u || index == 5u),
        select(0.0, 1.0, index == 2u || index == 3u || index == 5u),
    );
    let t = p.age / p.life;
    var pos = p.pos;
    if (draw.flags.y > 0.5) {
        pos = (draw.matrix * vec4<f32>(pos, 1.0)).xyz;
    }
    let size = p.size * size_at(t);
    let sc = vec2<f32>(sin(p.rot), cos(p.rot));
    let x = (draw.right.xyz * sc.y + draw.up.xyz * sc.x) * size;
    let y = (draw.up.xyz * sc.y - draw.right.xyz * sc.x) * size;
    let world = pos - (x + y) * 0.5 + x * corner.x + y * corner.y;
    out.position = draw.view_proj * vec4<f32>(world, 1.0);
    var uv = corner;
    let frames = draw.flags.z * draw.flags.w;
    if (frames > 0.5) {
        let f = min(floor(t * frames), frames - 1.0);
        let u = f - floor(f / draw.flags.z) * draw.flags.z;
        let v = floor(f / draw.flags.z);
        let du = 1.0 / draw.flags.z;
        let dv = 1.0 / draw.flags.w;
        uv = mix(vec2<f32>(u * du, (v + 1.0) * dv), vec2<f32>((u + 1.0) * du, v * dv), corner);
    } else {
        uv = vec2<f32>(corner.x, 1.0 - corner.y);
    }
    out.uv = uv;
    out.color = p.tint * color_at(t);
    return out;
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_particle(in: VsOut) -> @location(0) vec4<f32> {
    var color = textureSample(particle_texture, particle_sampler, in.uv) * in.color;
    if (draw.flags.x > 0.5) {
        color = vec4<f32>(linear_to_srgb(color.rgb), color.a);
    }
    return color;
}
