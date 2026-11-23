// GPU skinning: one thread per vertex poses the bind-pose vertices with the
// joint matrices and writes the result into the mesh's own vertex buffer.
// Mirrors `SkinSource::skin` (skinning.rs): up to four influences, weights
// renormalised over the joints that exist, a vertex without any keeps its
// bind pose, normals and tangents are turned and renormalised.

struct Params {
    vertex_count: u32,
    joint_count: u32,
    pad0: u32,
    pad1: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
// 12 floats per vertex: position, normal, uv, tangent (the `Vertex` layout).
@group(0) @binding(1) var<storage, read> bind_pose: array<f32>;
// Two vec4 per vertex: the joint indices, then the weights as raw bits.
@group(0) @binding(2) var<storage, read> influences: array<vec4<u32>>;
@group(0) @binding(3) var<storage, read> palette: array<mat4x4<f32>>;
@group(0) @binding(4) var<storage, read_write> posed: array<f32>;

fn unit_or_zero(v: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    return select(vec3<f32>(0.0), v / l, l > 1e-20);
}

@compute @workgroup_size(64)
fn cs_skin(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.vertex_count) {
        return;
    }
    let base = i * 12u;
    for (var k = 0u; k < 12u; k = k + 1u) {
        posed[base + k] = bind_pose[base + k];
    }
    let joints = influences[i * 2u];
    let weights = bitcast<vec4<f32>>(influences[i * 2u + 1u]);
    var m = mat4x4<f32>(
        vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
    var total = 0.0;
    for (var k = 0u; k < 4u; k = k + 1u) {
        let w = weights[k];
        let j = joints[k];
        if (w > 0.0 && j < params.joint_count) {
            m = m + palette[j] * w;
            total = total + w;
        }
    }
    if (total <= 1e-6) {
        return;
    }
    m = m * (1.0 / total);
    let p = vec3<f32>(bind_pose[base], bind_pose[base + 1u], bind_pose[base + 2u]);
    let n = vec3<f32>(bind_pose[base + 3u], bind_pose[base + 4u], bind_pose[base + 5u]);
    let t = vec3<f32>(bind_pose[base + 8u], bind_pose[base + 9u], bind_pose[base + 10u]);
    let pp = (m * vec4<f32>(p, 1.0)).xyz;
    let nn = unit_or_zero((m * vec4<f32>(n, 0.0)).xyz);
    let tt = unit_or_zero((m * vec4<f32>(t, 0.0)).xyz);
    posed[base] = pp.x;
    posed[base + 1u] = pp.y;
    posed[base + 2u] = pp.z;
    posed[base + 3u] = nn.x;
    posed[base + 4u] = nn.y;
    posed[base + 5u] = nn.z;
    posed[base + 8u] = tt.x;
    posed[base + 9u] = tt.y;
    posed[base + 10u] = tt.z;
}
