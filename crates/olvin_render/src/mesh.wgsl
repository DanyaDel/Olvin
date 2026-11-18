// ---- Engine: instanced PBR mesh shader template. The surface shader
// (`fn surface`) is pasted in front of this file and the globals.
//
// WebGL2 limits: 9 vertex attributes (4 vertex + 5 instance), 5 varyings,
// group 0 = view globals, group 1 = material (uniform + 4 textures + sampler),
// 6 sampled textures in the fragment stage.
//
// Lighting, Unity built-in style: hemisphere ambient + roughness-faded sky
// reflection, a sun with PCF shadows, up to 16 point / spot / extra
// directional lights, GGX + height-correlated Smith + Schlick
// (metallic-roughness), linear fog (towards the sky at the horizon when a sky
// is set), exposure + ACES, then sRGB.

struct SurfaceInput {
    world_position: vec3<f32>,
    // World space, normalized, normal map applied.
    normal: vec3<f32>,
    // After uv_scale / uv_offset.
    uv: vec2<f32>,
    // base_color * base_texture * instance colour.
    color: vec4<f32>,
    // Seconds since start.
    time: f32,
};

struct SurfaceOutput {
    // Albedo + alpha.
    color: vec4<f32>,
    // World space.
    normal: vec3<f32>,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
};

struct Material {
    base_color: vec4<f32>,
    // rgb: emissive; w: normal strength.
    emissive: vec4<f32>,
    // x: metallic; y: roughness; z: alpha cutoff.
    params: vec4<f32>,
    // xy: scale; zw: offset.
    uv: vec4<f32>,
    // x: unlit; y: triplanar; z: has a normal map.
    flags: vec4<f32>,
};

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var normal_texture: texture_2d<f32>;
@group(1) @binding(3) var orm_texture: texture_2d<f32>;
@group(1) @binding(4) var emissive_texture: texture_2d<f32>;
@group(1) @binding(5) var material_sampler: sampler;

// The material's sampled ORM and emissive texels for this pixel (sampled in
// uniform control flow before `surface` runs).
var<private> sampled_orm: vec3<f32>;
var<private> sampled_emissive: vec3<f32>;

// The standard material.
fn default_surface(in: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    out.color = in.color;
    out.normal = in.normal;
    out.metallic = material.params.x * sampled_orm.b;
    out.roughness = material.params.y * sampled_orm.g;
    out.emissive = material.emissive.rgb * sampled_emissive;
    return out;
}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
    @location(8) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) tangent: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    let world = model * vec4<f32>(in.position, 1.0);
    var out: VertexOut;
    out.clip = globals.view_proj * world;
    // Exact for uniform scale; non-uniform scale would need the inverse transpose.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.color = in.color;
    out.world = world.xyz;
    out.uv = in.uv;
    out.tangent = vec4<f32>((model * vec4<f32>(in.tangent.xyz, 0.0)).xyz, in.tangent.w);
    return out;
}

// GGX / height-correlated Smith / Schlick, Unity's convention (the lambert
// term isn't divided by pi, so a white light of 1 gives the full albedo).
// `l` points towards the light.
fn brdf(albedo: vec3<f32>, metallic: f32, roughness: f32, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>) -> vec3<f32> {
    let nl = dot(n, l);
    if (nl <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(l + v + vec3<f32>(0.0, 1e-5, 0.0));
    let nv = max(dot(n, v), 1e-4);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let a = max(roughness * roughness, 2e-3);
    let a2 = a * a;
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * dd * dd);
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    let vis = 0.5 / max(gv + gl, 1e-5);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
    let spec = d * vis * f * PI;
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    return (kd * albedo + spec) * radiance * nl;
}

