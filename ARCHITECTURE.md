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

## olvin (фасад)

Features `window` и `render` включены по умолчанию. Без них получается чистое ядро + ECS (сервер, инструменты). `DefaultPlugins` добавляет всё включённое. Для игр есть `prelude`.

## Зависимости

wgpu 30, winit 0.30, hecs 0.11, glam 0.34 (проекции в `glam::camera`), bytemuck, pollster, log/env_logger.

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
