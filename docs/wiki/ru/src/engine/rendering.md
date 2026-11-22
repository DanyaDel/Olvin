# Рендер

`olvin_render` на wgpu 30: PBR, свет, тени, небо, туман, прозрачность, постобработка. Настройки: `RenderSettings` (ресурс, [справочник](../reference/render-settings.md)) и `Environment` сцены.

## Тиры

`RenderTier` определяется по бэкенду при старте (`Renderer::new`):

| Тир | Когда | Что есть |
|---|---|---|
| **Full** | Vulkan, Metal, DX12 | всё: HDR-цепочка, bloom, GTAO, FXAA, LUT, PCSS, 4 каскада |
| **Compat** | GL, GLES 3, WebGL2 | минимум WebGL2: тени работают, не более 2 каскадов, Soft и PCSS откатываются к PCF, лимит spot/point-теней 2 + 1, без постобработки Full |

Устройство запрашивается с `Limits::downlevel_webgl2_defaults()`: 9 вершинных атрибутов, 5 varyings, 2 bind group, 6 текстур. Размер статистики кадра: `RenderStats { draw_calls, instances, culled, triangles }`.

## Конвейер

Инстансинг по (материал, меш), один draw на серию, отсечение по фрустуму, никаких аллокаций в установившемся режиме. Порядок: opaque, небо, прозрачные. 40 тысяч кубов идут за 2 draw call.

## Материалы

`MaterialDesc` (`*.mat.ron`): GGX + Smith + Schlick, normal map, ORM, emissive, alpha cutoff, unlit, triplanar; свой WGSL через контракт `surface()`. `MeshRenderer { material: None }` рисуется `Material::fallback()`. Прозрачность: `BlendMode::{Opaque, Alpha}`: смешиваемые инстансы сортируются от дальних, не пишут глубину и не отбрасывают тень. См. [редактор: материалы](../editor/materials.md).

## Свет

Компонент `Light { kind: Directional | Point | Spot, color, intensity, range, spot_angle_deg, shadows, shadow_bias, size }`, направление вперёд = −Z. Первый направленный свет становится солнцем; остальные: до `MAX_LIGHTS = 16` ближайших после отсечения. `RenderSettings.scene_lights = false` отключает компоненты (Solid-вид редактора).

## Тени

- **Солнце:** до 4 каскадов (`shadow_cascades`) в пределах `shadow_distance` (60 м по умолчанию), атлас 2×2 `Depth32Float`, привязка к текселям, плавные переходы.
- **Фильтры** (`ShadowFilter`): Hard, Pcf 3×3 (по умолчанию), Soft (16 выборок), Pcss (размер источника `Light.size`, только на Full).
- **Spot и point:** общий атлас 4×4 тайла: spot занимает тайл, point шесть (без cube-массивов, поэтому работает на WebGL2). По умолчанию 4 spot + 2 point; лампы ранжируются по важности, лишние остаются без тени. Включить: `Light.shadows`.

## Небо, туман, ambient

`SkyMode::{Color, Procedural{..}, Texture{..}}` (полноэкранный треугольник на дальней плоскости, диск солнца). Hemisphere-ambient: `ambient` (небо) и `ground_ambient`. Линейный туман `Fog { color, start, end }` и высотный `HeightFog { color, density, falloff, height }` (работает на всех тирах и во вьюпорте редактора). ACES-тонмаппинг и экспозиция.

## Постобработка (Full)

Сцена рисуется в `Rgba16Float`, затем: bloom (`Bloom { threshold, intensity }`, выключен по умолчанию), FXAA 3.11 (включён), GTAO (`ambient occlusion`, выключен: самый дорогой эффект), цветокоррекция (`ColorGrade`: LUT, saturation, contrast, lift/gamma/gain). Compat и редактор остаются на пути без неё, тонмаппинг в шейдере меша. Игровой UI рисуется после постобработки, поэтому не получает bloom.

## 2D, спрайты, UI

`Sprite`, `SpriteAtlas`, `Tilemap`, `Camera2d`, `Space2d::{World, Screen}` (экран: пиксели, поверх мира). `Overlay` это экранный слой, в который `olvin_gui` кладёт квады ([Игровой UI](game-ui.md)).

## Частицы

CPU-частицы: компонент `ParticleEmitter { params: EmitterParams, texture, playing }` с формами эмиссии (Point, Sphere, Hemisphere, Cone, Box, Circle), градиентом цвета и размера, билбордами по осям камеры.

> **Статус:** планируется: частицы в формате сцены и редакторе, GPU-частицы ([#11](https://github.com/DanyaDel/Olvin/issues/11)); компоненты частиц и спрайтов из сцены не читаются, во вьюпорте редактора не рисуются. Пресеты (библиотека настроек) в разработке, страница появится вместе с ними.

> **Статус:** не сделано: GPU-скиннинг ([#8](https://github.com/DanyaDel/Olvin/issues/8), скиннинг сейчас на CPU), TAA, кластерное освещение ([#18](https://github.com/DanyaDel/Olvin/issues/18)), IBL и reflection probes ([#19](https://github.com/DanyaDel/Olvin/issues/19)), LOD ([#16](https://github.com/DanyaDel/Olvin/issues/16)), декали ([#20](https://github.com/DanyaDel/Olvin/issues/20)), terrain ([#21](https://github.com/DanyaDel/Olvin/issues/21)), шейдерный граф ([#22](https://github.com/DanyaDel/Olvin/issues/22)).

## GL на NVIDIA под Wayland

Проприетарный NVIDIA + GL + Wayland не показывает кадр (ограничение wgpu). `check_gl` определяет случай: без `WGPU_BACKEND` рендер сам переходит на Vulkan, иначе пишет предупреждение. См. [Чёрное окно и GL](../troubleshooting/black-window.md).

## См. также

- [Настройки рендера](../reference/render-settings.md)
- [Материалы (редактор)](../editor/materials.md)
- [Игровой UI](game-ui.md)
- [Профилировщик](profiler.md)
- [Чёрное окно и GL](../troubleshooting/black-window.md)
