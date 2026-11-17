# Olvin: архитектура

Модульный игровой движок на Rust. Каждая подсистема живёт в отдельном крейте и подключается как плагин. Подсистемы общаются только через общие ресурсы и не зависят друг от друга напрямую.

## Принципы

- **Модульность на уровне подсистем, а не объектов.** Плагин регистрирует системы, система проходит по всем данным за один вызов. Нет виртуальных вызовов на каждый объект.
- **Горячие пути работают с плоскими массивами.** Рендер раскладывает инстансы по мешам в переиспользуемые `Vec` и делает один instanced draw на меш. В установившемся режиме кадр ничего не аллоцирует.
- **Ядро ничего не знает про GPU и окно.** Рендер и платформа связаны через `SurfaceTarget` (raw-window-handle), поэтому winit можно заменить на SDL или канвас, не трогая рендер.
- **Минимальный уровень железа — WebGL2 / GLES 3.0.** Устройство запрашивается с `Limits::downlevel_webgl2_defaults()`. Всё, что сильнее, включается по `RenderTier::Full`.

## Структура

```
Cargo.toml            workspace, общие версии зависимостей, профили (deps всегда -O3)
crates/
  olvin_core/         App, Plugin, Resources, Schedule/Stage, Time, SurfaceTarget
  olvin_ecs/          фасад над hecs + EcsPlugin (ресурс World)
  olvin_scene/        Transform, Camera — чистые данные на glam
  olvin_window/       winit: окно, Input, главный цикл (runner)
  olvin_render/       wgpu: меши, instancing, frustum culling, уровни железа
  olvin/              фасад: реэкспорты, DefaultPlugins, prelude, features window/render
apps/
  sandbox/            стресс-демо: N вращающихся кубов + свободная камера
editor/               olvin-editor — ставится отдельно через хаб
  olvin_editor/        ядро редактора: оболочка, рабочие области, докинг, базовые панели
  olvin_editor_console/  элемент «Консоль»
  olvin_editor_animator/ элемент «Аниматор»
hub/                  olvin-hub — отдельный инструмент, не часть движка
  olvin_hub/          ядро: каталог модулей, проекты игр, правка Cargo.toml, конфиг
  olvin_hub_cli/      бинарник `olvin-hub` (clap)
  olvin_hub_gui/      бинарник `olvin-hub-gui` (eframe/egui 0.35)
```

## olvin_core

- `app.rs`: `App` хранит `Resources`, `Schedule`, набор плагинов и runner. `add_plugin` пропускает повторы того же типа, поэтому плагин может сам добавить свои зависимости. `run()` отдаёт управление runner'у, без него крутится headless-цикл. Есть `startup()` (Stage::Startup, один раз) и `update()` (один кадр). `AppExit { requested }` останавливает цикл.
- `resources.rs`: `Resources` — синглтоны по `TypeId` в `RefCell`. Методы: `get/get_mut` возвращают Option, `fetch/fetch_mut` паникуют с именем типа. Разные ресурсы можно одновременно брать на запись, один и тот же нельзя.
- `schedule.rs`: стадии `Startup, First, PreUpdate, Update, PostUpdate, Render, Last`. Система — это `FnMut(&mut Resources)`. Пока всё однопоточное.
- `time.rs`: `Time` (delta ограничен 0.25 с, elapsed, frame) обновляется в `First`.
- `surface.rs`: `SurfaceTarget { handle: Arc<dyn WindowHandleSource>, size }` — контракт между платформой и рендером.

## olvin_ecs

Реэкспорт типов hecs (`World`, `Entity`, `Query`, `CommandBuffer`…). Остальной код импортирует их только отсюда, чтобы бэкенд ECS можно было заменить в одном месте. `EcsPlugin` вставляет `World`.

## olvin_scene

`Transform` (translation/rotation/scale, `matrix()`, `looking_at`, `forward/right/up`) и `Camera` (perspective, правая система координат, глубина 0..1 через `glam::camera::rh::proj::directx`). Нет зависимостей от ECS и GPU.

## olvin_window

- `lib.rs`: `WindowPlugin { title, width, height, resizable }` ставит runner на winit 0.30 (`ApplicationHandler`). Окно создаётся в `resumed`, потому что так требует Android. Там же вставляется `SurfaceTarget` и вызывается `app.startup()`. Кадр — это `RedrawRequested → app.update()`. На Resize обновляется `SurfaceTarget.size`.
- `input.rs`: `Input` хранит нажатые клавиши и кнопки мыши, `just_pressed/just_released`, позицию курсора и сырую `mouse_delta` из `DeviceEvent`. Сброс в `Stage::Last`, при потере фокуса всё отпускается.

## olvin_render

