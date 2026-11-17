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
  olvin_scene/        Transform, Camera — чистые данные на glam; feature serde: формат сцен
  olvin_csg/          CSG по выпуклым брашам: add/subtract/intersect в порядке иерархии, convex hull
  olvin_physics/      физика на Rapier 0.36: коллайдеры, Rigidbody, фиксированный шаг
  olvin_window/       winit: окно, Input, главный цикл (runner)
  olvin_render/       wgpu: меши, instancing, frustum culling, уровни железа
  olvin/              фасад: реэкспорты, DefaultPlugins, prelude, features window/render
apps/
  sandbox/            стресс-демо: N вращающихся кубов + свободная камера
  editor_dev/         `olvin-editor-dev`: редактор со всеми элементами, для разработки
editor/               olvin-editor — ставится отдельно через хаб, интерфейс на английском
  olvin_editor/        ядро редактора: оболочка, рабочие области, докинг, GPU-вьюпорт, базовые панели
  olvin_editor_console/  элемент Console
  olvin_editor_animator/ элемент Animator
  olvin_editor_csg/      элемент CSG: маппинг уровней в духе Realtime CSG
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

`Transform` (translation/rotation/scale, `matrix()`, `looking_at`, `forward/right/up`) и `Camera` (perspective, правая система координат, глубина 0..1 через `glam::camera::rh::proj::directx`). Рантайм-компоненты `Name`, `Tag`, `MeshGeometry` (треугольники для mesh-коллайдеров). `primitives.rs`: `Shape` (Cube, Sphere, Capsule, Cylinder, Plane, Quad — размеры как в Unity) и `Primitive` (позиции, нормали, индексы, контур для выделения). Это общий источник геометрии для движка и редактора. Нет зависимостей от ECS и GPU.

## olvin_window

- `lib.rs`: `WindowPlugin { title, width, height, resizable }` ставит runner на winit 0.30 (`ApplicationHandler`). Окно создаётся в `resumed`, потому что так требует Android. Там же вставляется `SurfaceTarget` и вызывается `app.startup()`. Кадр — это `RedrawRequested → app.update()`. На Resize обновляется `SurfaceTarget.size`.
- `input.rs`: `Input` хранит нажатые клавиши и кнопки мыши, `just_pressed/just_released`, позицию курсора и сырую `mouse_delta` из `DeviceEvent`. Сброс в `Stage::Last`, при потере фокуса всё отпускается.

## olvin_render

- `lib.rs`: `RenderPlugin { settings }`, `RenderSettings` (vsync, low_power, clear_color, light_dir, ambient), компонент `MeshRenderer { mesh, color }`, `Color` (линейный RGBA). Плагин добавляет `EcsPlugin`, ресурсы `Meshes` и `RenderStats`, системы `init` (Startup) и `render` (Render).
- `renderer.rs`: `Renderer` создаёт wgpu Instance с display handle (для GLES/Wayland), surface, adapter и device с лимитами WebGL2. Определяет `RenderTier`, при vsync выбирает Fifo, иначе Mailbox или Immediate. Кадр: загрузка новых мешей → проход по `(Transform, MeshRenderer)` с отсечением по сфере → бакеты по мешу → один буфер инстансов (растёт до степени двойки) → `draw_indexed` на меш. Результат в `RenderStats { draw_calls, instances, culled }`, сведения об адаптере в `RenderInfo`.
- `mesh.rs`: `Vertex { position, normal }`, `MeshData` (`cube`, `plane`, `bounding_sphere`). `Meshes::add` возвращает `MeshHandle` ещё до появления GPU, загрузка ленивая.
- `frustum.rs`: 6 плоскостей из view-proj (Gribb–Hartmann, глубина 0..1), тест сферы.
- `mesh.wgsl`: instanced-шейдер в пределах WebGL2: один uniform, 7 атрибутов (матрица модели — 4 vec4 + цвет), Ламберт + ambient.

## olvin_csg

