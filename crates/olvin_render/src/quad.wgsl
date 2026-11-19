// ---- Textured quads: particles and 2D sprites. One instance = one quad,
// given as an origin (the corner at (0, 0)) and two edge vectors, so any
// billboard, rotation or skew is already baked in by the CPU. Six vertices
// from the vertex index, no vertex buffer; one uniform, one texture.

struct Params {
    view_proj: mat4x4<f32>,
    // x = 1: encode sRGB here (the target isn't an sRGB format).
    flags: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(1) @binding(0) var quad_texture: texture_2d<f32>;
@group(1) @binding(1) var quad_sampler: sampler;

struct Instance {
    @location(0) origin: vec4<f32>,
    @location(1) axis_x: vec4<f32>,
    @location(2) axis_y: vec4<f32>,
    @location(3) color: vec4<f32>,
    // uv at corner (0, 0) in xy, at corner (1, 1) in zw.
    @location(4) uv: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_quad(@builtin(vertex_index) index: u32, inst: Instance) -> VsOut {
    // Triangles (0,0) (1,0) (0,1) and (0,1) (1,0) (1,1).
    let corner = vec2<f32>(
        select(0.0, 1.0, index == 1u || index == 4u || index == 5u),
        select(0.0, 1.0, index == 2u || index == 3u || index == 5u),
    );
    let world = inst.origin.xyz + inst.axis_x.xyz * corner.x + inst.axis_y.xyz * corner.y;
    var out: VsOut;
    out.position = params.view_proj * vec4<f32>(world, 1.0);
    out.uv = mix(inst.uv.xy, inst.uv.zw, corner);
    out.color = inst.color;
    return out;
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_quad(in: VsOut) -> @location(0) vec4<f32> {
    var color = textureSample(quad_texture, quad_sampler, in.uv) * in.color;
    if (params.flags.x > 0.5) {
        color = vec4<f32>(linear_to_srgb(color.rgb), color.a);
    }
    return color;
}