// Karis' analytic fit of the split-sum environment BRDF.
fn env_brdf(f0: vec3<f32>, roughness: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// 1 = lit, 0 = in the sun's shadow. 3x3 taps of a linear comparison sampler.
fn sun_shadow(p: vec3<f32>, n: vec3<f32>) -> f32 {
    if (globals.sun_dir.w < 0.5) {
        return 1.0;
    }
    // Normal offset: slide the lookup off the surface, more at grazing angles.
    let ndl = clamp(dot(n, -globals.sun_dir.xyz), 0.0, 1.0);
    let offset_p = p + n * globals.shadow_params.x * (1.0 - 0.5 * ndl);
    let c = globals.shadow_view_proj * vec4<f32>(offset_p, 1.0);
    let ndc = c.xyz / c.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let texel = globals.params.z;
    var sum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * texel, ndc.z);
        }
    }
    var lit = sum / 9.0;
    // Fade out towards the shadow distance instead of a hard edge.
    let depth = dot(globals.view_plane.xyz, p) + globals.view_plane.w;
    let fade = clamp((depth - globals.shadow_params.y) / max(globals.params.w - globals.shadow_params.y, 1e-3), 0.0, 1.0);
    lit = mix(lit, 1.0, fade);
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)) && ndc.z <= 1.0;
    return select(1.0, lit, inside);
}

// Tangent-space normal (green up) to world space.
fn perturb(n: vec3<f32>, tangent: vec4<f32>, tn: vec3<f32>) -> vec3<f32> {
    let t0 = tangent.xyz - n * dot(n, tangent.xyz);
    if (dot(t0, t0) < 1e-12) {
        return n;
    }
    let t = normalize(t0);
    let b = cross(n, t) * select(1.0, -1.0, tangent.w < 0.0);
    return normalize(t * tn.x + b * tn.y + n * tn.z);
}

