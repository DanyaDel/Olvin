# No sound

## Check in order

1. **Module and device.** The `audio` module is required (`olvin-hub add audio`). Without a sound device all calls quietly do nothing, and the engine looks for a device again every 2 s: sound appears by itself once there is one (likewise after a PipeWire restart). In `DefaultPlugins { audio: AudioPlugin { enabled: true, .. } }` sound is on by default, in `HeadlessPlugins` it is off.
2. **Path.** Files are looked up in `assets/` (`ProjectPaths::assets`). A path in code and in Lua is relative to it: `"sfx/door.wav"`, without `..` and absolute paths (Lua will refuse with an error).
3. **Log.** Decoder errors and missing files go to the log. Looping sounds whose file failed to load are forgotten.
4. **Listener.** 3D sound is heard from `AudioListener` (it is created with a camera in the scene). Without a listener panning and attenuation are wrong.
5. **Distance.** Beyond `max_distance` volume falls to zero; an `AudioSource` has 50 m by default.
6. **Pause.** While paused buses stop, except `music`.
7. **Voices.** The limit is 64 voices and 8 streams: lower-priority ones are virtualized.
8. **PipeWire/ALSA.** On Arch `alsa-lib` is needed to build, and sound goes through `pipewire-alsa` or `pipewire-pulse`; check with `speaker-test`.

## Check in the slice

`cargo run -p vertical_slice --release` should play music and the beacon's hum; `--no-audio` turns the device off.

## See also

- [Audio](../engine/audio.md)
- [Tutorial: step 6](../tutorial/06-sound.md)
- [FAQ](faq.md)
- [Lua API: sound](../reference/lua-api.md#sound)
