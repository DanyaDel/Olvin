// Stylised edge shading: an ink line (or a glowing neon edge) at the object's
// silhouette, from the surface angle to the camera. Surface shader for the
// Olvin engine, see SURFACE_SHADER_CONTRACT.
//
// This is NOT a true outline: an inverted hull needs a second draw of the
// mesh, which the contract cannot do, and a depth / normal edge filter needs
// the scene depth, which a surface shader cannot read. So the line follows the
// fresnel term: it hugs smooth convex shapes (spheres, characters, rocks),
// grows thin on flat faces seen head on and disappears from inner creases.
//
// The material stays PBR-lit. Material fields it reads:
//   base_color / base_texture  surface colour
//   emissive                   black = ink edge; any colour = glowing edge of
//                              that colour (values above 1 bloom)
// Edge width is a constant below (OL_WIDTH).

const OL_WIDTH: f32 = 0.28;
const OL_SOFT: f32 = 0.05;

fn ol_view(p: vec3<f32>) -> vec3<f32> {
    let c = globals.camera;
    return normalize(select(c.xyz, c.xyz - p, c.w > 0.5));
}

fn surface(in: SurfaceInput) -> SurfaceOutput {
    var out = default_surface(in);
    let n = normalize(in.normal);
    let v = ol_view(in.world_position);
    let facing = clamp(dot(n, v), 0.0, 1.0);
    // 1 at the silhouette, 0 facing the camera; a hard-edged band.
    let edge_w = max(fwidth(facing), OL_SOFT);
    let edge = 1.0 - smoothstep(OL_WIDTH - edge_w, OL_WIDTH + edge_w, facing);

    let glow = material.emissive.rgb;
    let neon = dot(glow, vec3<f32>(1.0)) > 0.001;
    if (neon) {
        // The material's emissive is the edge colour, not a glow everywhere.
        out.emissive = glow * edge;
    } else {
        out.emissive = vec3<f32>(0.0);
        // Ink: the edge is flat black and unlit, whatever the lights do.
        out.color = vec4<f32>(mix(out.color.rgb, vec3<f32>(0.0), edge), out.color.a);
        out.metallic = out.metallic * (1.0 - edge);
        out.roughness = mix(out.roughness, 1.0, edge);
    }
    return out;
}