fn unpack_normal(texel: vec4<f32>) -> vec3<f32> {
    let n = texel.xyz * 2.0 - 1.0;
    return vec3<f32>(n.xy * material.emissive.w, n.z);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let geo_n = normalize(in.normal);
    let p = in.world;
    let v = normalize(globals.camera.xyz - p * globals.camera.w);
    let uv = in.uv * material.uv.xy + material.uv.zw;

    // Textures, all in uniform control flow (branches on the material uniform).
    var n = geo_n;
    var base: vec4<f32>;
    var orm: vec4<f32>;
    var emissive: vec4<f32>;
    if (material.flags.y > 0.5) {
        // Triplanar: world projections blended by the normal.
        let w4 = pow(abs(geo_n), vec3<f32>(4.0));
        let w = w4 / max(w4.x + w4.y + w4.z, 1e-5);
        let ux = vec2<f32>(p.z, -p.y) * material.uv.xy + material.uv.zw;
        let uy = vec2<f32>(p.x, p.z) * material.uv.xy + material.uv.zw;
        let uz = vec2<f32>(p.x, -p.y) * material.uv.xy + material.uv.zw;
        base = textureSample(base_texture, material_sampler, ux) * w.x
            + textureSample(base_texture, material_sampler, uy) * w.y
            + textureSample(base_texture, material_sampler, uz) * w.z;
        orm = textureSample(orm_texture, material_sampler, ux) * w.x
            + textureSample(orm_texture, material_sampler, uy) * w.y
            + textureSample(orm_texture, material_sampler, uz) * w.z;
        emissive = textureSample(emissive_texture, material_sampler, ux) * w.x
            + textureSample(emissive_texture, material_sampler, uy) * w.y
            + textureSample(emissive_texture, material_sampler, uz) * w.z;
        if (material.flags.z > 0.5) {
            // Whiteout blend (Golus): each projection's tangent normal swizzled to world.
            let nx = unpack_normal(textureSample(normal_texture, material_sampler, ux));
            let ny = unpack_normal(textureSample(normal_texture, material_sampler, uy));
            let nz = unpack_normal(textureSample(normal_texture, material_sampler, uz));
            let tx = vec3<f32>(nx.xy + geo_n.zy, abs(nx.z) * geo_n.x);
            let ty = vec3<f32>(ny.xy + geo_n.xz, abs(ny.z) * geo_n.y);
            let tz = vec3<f32>(nz.xy + geo_n.xy, abs(nz.z) * geo_n.z);
            n = normalize(tx.zyx * w.x + ty.xzy * w.y + tz.xyz * w.z);
        }
    } else {
        base = textureSample(base_texture, material_sampler, uv);
        orm = textureSample(orm_texture, material_sampler, uv);
        emissive = textureSample(emissive_texture, material_sampler, uv);
        if (material.flags.z > 0.5) {
            n = perturb(geo_n, in.tangent, unpack_normal(textureSample(normal_texture, material_sampler, uv)));
        }
    }
    sampled_orm = orm.rgb;
    sampled_emissive = emissive.rgb;

    var si: SurfaceInput;
    si.world_position = p;
    si.normal = n;
    si.uv = uv;
    si.color = material.base_color * base * in.color;
    si.time = globals.shadow_params.w;
    let s = surface(si);
    if (s.color.a < material.params.z) {
        discard;
    }

    var color: vec3<f32>;
    if (material.flags.x > 0.5) {
        color = s.color.rgb + s.emissive;
    } else {
        let sn = normalize(s.normal);
        let albedo = s.color.rgb;
        let metallic = clamp(s.metallic, 0.0, 1.0);
        let roughness = clamp(s.roughness, 0.03, 1.0);
        let ao = sampled_orm.r;
        let nv = max(dot(sn, v), 1e-4);
        let f0 = mix(vec3<f32>(0.04), albedo, metallic);

        // Ambient: hemisphere diffuse + the sky along the reflection, faded
        // towards the normal (a cheap blur) as roughness grows.
        color = albedo * (1.0 - metallic) * hemisphere(sn) * ao;
        let r = reflect(-v, sn);
        let spec_dir = normalize(mix(r, sn, roughness * roughness));
        color += env_brdf(f0, roughness, nv) * sky_radiance(spec_dir, roughness) * ao;

        color += brdf(albedo, metallic, roughness, sn, v, -globals.sun_dir.xyz, globals.sun_color.rgb * sun_shadow(p, geo_n));

        let count = min(u32(globals.ground.w), MAX_LIGHTS);
        for (var i = 0u; i < count; i++) {
            let light = globals.lights[i];
            let range = light.position_range.w;
            let dir = light.direction_spot_offset.xyz;
            var l = -dir;
            var atten = 1.0;
            if (range > 0.0) {
                let to_light = light.position_range.xyz - p;
                let d2 = max(dot(to_light, to_light), 1e-8);
                l = to_light * inverseSqrt(d2);
                // Unity-style: inverse square with a smooth window to zero at `range`.
                let f = d2 / (range * range);
                let window = clamp(1.0 - f * f, 0.0, 1.0);
                atten = window * window / (d2 + 1.0);
                // Spot cone (scale 0, offset 1 for point lights).
                let cone = clamp(dot(-l, dir) * light.color_spot_scale.w + light.direction_spot_offset.w, 0.0, 1.0);
                atten *= cone * cone;
            }
            if (atten > 0.0) {
                color += brdf(albedo, metallic, roughness, sn, v, l, light.color_spot_scale.rgb * atten);
            }
        }
        color += s.emissive;
    }

    if (globals.fog_color.w > 0.5) {
        let depth = dot(globals.view_plane.xyz, p) + globals.view_plane.w;
        let fog = clamp((depth - globals.params.x) / max(globals.params.y - globals.params.x, 1e-3), 0.0, 1.0);
        var fog_color = globals.fog_color.rgb;
        if (globals.sky_params.x > 0.5) {
            // Into the sky at the horizon behind the fragment.
            let flat_dir = vec3<f32>(-v.x, 0.0, -v.z);
            let d = select(vec3<f32>(0.0, 0.0, -1.0), normalize(flat_dir), dot(flat_dir, flat_dir) > 1e-8);
            fog_color = sky_radiance(d, 0.5);
        }
        color = mix(color, fog_color, fog);
    }

    return vec4<f32>(finish(color), s.color.a);
}
