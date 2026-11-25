//! Never opens a device: everything runs on the no-device fallback.

use glam::{Quat, Vec3};
use olvin_core::{App, ProjectPaths, Time};
use olvin_scene::Transform;

use super::*;

#[test]
fn attenuation_and_pan() {
    assert_eq!(attenuation(0.5, 1.0, 10.0), 1.0);
    assert_eq!(attenuation(1.0, 1.0, 10.0), 1.0);
    assert_eq!(attenuation(10.0, 1.0, 10.0), 0.0);
    let a = attenuation(2.0, 1.0, 10.0);
    let b = attenuation(5.0, 1.0, 10.0);
    assert!(a < 0.5 && a > 0.4 && b < a && b > 0.0, "{a} {b}");

    let right = pan(Vec3::ZERO, Quat::IDENTITY, Vec3::X);
    let left = pan(Vec3::ZERO, Quat::IDENTITY, Vec3::NEG_X);
    assert!(right > 0.5 && left < -0.5);
    assert_eq!(
        pan(Vec3::ZERO, Quat::IDENTITY, Vec3::NEG_Z),
        0.0,
        "ahead is centred"
    );
    // Turned 180°: right becomes left.
    let turned = Quat::from_rotation_y(std::f32::consts::PI);
    assert!(pan(Vec3::ZERO, turned, Vec3::X) < -0.5);
}

#[test]
fn no_device_is_a_silent_no_op() {
    let mut audio = Audio::disabled("assets");
    assert!(!audio.is_available());
    let shot = audio.play("sfx/missing.ogg");
    audio.set_volume(shot, 0.5);
    audio.set_pitch(shot, 2.0);
    assert!(!audio.is_playing(shot));
    let music = audio.play_music("music/theme.ogg", 0.8, true);
    assert_ne!(shot, music);
    audio.set_master_volume(0.3);
    assert_eq!(audio.master_volume(), 0.3);
    assert!(!audio.preload("sfx/missing.ogg"));
    let mut world = World::new();
    audio.update(&mut world);
    // The one-shot is forgotten, looping music stays tracked until stopped.
    assert_eq!(audio.active_count(), 1);
    audio.stop_music();
    audio.stop_all();
    assert_eq!(audio.active_count(), 0);
}

#[test]
fn sources_start_on_awake_follow_entities_and_stop_on_despawn() {
    let mut app = App::new();
    app.add_plugin(AudioPlugin {
        enabled: false,
        ..Default::default()
    });
    let (awake, idle) = {
        let mut world = app.resources.fetch_mut::<World>();
        world.spawn((Transform::IDENTITY, AudioListener));
        let awake = world.spawn((
            Transform::from_xyz(3.0, 0.0, 0.0),
            AudioSourceDesc {
                clip: "sfx/engine.ogg".into(),
                looping: true,
                ..Default::default()
            },
        ));
        let idle = world.spawn((
            Transform::IDENTITY,
            AudioSourceDesc {
                clip: "sfx/door.ogg".into(),
                play_on_awake: false,
                ..Default::default()
            },
        ));
        (awake, idle)
    };
    app.update();
    let sound = {
        let world = app.resources.fetch::<World>();
        assert!(
            world
                .get::<&AudioSourceState>(idle)
                .unwrap()
                .sound
                .is_none()
        );
        world
            .get::<&AudioSourceState>(awake)
            .unwrap()
            .sound
            .expect("played on awake")
    };
    assert_eq!(app.resources.fetch::<Audio>().active_count(), 1);

    // Play / stop from code.
    {
        let mut audio = app.resources.fetch_mut::<Audio>();
        let mut world = app.resources.fetch_mut::<World>();
        let door = audio.play_source(&mut world, idle).unwrap();
        assert_ne!(door, sound);
        audio.stop_source(&mut world, idle);
        assert!(
            world
                .get::<&AudioSourceState>(idle)
                .unwrap()
                .sound
                .is_none()
        );
    }

    app.resources.fetch_mut::<World>().despawn(awake).unwrap();
    app.update();
    assert_eq!(
        app.resources.fetch::<Audio>().active_count(),
        0,
        "despawn stops it"
    );
}

