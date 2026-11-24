# Render settings

Two levels: the `RenderSettings` resource (code, `olvin_render`) and `Environment` in a scene (`olvin_scene::file`). `olvin::scene_file::apply_lighting(scene, &mut RenderSettings)` moves the latter into the former when a scene loads. Defaults below are taken from the code.

## RenderSettings

| Field | Default | Meaning |
|---|---|---|
| `vsync` | `true` | Fifo; otherwise Mailbox or Immediate |
| `low_power` | `false` | prefer the power-efficient adapter |
| `clear_color` | (0.02, 0.02, 0.03) | background colour for `SkyMode::Color` |
| `sky` | `SkyMode::Color` | `Color`, `Procedural { zenith, horizon, ground, sun_size }`, `Texture { texture, rotation, intensity }` |
| `light_dir`, `light_color`, `light_shadows` | (-0.4, -1, -0.3), white, `false` | the fallback sun when the scene has no Directional Light |
| `ambient`, `ground_ambient` | (0.15, 0.15, 0.18), (0.07, 0.065, 0.06) | hemisphere ambient: sky and ground |
| `exposure` | `1.0` | exposure |
| `tonemapping` | `Aces` | `None` or `Aces` |
| `fog` | `None` | `Fog { color, start, end }` |
| `height_fog` | `None` | `HeightFog { color, density, falloff, height }` |
| `bloom` | `None` | `Bloom { threshold, intensity }`, Full |
| `post` | see below | `PostSettings { fxaa, ao, grade }`, Full |
| `shadow_distance` | `60.0` | m from the camera |
| `shadow_map_size` | `2048` | sun map resolution |
| `shadow_cascades` | `4` | 1..4 (at most 2 on Compat) |
| `shadow_filter` | `Pcf` | `Hard`, `Pcf`, `Soft`, `Pcss` |
| `shadow_filter_radius` | `1.0` | filter radius |
| `shadow_light_size` | `3.0` | angular size of the fallback sun for PCSS, degrees |
| `max_spot_shadows` / `max_point_shadows` | `4` / `2` | 2 / 1 on Compat |
| `local_shadow_size` | `2048` | spot and point atlas (4×4 tiles) |
| `scene_lights` | `true` | `false`: ignore `Light` components |
| `retro` | off, PS1 values | `RetroSettings`, see [Retro / PS1 style](../engine/retro-ps1.md) |

`PostSettings`: `fxaa: bool` (on), `ao: Option<AmbientOcclusion { radius, strength, bias }>` (off), `grade: ColorGrade { lut, lut_strength, saturation, contrast, lift, gamma, gain }` (a neutral one is skipped).

## Environment (scene)

Written to the file only if it differs from the default; colours are linear.

| Field | Default | Meaning |
|---|---|---|
| `sky`, `ground` | (0.15, 0.15, 0.18), (0.07, 0.065, 0.06) | ambient from above and below |
| `ambient_intensity` | `1.0` | |
| `background` | (0.02, 0.02, 0.03) | background where nothing is drawn |
| `exposure` | `1.0` | |
| `filmic` | `true` | ACES; otherwise clamp |
| `fog` | `None` | `FogDesc { color, start, end }` |
| `height_fog` | `None` | `HeightFogDesc { color, density, falloff, height }` |
| `skybox` | `Sky::Color` | `Color`, `Procedural { .. }`, `Texture { path, rotation_deg, intensity }` |
| `shadow_distance` | `60.0` | |
| `shadow_resolution` | `2048` | 512..4096 |
| `shadow_filter` | `Pcf` | |
| `shadow_cascades` | `4` | 1..4 |
| `max_spot_shadows`, `max_point_shadows` | `4`, `2` | |
| `local_shadow_resolution` | `2048` | |
| `retro` | `None` | `Some(RetroDesc)` = the PS1 style on, see [Retro / PS1 style](../engine/retro-ps1.md) |
| `post` | `PostDesc::default()` | `fxaa`, `ao`, `saturation`, `contrast`, `lift`/`gamma`/`gain`, `lut` (a path), `lut_strength` |

The `Environment::PRESETS`: Day, Sunset, Night, Studio (they set the sky too). New fields have serde defaults, old scenes load without a migration.

## Environment variables and backend

`WGPU_BACKEND=gl` selects GL (the Compat tier); without it Vulkan/Metal/DX12 (Full). See [Environment variables](env-vars.md).

## See also

- [Rendering](../engine/rendering.md)
- [Settings (editor)](../editor/settings.md)
- [Scene format](../engine/scene-format.md)
- [Materials](../editor/materials.md)
