//! Sound for Lua scripts (feature `audio`): one-shots and loops through the
//! `olvin_audio::Audio` resource, and the entity's own `AudioSource`.
//!
//! ```lua
//! local s = ctx:play_sound("sfx/door.wav", { volume = 0.8 })          -- 2D
//! ctx:play_sound_at("sfx/thud.wav", ctx:world_position(), { max_distance = 20 })
//! ctx:play_sound_at("sfx/door.wav", door_id)                           -- at an entity
//! ctx:stop_sound(s, 0.5)                                               -- fade out
//! ctx:play_source()                                                    -- own AudioSource
//! ```
//!
//! Without the `Audio` resource (headless, no plugin) every call does nothing
//! and `play_*` returns nil. Clip paths are relative to the audio assets
//! folder; absolute paths and `..` are refused.

use std::path::{Component, Path};
use std::time::Duration;

use glam::Vec3;
use mlua::{LuaString, Table, UserData, UserDataMethods, UserDataRef};
use olvin_audio::{Audio, PlayParams, SoundId};

use super::api::{CtxHandle, entity_from};

/// Sounds the `Audio` resource may track before Lua `play_*` calls refuse.
pub(crate) const MAX_TRACKED_SOUNDS: usize = 1024;

/// A sound started from Lua; pass it to `ctx:stop_sound` and friends.
#[derive(Clone, Copy)]
pub(crate) struct Sound(SoundId);

impl UserData for Sound {}

fn clip_path(path: &str) -> mlua::Result<&Path> {
    let p = Path::new(path);
    if path.is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(mlua::Error::runtime(format!(
            "sound `{path}`: use a path relative to the assets folder, without `..`"
        )));
    }
    Ok(p)
}

/// `{ volume, pitch, loop, bus, min_distance, max_distance, priority, stream, occludable }`
fn play_params(audio: &Audio, opts: Option<&Table>) -> mlua::Result<PlayParams> {
    let mut p = PlayParams::default();
    let Some(t) = opts else { return Ok(p) };
    if let Some(v) = t.get::<Option<f32>>("volume")? {
        p.volume = v.max(0.0);
    }
    if let Some(v) = t.get::<Option<f32>>("pitch")? {
        p.pitch = v.max(0.01);
    }
    if let Some(v) = t.get::<Option<bool>>("loop")? {
        p.looping = v;
    }
    if let Some(v) = t.get::<Option<f32>>("min_distance")? {
        p.min_distance = v.max(0.0);
    }
    if let Some(v) = t.get::<Option<f32>>("max_distance")? {
        p.max_distance = v.max(0.0);
    }
    if let Some(v) = t.get::<Option<i64>>("priority")? {
        p.priority = v.clamp(0, 255) as u8;
    }
    if let Some(v) = t.get::<Option<bool>>("stream")? {
        p.stream = v;
    }
    if let Some(v) = t.get::<Option<bool>>("occludable")? {
        p.occludable = v;
    }
    if let Some(name) = t.get::<Option<LuaString>>("bus")? {
        let name = name.to_str()?;
        p.bus = audio
            .bus(&name)
            .ok_or_else(|| mlua::Error::runtime(format!("no audio bus `{}`", &*name)))?;
    }
    Ok(p)
}

fn play(
    h: &CtxHandle,
    path: &LuaString,
    at: Option<Vec3>,
    opts: Option<Table>,
) -> mlua::Result<Option<Sound>> {
    let path = path.to_str()?;
    let clip = clip_path(&path)?;
    h.with(|c| {
        let Some(mut audio) = c.res_mut::<Audio>() else {
            return Ok(None);
        };
        if audio.active_count() >= MAX_TRACKED_SOUNDS {
            c.warn(format!(
                "not playing `{}`: {MAX_TRACKED_SOUNDS} sounds already going",
                clip.display()
            ));
            return Ok(None);
        }
        let mut params = play_params(&audio, opts.as_ref())?;
        params.position = at;
        Ok(Some(Sound(audio.play_with(clip, params))))
    })?
}

pub(crate) fn add_audio<M: UserDataMethods<CtxHandle>>(m: &mut M) {
    // 2D sound (no position).
    m.add_method(
        "play_sound",
        |_, h, (path, opts): (LuaString, Option<Table>)| play(h, &path, None, opts),
    );
    // Positional sound at a fixed point; `ctx:play_sound_at(path, where, opts)`
    // (vector or entity id) is the prelude's wrapper.
    m.add_method(
        "play_sound_at_xyz",
        |_, h, (path, x, y, z, opts): (LuaString, f32, f32, f32, Option<Table>)| {
            play(h, &path, Some(Vec3::new(x, y, z)), opts)
        },
    );
    m.add_method(
        "stop_sound",
        |_, h, (s, fade): (UserDataRef<Sound>, Option<f32>)| {
            let id = s.0;
            h.with(|c| {
                if let Some(mut audio) = c.res_mut::<Audio>() {
                    match fade {
                        Some(f) => audio.stop_fade(id, Duration::from_secs_f32(f.clamp(0.0, 60.0))),
                        None => audio.stop(id),
                    }
                }
            })
        },
    );
    // False without an audio device.
    m.add_method("sound_playing", |_, h, s: UserDataRef<Sound>| {
        let id = s.0;
        h.with(|c| c.res::<Audio>().is_some_and(|a| a.is_playing(id)))
    });
    m.add_method(
        "set_sound_volume",
        |_, h, (s, volume): (UserDataRef<Sound>, f32)| {
            let id = s.0;
            h.with(|c| {
                if let Some(mut audio) = c.res_mut::<Audio>() {
                    audio.set_volume(id, volume.max(0.0));
                }
            })
        },
    );
    m.add_method(
        "set_sound_pitch",
        |_, h, (s, pitch): (UserDataRef<Sound>, f32)| {
            let id = s.0;
            h.with(|c| {
                if let Some(mut audio) = c.res_mut::<Audio>() {
                    audio.set_pitch(id, pitch.max(0.01));
                }
            })
        },
    );
    // The entity's `AudioSource` from the scene (this entity, or `id`).
    m.add_method("play_source", |_, h, id: Option<u64>| {
        h.with(|c| {
            let e = match id {
                Some(id) => entity_from(id)?,
                None => c.entity(),
            };
            let res = c.res;
            let mut audio = res.get_mut::<Audio>()?;
            audio.play_source(c.world, e).map(Sound)
        })
    });
    m.add_method("stop_source", |_, h, id: Option<u64>| {
        h.with(|c| {
            let Some(e) = id.map_or(Some(c.entity()), entity_from) else {
                return;
            };
            let res = c.res;
            if let Some(mut audio) = res.get_mut::<Audio>() {
                audio.stop_source(c.world, e);
            }
        })
    });
}