fn looping(priority: u8) -> PlayParams {
    PlayParams {
        looping: true,
        priority,
        ..Default::default()
    }
}

#[test]
fn occlusion_curves() {
    assert_eq!(occlusion_gain(0.0), 1.0);
    assert!((occlusion_gain(1.0) - spatial::OCCLUDED_GAIN).abs() < 1e-6);
    assert!(occlusion_gain(0.5) < 1.0 && occlusion_gain(0.5) > occlusion_gain(1.0));
    assert_eq!(occlusion_gain(7.0), occlusion_gain(1.0), "clamped");
    assert!((occlusion_cutoff(0.0) - spatial::OPEN_CUTOFF).abs() < 1e-6);
    assert!((occlusion_cutoff(1.0) - spatial::OCCLUDED_CUTOFF).abs() < 1e-6);
    let (a, b) = (occlusion_cutoff(0.25), occlusion_cutoff(0.75));
    assert!(a > b && b > spatial::OCCLUDED_CUTOFF);
}

#[test]
fn buses_keep_their_settings() {
    let mut audio = Audio::disabled("assets");
    for (name, id) in [
        ("sfx", BusId::SFX),
        ("music", BusId::MUSIC),
        ("ambient", BusId::AMBIENT),
        ("voice", BusId::VOICE),
    ] {
        assert_eq!(audio.bus(name), Some(id));
        assert_eq!(audio.bus_name(id), Some(name));
    }
    assert_eq!(audio.bus("nope"), None);
    let ui = audio.add_bus("ui");
    assert_eq!(audio.add_bus("ui"), ui, "same name, same bus");
    assert_eq!(audio.bus_volume(ui), 1.0);

    audio.set_bus_volume(ui, 0.4);
    audio.set_bus_muted(ui, true);
    assert!(audio.is_bus_muted(ui));
    assert_eq!(audio.bus_volume(ui), 0.4, "mute keeps the volume");
    audio.set_bus_muted(ui, false);

    audio.set_bus_eq(
        ui,
        EqSettings {
            low_db: 6.0,
            mid_db: -100.0,
            high_db: 100.0,
        },
    );
    let eq = audio.bus_eq(ui);
    assert_eq!(
        (eq.low_db, eq.mid_db, eq.high_db),
        (6.0, -EQ_MAX_DB, EQ_MAX_DB)
    );

    audio.set_bus_reverb(ui, 3.0);
    assert_eq!(audio.bus_reverb(ui), 1.0);
    audio.set_reverb(ReverbSettings {
        feedback: 2.0,
        damping: -1.0,
        stereo_width: 0.5,
    });
    let r = audio.reverb();
    assert!(r.feedback < 1.0 && r.damping == 0.0 && r.stereo_width == 0.5);
    // Unknown buses are ignored, not a panic.
    audio.set_bus_volume(BusId(99), 1.0);
    assert_eq!(audio.bus_volume(BusId(99)), 0.0);
}

#[test]
fn sounds_play_on_their_bus_and_music_has_top_priority() {
    let mut audio = Audio::disabled("assets");
    let m = audio.play_music("music/a.ogg", 1.0, true);
    let p = &audio.playing[&m];
    assert_eq!((p.params.bus, p.params.priority), (BusId::MUSIC, 0));
    assert!(p.params.stream && !p.params.occludable);
    assert_eq!(audio.music(), Some(m));
    // A crossfade replaces it.
    let m2 = audio.play_music_fade("music/b.ogg", 1.0, true, Duration::from_secs(2));
    assert_ne!(m, m2);
    assert_eq!(audio.music(), Some(m2));
    assert_eq!(audio.active_count(), 1);
}

