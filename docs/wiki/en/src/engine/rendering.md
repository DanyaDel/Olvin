# Rendering

`olvin_render` on wgpu 30: PBR, lights, shadows, sky, fog, transparency, post-processing. Settings: `RenderSettings` (a resource, [reference](../reference/render-settings.md)) and the scene's `Environment`.

## Tiers

`RenderTier` is determined from the backend at startup (`Renderer::new`):

| Tier | When | What is there |
|---|---|---|
| **Full** | Vulkan, Metal, DX12 | everything: HDR chain, bloom, GTAO, FXAA, LUT, PCSS, 4 cascades |
| **Compat** | GL, GLES 3, WebGL2 | the WebGL2 minimum: shadows work, at most 2 cascades, Soft and PCSS fall back to PCF, spot/point shadow limit 2 + 1, no Full post-processing |

The device is requested with `Limits::downlevel_webgl2_defaults()`: 9 vertex attributes, 5 varyings, 2 bind groups, 6 textures. Frame statistics: `RenderStats { draw_calls, instances, culled, triangles }`.

## Pipeline

Instancing by (material, mesh), one draw per run, frustum culling, no allocations in steady state. Order: opaque, sky, transparent. 40 thousand cubes go in 2 draw calls.

## Materials

`MaterialDesc` (`*.mat.ron`): GGX + Smith + Schlick, normal map, ORM, emissive, alpha cutoff, unlit, triplanar; a custom WGSL through the `surface()` contract. `MeshRenderer { material: None }` is drawn with `Material::fallback()`. Transparency: `BlendMode::{Opaque, Alpha}`: blended instances are sorted far to near, do not write depth and do not cast shadows. See [editor: materials](../editor/materials.md).

## Lights

The component `Light { kind: Directional | Point | Spot, color, intensity, range, spot_angle_deg, shadows, shadow_bias, size }`, forward is −Z. The first directional light becomes the sun; the others: up to `MAX_LIGHTS = 16` nearest after culling. `RenderSettings.scene_lights = false` disables the components (the editor's Solid view).

## Many lights

The fragment shader tries the 16 most important lights directly (they also carry the shadows). Every light past them goes into **view clusters**: a grid of 16 × 9 screen tiles and 24 depth slices (exponential, from 0.1 m to 250 m), built on the CPU each frame. A fragment finds its cluster and loops only over the lights that reach it, so a scene with hundreds of lamps costs about what its closest few cost. Up to 1024 extra lights are kept (a cluster lists at most 255 and the lists together at most 65 536 entries; what does not fit is dropped and logged). The data goes through three textures read with `textureLoad`, not storage buffers, so Compat devices (GL, WebGL2) work too. The most important are chosen by distance from the camera minus range; lights outside the view are skipped. Retro per-vertex lighting still uses only the best 16.

## Shadows

- **Sun:** up to 4 cascades (`shadow_cascades`) within `shadow_distance` (60 m by default), a 2×2 `Depth32Float` atlas, texel snapping, smooth transitions.
- **Filters** (`ShadowFilter`): Hard, Pcf 3×3 (default), Soft (16 samples), Pcss (source size `Light.size`, Full only).
- **Spot and point:** a shared 4×4-tile atlas: a spot takes a tile, a point six (no cube arrays, so it works on WebGL2). 4 spot + 2 point by default; lights are ranked by importance and the rest get no shadow. Enable with `Light.shadows`.

## Sky, fog, ambient

`SkyMode::{Color, Procedural{..}, Texture{..}}` (a full-screen triangle at the far plane, a sun disc). Hemisphere ambient: `ambient` (sky) and `ground_ambient`. Linear fog `Fog { color, start, end }` and height fog `HeightFog { color, density, falloff, height }` (works on all tiers and in the editor viewport). ACES tonemapping and exposure.

## Levels of detail

A far object does not need a million triangles. `meshes.add_with_lods(data, &DEFAULT_LODS)` registers a mesh with simpler copies made by meshoptimizer (half the triangles, then a fifth); the handle it returns is the full mesh, and a `MeshRenderer` points at it as usual. The renderer picks a copy per object by how big it looks: the fraction of the screen's height that its bounding sphere covers (a quarter of it and below: the first copy, a twelfth and below: the second), so the choice is the same at any resolution or field of view. Meshes with fewer than 120 triangles get no copies. `RenderSettings::lod_bias` scales the size (above 1 keeps detail longer, infinity always draws the full mesh). The glTF models of a scene get copies automatically; skinned meshes, CSG levels and primitives do not. Replacing a mesh's geometry with `Meshes::set` drops its copies.

Instances of the same mesh and material are already drawn in one instanced call; culling on the GPU and indirect draws are not done yet.

## Retro style

A PlayStation 1 look (vertex jitter, affine textures, 15-bit dithered colour, Gouraud lighting, draw distance, low resolution) lives in the mesh shader and switches on per scene or per material: [Retro / PS1 style](retro-ps1.md).

## Post-processing (Full)

The scene is drawn into `Rgba16Float`, then: bloom (`Bloom { threshold, intensity }`, off by default), FXAA 3.11 (on), GTAO (ambient occlusion, off: the most expensive effect), colour grading (`ColorGrade`: LUT, saturation, contrast, lift/gamma/gain). Compat and the editor stay on the path without it, with tonemapping in the mesh shader. The game UI is drawn after post-processing, so it gets no bloom.

## 2D, sprites, UI

`Sprite`, `SpriteAtlas`, `Tilemap`, `Camera2d`, `Space2d::{World, Screen}` (screen: pixels, over the world). `Overlay` is the screen layer into which `olvin_gui` puts quads ([Game UI](game-ui.md)).

## Particles

CPU particles: the component `ParticleEmitter { params: EmitterParams, texture, playing }` with emission shapes (Point, Sphere, Hemisphere, Cone, Box, Circle), colour and size gradients, billboards along the camera axes.

> **Status:** planned: particles in the scene format and the editor, GPU particles ([#11](https://github.com/DanyaDel/Olvin/issues/11)); particle and sprite components are not read from scenes and are not drawn in the editor viewport. Presets (a settings library) are in development, a page will appear with them.

> **Status:** not done: GPU skinning ([#8](https://github.com/DanyaDel/Olvin/issues/8), skinning is on the CPU now), TAA, clustered lighting ([#18](https://github.com/DanyaDel/Olvin/issues/18)), IBL and reflection probes ([#19](https://github.com/DanyaDel/Olvin/issues/19)), LOD ([#16](https://github.com/DanyaDel/Olvin/issues/16)), decals ([#20](https://github.com/DanyaDel/Olvin/issues/20)), terrain ([#21](https://github.com/DanyaDel/Olvin/issues/21)), shader graph ([#22](https://github.com/DanyaDel/Olvin/issues/22)).

## GL on NVIDIA under Wayland

Proprietary NVIDIA + GL + Wayland does not show a frame (a wgpu limitation). `check_gl` detects the case: without `WGPU_BACKEND` the renderer switches to Vulkan itself, otherwise it warns. See [Black window and GL](../troubleshooting/black-window.md).

## See also

- [Render settings](../reference/render-settings.md)
- [Materials (editor)](../editor/materials.md)
- [Game UI](game-ui.md)
- [Profiler](profiler.md)
- [Black window and GL](../troubleshooting/black-window.md)
