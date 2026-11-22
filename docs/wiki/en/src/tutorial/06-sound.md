# Step 6. Sound

The `audio` module is needed: `DefaultPlugins` installs `AudioPlugin`, which takes sounds from `assets/`. Without an audio device all calls quietly do nothing.

## Effects from Lua

The call is already in `plate.lua` ([step 5](05-door.md)): a positional sound at the door.

```lua
ctx:play_sound_at("sfx/door.wav", door, { volume = 0.9, max_distance = 30 })
```

The path is relative to `assets/`; `..` and absolute paths are forbidden. The second argument is a vector or an entity id. Options: `volume`, `pitch`, `loop`, `bus`, `min_distance`, `max_distance`, `priority`, `stream`, `occludable`. Stop with `ctx:stop_sound(handle, fade)`, ask with `ctx:sound_playing(handle)`.

Volume falls with distance, panning is computed from the **Audio Listener** component (it is on the camera).

## An object's sound from the editor

The slice's beacon hums all the time: **Add Component → Audio → Audio Source**, Clip `sfx/hum.wav`, switch on Looping and Spatial, set Max Distance. Play On Awake is on by default: the source starts when the scene appears and follows the object. From Lua you can start and stop it with `ctx:play_source()` and `ctx:stop_source()`.

## Music

Music plays from Rust (the `Audio` resource, the `music` bus, streaming). In the project's `setup`:

```rust
res.fetch_mut::<Audio>().play_music("music/theme.wav", 0.4, true); // path, volume, loop
```

The `music` bus does not pause with the game; the others (`sfx`, `ambient`, `voice`) do.

## Check

`olvin-hub run`: stepping on the plate plays the door sound. If you hear nothing see [No sound](../troubleshooting/sound.md).

## See also

- [Audio](../engine/audio.md)
- [No sound](../troubleshooting/sound.md)
- [Lua API: sound](../reference/lua-api.md#sound)
- [Next: step 7, on-screen text](07-ui.md)
