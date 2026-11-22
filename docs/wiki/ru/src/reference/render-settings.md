# Настройки рендера

Два уровня: ресурс `RenderSettings` (код, `olvin_render`) и `Environment` в сцене (`olvin_scene::file`). `olvin::scene_file::apply_lighting(scene, &mut RenderSettings)` переносит второе в первое при загрузке сцены. Значения по умолчанию ниже взяты из кода.

## RenderSettings

| Поле | По умолчанию | Смысл |
|---|---|---|
| `vsync` | `true` | Fifo; иначе Mailbox или Immediate |
| `low_power` | `false` | предпочесть энергоэффективный адаптер |
| `clear_color` | (0.02, 0.02, 0.03) | цвет фона для `SkyMode::Color` |
| `sky` | `SkyMode::Color` | `Color`, `Procedural { zenith, horizon, ground, sun_size }`, `Texture { texture, rotation, intensity }` |
| `light_dir`, `light_color`, `light_shadows` | (-0.4, -1, -0.3), белый, `false` | запасное солнце, когда в сцене нет Directional Light |
| `ambient`, `ground_ambient` | (0.15, 0.15, 0.18), (0.07, 0.065, 0.06) | полусферический ambient: небо и земля |
| `exposure` | `1.0` | экспозиция |
| `tonemapping` | `Aces` | `None` или `Aces` |
| `fog` | `None` | `Fog { color, start, end }` |
| `height_fog` | `None` | `HeightFog { color, density, falloff, height }` |
| `bloom` | `None` | `Bloom { threshold, intensity }`, Full |
| `post` | см. ниже | `PostSettings { fxaa, ao, grade }`, Full |
| `shadow_distance` | `60.0` | м от камеры |
| `shadow_map_size` | `2048` | разрешение карты солнца |
| `shadow_cascades` | `4` | 1..4 (на Compat не более 2) |
| `shadow_filter` | `Pcf` | `Hard`, `Pcf`, `Soft`, `Pcss` |
| `shadow_filter_radius` | `1.0` | радиус фильтра |
| `shadow_light_size` | `3.0` | угловой размер запасного солнца для PCSS, градусы |
| `max_spot_shadows` / `max_point_shadows` | `4` / `2` | на Compat 2 / 1 |
| `local_shadow_size` | `2048` | атлас spot и point (4×4 тайла) |
| `scene_lights` | `true` | `false`: игнорировать компоненты `Light` |

`PostSettings`: `fxaa: bool` (включён), `ao: Option<AmbientOcclusion { radius, strength, bias }>` (выключен), `grade: ColorGrade { lut, lut_strength, saturation, contrast, lift, gamma, gain }` (нейтральный пропускается).

## Environment (сцена)

Пишется в файл, только если отличается от умолчания; цвета линейные.

| Поле | По умолчанию | Смысл |
|---|---|---|
| `sky`, `ground` | (0.15, 0.15, 0.18), (0.07, 0.065, 0.06) | ambient сверху и снизу |
| `ambient_intensity` | `1.0` | |
| `background` | (0.02, 0.02, 0.03) | фон там, где ничего не нарисовано |
| `exposure` | `1.0` | |
| `filmic` | `true` | ACES; иначе обрезка |
| `fog` | `None` | `FogDesc { color, start, end }` |
| `height_fog` | `None` | `HeightFogDesc { color, density, falloff, height }` |
| `skybox` | `Sky::Color` | `Color`, `Procedural { .. }`, `Texture { path, rotation_deg, intensity }` |
| `shadow_distance` | `60.0` | |
| `shadow_resolution` | `2048` | 512..4096 |
| `shadow_filter` | `Pcf` | |
| `shadow_cascades` | `4` | 1..4 |
| `max_spot_shadows`, `max_point_shadows` | `4`, `2` | |
| `local_shadow_resolution` | `2048` | |
| `post` | `PostDesc::default()` | `fxaa`, `ao`, `saturation`, `contrast`, `lift`/`gamma`/`gain`, `lut` (путь), `lut_strength` |

Пресеты `Environment::PRESETS`: Day, Sunset, Night, Studio (задают и небо). Новые поля имеют serde-умолчания, старые сцены грузятся без миграции.

## Переменные окружения и бэкенд

`WGPU_BACKEND=gl` включает GL (тир Compat); без него Vulkan/Metal/DX12 (Full). См. [Переменные окружения](env-vars.md).

## См. также

- [Рендер](../engine/rendering.md)
- [Настройки (редактор)](../editor/settings.md)
- [Формат сцены](../engine/scene-format.md)
- [Материалы](../editor/materials.md)
