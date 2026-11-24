# Navigation

`olvin_nav` (feature `nav`, `olvin::nav`) lets characters find their way: it bakes the walkable ground of a level into a grid and finds paths with A*.

```rust
app.add_plugin(olvin::nav::NavPlugin::new().auto_bake(NavConfig::default()));
// on an object: a NavAgent component, then
agent.go_to(Vec3::new(10.0, 0.0, 4.0));
```

## Baking

The geometry is the **static colliders** of the scene: boxes and mesh colliders without a rigidbody (triggers, spheres and capsules are not ground or walls). `auto_bake` bakes on the first frame that has colliders; call `Navigation::bake(&world, config)` yourself after loading another level.

`NavConfig` describes the agent: `cell` (grid cell, 0.25 m), `agent_radius` (0.4), `agent_height` (1.8), `max_step` (0.4) and `max_slope_deg` (45). A ray through each cell's centre finds the surfaces; a spot is walkable when its slope is gentle, there is head room, no wall stands in the way, and it is farther than the radius from a wall or an edge. Floors above each other are separate layers. A level that would need more than 4 000 000 cells is an error (`BakeError::TooLarge`): use a larger cell.

## Paths

`grid.find_path(from, to, snap)` returns the points to walk through, or `None` when an end is farther than `snap` from walkable ground, the ends are not connected, or the search gave up (400 000 nodes). A* uses 8 neighbours without cutting corners, climbs steps up to `max_step`, and the result is straightened by line of sight. `is_reachable` answers only yes or no.

## Moving obstacles

`grid.add_blocker(center, radius, height)` shuts every spot in the disc (a closed door, a barricade) until `remove_blocker(id)`; no re-bake.

## Agents

`NavAgent` is a component: `go_to(target)` sets the target, `speed`, `stop_distance`. `status` is `Idle`, `Moving`, `Arrived` or `NoPath`. With `drive: true` the plugin moves the object's `Transform`; with `drive: false` it only writes `desired_velocity`, for a character controller or a rigidbody to follow. A moving target is followed by re-planning twice a second, and at most 8 paths are planned per frame.

Scripts steer agents with `ctx:nav_go`, `nav_stop`, `nav_status` and `nav_reachable` (feature `nav` of `olvin_script`, see the [Lua reference](../reference/lua-api.md)).

Not done yet: avoiding other agents, off-mesh links (jumps, ladders), baking from visual meshes and a preview in the editor.

## See also

- [Physics](physics.md)
- [Scene format](scene-format.md)
- [Scripting](scripting.md)
