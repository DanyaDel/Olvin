// Instanced mesh shader. Written to WebGL2 / GLES 3.0 limits: one uniform
// buffer, 7 vertex attributes, no storage buffers.

struct Globals {
    view_proj: mat4x4<f32>,
    // xyz: direction the light travels, normalized.
    light_dir: vec4<f32>,
    ambient: vec4<f32>,
    light_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(6) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    var out: VertexOut;
    out.clip = globals.view_proj * model * vec4<f32>(in.position, 1.0);
    // Exact for uniform scale; non-uniform scale would need the inverse transpose.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let diffuse = max(dot(n, -globals.light_dir.xyz), 0.0);
    return vec4<f32>(in.color.rgb * (globals.ambient.rgb + diffuse * globals.light_color.rgb), in.color.a);
}