#[test]
fn voice_limit_keeps_the_most_important_sounds() {
    let mut audio = Audio::disabled("assets");
    audio.set_max_voices(2);
    assert_eq!(audio.max_voices(), 2);
    let low = audio.play_with("a.ogg", looping(200));
    let mid = audio.play_with("a.ogg", looping(100));
    assert_eq!(audio.voice_count(), 2);
    assert!(!audio.is_virtual(low) && !audio.is_virtual(mid));

    // A more important sound steals the voice of the least important one.
    let high = audio.play_with("a.ogg", looping(10));
    assert_eq!(audio.voice_count(), 2);
    assert!(audio.is_virtual(low), "looping loser waits");
    assert!(!audio.is_virtual(mid) && !audio.is_virtual(high));
    assert_eq!(audio.active_count(), 3);

    // A one-shot that can't get a voice is dropped.
    let shot = audio.play_with(
        "b.ogg",
        PlayParams {
            priority: 250,
            ..Default::default()
        },
    );
    assert!(!audio.playing.contains_key(&shot));
    assert_eq!(audio.active_count(), 3);

    // Changing priorities reshuffles the voices.
    audio.set_priority(low, 0);
    assert!(!audio.is_virtual(low) && audio.is_virtual(mid));

    // A freed voice goes to a waiting sound.
    audio.stop(high);
    let mut world = World::new();
    audio.update_with(&mut world, 0.016);
    assert!(!audio.is_virtual(low) && !audio.is_virtual(mid));
    assert_eq!(audio.voice_count(), 2);

    // Raising the limit wakes everything.
    audio.set_max_voices(8);
    assert_eq!(audio.voice_count(), audio.active_count());
}

#[test]
fn equal_priority_goes_to_the_louder_sound() {
    let mut audio = Audio::disabled("assets");
    audio.set_max_voices(1);
    let far = audio.play_with(
        "a.ogg",
        PlayParams {
            position: Some(Vec3::new(30.0, 0.0, 0.0)),
            looping: true,
            ..Default::default()
        },
    );
    let near = audio.play_with(
        "a.ogg",
        PlayParams {
            position: Some(Vec3::new(2.0, 0.0, 0.0)),
            looping: true,
            ..Default::default()
        },
    );
    assert!(audio.is_virtual(far) && !audio.is_virtual(near));
    // The listener walks over to the far one: the voices trade places.
    audio.listener = (Vec3::new(30.0, 0.0, 0.0), Quat::IDENTITY);
    audio.update_with(&mut World::new(), 0.016);
    assert!(!audio.is_virtual(far) && audio.is_virtual(near));
}

#[test]
fn streams_are_limited_separately() {
    let mut audio = Audio::disabled("assets");
    audio.set_max_streams(1);
    let stream = |priority| PlayParams {
        stream: true,
        looping: true,
        priority,
        ..Default::default()
    };
    let a = audio.play_with("m1.ogg", stream(50));
    let b = audio.play_with("m2.ogg", stream(60));
    let c = audio.play_with("s.ogg", looping(70));
    assert_eq!(audio.stream_count(), 1);
    assert!(
        !audio.is_virtual(a) && audio.is_virtual(b),
        "one stream only"
    );
    assert!(!audio.is_virtual(c), "a short clip is not a stream");
    assert_eq!(audio.max_streams(), 1);
    audio.set_max_streams(2);
    assert_eq!(audio.stream_count(), 2);
    audio.set_stream_threshold(1024);
    assert_eq!(audio.stream_threshold(), 1024);
}

