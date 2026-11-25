// Full tier: the post-processed scene image and its depth, written into
// egui's pass over the viewport rect, so editor geometry drawn after it is
// still depth-tested against the scene.

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_depth_2d;

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    // One triangle covering the viewport.
    let xy = vec2f(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: VOut;
    out.pos = vec4f(xy, 0.0, 1.0);
    out.uv = vec2f(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

struct FOut {
    @location(0) color: vec4f,
    @builtin(frag_depth) depth: f32,
};

fn texel(uv: vec2f, size: vec2u) -> vec2i {
    return vec2i(clamp(uv * vec2f(size), vec2f(0.0), vec2f(size) - 1.0));
}

@fragment
fn fs(in: VOut) -> FOut {
    var out: FOut;
    out.color = vec4f(textureLoad(color_tex, texel(in.uv, textureDimensions(color_tex)), 0).rgb, 1.0);
    // The depth may be smaller (retro low resolution): its own texel.
    out.depth = textureLoad(depth_tex, texel(in.uv, textureDimensions(depth_tex)), 0);
    return out;
}
