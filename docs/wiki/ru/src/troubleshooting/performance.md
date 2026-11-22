# Медленные кадры

## Диагностика

1. Соберите `--release` (`olvin-hub run --release`). В debug игра на порядки медленнее; зависимости всегда `-O3`, но ваш код нет.
2. Включите профилировщик: фича `profile` и `OLVIN_PROFILE=1`, либо панель Profiler в редакторе ([Профилировщик](../engine/profiler.md)). Таблица «Systems and passes» показывает, какая система или проход тратит время.
3. Смотрите `RenderStats`: `draw_calls`, `instances`, `culled`, `triangles`. Много draw call значит много разных (материал, меш) пар.
4. Проверьте бэкенд: GL / llvmpipe это CPU-растеризатор; Vulkan должен давать тир Full.

## Типичные причины

| Причина | Что делать |
|---|---|
| vsync | при `vsync: true` кадр упирается в частоту монитора, это не медленно |
| тени | уменьшите `shadow_distance`, `shadow_cascades`, `shadow_map_size`; фильтр Pcf дешевле Soft и PCSS |
| GTAO, bloom | выключены по умолчанию; GTAO самый дорогой эффект |
| много ламп | до 16 источников на кадр, дальние отсекаются; теней spot/point по умолчанию 4 + 2 |
| много разных материалов | каждый (материал, меш) это отдельный draw; делите меньше |
| Lua | горячий код на `*_xyz` без таблиц; лимит 5 млн инструкций на колбэк |
| физика | много динамических тел и mesh-коллайдеры; триггеры дешевле |
| скиннинг | скиннинг CPU, один слот меша на экземпляр, GPU-скиннинга нет ([#8](https://github.com/DanyaDel/Olvin/issues/8)) |
| повторный спавн сцен | `Meshes` не освобождает слоты, GPU-память растёт |

Эталон: Vulkan, RTX 3050: 40 тыс. кубов за 2 draw call, упор в vsync; llvmpipe: 10 тыс. кубов около 90 fps.

> **Статус:** бенчмарки в CI: [#68](https://github.com/DanyaDel/Olvin/issues/68). LOD и GPU-culling: [#16](https://github.com/DanyaDel/Olvin/issues/16), [#17](https://github.com/DanyaDel/Olvin/issues/17).

## См. также

- [Профилировщик](../engine/profiler.md)
- [Рендер](../engine/rendering.md)
- [Настройки рендера](../reference/render-settings.md)
- [Консоль и профилировщик](../editor/console-profiler.md)
