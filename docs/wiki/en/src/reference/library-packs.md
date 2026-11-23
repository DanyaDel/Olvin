# Library packs

The built-in library is a set of ready packs, like the old Unity Asset Store but offline and small: **Window > Add Panel > Library**, pick a pack, **Import**; or `olvin-hub library add <id>`. A pack copies its files into the project's `assets/` under its own name (`scripts/<id>/...`, `prefabs/<id>/...`), so packs never collide. Format and rules: `library/README.md` in the repository.

## Packs

| Pack | Category | What it gives | Needs |
|---|---|---|---|
| `fps-controller` | Characters | first-person player | |
| `third-person-controller` | Characters | third-person character and orbit camera | |
| `top-down-controller` | Characters | top-down character | |
| `interactive-door` | Gameplay | `Door` (slide or hinge; proximity, key press or message; lockable) and a `Switch` plate | `fps-controller` |
| `pickups` | Gameplay | `Coin`, `HealthPack`, `Key` | `fps-controller` |
| `health-damage` | Gameplay | `health.lua`, `HazardZone`, `SpikeTrap`, `PlayerHealth` | `fps-controller` |
| `checkpoints` | Gameplay | `Checkpoint`, `KillPlane`, respawn | `fps-controller` |
| `moving-platforms` | Gameplay | `MovingPlatform` between `Point1..N` carrying the player, `Spinner` | `fps-controller` |
| `enemy-ai` | Gameplay | `Enemy`: patrol, sight with line of sight, chase, attack | `fps-controller`, `health-damage` |
| `spawners` | Gameplay | `Spawner` (interval, message, waves, `max_alive`), `Crate`, `lifetime.lua` | `fps-controller` |
| `camera-effects` | Gameplay | `camera_shake`, `smooth_follow`, `look_at_target`, `trigger_zone` | `fps-controller` |
| `gameplay-demo` | Gameplay | an arena that uses all of them | all of the above |
| `basic-materials`, `*-shaders`, `looks-cinematic` | Materials, Shaders, Looks | PBR materials, shader effects, picture looks | |

Every gameplay pack has a demo scene `scenes/<id>/demo.scene.ron`: open it and press Play. Scripts declare `params`, so everything can be tuned in the Inspector. No pack ships sound; each sound hook is an optional path parameter (`open_sound`, `pickup_sound`, ...) that is empty by default.

## Message conventions

The packs work together through messages (`ctx:send(name, value[, target])`, handled in `on_message`) and shared variables (`ctx:var`, `ctx:set_var`). "To X" means addressed to that entity, otherwise a broadcast.

| Message | Value | Sent by | Understood by |
|---|---|---|---|
| `open`, `close`, `toggle`, `lock`, `unlock` | door id (string) or nothing | `door_switch`, `trigger_zone`, anything | doors: to the door, or broadcast with its `id`; no value reaches every door |
| `door_opened`, `door_closed`, `door_locked` | door id | doors | anyone |
| `picked_up` | kind | pickups | anyone |
| `damage` | amount | hazards, `touch_damage`, enemies, kill plane | `health.lua`, to the victim |
| `heal` | amount | health packs | `health.lua`, to the target |
| `kill`, `revive` | | anyone | `health.lua` |
| `damaged`, `healed` | amount | `health.lua` | camera shake (`listen`) |
| `died`, `revived` | entity name | `health.lua` | enemies |
| `respawn` | entity id | `health.lua` after `respawn_delay`, kill plane | the active checkpoint |
| `checkpoint_reached`, `respawned` | checkpoint name | checkpoints | anyone |
| `enemy_alert`, `enemy_lost` | enemy name | enemies | anyone |
| `spawn`, `start`, `stop`, `clear` | | anyone | spawners, to the spawner (`start` and `stop` also platforms) |
| `spawned` | new object's id | the engine, after `ctx:spawn_prefab` | the requesting spawner |
| `wave_started`, `waves_done` | wave number / name | spawners | anyone |
| `shake` | `0..1`, or `vec3(strength, seconds, 0)` | `trigger_zone`, anyone | `camera_shake`, to the camera |

| Variable | Meaning | Written by |
|---|---|---|
| `score` | sum of collected coins | `pickup.lua` |
| `keys`, `key_<id>` | key count, and whether that key is held | `pickup.lua`; doors read `key_<id>` |
| `health`, `health_max` | health of the entity whose `var` is `health` | `health.lua` |
| `checkpoint`, `checkpoint_pos` | the active checkpoint's name and respawn point | `checkpoint.lua` |

The player is the entity named `Player` (parameter `player`) or tagged `Player`.

## Showing health and score

Lua has no UI binding. To draw a health bar, read the variables in a Rust system and set the text of an `olvin_gui` label, the way `apps/vertical_slice/src/hud.rs` shows its messages: `res.fetch::<ScriptVars>().get("health")`, then `gui.ui.set_text(node, &format!("HP {}", hp))`.

## Things to know

- A Character Controller cannot walk on a Kinematic body (it stalls). So the platform's `Body` is a plain collider moved through its Transform, and `platform.lua` moves the player standing on it by the same step. Crates are not carried.
- `ctx:spawn_prefab` creates objects after the script pass: the spawner learns the new id from the `spawned` message. At most 16 per frame and 256 alive in total.
- Enemies walk on the Character Controller (gravity, steps and sliding for free) and see through a ray, so walls hide the player.

## See also

- [Lua API](lua-api.md)
- [Scripting](../engine/scripting.md)
- [Prefabs](../engine/prefabs.md)
- [Hub CLI](hub-cli.md)
