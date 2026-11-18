// Depth-only pass into the sun's shadow map. Same vertex / instance buffers
// as mesh.wgsl, only position and the model matrix are read.

struct ShadowGlobals {
    light_view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> shadow: ShadowGlobals;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
};

@vertex
fn vs_shadow(in: VertexIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    return shadow.light_view_proj * model * vec4<f32>(in.position, 1.0);
}