Классический CSG редакторов уровней (Hammer, Realtime CSG) по выпуклым брашам.
- `shapes.rs`: `BrushShape` (Box, Wedge, Cylinder{sides}, Pyramid{sides}, Stairs{steps}) в локальном единичном кубе `[-0.5, 0.5]³`, поэтому размер браша — это его scale. Каждая форма — это набор выпуклых кусков (у лестницы кусок на ступень).
- `convex_hull(points)` (перебор троек, для брашей с десятками вершин), `shape_vertices(shape)` (углы однокусковой формы). `BrushInput.vertices: Option<Vec<Vec3>>` — отредактированный браш, это оболочка своих вершин вместо параметрической формы.
- `lib.rs`: `build(&[BrushInput]) -> Vec<Polygon>`. Каждая грань дробится плоскостями пересекающихся кусков (AABB-отсев), так что каждый фрагмент целиком внутри или снаружи каждого браша. Фрагмент остаётся, если твёрдость по разные стороны от него различается: точки берутся на ±2 мм по нормали, а операции `CsgOp::{Add, Subtract, Intersect}` применяются по порядку. Совпадающие сонаправленные грани оставляет только последний кусок. Метод устойчив к касающимся и совпадающим граням, где BSP-подходы ломаются. Зеркальные трансформы переориентируются. Есть `brush_edges` для контуров и `TriMesh::from_polygons`. 8 тестов: комната, туннель, порядок операций, пересечение, замкнутость всех форм.

## olvin_scene::file (feature `serde`)

Формат сцен `*.scene.ron`, общий для редактора и игр. `SceneFile { entities, extensions }`, `EntityDesc` (стабильный `id`, `active`, `tag`, `layer`, `parent`, позиция, поворот в градусах по Эйлеру YXZ, масштаб, `mesh`, `camera`, `brush: BrushDesc { shape, op, color, vertices }`, `light: LightDesc` (Directional/Point/Spot), `collider: ColliderDesc` (Box/Sphere/Capsule/Mesh, center, trigger, friction, bounciness), `rigidbody` (Dynamic/Kinematic, mass, drag, gravity, freeze rotation, CCD), `character_controller`, `audio_source`, `audio_listener`, `scripts: [ScriptDesc { name, params }]`). Пустые поля в файл не пишутся. `active_in_hierarchy`, `sun()` — первый активный Directional Light. Родители в списке всегда идут раньше детей. `world_matrices[_with]` разворачивает иерархию в мировые матрицы, `hierarchy_order()` даёт порядок DFS, как в панели иерархии, `csg_brushes()` — браши в порядке операций. `extensions` — данные элементов редактора (клипы аниматора).

## olvin_physics

`PhysicsPlugin { gravity }` и ресурс `Physics { world: rapier PhysicsWorld }`. На каждом кадре (PreUpdate): сущности с `ColliderDesc` или `RigidbodyDesc` без маркера `Body` получают тела (Dynamic, Kinematic или Fixed, если Rigidbody нет). Масштаб объекта запекается в форму коллайдера, у Rapier масштаба нет. `Mesh` берёт `MeshGeometry`. Кинематические тела следуют за `Transform`, затем фиксированные шаги по 1/60 (не больше 5 за кадр), затем динамические пишут позу обратно. Rapier 0.36 считает в glam 0.33, конвертация через массивы. Тесты: ящик ложится на землю, шар — на mesh-пол, сквозь триггер проходит.

## olvin (фасад)

Features `window` и `render` включены по умолчанию. `scene` добавляет `scene_file::{load, spawn}`: загрузка сцены редактора в `World` — только активные объекты; компоненты переносятся как данные (`LightDesc`, `ColliderDesc`, `RigidbodyDesc`, `AudioSourceDesc`, `Scripts`, `AudioListener`, `Name`, `Tag`); для Mesh Collider примитива добавляется `MeshGeometry`. Первый Directional Light задаёт `RenderSettings.light_dir/light_color`. CSG собирается при загрузке: меши по цвету граней плюс одна сущность «CSG Level» со статическим mesh-коллайдером. Иерархия разворачивается в мировые трансформы. `physics` подключает `olvin_physics` в `DefaultPlugins`. Без них получается чистое ядро + ECS (сервер, инструменты). `DefaultPlugins` добавляет всё включённое. Для игр есть `prelude`.

## editor (olvin-editor)

