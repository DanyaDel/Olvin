# Animation

Two independent parts: scene object animation (F-curves, as in Blender) and skeletal animation of glTF models (clips and a state graph). Code: `olvin_scene::action`, `olvin_anim` (the `anim` module, does not need rendering).

## Object animation

`Action { fps (24), frame_start (1), frame_end (250), autoplay, looping, curves }` in `SceneFile::animation`. Curves `FCurve { object, prop: Location | Rotation | Scale, index, extrapolation, keys }`, a `Key`: frame, value, handles, `Interp::{Constant, Linear, Bezier}`, `HandleType::{Free, Auto, Vector, Aligned, AutoClamped}`. Handle computation and evaluation are ported from Blender 4.x. Rotation: Euler degrees, per channel.

In a game `ActionPlayer` (a resource created by `AnimPlugin`) plays the action in game time: it finds entities by `SceneId` and writes `Transform` (channels without a curve keep their authored values). `scene_file::spawn` fills it from the scene. The editor: [Animation](../editor/animation.md).

## Skeleton and clips

- `Skeleton` (`Joint { parent, target, rest, inverse_bind }`), `Pose`, `skin_matrices`.
- `Clip { channels, events }`: translation / rotation / scale, interpolations Step, Linear (slerp), CubicSpline as in glTF; clip events.
- `AnimGraph`: states (`Motion::Clip` or `Blend1D`), transitions with conditions on `Params` (float, bool, trigger), `exit_time`, crossfade.
- `Animator { skeleton, clips, params, layers, .. }`: layers blend by `weight` and a joint mask; `AnimPlugin` advances all `Animator`s in `Update` by `Time::delta` and sends `Events<AnimationEvent>`.

A model in a scene: `EntityDesc::model = ModelDesc { path, animation: Clip(name) | Graph(path .animgraph.ron) | Inline(..) }`. `ModelPlugin` builds meshes, a skeleton, `SkinnedMesh` and `Animator` under the object. `olvin::models::pending_count(res)` shows how many models are still loading. Graph parameters are set from code: `Animator.params`.

## Skinning

`SkinnedMesh` is skinned on the CPU (4 influences, normals and tangents rotate) in `PostUpdate` when the `Animator`'s `version` grew. It works on all tiers and gives the pose to shadows and transparent objects; one mesh slot per instance.

> **Status:** GPU skinning is not done and is in progress separately: [#8](https://github.com/DanyaDel/Olvin/issues/8). Morph targets: [#10](https://github.com/DanyaDel/Olvin/issues/10). In the editor viewport and in Play models stand in the default pose.

## See also

- [Animation (editor)](../editor/animation.md)
- [Scene format](scene-format.md)
- [Rendering](rendering.md)
- [Time and events](time-events.md)
