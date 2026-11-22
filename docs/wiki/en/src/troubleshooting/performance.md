# Slow frames

## Diagnosis

1. Build `--release` (`olvin-hub run --release`). Debug is orders of magnitude slower; dependencies are always `-O3`, your code is not.
2. Turn on the profiler: the `profile` feature and `OLVIN_PROFILE=1`, or the Profiler panel in the editor ([Profiler](../engine/profiler.md)). The "Systems and passes" table shows which system or pass takes the time.
3. Look at `RenderStats`: `draw_calls`, `instances`, `culled`, `triangles`. Many draw calls mean many distinct (material, mesh) pairs.
4. Check the backend: GL / llvmpipe is a CPU rasterizer; Vulkan should give the Full tier.

## Typical causes

| Cause | What to do |
|---|---|
| vsync | with `vsync: true` the frame is bound to the monitor rate, this is not slow |
| shadows | reduce `shadow_distance`, `shadow_cascades`, `shadow_map_size`; Pcf is cheaper than Soft and PCSS |
| GTAO, bloom | off by default; GTAO is the most expensive effect |
| many lights | up to 16 sources per frame, far ones are culled; spot/point shadows are 4 + 2 by default |
| many distinct materials | each (material, mesh) is a separate draw; split less |
| Lua | hot code on `*_xyz` without tables; limit 5 million instructions per callback |
| physics | many dynamic bodies and mesh colliders; triggers are cheaper |
| skinning | skinning is on the CPU, one mesh slot per instance, no GPU skinning ([#8](https://github.com/DanyaDel/Olvin/issues/8)) |
| spawning scenes repeatedly | `Meshes` does not free slots, GPU memory grows |

Reference: Vulkan, RTX 3050: 40k cubes in 2 draw calls, bound by vsync; llvmpipe: 10k cubes at about 90 fps.

> **Status:** benchmarks in CI: [#68](https://github.com/DanyaDel/Olvin/issues/68). LOD and GPU culling: [#16](https://github.com/DanyaDel/Olvin/issues/16), [#17](https://github.com/DanyaDel/Olvin/issues/17).

## See also

- [Profiler](../engine/profiler.md)
- [Rendering](../engine/rendering.md)
- [Render settings](../reference/render-settings.md)
- [Console and profiler](../editor/console-profiler.md)
