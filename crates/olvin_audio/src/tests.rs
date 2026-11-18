//! Never opens a device: everything runs on the no-device fallback.

use glam::{Quat, Vec3};
use olvin_core::App;
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