- `lib.rs`: `RenderPlugin { settings }`, `RenderSettings` (vsync, low_power, clear_color, light_dir, ambient), компонент `MeshRenderer { mesh, color }`, `Color` (линейный RGBA). Плагин добавляет `EcsPlugin`, ресурсы `Meshes` и `RenderStats`, системы `init` (Startup) и `render` (Render).
- `renderer.rs`: `Renderer` создаёт wgpu Instance с display handle (для GLES/Wayland), surface, adapter и device с лимитами WebGL2. Определяет `RenderTier`, при vsync выбирает Fifo, иначе Mailbox или Immediate. Кадр: загрузка новых мешей → проход по `(Transform, MeshRenderer)` с отсечением по сфере → бакеты по мешу → один буфер инстансов (растёт до степени двойки) → `draw_indexed` на меш. Результат в `RenderStats { draw_calls, instances, culled }`, сведения об адаптере в `RenderInfo`.
- `mesh.rs`: `Vertex { position, normal }`, `MeshData` (`cube`, `plane`, `bounding_sphere`). `Meshes::add` возвращает `MeshHandle` ещё до появления GPU, загрузка ленивая.
- `frustum.rs`: 6 плоскостей из view-proj (Gribb–Hartmann, глубина 0..1), тест сферы.
- `mesh.wgsl`: instanced-шейдер в пределах WebGL2: один uniform, 7 атрибутов (матрица модели — 4 vec4 + цвет), Ламберт + ambient.

## olvin_scene::file (feature `serde`)

Формат сцен `*.scene.ron`, общий для редактора и игр. `SceneFile { entities, extensions }`, `EntityDesc` (стабильный `id`, `parent`, позиция, поворот в градусах по Эйлеру YXZ, масштаб, `mesh: Shape+цвет`, `camera`). Родители в списке всегда идут раньше детей. `world_matrices[_with]` разворачивает иерархию в мировые матрицы. `extensions` — данные элементов редактора (клипы аниматора).

## olvin (фасад)

Features `window` и `render` включены по умолчанию. `scene` добавляет `scene_file::{load, spawn}`: загрузка сцены редактора в `World`. Иерархия при загрузке разворачивается в мировые трансформы, иерархии в рантайме пока нет. Без них получается чистое ядро + ECS (сервер, инструменты). `DefaultPlugins` добавляет всё включённое. Для игр есть `prelude`.

## editor (olvin-editor)

Редактор сцен на egui 0.35 + egui_dock 0.20, стиль Blender + Unity. Ставится отдельно, отдельные элементы подключаются к нему как крейты-плагины. Хаб генерирует крейт установки, в его `main.rs` перечислены плагины. Плагины линкуются статически, без dylib: так быстрее и без проблем с ABI.

- `lib.rs`: `run(plugins)` — первый аргумент командной строки это папка проекта. Реэкспорт egui, egui_dock, glam, olvin_scene, чтобы плагины собирались строго на тех же версиях.
- `registry.rs`: трейты `Panel` (`ui(ui, &mut EditorState)`, по экземпляру на вкладку) и `EditorPlugin` (`build(&mut Registry)`). `PanelKind { id, title, factory }`, `WorkspaceTemplate { name, build }`, `TabAlloc`.
- `state.rs`: `EditorState` — сцена, путь, проект, выделение, `preview` (переопределение поз, не сохраняется), лог, `scene_generation`. Undo/redo снимками: изменения коммитятся, когда отпущены мышь и клавиатура, так что перетаскивание = один шаг. Операции: `create/delete/duplicate/reparent` (reparent сохраняет мировую позицию и запрещает циклы), `open/save/save_as/new_scene`, `request_open`.
- `workspace.rs`: `Tab { kind, id }`, `Workspace { name, dock: DockState<Tab> }`, `Layout` (вкладки рабочих областей, сохраняется в `<config>/olvin-editor/layout.ron` при выходе). Дублирование выдаёт новые id вкладок, у копий независимое состояние панелей.
- `app.rs`: меню (Файл/Правка/Объект/Окно), вкладки рабочих областей как в браузере (клик, двойной клик — переименовать, СКМ или × — закрыть, ПКМ — дублировать или сдвинуть, «+» — шаблон, пустая или копия), DockArea (перетаскивание, разделение, всплывающие окна, «+» в каждой группе вкладок), горячие клавиши (Ctrl+S/N/Z/Shift+Z/Y/D, Del/X), подтверждение при несохранённых изменениях, статус-бар, заголовок окна.
- `theme.rs`: серые тона Blender, синее выделение Unity, оранжевый для выбранного во вьюпорте, цвета осей.
- `panels/viewport.rs`: программная отрисовка средствами egui: трансформ на CPU, клиппинг по near (Sutherland–Hodgman), сортировка по дальней вершине, плоскость бьётся на тайлы 8×8. Орбитальная камера (СКМ/ПКМ вращать, Shift — сдвиг, колесо, F, 1/3/7), сетка, каркас, контур выделения, иконки камер, стрелки перемещения (с учётом родителя), выбор кликом, индикатор осей. На GPU не переведён, потому что eframe 0.35 сидит на wgpu 29, а движок на 30.
- `panels/hierarchy.rs`: дерево с drag&drop для смены родителя, поиск, контекстное меню, двойной клик — переименование.
- `panels/inspector.rs`: имя, Transform (XYZ в цветах осей), Mesh Renderer, Камера, добавить или убрать компонент.
- `panels/assets.rs`: браузер `assets/` проекта: хлебные крошки, плитки, фильтр, создание папки или сцены, двойной клик открывает сцену.
- `olvin_editor_console`: панель лога с фильтрами по уровню.
- `olvin_editor_animator`: таймлайн. Клип `{ length, tracks: entity id → ключи }` хранится в `scene.extensions["animator"]`. Есть ключ по кнопке, перетаскивание ключей, ПКМ — удалить, скраб, play/loop, превью через `state.preview`. Добавляет шаблон рабочей области «Анимация».