#[test]
fn occluder_muffles_and_eases() {
    let mut audio = Audio::disabled("assets");
    let wall = std::rc::Rc::new(std::cell::Cell::new(1.0f32));
    let asked = std::rc::Rc::new(std::cell::Cell::new(None));
    audio.set_occluder({
        let (wall, asked) = (wall.clone(), asked.clone());
        move |from, to| {
            asked.set(Some((from, to)));
            wall.get()
        }
    });
    let at = Vec3::new(5.0, 0.0, 0.0);
    let heard = audio.play_with(
        "a.ogg",
        PlayParams {
            looping: true,
            ..PlayParams::at(at)
        },
    );
    let flat = audio.play_with("a.ogg", looping(128));
    let deaf = audio.play_with(
        "a.ogg",
        PlayParams {
            occludable: false,
            looping: true,
            ..PlayParams::at(at)
        },
    );
    let mut world = World::new();
    audio.update_with(&mut world, 0.016);
    assert_eq!(asked.get(), Some((Vec3::ZERO, at)), "listener to source");
    assert_eq!(
        audio.occlusion(heard),
        Some(1.0),
        "first measure applies at once"
    );
    assert_eq!(
        audio.occlusion(flat),
        Some(0.0),
        "2D sounds are never occluded"
    );
    assert_eq!(audio.occlusion(deaf), Some(0.0));

    // The wall goes away: the occlusion fades, not jumps.
    wall.set(0.0);
    audio.update_with(&mut world, 0.05);
    let mid = audio.occlusion(heard).unwrap();
    assert!(mid > 0.0 && mid < 1.0, "{mid}");
    for _ in 0..100 {
        audio.update_with(&mut world, 0.05);
    }
    assert!(audio.occlusion(heard).unwrap() < 0.01);

    // Manual override beats the occluder; `None` hands control back.
    audio.set_occlusion(heard, Some(0.5));
    audio.update_with(&mut world, 10.0);
    assert!((audio.occlusion(heard).unwrap() - 0.5).abs() < 0.01);
    audio.set_occlusion(heard, None);
    audio.update_with(&mut world, 10.0);
    assert!(audio.occlusion(heard).unwrap() < 0.01);

    // Muffled sounds count as quieter for the voice ranking.
    let (v, _) = audio.gain(&audio.playing[&heard].params, 1.0);
    let (clear, _) = audio.gain(&audio.playing[&heard].params, 0.0);
    assert!(v < clear);
    audio.clear_occluder();
    assert!(audio.occluder.is_none());
}

#[test]
fn clip_cache_bookkeeping() {
    let mut audio = Audio::disabled("assets");
    assert_eq!(audio.cached_clips(), 0);
    assert!(!audio.unload_clip("x.ogg"));
    audio.clear_clips();
    assert_eq!(audio.position(SoundId(0)), None);
}

#[test]
fn entity_source_loses_its_voice_cleanly() {
    let mut app = App::new();
    app.add_plugin(AudioPlugin {
        enabled: false,
        ..Default::default()
    });
    app.resources.fetch_mut::<Audio>().set_max_voices(1);
    let (keep, shot) = {
        let mut world = app.resources.fetch_mut::<World>();
        world.spawn((Transform::IDENTITY, AudioListener));
        let keep = world.spawn((
            Transform::IDENTITY,
            AudioSourceDesc {
                clip: "a.ogg".into(),
                looping: true,
                ..Default::default()
            },
        ));
        let shot = world.spawn((
            Transform::from_xyz(40.0, 0.0, 0.0),
            AudioSourceDesc {
                clip: "b.ogg".into(),
                ..Default::default()
            },
        ));
        (keep, shot)
    };
    app.update();
    let world = app.resources.fetch::<World>();
    assert!(
        world
            .get::<&AudioSourceState>(keep)
            .unwrap()
            .sound
            .is_some()
    );
    assert!(
        world
            .get::<&AudioSourceState>(shot)
            .unwrap()
            .sound
            .is_none(),
        "the far one-shot got no voice and its state says so"
    );
    assert_eq!(app.resources.fetch::<Audio>().active_count(), 1);
}

