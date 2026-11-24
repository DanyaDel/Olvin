# Retro / PS1 style

A PlayStation 1 look in the 3D renderer: wobbling vertices, swimming textures, dithered 15-bit colour, per-vertex lighting, hard fog and draw distance, nearest textures, optionally a low resolution. Why it is built this way: [ADR 0003](https://github.com/DanyaDel/Olvin/blob/main/docs/adr/0003-retro-render-style.en.md).

## In 3 steps

1. **Switch it on.** Settings > Rendering > **Retro / PS1** > *PS1 preset* (or apply the *PS1* look under *Look*). In a scene file this is `environment: (retro: Some(( .. )))`; `None` (the default) means off.
2. **Give the scene fog.** The draw distance (60 m in the preset) hides pop-in with fog; set the fog and the background to the same colour. Dark, close rooms read as PS1 best.
3. **Check the materials.** Materials with *Retro* on *Follow scene* are retro too. Use *On* to keep a material retro in a scene that has the style off, *Off* to keep a UI or water material clean. The `retro-ps1` library pack has eight ready materials and a demo scene.

In code: `settings.retro = RetroSettings::ps1();` and `Material { retro: Some(true), .. }`.

## What each knob does

| Knob | Effect |
|---|---|
| `snap_resolution` (320x240) | vertices snap to the pixel grid of this virtual screen after projection: the jitter; works on any real resolution. 0 = off |
| `snap_grid_m` (0) | also snap world positions to a grid of this many metres. Moves whole objects in steps: only for coarse level geometry |
| `affine_strength` (1) | 0 = normal textures, 1 = PS1 affine mapping: textures slide on big polygons seen at an angle |
| `affine_fade_near` (4 m) | closer than this the effect fades out, so near surfaces don't tear |
| `color_bits` (5) | bits per channel, 5 = the PS1's 15-bit colour; 0 or 8 = full colour |
| `dither_strength` (1) | the 4x4 ordered dither in quantisation steps; 0 = banding |
| `vertex_lighting` (on) | lights are computed per vertex (Gouraud), diffuse only: the light centre of a large polygon is dark |
| `shadows` (off) | the sun's shadow map per pixel in vertex-lit mode (blocky when coarse) |
| `draw_distance` (60 m) | nothing is drawn past this depth; the fog is made to finish there. 0 = off |
| `dissolve` (off) | dither the last tenth before the cut |
| `cull_small_px` (1) | objects smaller than this many virtual-screen pixels are skipped |
| `force_nearest` (on) | retro materials use nearest filtering without mipmaps, no re-import needed |
| `internal_resolution` (320x240) / `pixel_scale` | the scene is drawn this small and scaled up with nearest filtering; `pixel_scale` = window divided by N. `letterbox` keeps the internal aspect with black bars, otherwise the internal height and the window's aspect |

Match `snap_resolution` to `internal_resolution`. Good starting points tuned on a previous PS1 project: affine fade 4 m, vertex snapping on the virtual screen.

## Limits

- Large polygons warp a lot under affine mapping, as on the real console. Build level geometry from small pieces (the demo pack uses 2 m cubes).
- Vertex lighting ignores normal maps, specular and sky reflections; metals shade as dielectrics. Point and spot shadows do not apply to it.
- Triplanar materials sample by world position, so the affine warp does not touch them.
- Surface shaders get the affine `in.uv`, but cannot change vertices.
- There is no per-triangle culling (no geometry shaders, no compute culler): only whole objects are skipped.
- The draw distance and fog look best with the fog colour equal to the background.

## Compat or Full

| | Compat (GL, WebGL2) | Full (Vulkan, Metal, DX12) |
|---|---|---|
| Snapping, affine, vertex lighting, draw distance, nearest | yes | yes |
| Colour depth and dither | in the mesh shader, per material | once on the final image in the post chain (all materials), no FXAA |
| Low internal resolution | no, ignored | game window only |
| Editor viewport | everything except the low resolution | everything except the low resolution |
| Particles and sprites | not quantised | quantised with the rest |

The editor viewport is one egui pass with MSAA, so it cannot render at a lower resolution; the game window can.

## See also

- [Rendering](rendering.md)
- [Materials (editor)](../editor/materials.md)
- [Render settings](../reference/render-settings.md)
