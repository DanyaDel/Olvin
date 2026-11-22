# Physics

`olvin_physics` on Rapier 0.36: bodies, colliders, triggers, a character controller, queries. The `physics` module (a facade feature).

## Bodies and colliders

`PhysicsPlugin { gravity }` and the `Physics` resource. Every frame (PreUpdate) entities with `ColliderDesc` or `RigidbodyDesc` and no `Body` marker get bodies: Dynamic, Kinematic or Fixed (if there is no Rigidbody). The object's scale is baked into the collider shape. `Mesh` takes `MeshGeometry`. Frame order: kinematic bodies follow `Transform`, then fixed steps of 1/60 s (at most 5 per frame), then dynamic bodies write their pose back.

**Pose ownership:** Fixed and Dynamic bodies are teleported when code changed `Transform` (velocity is kept); a static collider that moves every frame is better made Kinematic. `Physics::teleport(world, entity)` places a body at once and zeroes velocities.

## Forces and velocities

`Physics::apply_force` (one-shot, for the next step), `apply_impulse` (Δv = impulse / mass), `set_velocity`, `velocity`, `angular_velocity`. They work only with a Dynamic body, otherwise false. From Lua: `ctx:apply_force`, `ctx:apply_impulse`, `ctx:set_velocity`, `ctx:velocity`.

## Character controller

A capsule + `CharacterMotor { velocity, grounded, sliding, gravity_scale }`, steps, slopes, `move_character`. The character pushes dynamic bodies (`push_mass`, 80 kg by default, 0 turns it off) and sends `ControllerHit { character, other, point, normal, direction }` into `Events<ControllerHit>`.

## Triggers and events

A collider with `is_trigger: true` does not collide but produces events. `CollisionEvents` by entity: in `Behaviour` and Lua these are `on_collision_enter/exit` and `on_trigger_enter/exit` ([Scripting](scripting.md)). Collision events are processed in the `Update` pass.

## Queries

`raycast(_filtered)`, `spherecast(_filtered)`, `overlap_sphere`. In Lua: `ctx:raycast(origin, dir, max)`.

> **Status:** not done: joints, collision layers and masks ([#2](https://github.com/DanyaDel/Olvin/issues/2)). A NaN step is ignored, a mesh collider with indices outside the vertices is skipped with a warning.

## See also

- [Scripting](scripting.md)
- [Scene format](scene-format.md)
- [Tutorial: step 3](../tutorial/03-physics.md)
- [Time and events](time-events.md)