/// Mono 16-bit PCM WAV of `seconds` of a quiet tone.
fn wav(seconds: f32) -> Vec<u8> {
    let rate = 44_100u32;
    let n = (rate as f32 * seconds) as u32;
    let mut out = Vec::new();
    out.extend(b"RIFF");
    out.extend((36 + n * 2).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend((n * 2).to_le_bytes());
    for i in 0..n {
        let v = ((i as f32 * 0.05).sin() * 3000.0) as i16;
        out.extend(v.to_le_bytes());
    }
    out
}

/// The mixer for real, on kira's mock backend (no sound card): tracks, sends
/// and effects must build, sounds must start on them and finish.
#[test]
fn mixer_runs_on_the_mock_backend() {
    let dir = std::env::temp_dir().join(format!("olvin_audio_mix_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tone.wav"), wav(0.2)).unwrap();
    std::fs::write(dir.join("long.wav"), wav(1.0)).unwrap();

    let mut audio = Audio::open(&dir);
    assert!(audio.is_available());
    assert!(audio.reverb.is_some(), "reverb send track exists");
    assert!(
        audio.buses.iter().all(|b| b.hw.is_some()),
        "every bus has a track"
    );

    // Settings reach the tracks without errors, also on a bus added later.
    let ui = audio.add_bus("ui");
    assert!(audio.buses[ui.0].hw.is_some());
    audio.set_bus_volume(BusId::MUSIC, 0.5);
    audio.set_bus_muted(BusId::VOICE, true);
    audio.set_bus_eq(
        ui,
        EqSettings {
            low_db: 3.0,
            mid_db: -2.0,
            high_db: 6.0,
        },
    );
    audio.set_bus_reverb(ui, 0.4);
    audio.set_reverb(ReverbSettings {
        feedback: 0.9,
        damping: 0.3,
        stereo_width: 0.7,
    });
    audio.set_master_volume(0.8);

    // A positional sound gets its own low-pass track; a 2D one does not.
    let heard = audio.play_with(
        "tone.wav",
        PlayParams {
            bus: ui,
            ..PlayParams::at(Vec3::new(4.0, 0.0, 0.0))
        },
    );
    let flat = audio.play("tone.wav");
    let streamed = audio.play_with(
        "long.wav",
        PlayParams {
            stream: true,
            ..Default::default()
        },
    );
    let music = audio.play_music_fade("long.wav", 1.0, true, Duration::from_millis(50));
    assert!(audio.playing[&heard].occ_track.is_some());
    assert!(audio.playing[&flat].occ_track.is_none());
    assert!(audio.playing[&music].occ_track.is_none());
    assert_eq!(
        audio.cached_clips(),
        1,
        "only the decoded clip is cached, streams are not"
    );
    audio.set_occlusion(heard, Some(1.0));

    let mut world = World::new();
    let run = |audio: &mut Audio, frames: usize| {
        let world = &mut World::new();
        for _ in 0..frames {
            audio
                .manager
                .as_mut()
                .unwrap()
                .backend_mut()
                .on_start_processing();
            audio.manager.as_mut().unwrap().backend_mut().process();
            audio.update_with(world, 0.016);
        }
    };
    run(&mut audio, 4);
    assert!(audio.is_playing(heard) && audio.is_playing(streamed) && audio.is_playing(music));
    assert!(audio.occlusion(heard).unwrap() > 0.99);
    assert!(audio.position(music).is_some());

    // Voice stealing on a real mixer: a loop loses its voice and gets it back.
    audio.set_max_voices(1);
    audio.update_with(&mut world, 0.016);
    assert_eq!(audio.voice_count(), 1);
    assert!(!audio.is_virtual(music), "music has priority 0");
    assert!(!audio.is_playing(heard));
    audio.set_max_voices(16);
    run(&mut audio, 2);
    assert!(audio.is_playing(music));

    audio.stop_all();
    assert_eq!(audio.active_count(), 0);
    assert!(audio.unload_clip("tone.wav"));
    assert_eq!(audio.cached_clips(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `Time::pause` holds sfx and ambient voices where they are, music plays on,
/// a sound started during the pause is not held, resume continues the rest.
#[test]
fn time_pause_holds_pausable_buses() {
    let dir = std::env::temp_dir().join(format!("olvin_audio_pause_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("assets/loop.wav"), wav(1.0)).unwrap();

    let mut app = App::new();
    app.insert_resource(ProjectPaths::new(&dir))
        .add_plugin(AudioPlugin::default());
    let looping = |bus| PlayParams {
        looping: true,
        bus,
        ..Default::default()
    };
    let (sfx, ambient, music) = {
        let mut audio = app.resources.fetch_mut::<Audio>();
        assert_eq!(audio.assets_dir(), dir.join("assets"), "project assets");
        assert!(audio.is_available(), "mock backend");
        assert!(!audio.is_bus_pausable(BusId::MUSIC) && audio.is_bus_pausable(BusId::SFX));
        (
            audio.play_with("loop.wav", looping(BusId::SFX)),
            audio.play_with("loop.wav", looping(BusId::AMBIENT)),
            audio.play_music("loop.wav", 1.0, true),
        )
    };
    let frames = |app: &mut App, n: usize| {
        for _ in 0..n {
            {
                let mut audio = app.resources.fetch_mut::<Audio>();
                let backend = audio.manager.as_mut().unwrap().backend_mut();
                backend.on_start_processing();
                backend.process();
            }
            app.update();
        }
    };
    let state = |app: &App, id| {
        let audio = app.resources.fetch::<Audio>();
        audio.playing[&id].handle.as_ref().unwrap().state()
    };
    frames(&mut app, 3);
    assert_eq!(state(&app, sfx), PlaybackState::Playing);

    app.resources.fetch_mut::<Time>().pause();
    frames(&mut app, 20);
    let menu = app
        .resources
        .fetch_mut::<Audio>()
        .play_with("loop.wav", looping(BusId::SFX));
    frames(&mut app, 3);
    {
        let audio = app.resources.fetch::<Audio>();
        assert!(audio.is_paused());
        assert!(audio.is_sound_paused(sfx) && audio.is_sound_paused(ambient));
        assert!(!audio.is_sound_paused(music), "music is not pausable");
        assert!(!audio.is_sound_paused(menu), "started during the pause");
    }
    assert_eq!(state(&app, sfx), PlaybackState::Paused);
    assert_eq!(state(&app, ambient), PlaybackState::Paused);
    assert_eq!(state(&app, music), PlaybackState::Playing);
    let held_at = app.resources.fetch::<Audio>().position(sfx).unwrap();
    frames(&mut app, 3);
    assert_eq!(app.resources.fetch::<Audio>().position(sfx), Some(held_at));

    // Music can be made pausable while paused; a single sound held by hand
    // stays held through resume.
    app.resources
        .fetch_mut::<Audio>()
        .set_bus_pausable(BusId::MUSIC, true);
    app.resources.fetch_mut::<Audio>().pause_sound(ambient);
    frames(&mut app, 20);
    assert_eq!(state(&app, music), PlaybackState::Paused);

    app.resources.fetch_mut::<Time>().resume();
    frames(&mut app, 3);
    {
        let audio = app.resources.fetch::<Audio>();
        assert!(!audio.is_paused() && !audio.is_sound_paused(sfx));
        assert!(audio.is_sound_paused(ambient), "held by pause_sound");
    }
    let going = |s| matches!(s, PlaybackState::Playing | PlaybackState::Resuming);
    assert!(going(state(&app, sfx)));
    assert!(app.resources.fetch::<Audio>().position(sfx).unwrap() > held_at);
    app.resources.fetch_mut::<Audio>().resume_sound(ambient);
    frames(&mut app, 3);
    assert!(going(state(&app, ambient)));

    // Opting out: the game pause no longer reaches audio.
    app.resources
        .fetch_mut::<Audio>()
        .set_follow_time_pause(false);
    app.resources.fetch_mut::<Time>().pause();
    frames(&mut app, 2);
    assert!(!app.resources.fetch::<Audio>().is_sound_paused(sfx));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sounds_that_fail_to_load_do_not_hold_voices() {
    let dir = std::env::temp_dir().join(format!("olvin_audio_missing_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut audio = Audio::open(&dir);
    assert!(audio.is_available());
    audio.set_max_voices(2);
    for _ in 0..4 {
        audio.play_with("typo.wav", looping(0));
    }
    let mut world = World::new();
    // Those waiting for a voice are tried (and forgotten) once one frees up.
    for _ in 0..3 {
        audio.update_with(&mut world, 0.016);
    }
    assert_eq!(audio.active_count(), 0, "looping ghosts are forgotten");
    std::fs::write(dir.join("tone.wav"), wav(0.2)).unwrap();
    let id = audio.play_with("tone.wav", looping(128));
    audio.update_with(&mut world, 0.016);
    assert!(audio.is_playing(id), "a real sound still gets a voice");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_finite_inputs_stay_silent_not_nan() {
    assert_eq!(attenuation(f32::NAN, 1.0, 10.0), 0.0);
    assert_eq!(pan(Vec3::NAN, Quat::IDENTITY, Vec3::X), 0.0);
    assert_eq!(pan(Vec3::ZERO, Quat::IDENTITY, Vec3::INFINITY), 0.0);
    assert_eq!(amplitude_to_db(f32::NAN), Decibels::SILENCE);
}

/// The audio server goes down (the heartbeat stops) and comes back: loops and
/// music wait and restart, one-shots are dropped, buses and master volume survive.
#[test]
fn a_lost_output_is_noticed_and_reconnected() {
    let dir = std::env::temp_dir().join(format!("olvin_audio_lost_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tone.wav"), wav(0.2)).unwrap();
    std::fs::write(dir.join("long.wav"), wav(1.0)).unwrap();

    let mut audio = Audio::open(&dir);
    audio.set_bus_volume(BusId::SFX, 0.3);
    audio.set_master_volume(0.6);
    let looped = audio.play_with("tone.wav", looping(5));
    let shot = audio.play("long.wav");
    let music = audio.play_music("long.wav", 1.0, false);
    let mut world = World::new();
    // The stream renders: the heartbeat moves, nothing happens.
    let render = |audio: &mut Audio, world: &mut World, frames: usize| {
        for _ in 0..frames {
            let backend = audio.manager.as_mut().unwrap().backend_mut();
            backend.on_start_processing();
            backend.process();
            audio.update_with(world, 0.016);
        }
    };
    let t0 = Instant::now();
    render(&mut audio, &mut world, 20);
    audio.supervise(t0);
    assert!(
        audio.device.probe.as_ref().unwrap().ticks > 0,
        "the heartbeat ticks while the stream renders"
    );
    audio.supervise(t0 + Duration::from_secs(1));
    assert!(audio.is_available(), "a beating heart is alive");
    assert!(audio.is_playing(looped) && audio.is_playing(shot) && audio.is_playing(music));

    // The server dies: no more rendering, the heartbeat stands still.
    audio.supervise(t0 + device::STALL + Duration::from_secs(2));
    assert!(!audio.is_available(), "a stalled output counts as lost");
    assert!(audio.is_virtual(looped) && audio.is_virtual(music));
    audio.update_with(&mut world, 0.016);
    assert!(
        !audio.is_playing(shot) && !audio.is_virtual(shot),
        "one-shots are dropped"
    );
    // While it is down, frames go on and the loop keeps waiting.
    for _ in 0..5 {
        audio.update_with(&mut world, 0.016);
    }
    assert!(audio.is_virtual(looped));
    // Too early to retry.
    audio.supervise(t0 + device::STALL + Duration::from_secs(3));
    assert!(!audio.is_available());

    // It is back.
    audio.supervise(t0 + device::STALL + device::RETRY + Duration::from_secs(3));
    assert!(audio.is_available());
    assert_eq!(audio.reconnect_count(), 1);
    assert!(audio.reverb.is_some() && audio.buses.iter().all(|b| b.hw.is_some()));
    assert_eq!(audio.bus_volume(BusId::SFX), 0.3);
    assert_eq!(audio.master_volume(), 0.6);
    render(&mut audio, &mut world, 4);
    assert!(audio.is_playing(looped), "the loop restarted");
    assert!(audio.is_playing(music), "the music restarted");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn disabled_audio_never_looks_for_a_device() {
    let mut audio = Audio::disabled(".");
    audio.supervise(Instant::now() + Duration::from_secs(60));
    assert!(!audio.is_available());
    assert_eq!(audio.reconnect_count(), 0);
}