## hub (olvin-hub, рабочее имя)

Хаб создаёт проекты игр и ставит в них модули. Источник правды о модулях — `Cargo.toml` игры, у хаба нет своего состояния об установленном.

- `olvin_hub/catalog.toml`: встроенный каталог. `kind = "olvin"` — фича фасада `olvin` (плагин подключит `DefaultPlugins`). `kind = "crate"` — зависимость с crates.io (rapier3d, kira, gltf, rand, serde, ron). Поле `requires` ставит модули вместе.
- `catalog.rs`: `Catalog` (парсинг, проверка `requires`, `resolve` — сначала зависимости, ловит циклы, `categories`), `Module`, `Source`.
- `project.rs`: `Project::create` (шаблон + модули с `default = true`), `open`, `discover` (ищет вверх по папкам), `add` (вместе с requires), `remove` (не даёт удалить то, от чего зависят другие, если нет `force`), `save`. Правки через `toml_edit`, комментарии и форматирование пользователя сохраняются. `olvin` подключается с `default-features = false`, список модулей Olvin = `features`.
- `config.rs`: `HubConfig` в `<config_dir>/olvin-hub/config.toml` (на Linux учитывает `XDG_CONFIG_HOME`): список проектов и `EngineSource` (`path` к checkout'у или `git`). По умолчанию это папка, из которой собран хаб, если её нет — GitHub.
- `template.rs`: Cargo.toml (deps всегда -O3), `src/main.rs` (вращающийся куб, Esc выход), README, .gitignore.
- `editor.rs`: `EditorInstall` в `<data_dir>/olvin-hub/editor` — сгенерированный крейт (`Cargo.toml` + `main.rs` с плагинами, бинарник `olvin-editor`). Источник правды — его `Cargo.toml`. Методы: `write`, `build_command` (`cargo build --release`), `launch(project)`. Элементы редактора описаны в каталоге как `[[editor_module]]`.
- CLI: `new`, `list`, `add`, `remove [--force]`, `run [--release]`, `projects`, `register`, `forget`, `engine [--path|--git]`, `editor install|list|add|remove|open [проект]`.
- GUI: слева проекты, создание и добавление существующего, раздел «Редактор» (элементы галочками, установить или пересобрать, открыть); справа каталог с галочками по категориям, «Открыть в редакторе», запуск или остановка `cargo run`, статус-бар.

## Зависимости

wgpu 30, winit 0.30, hecs 0.11, glam 0.34 (проекции в `glam::camera`), bytemuck, pollster, log/env_logger.
Хаб: toml_edit, toml, serde, clap, dirs, anyhow, eframe 0.35 (0.36+ требует rustc 1.95).
Редактор: eframe/egui 0.35, egui_dock 0.20 (0.21 требует egui 0.36), ron, dirs.

## Проверено

- Vulkan, RTX 3050: тир Full, 40k кубов за 2 draw call, упор в vsync.
- GL через Mesa llvmpipe (`__EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json WGPU_BACKEND=gl`): тир Compat, 10k кубов, около 90 fps на CPU-растеризаторе.
- GL на проприетарном NVIDIA под Wayland не работает: EGL не даёт native-renderable конфиг, и wgpu-hal отказывается презентовать. Это ограничение wgpu, не движка.

## Дальше (план)

1. Иерархия трансформов (Parent / GlobalTransform) и стадия пропагации.
2. Ассеты: загрузка glTF, текстуры и материалы, хэндлы с подсчётом ссылок.
3. Тир Full: culling на compute и indirect draw. LOD через meshoptimizer.
4. Параллельный планировщик (rayon) для систем с непересекающимися ресурсами.
5. Веб (wasm + WebGL2/WebGPU) и Android: `web-time` вместо `std::time`, пересоздание surface при suspend/resume.
6. Физика (rapier) и аудио — отдельными плагинами-крейтами.
7. 2D: отдельный лёгкий конвейер спрайтов.