Редактор сцен на egui 0.35 + egui_dock 0.20, стиль Blender + Unity, интерфейс на английском. Ставится отдельно, отдельные элементы подключаются к нему как крейты-плагины. Хаб генерирует крейт установки, в его `main.rs` перечислены плагины. Плагины линкуются статически, без dylib: так быстрее и без проблем с ABI.

- `lib.rs`: `run(plugins)` — первый аргумент командной строки это папка проекта. Нативные опции: wgpu, depth 32, MSAA 4. Реэкспорт egui, egui_dock, glam, olvin_scene, чтобы плагины собирались строго на тех же версиях.
- `registry.rs`: трейты `Panel` (`ui(ui, &mut EditorState)`, по экземпляру на вкладку) и `EditorPlugin` (`build(&mut Registry)`). `PanelKind`, `WorkspaceTemplate`, `TabAlloc`. Точки расширения: `add_viewport_extension`, `add_component`, `add_create_item`.
- `ext.rs`: API для элементов. `Geometry` (мировые полигоны с флагами `GRID`/`SELECTED`, линии с глубиной и поверх всего, `raycast`), `ViewCamera` (project, ray, world_per_point, линия egui с клиппингом по near), `Ray` (плоскость, треугольник, ближайшая точка на прямой), `Snap`, `ToolDesc`, `ViewportCtx`, трейт `ViewportExtension` (`geometry`, `interact`, `tools`), `ComponentEditor`, `CreateItem` (путь вида «CSG/Box» даёт подменю).
- `render.rs` + `viewport.wgsl`: GPU-вьюпорт рисует прямо в render pass egui через `egui_wgpu::CallbackTrait` (wgpu 29 от eframe), в общий буфер глубины. Конвейеры: меши (освещение, трипланарная сетка 1 м и ¼ м для `GRID`, подсветка выделения), бесконечная адаптивная сетка (полноэкранный треугольник, пересечение луча с плоскостью, `frag_depth`, затухание мелких линий по `fwidth`, оси X/Z), линии с глубиной и поверх всего. Буферы на вьюпорт растут до степени двойки.
- `state.rs`: `EditorState` — сцена, путь, проект, выделение, `preview` (переопределение поз, не сохраняется), лог, `scene_generation`, активный инструмент (`tools::{VIEW, MOVE, ROTATE, SCALE}` или id элемента), `local_space`, `SnapSettings { enabled, step: Option (None = адаптивный), angle_deg }`, расширения. Undo/redo снимками: изменения коммитятся, когда отпущены мышь и клавиатура, так что перетаскивание = один шаг. Операции: `create/delete/duplicate/reparent/move_sibling`, `open/save/save_as/new_scene`, `request_open`.
- `workspace.rs`: `Tab { kind, id }`, `Workspace { name, dock: DockState<Tab> }`, `Layout` (вкладки рабочих областей, сохраняется в `<config>/olvin-editor/layout.ron` при выходе). Дублирование выдаёт новые id вкладок, у копий независимое состояние панелей.
- `app.rs`: меню (File/Edit/GameObject/Window). GameObject как в Unity: Create Empty, 3D Object/…, Light/…, Audio/…, Camera и пункты элементов. Примитивы создаются со своим коллайдером, камера с Audio Listener. Если инструмент выставил `state.tool_owns_keys`, Delete и Ctrl+D отдаются ему. вкладки рабочих областей как в браузере (клик, двойной клик — переименовать, СКМ или × — закрыть, ПКМ — дублировать или сдвинуть, «+» — шаблон, пустая или копия), DockArea (перетаскивание, разделение, всплывающие окна, «+» в каждой группе вкладок), горячие клавиши (Ctrl+S/N/Z/Shift+Z/Y/D, Del/X), подтверждение при несохранённых изменениях, статус-бар, заголовок окна.
- `theme.rs`: серые тона Blender, синее выделение Unity, оранжевый для выбранного во вьюпорте, цвета осей.
- `panels/viewport.rs`: камера как в Unity: СКМ — сдвиг, Alt+ЛКМ — орбита, Alt+ПКМ — приближение, ПКМ — обзор, с WASD/QE — полёт (Shift ускоряет, колесо при зажатой ПКМ меняет скорость), Hand(Q)+ЛКМ — сдвиг, колесо, F. Кликабельное гизмо вида: оси ±X/±Y/±Z дают вид по ним, центр переключает Persp/Iso (ортография). Примитивы из `olvin_scene::primitives` с гладкими нормалями, неактивные объекты скрыты, свет вьюпорта от Directional Light сцены. Гизмо компонентов выделенного: коллайдеры (зелёные), свет (лучи, сфера, конус), радиусы звука, Character Controller. Иконки света и звука кликабельны. Инструменты: Q/W/E/R и инструменты элементов, X переключает Global/Local. Гизмо: Move (оси по ближайшей точке луча, плоскости, свободная ручка в плоскости экрана), Rotate (кольца по осям с передней полуокружностью, кольцо вида, угол из экранного угла вокруг центра), Scale (оси с кубиками, равномерный). Перетаскивание считается от снимка начала: привязка позиции к абсолютной сетке на мировых осях и к шагу на локальных, угол 1–90°, масштаб 0.1. Ctrl инвертирует Snap. Шаг Auto — степень двойки от высоты камеры, совпадает с сеткой. Порядок кадра: расширения → гизмо → выбор лучом → сборка геометрии → callback в заранее зарезервированный слот painter'а (3D под оверлеями). Панель инструментов и индикатор осей сверху.
- `panels/hierarchy.rs`: дерево с drag&drop для смены родителя, поиск, контекстное меню (Create child с пунктами элементов, Rename, Duplicate, Move Up/Down — важно для порядка CSG, Unparent, Delete). `create_menu` общий с меню GameObject.
- `panels/inspector.rs`: заголовок Active / имя / Tag / Layer. Сворачиваемые секции: Transform, Mesh Renderer (выбор меша), Camera, Light, Box/Sphere/Capsule/Mesh Collider, Rigidbody, Character Controller, Audio Source, Audio Listener, Scripts (имя + параметры ключ–значение), компоненты элементов. Add Component — всплывающее меню с поиском и категориями Rendering / Physics / Audio / Scripts / Elements.
- `panels/assets.rs`: браузер `assets/` проекта: хлебные крошки, плитки, фильтр, создание папки или сцены, двойной клик открывает сцену.
- `olvin_editor_console`: панель лога с фильтрами по уровню.
- `olvin_editor_csg` (элемент CSG): `CsgViewport` пересобирает CSG, только когда изменился набор брашей (сравнение `BrushInput`), рисует полигоны с сеткой и подсветкой выделения, контуры брашей по цвету операции (с глубиной и бледно поверх — вычитания видны в стенах). Окно «Mapping» (панель + рабочая область): режимы Object / Draw / Resize / Edit Polygons, блоки Edit Polygons, Boolean (операция, Earlier/Later, Duplicate as Carve, Hollow, Rotate 90°, Flip), New brushes, Display, Build. Edit Polygons (G, `poly.rs`): выбор вершин, рёбер и граней (1/2/3, Shift добавляет; клик по другому брашу переключает браш), каркас и подсветка, гизмо выделения по осям мира, свободная ручка и голубая стрелка по нормали в режиме граней. Сдвиг привязан к сетке абсолютно. При первом перетаскивании браш становится вершинным (`vertices`) и остаётся выпуклым как оболочка. Delete удаляет вершины, если оболочка не схлопывается. Extrude Face создаёт браш-призму, Reset to Shape сбрасывает правки. Инструмент Draw Brush (B): по лучу выбирается осевая плоскость поверхности, прямоугольник по сетке, затем высота по ближайшей точке луча к нормали; вниз — вычитание, Shift — вычитание всегда, Esc/ПКМ отменяют. Получается браш выбранной формы с размерами по рамке. Resize (T): 6 ручек граней, грань двигается по своей оси и встаёт точно на линию сетки, противоположная остаётся на месте (scale и position пересчитываются в пространстве родителя). Панель CSG: настройки новых брашей (операция, форма, цвет), Hollow (вычитающий потомок со стенами заданной толщины), Rotate 90°, Flip X/Z, Earlier/Later, отображение, статистика сборки (браши → полигоны → треугольники, мс). Компонент CSG Brush в инспекторе, GameObject → CSG/{Box, Wedge, Cylinder, Pyramid, Stairs, Room}, рабочая область Mapping. Общие настройки лежат в thread_local `Shared`.
- `olvin_editor_animator`: таймлайн. Клип `{ length, tracks: entity id → ключи }` хранится в `scene.extensions["animator"]`. Есть ключ по кнопке, перетаскивание ключей, ПКМ — удалить, скраб, play/loop, превью через `state.preview`. Добавляет шаблон рабочей области Animation.

