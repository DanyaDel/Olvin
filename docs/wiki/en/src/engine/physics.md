# Physics

`olvin_physics` on Rapier 0.36: bodies, colliders, triggers, a character controller, queries. The `physics` module (a facade feature).

## Bodies and colliders

`PhysicsPlugin { gravity }` and the `Physics` resource. Every frame (PreUpdate) entities with `ColliderDesc` or `RigidbodyDesc` and no `Body` marker get bodies: Dynamic, Kinematic or Fixed (if there is no Rigidbody). The object's scale is baked into the collider shape. `Mesh` takes `MeshGeometry`. Frame order: kinematic bodies follow `Transform`, then fixed steps of 1/60 s (at most 5 per frame), then dynamic bodies write their pose back.

**Pose ownership:** Fixed and Dynamic bodies are teleported when code changed `Transform` (velocity is kept); a static collider that moves every frame is better made Kinematic. `Physics::teleport(world, entity)` places a body at once and zeroes velocities.

**Layers:** a collider has a `layer` (0..31) and a `mask`, the layers it collides with (bit `n` is layer `n`; the default is layer 0 and every layer). Two colliders collide only when **each** one's mask has the other's layer, so a ghost on layer 3 passes through what leaves layer 3 out. In the inspector: *Layer* and *Collides With* of the collider. Queries pick layers on their own, see Queries.

## Joints

A `JointDesc` on an object with a body joins it to another object (`connected`: its scene id) or to the world. Kinds: **Fixed** (the bodies stay as placed), **Hinge** (rotation about an `axis` of this object, with optional angle `limits` in degrees: a door), **Ball** (free rotation about the anchor: a pendulum, a rope link), **Slider** (movement along an `axis`, optional `limits` in metres: a drawer, a piston) and **Spring** (pulls the two anchors to `rest_length` with `stiffness` and `damping`). `anchor` is the joint point in this object's local space (scale applied), `connected_anchor` the same point in the other object's; `collide_connected` lets the two bodies still collide (off by default).

The joint is made when both bodies exist, in the pose they have then, so nothing jumps: a hinge door hangs where you put it and its limits count from there (`-45..45` is 45 degrees either way). A joint to the world fixes the anchor point in space where it is at that moment. Take the `JointDesc` away and the joint is removed. A partner that does not exist is logged and the joint is skipped. In the inspector: Add Component, Physics, *Fixed / Hinge / Ball / Slider / Spring Joint*.

## Forces and velocities

`Physics::apply_force` (one-shot, for the next step), `apply_impulse` (Δv = impulse / mass), `set_velocity`, `velocity`, `angular_velocity`. They work only with a Dynamic body, otherwise false. From Lua: `ctx:apply_force`, `ctx:apply_impulse`, `ctx:set_velocity`, `ctx:velocity`.

## Character controller

A capsule + `CharacterMotor { velocity, grounded, sliding, gravity_scale }`, steps, slopes, `move_character`. The character pushes dynamic bodies (`push_mass`, 80 kg by default, 0 turns it off) and sends `ControllerHit { character, other, point, normal, direction }` into `Events<ControllerHit>`.

## Triggers and events

A collider with `is_trigger: true` does not collide but produces events. `CollisionEvents` by entity: in `Behaviour` and Lua these are `on_collision_enter/exit` and `on_trigger_enter/exit` ([Scripting](scripting.md)). Collision events are processed in the `Update` pass.

## Queries

`raycast(_filtered)`, `spherecast(_filtered)`, `overlap_sphere`, and the general ones with `QueryOptions { mask, exclude, hit_triggers }`: `raycast_with`, `shapecast(shape, position, rotation, direction, max, &options)` (a sphere, a box or a capsule: `CastShape`) and `overlap(shape, position, rotation, &options)`. The `mask` picks collision layers by the **collider's layer** (bit `n` is layer `n`), not by its own mask, like Unity's `LayerMask`.

In Lua: `ctx:raycast(origin, dir, max[, mask])`, `ctx:spherecast(origin, radius, dir, max[, mask])`, `ctx:boxcast(origin, half_extents, dir, max[, mask])` (all return `{ entity, point, normal, distance }` or `nil`) and `ctx:overlap_sphere(center, radius[, mask])` (a list of ids). The caster's own collider is skipped.

> **Status:** a NaN step is ignored, a mesh collider with indices outside the vertices is skipped with a warning. Not done: joint motors and breaking forces, joints between a character controller and bodies.

## See also

- [Scripting](scripting.md)
- [Scene format](scene-format.md)
- [Tutorial: step 3](../tutorial/03-physics.md)
- [Time and events](time-events.md)
