# Архитектура: обзор

Короткая карта. Полное описание каждого крейта, формата и решения: [ARCHITECTURE.md](https://github.com/DanyaDel/Olvin/blob/main/ARCHITECTURE.md) (русская основная версия, [английская](https://github.com/DanyaDel/Olvin/blob/main/ARCHITECTURE.en.md)). Читайте его вместо сканирования кода.

## Слои

```
apps/ (sandbox, vertical_slice, editor_dev)     игры и примеры
editor/ (olvin_editor + элементы)  hub/         инструменты, не часть движка
crates/olvin                                    фасад: DefaultPlugins, prelude, features
crates/                                         подсистемы-плагины
  olvin_core      App, Plugin, Resources, Schedule, Time, Events, ProjectPaths, logging, profile
  olvin_ecs       hecs
  olvin_scene     Transform, Camera, формат сцены, префабы, Action
  olvin_transform иерархия
  olvin_window    winit, Input, runner
  olvin_render    wgpu: PBR, тени, post, частицы, спрайты, overlay
  olvin_physics   Rapier
  olvin_audio     kira
  olvin_input     карты действий
  olvin_script    Behaviour, Lua
  olvin_game      состояния, загрузка сцен, сохранения
  olvin_gui, olvin_text   игровой UI и текст
  olvin_assets, olvin_anim, olvin_csg, olvin_mesh   ассеты, анимация, CSG, меши
  olvin_ui        общий UI-кит хаба и редактора (egui)
```

## Принципы

- Плагин регистрирует системы; подсистемы общаются через ресурсы, а не напрямую.
- Ядро не знает про GPU и окно (`SurfaceTarget`).
- Минимум железа: WebGL2/GLES3, остальное через тир `Full`.
- Данные (`*Desc`, serde) отделены от рантайма: сцена, редактор и игра делят одни типы.
- Плагины редактора линкуются статически.

## Где что искать

| Вопрос | Страница |
|---|---|
| Как устроен кадр, расписание, события | [Время и события](../engine/time-events.md) |
| Что в сцене | [Формат сцены](../engine/scene-format.md) |
| Как рендерится | [Рендер](../engine/rendering.md) |
| Как работают скрипты | [Скрипты](../engine/scripting.md) |
| Как редактор подключает элементы | [Руководство контрибьютора](contributing.md#добавить-элемент-редактора) |

Состояние кода в ARCHITECTURE.md отмечено датой и коммитом; расхождение с кодом это баг документации, заводите issue.

## См. также

- [План (ROADMAP)](roadmap.md)
- [Руководство контрибьютора](contributing.md)
- [Что такое Olvin](../start/what-is-olvin.md)
- [Глоссарий](../reference/glossary.md)