## hub (olvin-hub, рабочее имя)

Хаб создаёт проекты игр и ставит в них модули. Источник правды о модулях — `Cargo.toml` игры, у хаба нет своего состояния об установленном.

- `olvin_hub/catalog.toml`: встроенный каталог. `kind = "olvin"` — фича фасада `olvin` (плагин подключит `DefaultPlugins`). `kind = "crate"` — зависимость с crates.io (rapier3d, kira, gltf, rand, serde, ron). Поле `requires` ставит модули вместе.
- `catalog.rs`: `Catalog` (парсинг, проверка `requires`, `resolve` — сначала зависимости, ловит циклы, `categories`), `Module`, `Source`.
- `project.rs`: `Project::create` (модули с `default = true`) и `create_with(modules)` для шаблонов, `modified()`, `open`, `discover` (ищет вверх по папкам), `add` (вместе с requires), `remove` (не даёт удалить то, от чего зависят другие, если нет `force`), `save`. Правки через `toml_edit`, комментарии и форматирование пользователя сохраняются. `olvin` подключается с `default-features = false`, список модулей Olvin = `features`.
- `config.rs`: `HubConfig` в `<config_dir>/olvin-hub/config.toml` (на Linux учитывает `XDG_CONFIG_HOME`): список проектов и `EngineSource` (`path` к checkout'у или `git`). По умолчанию это папка, из которой собран хаб, если её нет — GitHub.
- `template.rs`: Cargo.toml (deps всегда -O3), `src/main.rs` — графический (грузит `assets/scenes/main.scene.ron`, иначе куб) или headless, если нет window+render+scenes; README, .gitignore.
- `editor.rs`: `EditorInstall` в `<data_dir>/olvin-hub/editor` — сгенерированный крейт (`Cargo.toml` + `main.rs` с плагинами, бинарник `olvin-editor`). Источник правды — его `Cargo.toml`. Методы: `write`, `build_command` (`cargo build --release`), `launch(project)`. Элементы редактора описаны в каталоге как `[[editor_module]]`.
- CLI: `new`, `list`, `add`, `remove [--force]`, `run [--release]`, `projects`, `register`, `forget`, `engine [--path|--git]`, `editor install|list|add|remove|open [проект]`.
- GUI (как Unity Hub, на русском). Важно: в модальных окнах нельзя использовать `ui.separator()` внутри горизонтального layout, он берёт доступную высоту и окно растёт каждый кадр. Тест `dialogs_fit_their_content` гоняет модалки headless через `Context::run_ui` 40 кадров. Остальное: боковая навигация «Проекты» / «Редактор». Проекты — таблица (иконка, имя, путь, «изменён N мин назад», чипы модулей, меню «⋯»: открыть в редакторе, запустить, release, модули, показать в папке, убрать). Клик по строке открывает проект в редакторе. Модальные окна: «Новый проект» (шаблоны 3D / 3D + физика / Пустой, имя, расположение), «Добавить», «Модули», «Элементы редактора». Страница «Редактор» — карточка установки (статус, элементы, Открыть / Элементы… / Пересобрать) и журнал сборки: stderr cargo читается в фоновом потоке.

## Зависимости

wgpu 30, winit 0.30, hecs 0.11, glam 0.34 (проекции в `glam::camera`), bytemuck, pollster, log/env_logger.
Хаб: toml_edit, toml, serde, clap, dirs, anyhow, eframe 0.35 (0.36+ требует rustc 1.95).
Редактор: eframe/egui 0.35 (wgpu 29 внутри, отдельно от wgpu 30 движка), egui_dock 0.20 (0.21 требует egui 0.36), bytemuck, ron, dirs.

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
