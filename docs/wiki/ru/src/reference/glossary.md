# Глоссарий

| Термин | Значение |
|---|---|
| **Action / действие** | именованный ввод игры (`Jump`, `Move`) в `InputMap`, независимый от клавиши |
| **Asset / ассет** | файл в `assets/` с `.meta`: картинка, модель, материал, префаб, звук, сцена |
| **Behaviour** | поведение объекта на Rust: `start`, `update`, колбэки столкновений |
| **Brush / браш** | выпуклый объём для CSG-уровня; операция Add / Subtract / Intersect |
| **CSG** | constructive solid geometry: уровень из булевых операций над брашами |
| **Compat** | тир рендера для GL/WebGL2 ([Рендер](../engine/rendering.md)) |
| **DefaultPlugins** | набор плагинов движка по включённым фичам |
| **ECS** | entity-component-system; бэкенд `hecs`, типы реэкспортирует `olvin_ecs` |
| **Element / элемент** | плагин редактора: console, profiler, animator, csg, modeling, ide, uv |
| **Environment** | настройки неба, света и постобработки в сцене |
| **Facade / фасад** | крейт `olvin`, реэкспортирует подсистемы по фичам |
| **Full** | тир рендера для Vulkan/Metal/DX12 |
| **F-curve** | кривая анимации свойства объекта (как в Blender) |
| **GUID** | постоянный идентификатор ассета в `.meta` |
| **Headless** | запуск без окна и GPU (`HeadlessPlugins`) |
| **Hub / хаб** | `olvin-hub`: проекты, модули, сборка, установка редактора |
| **Module / модуль** | запись каталога хаба: фича `olvin` или внешний крейт |
| **Override** | отличие экземпляра префаба от ассета |
| **Plugin** | `impl Plugin`: регистрирует ресурсы и системы в `App` |
| **PolyMesh** | редактируемая полигональная сетка (Edit Mode) |
| **Prefab / префаб** | `*.prefab.ron`: сцена с одним корнем для многократного использования |
| **ProjectPaths** | ресурс с корнем проекта и папкой `assets/` |
| **Resource / ресурс** | синглтон приложения по `TypeId` |
| **SceneId** | id объекта сцены, кладётся на сущность при спавне |
| **Stage** | стадия расписания: Startup, First, PreUpdate, Update, PostUpdate, Render, Last |
| **Slice / срез** | `apps/vertical_slice`, готовая игра для проверки движка |
| **Tier / тир** | уровень возможностей рендера: Full или Compat |
| **Trigger / триггер** | коллайдер без столкновения, даёт события входа и выхода |
| **Workspace / рабочая область** | вкладка редактора с раскладкой панелей |

## См. также

- [Что такое Olvin](../start/what-is-olvin.md)
- [Архитектура](../dev/architecture.md)
- [Фичи olvin](olvin-features.md)
- [Lua API](lua-api.md)
