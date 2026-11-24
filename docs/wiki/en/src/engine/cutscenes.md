# Cutscenes

`olvin_cutscene` (feature `cutscene`, `olvin::cutscene`) plays a timeline: it moves objects, switches the camera, plays sounds and sends messages to scripts.

```rust
app.add_plugin(olvin::cutscene::CutscenePlugin);
app.resources.fetch_mut::<Cutscenes>().play_file(&assets, "cutscenes/intro.cutscene.ron");
```

## The file

A `*.cutscene.ron` file in `assets/cutscenes/`:

```ron
(
    name: "intro",
    duration: 6.0,
    tracks: [
        (target: "Shot Camera", keys: [
            (time: 0.0, position: Some((0.0, 2.0, 8.0)), ease: Smooth),
            (time: 4.0, position: Some((0.0, 2.0, 3.0)), rotation: Some((-10.0, 0.0, 0.0))),
        ]),
    ],
    cues: [
        (time: 0.0, action: Camera("Shot Camera")),
        (time: 2.0, action: Message(name: "open_door", target: Some("Door"))),
        (time: 3.0, action: Sound(path: "sfx/boom.ogg", volume: 1.0)),
    ],
)
```

- **Tracks** move the object called `target` (found by its `Name`). A key sets any of `position`, `rotation` (Euler degrees, like the scene) and `scale`; each channel blends between the keys that set it, holds the first before and the last after. `ease` is `Linear`, `Smooth` (slow in, slow out) or `Step` for the segment that starts at the key. Channels no key sets are left alone.
- **Cues** happen once at their time: `Camera(name)` draws that object's camera (an empty name hands the view back), `Message { name, value, target }` sends a script message (to the object `target`, or to everyone), `Sound { path, volume }` plays a sound (needs the `audio` feature).
- An object a track or a cue names that does not exist is skipped with a log line, the rest plays.

The file is checked on load: finite numbers, times of 0 or more, a duration up to an hour, up to 256 tracks, 20 000 keys and 4096 cues, names up to 256 bytes, and sound paths that stay inside the assets folder.

## Playing

`Cutscenes` (a resource) starts `play`, `play_looping` or `play_file` and returns an id for `pause`, `set_speed`, `skip` (jump to the end: objects take their final places and the message and camera cues still ahead happen, sounds do not) and `stop` (stay where they are). `is_busy()` says whether anything runs, so the game can hold the player still. At most 16 run at once.

The camera cue puts the marker component `ActiveCamera` on the shot's camera; the renderer draws the camera with it, else the first camera. When the cutscene ends the marker goes away and the game's camera is back.

When one ends (by running out, `skip` or `stop`) the event `CutsceneFinished { id, name, cut_short }` is sent and every script gets the message `cutscene_finished` with the name.

## From scripts

No extra functions: scripts talk to the player with messages.

```lua
ctx:send("cutscene:play", "cutscenes/intro.cutscene.ron")
ctx:send("cutscene:skip")      -- all running
ctx:send("cutscene:stop")
-- on_message(self, ctx, name, value): name == "cutscene_finished"
```

Not done yet: a timeline panel in the editor (files are written by hand), spline paths and rotation by quaternion slerp (Euler degrees blend per channel), animating material and light properties.

## See also

- [Animation](animation.md)
- [Scripting](scripting.md)
- [Audio](audio.md)
