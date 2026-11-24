# 0003. Retro (PS1) render style in the main mesh shader

<p align="center"><a href="0003-retro-render-style.md">Русский</a></p>

**Status:** Accepted — 2026-11-24

## Context

The owner wants a real PlayStation 1 look in the 3D renderer: vertex jitter, affine textures, 15-bit dithered colour, per-vertex lighting, a hard draw distance, nearest textures and optionally a low internal resolution. It has to work on both tiers (Full: Vulkan / Metal / DX12, Compat: GL / WebGL2 with GLSL ES 3.00, so no `noperspective`), in the editor viewport, per material (retro props next to a clean UI or water), and it must cost nothing when it is off.

Constraint from the library contract: a surface shader (`fn surface`) cannot touch vertices or the final colour. Snapping, lighting mode and quantisation therefore live in `mesh.wgsl` itself.

## Decision

A `RetroSettings` struct in `RenderSettings.retro` (global), `Environment.retro: Option<RetroDesc>` in the scene file (`None` = off, skipped when writing, serde defaults: old scenes load unchanged, `FORMAT_VERSION` stays 5), a retro block in `LookDesc`, and `MaterialDesc.retro: Option<bool>` (`None` follows the scene, `Some` forces on / off). The values are always uploaded in three `Globals` vec4s (`retro_a/b/c`); `enabled` and the material flag (`Material.flags.w`: 0 inherit, 1 on, 2 off) decide per material.

- **Vertex snapping.** In `vs_main`, after projection: `xy / w` goes to a grid of `snap_resolution` virtual pixels (default 320x240, independent of the real framebuffer), then is multiplied by `w` again, so depth and perspective are untouched. Vertices with `w <= 1e-4` (behind the camera) are left alone, so there is no NaN. Rounding is deterministic, so the picture does not shimmer. A second variant, `snap_grid_m`, snaps the world position to a metric grid before projection. The default is the screen grid (0 m grid): a 1 m world grid moves whole props in steps of a metre, which only suits large level geometry; it is a slider for tuning.
- **Affine mapping.** The vertex shader passes `uv * w`; the fragment shader multiplies the interpolated value by `position.w` (which is `1 / w`), and that is exactly the screen-linear uv. It needs one extra varying and no `noperspective`, so it is the same on every tier. `affine_strength` blends towards the perspective uv, and the strength fades with `smoothstep(0, affine_fade_near, view depth)` (4 m by default), so near surfaces don't swim. All textures of a material (base, normal, ORM, emissive) share this uv, and surface shaders get the affine `in.uv`. Triplanar materials sample by world position and are not affected. Large polygons warp a lot: PS1 content was tessellated, and the demo pack builds its walls from 2 m cubes for the same reason.
- **Colour depth and dither.** `color_bits` (5 = 15-bit) rounds the display-encoded (sRGB) colour with a 4x4 Bayer matrix in screen space, scaled by `dither_strength` quantisation steps (0 = plain rounding). The rule against doing it twice: **when the post chain is off (Compat, the editor), `finish()` in the mesh and sky shaders does it; when the Full window renderer's HDR chain is on, only the composite pass of `post.wgsl` does it, once, on the final image after grading** (the mesh shader returns linear HDR and skips it). FXAA is skipped in retro mode, it would blur the dither.
- **Gouraud lighting.** `vertex_lighting` evaluates in `vs_main` the hemisphere ambient plus the point / spot lights (Lambert, the existing attenuation, no specular, no sky reflection, no shadows) and the sun's `n.l` as a separate value; the fragment shader multiplies the albedo by the interpolated light. `retro.shadows` adds the sun's shadow map per pixel to the sun term (blocky when the map is coarse); point / spot shadows do not apply. Normal maps are ignored in this mode, metals shade as dielectrics. Vertex colours and `MeshRenderer.color` multiply as before.
- **Draw distance and culling.** `draw_distance` discards fragments past that view depth (optionally dissolving the last tenth with the Bayer pattern) and bends the fog so it is complete exactly there (without fog: the background colour). The CPU culling pass drops objects whose bounding sphere starts beyond the distance, and, with `cull_small_px`, objects whose sphere is smaller than that many pixels of the virtual screen. What is **not** possible: skipping individual small or far triangles. Without geometry shaders or compute a triangle cannot be removed per vertex, and a compute triangle culler is out of scope. Back-face culling stays on.
- **Textures.** Each material keeps a second bind group with the nearest, mip-less sampler; for a retro material with `force_nearest` it is bound at draw time. No re-import needed.
- **Low internal resolution.** Full window renderer only. `PostProcess` runs the whole chain at `RetroSettings::scene_size` (`internal_resolution`, or the window divided by `pixel_scale`; the camera aspect follows the scene size), the composite quantises, and a nearest pass scales the result into the window (`letterbox`: keep the internal aspect with black bars; otherwise the internal height and the window's aspect). The overlay (`olvin_gui`) is drawn afterwards at full resolution. The editor viewport (one egui MSAA pass) and Compat ignore it, and the UI says so.
- **Looks.** The old built-in "Retro PS1" look (a posterising grade) became "PS1": the same grade plus `retro: Some(ps1)`; a look now always sets `Environment.retro` (so Neutral switches it off). `RetroSettings::ps1()` / `RetroDesc::ps1()` is the preset.

## Considered options

- **Separate pipeline variants or `override` constants for retro.** Zero cost in the default path by construction, but it multiplies pipelines (surface shader x blend x retro), and per-material switching needs a draw-time pipeline switch. Rejected: the retro paths are uniform branches (`globals.retro_*`, `material.flags.w`), which all GPUs execute coherently. Measured on llvmpipe (release, 256x256, 400 lit cubes, 8 lights, the machine under load, alternating runs): main 4.32-4.67 ms per frame, this branch 4.42-4.45 ms, so no difference beyond the noise. The default path is also pixel-identical: every existing render test passes untouched.
- **`noperspective` interpolation** for affine mapping: not in GLSL ES 3.00 / WebGL2.
- **Quantising in the post chain only.** The editor and Compat have no chain.
- **Lighting in a separate pass or in the surface shader.** The contract forbids it, and the vertex stage is where Gouraud belongs.
- **Per-triangle culling by compute.** Out of scope, see above.

## Consequences

- The mesh shader has 7 varyings (was 5) and the material uniform is read by the vertex stage as well.
- On the Full chain the colour reduction is a whole-frame effect: a material with Retro Off still gets quantised (the post pass does not know materials). On Compat and in the editor it is per material.
- Particles and 2D sprites are not quantised on Compat.
- A script cannot change a light's intensity, so the demo pack's flickering lamp parks the light out of range for moments instead.
- Open: a snap that follows the internal resolution automatically, tessellating large polygons at import, a retro mode for particles.
