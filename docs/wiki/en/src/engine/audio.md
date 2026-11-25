# Audio

`olvin_audio` on kira 0.12: 2D and 3D sound, streamed music, buses, reverb, EQ, occlusion. The `audio` module.

## Usage

`AudioPlugin { assets_dir, enabled }` (`ProjectPaths::assets` by default), the `Audio` resource:

```rust
let mut audio = res.fetch_mut::<Audio>();
audio.play("sfx/click.wav");                       // 2D, once
audio.play_at("sfx/thud.wav", Vec3::new(1.0, 0.0, 2.0)); // 3D
audio.play_music("music/theme.wav", 0.4, true);    // stream, the single music track
```

`play_with(path, PlayParams { volume, pitch, looping, position, min_distance, max_distance, bus, priority, stream, occludable })`. An `AudioSourceDesc` from a scene plays on appearance and follows the position. 3D: attenuation and panning from `AudioListener` (on the camera). Without a device all calls are no-ops.

**Losing the device.** A heartbeat clock ticks inside the mixer; when it stands still for more than 2 s (headphones unplugged, PipeWire crashed or restarted), or there was no device at start, `Audio` tries the default device again every 2 s. While there is no sound, loops and music wait virtualized and one-shots are dropped; when the device is back, buses, EQ, reverb and master volume are restored and the waiting sounds start from the beginning. The log gets an error on loss and a warning on return; `audio.reconnect_count()` counts returns. Manual check: `cargo run -p olvin_audio --example reconnect` and `systemctl --user restart pipewire pipewire-pulse wireplumber` while the tone plays.

## Buses and effects

`BusId::{SFX, MUSIC, AMBIENT, VOICE}` and `add_bus(name)`: volume, mute, a 3-band EQ (shelf 250 Hz, bell 1 kHz, shelf 4 kHz) and a send level into the shared reverb.

## Occlusion, priorities

`set_occluder(|listener, source| 0..1)` (usually a physics ray) gives a low-pass (20 kHz → 500 Hz) and a volume drop down to 0.35. Up to 64 voices (`set_max_voices`) and 8 streams; extras are virtualized by (priority, volume, recency): loops wait, one-shots are dropped. `priority` 0 is the most important.

## Pause

The plugin's system follows `Time::paused()`: sounds on `pausable` buses (all but `music`) pause and resume from the same place; sounds started during pause (menus) play. Turn off: `set_follow_time_pause(false)`.

## Lua

`ctx:play_sound`, `ctx:play_sound_at`, `ctx:stop_sound`, `ctx:play_source`: [reference](../reference/lua-api.md#sound). Paths only inside `assets/`.

## See also

- [No sound](../troubleshooting/sound.md)
- [Tutorial: step 6](../tutorial/06-sound.md)
- [Scripting](scripting.md)
- [Time and events](time-events.md)
