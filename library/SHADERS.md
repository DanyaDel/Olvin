# Authoring notes: surface shaders, materials and looks

Pack contents of `library/<pack-id>/assets/` are copied into a project's `assets/`, so every
reference inside a file is written against the final path (`<pack-id>/shaders/toon.wgsl`).

## Surface shader contract (what a library shader may use)

A shader defines `fn surface(in: SurfaceInput) -> SurfaceOutput` (see
`olvin_scene::material::SURFACE_SHADER_CONTRACT`). Besides `in` it can read, because the shader is pasted
in front of the engine's template:

- `globals.camera` (perspective: `(eye, 1)`; orthographic: `(direction to the camera, 0)`), so a view
  vector is `normalize(select(c.xyz, c.xyz - p, c.w > 0.5))`;
- `globals.sun_dir / sun_color`, `globals.lights[]` with `globals.ground.w` lights (what `toon.wgsl` uses to
  light itself), `hemisphere(n)` for ambient;
- `material.params / emissive / flags`: metallic, roughness, emissive of the `.mat.ron` (shaders reuse
  these as parameters, see each file's header);
- `in.time` (seconds), `in.world_position`, `in.uv`, `in.normal`, `in.color`.

Limits that shaped the shaders:

- No custom uniforms and no extra textures: parameters are constants in the file or reused material fields.
- No scene depth, no second draw of the mesh: no inverted-hull outline, no shore foam. `outline-shaders`
  uses the fresnel term, `water-shaders` puts foam on wave crests.
- No shadow lookup from `surface` (it needs the fragment position): `toon.wgsl` ignores shadows.
- Do not sample textures inside `surface` (non-uniform control flow); noise is procedural.
- Transparency is a material flag, `alpha_blend: true` in `MaterialDesc` (added with this library), not
  something the shader decides. Cut-outs use `alpha_cutoff` with alpha 0/1 from the shader.
- Keep Compat friendly: no storage buffers, no extra samplers. Every shader is validated by naga as WGSL
  and converted to GLSL ES 3.00 (WebGL2) in `crates/olvin_render/src/core/tests/library.rs`.

## Tools (`library/tools/`)

- `gen_basic_materials.py`: writes the `basic-materials` `.mat.ron` files.
- `gen_luts.py`: writes the LUT strips of `looks-cinematic` (16 slices of 16x16 = 256x16 PNG).
- `update_provides.py`: refreshes `provides` in every `pack.ron` from `assets/`.
- Looks: `OLVIN_WRITE_LOOKS=1 cargo test -p olvin_scene --features serde looks_pack` rewrites the pack's
  `*.look.ron` from `LookDesc::built_in()`.
- Previews: `OLVIN_LIBRARY_PREVIEWS=<dir> cargo test -p olvin_render --lib render_library_previews`
  renders `<pack>.png` for the packs of this set; copy them to `library/<pack>/preview.png`.

## Looks

A `*.look.ron` is a `LookDesc`: exposure, `filmic`, bloom, FXAA, AO, saturation / contrast / lift / gamma /
gain, a LUT strip and optional fog / height fog (`None` keeps the scene's own fog). Applying a look only sets
those fields of the scene's `Environment`; lighting and sky are untouched. Bloom, grading, LUT, FXAA and AO
run on the Full tier in the game window; Compat and the editor viewport apply exposure, tonemapping and fog only.
