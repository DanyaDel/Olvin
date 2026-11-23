# Паки библиотеки

Встроенная библиотека — набор готовых паков, как старый Unity Asset Store, только офлайн и компактный: **Window > Add Panel > Library**, выберите пак, **Import**; или `olvin-hub library add <id>`. Пак копирует файлы в `assets/` проекта под своим именем (`scripts/<id>/...`, `prefabs/<id>/...`), поэтому паки не пересекаются. Формат и правила: `library/README.md` в репозитории.

## Паки

| Пак | Категория | Что даёт | Нужно |
|---|---|---|---|
| `fps-controller` | Characters | игрок от первого лица | |
| `third-person-controller` | Characters | персонаж от третьего лица и орбитальная камера | |
| `top-down-controller` | Characters | персонаж с видом сверху | |
| `interactive-door` | Gameplay | `Door` (сдвижная или на петле; вблизи, по клавише или по сообщению; запираемая) и плита `Switch` | `fps-controller` |
| `pickups` | Gameplay | `Coin`, `HealthPack`, `Key` | `fps-controller` |
| `health-damage` | Gameplay | `health.lua`, `HazardZone`, `SpikeTrap`, `PlayerHealth` | `fps-controller` |
| `checkpoints` | Gameplay | `Checkpoint`, `KillPlane`, респаун | `fps-controller` |
| `moving-platforms` | Gameplay | `MovingPlatform` между `Point1..N`, несёт игрока; `Spinner` | `fps-controller` |
| `enemy-ai` | Gameplay | `Enemy`: патруль, зрение с прямой видимостью, погоня, атака | `fps-controller`, `health-damage` |
| `spawners` | Gameplay | `Spawner` (по таймеру, по сообщению, волнами, `max_alive`), `Crate`, `lifetime.lua` | `fps-controller` |
| `camera-effects` | Gameplay | `camera_shake`, `smooth_follow`, `look_at_target`, `trigger_zone` | `fps-controller` |
| `gameplay-demo` | Gameplay | арена, где работает всё перечисленное | все перечисленные |
| `basic-materials`, `*-shaders`, `looks-cinematic` | Materials, Shaders, Looks | PBR-материалы, шейдерные эффекты, картинка | |

У каждого геймплей-пака есть демо-сцена `scenes/<id>/demo.scene.ron`: откройте и нажмите Play. Скрипты объявляют `params`, так что всё настраивается в Инспекторе. Звуков в паках нет: каждый звуковой крючок — необязательный параметр-путь (`open_sound`, `pickup_sound`, ...), по умолчанию пустой.

## Соглашения о сообщениях

Паки работают вместе через сообщения (`ctx:send(name, value[, target])`, приём в `on_message`) и общие переменные (`ctx:var`, `ctx:set_var`). «Кому X» значит адресовано этому объекту, иначе — широковещательно.

| Сообщение | Значение | Кто шлёт | Кто понимает |
|---|---|---|---|
| `open`, `close`, `toggle`, `lock`, `unlock` | id двери (строка) или пусто | `door_switch`, `trigger_zone`, что угодно | двери: самой двери или широковещательно с её `id`; без значения доходит до всех дверей |
| `door_opened`, `door_closed`, `door_locked` | id двери | двери | любой |
| `picked_up` | вид | подбираемые предметы | любой |
| `damage` | величина | опасные зоны, `touch_damage`, враги, kill plane | `health.lua`, жертве |
| `heal` | величина | аптечки | `health.lua`, цели |
| `kill`, `revive` | | любой | `health.lua` |
| `damaged`, `healed` | величина | `health.lua` | тряска камеры (`listen`) |
| `died`, `revived` | имя объекта | `health.lua` | враги |
| `respawn` | id объекта | `health.lua` после `respawn_delay`, kill plane | активный чекпоинт |
| `checkpoint_reached`, `respawned` | имя чекпоинта | чекпоинты | любой |
| `enemy_alert`, `enemy_lost` | имя врага | враги | любой |
| `spawn`, `start`, `stop`, `clear` | | любой | спавнеры, самому спавнеру (`start` и `stop` понимают и платформы) |
| `spawned` | id нового объекта | движок после `ctx:spawn_prefab` | спавнер, который просил |
| `wave_started`, `waves_done` | номер волны / имя | спавнеры | любой |
| `shake` | `0..1` или `vec3(сила, секунды, 0)` | `trigger_zone`, любой | `camera_shake`, камере |

| Переменная | Смысл | Кто пишет |
|---|---|---|
| `score` | сумма собранных монет | `pickup.lua` |
| `keys`, `key_<id>` | число ключей и есть ли этот ключ | `pickup.lua`; двери читают `key_<id>` |
| `health`, `health_max` | здоровье объекта, у которого `var` равен `health` | `health.lua` |
| `checkpoint`, `checkpoint_pos` | имя активного чекпоинта и точка респауна | `checkpoint.lua` |

Игрок — объект с именем `Player` (параметр `player`) или с тегом `Player`.

## Показать здоровье и счёт

В Lua нет привязки к UI. Чтобы нарисовать полоску здоровья, прочитайте переменные в Rust-системе и задайте текст метки `olvin_gui`, как `apps/vertical_slice/src/hud.rs` показывает свои сообщения: `res.fetch::<ScriptVars>().get("health")`, затем `gui.ui.set_text(node, &format!("HP {}", hp))`.

## Что нужно знать

- Character Controller не может ходить по Kinematic-телу (застревает). Поэтому `Body` платформы — обычный коллайдер, который двигают через Transform, а `platform.lua` сдвигает стоящего на нём игрока на тот же шаг. Ящики не переносятся.
- `ctx:spawn_prefab` создаёт объекты после прохода скриптов: id нового объекта спавнер узнаёт из сообщения `spawned`. Не больше 16 за кадр и 256 живых всего.
- Враги ходят на Character Controller (гравитация, ступени и скольжение бесплатно) и видят лучом, поэтому стены прячут игрока.

## См. также

- [Lua API](lua-api.md)
- [Скрипты](../engine/scripting.md)
- [Префабы](../engine/prefabs.md)
- [CLI хаба](hub-cli.md)
