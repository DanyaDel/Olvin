# Формат сцены

Сцена это `*.scene.ron` (RON), общий для редактора и игр. Код: `olvin_scene::file` (фича `serde`). Игра загружает её через `olvin::scene_file` (фича `scene`).

## Структура

```ron
(
    version: 4,
    entities: [ ... ],
    // environment, animation, extensions: пишутся, только если не по умолчанию
)
```

`SceneFile { version, entities, environment, animation, extensions }`:

| Поле | Что |
|---|---|
| `entities` | список `EntityDesc`, родители всегда раньше детей |
| `environment` | небо, ambient, экспозиция, туман, тени, постобработка ([Рендер](rendering.md)) |
| `animation` | F-кривые трансформов объектов (`Action`, [Анимация](animation.md)) |
| `extensions` | данные элементов редактора |

## EntityDesc

Пустые поля в файл не пишутся.

| Поле | Смысл |
|---|---|
| `id`, `name`, `active`, `tag`, `layer`, `parent` | идентификация и иерархия |
| `position`, `rotation`, `scale` | локальный Transform; поворот в градусах, Эйлер YXZ |
| `mesh` | `MeshDesc { shape: Cube/Sphere/Capsule/Cylinder/Plane/Quad, color }` |
| `poly` | отредактированный `PolyMesh` (вместо `mesh.shape`) |
| `brush` | CSG-браш `{ shape, op, color, vertices }` |
| `material` | путь `*.mat.ron` от `assets/` |
| `model` | `ModelDesc { path, animation }`: glTF под объектом |
| `camera` | `CameraDesc { fov_deg, near, far, .. }` |
| `light` | `LightDesc { kind, color, intensity, range, spot_angle, shadows, shadow_bias, size }` |
| `collider` | `ColliderDesc { shape: Box/Sphere/Capsule/Mesh, center, is_trigger, friction, restitution }` |
| `rigidbody` | `RigidbodyDesc { kind: Dynamic/Kinematic, mass, drag, angular_drag, gravity_scale, freeze_rotation, ccd }` |
| `character_controller` | `CharacterControllerDesc { height, radius, step_offset, slope_limit_deg, push_mass }` |
| `audio_source`, `audio_listener` | звук |
| `scripts` | `[ScriptDesc { name, params }]`, параметры строками |
| `components` | `BTreeMap<имя типа, RON-текст>`: игровые компоненты |
| `prefab` | экземпляр префаба ([Префабы](prefabs.md)) |

Пример из `apps/vertical_slice/assets/scenes/level.scene.ron`:

```ron
(id: 31, name: "Plate Trigger", position: (4.0, 1.0, 6.0), rotation: (0.0, 0.0, 0.0), scale: (2.0, 2.0, 2.0),
    collider: Some((shape: Box(size: (1.0, 1.0, 1.0)), center: (0.0, 0.0, 0.0), is_trigger: true, friction: 0.6, restitution: 0.0)),
    scripts: [(name: "plate", params: {"door": "Door", "message": "Дверь открыта: зайди к роботу"})]),
```

## Игровые компоненты из сцены

Зарегистрируйте тип в `olvin::scene_file::SceneComponents` (`register::<T>(name)` для `T: Component + Deserialize`), и сцена может нести его в `components`:

```ron
components: {"Player": "()", "PlayerLook": "(yaw: 0.0, pitch: 0.0)"}
```

Неизвестные имена и битые значения пропускаются с предупреждением, редактор их сохраняет. При спавне на сущность кладётся `SceneId(id)`; `scene_file::find(world, id)` и `spawn_ids` возвращают сущность по id.

## Версии и миграции

`FORMAT_VERSION` сейчас 4, пишется всегда; файл без поля это версия 0. `SceneFile::parse` / `load` отклоняют версию новее поддерживаемой и прогоняют цепочку `MIGRATIONS` (`fn(&mut SceneFile)`, запись `n` переводит `n → n+1`).

| Версия | Что изменилось |
|---|---|
| v1 → v2 | миграция пустая, но появилось поле `model` (старые сборки не откроют файл) |
| v2 → v3 | клип старого таймлайна (секунды) превращён в F-кривые на 24 fps; расширение `animator` убрано |
| v3 → v4 | миграция пустая, добавлено `EntityDesc::prefab` |

Переименования и новые поля покрывает `#[serde(alias/default)]`, миграция исправляет смысл данных. Ломающее изменение: `FORMAT_VERSION += 1` и новая миграция. Фикстуры старых версий: `crates/olvin_scene/tests/fixtures/`. Срез записан как `version: 2` и успешно мигрируется.

## Загрузка

- `olvin::scene_file::load(res, path)` читает и спавнит (только активные объекты): компоненты переносятся как данные, к свету добавляется рантаймовый `Light`, CSG собирается при загрузке, `apply_lighting` переносит солнце и `Environment` в `RenderSettings`.
- `SceneLoader` (`olvin_game`) читает и разбирает сцену в рабочем потоке, `scene_file::spawn_loaded` спавнит результат ([Состояния и сохранения](game-states-saves.md)).

> **Статус:** частицы (`ParticleEmitter`) в формат сцены ещё не входят: планируется, см. [#11](https://github.com/DanyaDel/Olvin/issues/11). То же для спрайтов и тайлмапов.

## См. также

- [Префабы](prefabs.md)
- [Ассеты](assets.md)
- [Рендер](rendering.md)
- [Инспектор](../editor/inspector.md)
