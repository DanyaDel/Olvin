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

`SkinnedMesh` is skinned in `PostUpdate` when the `Animator`'s `version` grew (4 influences, normals and tangents rotate): by a compute pass on the GPU on the Full tier, on the CPU on Compat. Shadows and transparent objects get the pose; one mesh slot per instance.

> **Status:** in the editor viewport models stand in the default pose; in Play they animate. A sequencer/cutscenes is in the plan.

## Morph targets (blend shapes)

glTF morph targets come in with the model: a primitive that has them gets a `MorphMesh` component (the base mesh, the targets by name, their weights), and the model's weight animations become `MorphClips`. When the weights change, the blended mesh is rebuilt and uploaded into the part's own mesh slot; a skinned face is blended in its bind pose and then posed, on both tiers. Target names come from the mesh's `extras.targetNames` (Blender writes shape key names there), else `target 0`, `target 1`, ... Normal offsets are used when the file has them. Not supported: tangent offsets, and meshes without normals (their generated flat normals ignore the targets).

```lua
-- on the model's root object
ctx:set_morph("smile", 0.7)          -- 0..1, false if there is no such shape
local names = ctx:morph_names()      -- {"smile", "blink_L", ...}
ctx:morph_play("talk", true)         -- a glTF weight animation, looping
ctx:morph_stop()
```

From Rust: `MorphMesh::set("smile", 0.7)`, `MorphPlayer::new(clip, looping)`. The editor viewport shows models without morphing (the base mesh); Play and games morph.

## See also

- [Animation (editor)](../editor/animation.md)
- [Scene format](scene-format.md)
- [Rendering](rendering.md)
- [Time and events](time-events.md)
