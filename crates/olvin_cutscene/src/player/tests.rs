use glam::Vec3;
use olvin_scene::file::euler_to_quat;

use super::*;
use crate::data::{Cue, CueAction, Ease, Key, TransformTrack};

fn key(time: f32, x: f32, ease: Ease) -> Key {
    Key {
        time,
        position: Some([x, 0.0, 0.0]),
        ease,
        ..Default::default()
    }
}

fn intro() -> Cutscene {
    Cutscene {
        name: "intro".into(),
        duration: 4.0,
        tracks: vec![TransformTrack {
            target: "Door".into(),
            keys: vec![key(0.0, 0.0, Ease::Linear), key(4.0, 4.0, Ease::Linear)],
        }],
        cues: vec![
            Cue {
                time: 0.0,
                action: CueAction::Camera("Shot".into()),
            },
            Cue {
                time: 2.0,
                action: CueAction::Message {
                    name: "open".into(),
                    value: CueValue::Number(7.0),
                    target: Some("Door".into()),
                },
            },
        ],
    }
}

struct Scene {
    app: App,
    door: Entity,
    shot: Entity,
    game: Entity,
}

fn scene() -> Scene {
    let mut app = App::new();
    app.insert_resource(Time::with_fixed_delta(0.5));
    app.add_plugin(CutscenePlugin);
    let (door, shot, game) = {
        let mut w = app.resources.fetch_mut::<World>();
        (
            w.spawn((Name("Door".into()), Transform::IDENTITY)),
            w.spawn((Name("Shot".into()), Transform::IDENTITY, Camera::default())),
            w.spawn((Name("Game".into()), Transform::IDENTITY, Camera::default())),
        )
    };
    Scene {
        app,
        door,
        shot,
        game,
    }
}

impl Scene {
    fn x(&self) -> f32 {
        self.app
            .resources
            .fetch::<World>()
            .get::<&Transform>(self.door)
            .unwrap()
            .translation
            .x
    }
    fn active(&self) -> Vec<Entity> {
        self.app
            .resources
            .fetch::<World>()
            .query::<(Entity, &ActiveCamera)>()
            .iter()
            .map(|(e, _)| e)
            .collect()
    }
    fn play(&mut self, c: Cutscene) -> CutsceneId {
        self.app
            .resources
            .fetch_mut::<Cutscenes>()
            .play(Arc::new(c))
            .unwrap()
    }
}

#[test]
fn a_cutscene_moves_objects_switches_cameras_and_sends_messages() {
    let mut s = scene();
    let id = s.play(intro());
    let mut messages = EventCursor::default();
    let mut seen = Vec::new();
    let mut finished = EventCursor::default();
    let mut ends = Vec::new();
    let mut positions = Vec::new();
    for _ in 0..8 {
        s.app.update();
        positions.push(s.x());
        let ev = s.app.resources.fetch::<Events<ScriptMessage>>();
        seen.extend(ev.read(&mut messages).cloned());
        let fin = s.app.resources.fetch::<Events<CutsceneFinished>>();
        ends.extend(fin.read(&mut finished).cloned());
    }
    // 0.5 s per frame over 4 s of 0 -> 4 metres.
    assert_eq!(&positions[..4], &[0.5, 1.0, 1.5, 2.0]);
    assert_eq!(s.x(), 4.0, "it stays where the cutscene left it");
    let open: Vec<_> = seen.iter().filter(|m| m.name == "open").collect();
    assert_eq!(open.len(), 1, "a cue happens once");
    assert_eq!(open[0].value, ScriptValue::Number(7.0));
    assert_eq!(open[0].target, Some(s.door));
    assert!(
        seen.iter()
            .any(|m| m.name == "cutscene_finished" && m.value == ScriptValue::Str("intro".into()))
    );
    assert_eq!(ends.len(), 1);
    assert_eq!((ends[0].id, ends[0].cut_short), (id, false));
    assert!(
        s.active().is_empty(),
        "the game's camera is back when it ends"
    );
    assert!(!s.app.resources.fetch::<Cutscenes>().is_busy());
}

#[test]
fn the_camera_cue_hands_the_view_to_the_shot_while_it_runs() {
    let mut s = scene();
    s.play(intro());
    s.app.update();
    assert_eq!(s.active(), [s.shot]);
    assert_ne!(s.shot, s.game);
}

#[test]
fn skip_jumps_to_the_end_with_the_cues_still_ahead() {
    let mut s = scene();
    let id = s.play(intro());
    s.app.update();
    assert!(s.app.resources.fetch_mut::<Cutscenes>().skip(id));
    s.app.update();
    assert_eq!(s.x(), 4.0);
    let ev = s.app.resources.fetch::<Events<ScriptMessage>>();
    let mut cursor = EventCursor::default();
    assert_eq!(
        ev.read(&mut cursor).filter(|m| m.name == "open").count(),
        1,
        "the open cue was not lost"
    );
    let fin = s.app.resources.fetch::<Events<CutsceneFinished>>();
    assert!(fin.read(&mut EventCursor::default()).any(|f| f.cut_short));
    drop((ev, fin));
    assert!(s.active().is_empty());
    assert!(!s.app.resources.fetch::<Cutscenes>().is_playing(id));
}

#[test]
fn stop_leaves_things_where_they_are_and_pause_holds_time() {
    let mut s = scene();
    let id = s.play(intro());
    s.app.update();
    s.app.update();
    let at = s.x();
    s.app.resources.fetch_mut::<Cutscenes>().pause(id, true);
    s.app.update();
    assert_eq!(s.x(), at, "paused");
    s.app.resources.fetch_mut::<Cutscenes>().pause(id, false);
    s.app.update();
    assert!(s.x() > at);
    let at = s.x();
    s.app.resources.fetch_mut::<Cutscenes>().stop(id);
    s.app.update();
    s.app.update();
    assert_eq!(s.x(), at);
    assert!(!s.app.resources.fetch::<Cutscenes>().is_busy());
    assert!(s.active().is_empty());
}

#[test]
fn a_looping_cutscene_runs_again_and_its_cues_repeat() {
    let mut s = scene();
    let looping = Cutscene {
        name: "loop".into(),
        duration: 1.0,
        tracks: Vec::new(),
        cues: vec![Cue {
            time: 0.5,
            action: CueAction::Message {
                name: "tick".into(),
                value: CueValue::Nil,
                target: None,
            },
        }],
    };
    let id = s
        .app
        .resources
        .fetch_mut::<Cutscenes>()
        .play_looping(Arc::new(looping))
        .unwrap();
    let mut cursor = EventCursor::default();
    let mut ticks = 0;
    for _ in 0..8 {
        s.app.update();
        ticks += s
            .app
            .resources
            .fetch::<Events<ScriptMessage>>()
            .read(&mut cursor)
            .filter(|m| m.name == "tick")
            .count();
    }
    assert_eq!(ticks, 4, "once per second of the 4 s that passed");
    assert!(s.app.resources.fetch::<Cutscenes>().is_playing(id));
}

#[test]
fn a_missing_object_is_skipped_not_fatal() {
    let mut s = scene();
    let mut c = intro();
    c.tracks.push(TransformTrack {
        target: "Nobody".into(),
        keys: vec![key(0.0, 1.0, Ease::Linear)],
    });
    c.cues.push(Cue {
        time: 1.0,
        action: CueAction::Camera("Nowhere".into()),
    });
    s.play(c);
    for _ in 0..8 {
        s.app.update();
    }
    assert_eq!(s.x(), 4.0);
}

#[test]
fn scripts_start_and_skip_cutscenes_with_messages() {
    let dir = std::env::temp_dir().join(format!("olvin-cutscene-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets/cutscenes")).unwrap();
    std::fs::write(
        dir.join("assets/cutscenes/intro.cutscene.ron"),
        intro().to_ron().unwrap(),
    )
    .unwrap();
    let mut app = App::new();
    app.insert_resource(ProjectPaths::new(&dir));
    app.insert_resource(Time::with_fixed_delta(0.5));
    app.add_plugin(CutscenePlugin);
    let door = app
        .resources
        .fetch_mut::<World>()
        .spawn((Name("Door".into()), Transform::IDENTITY));
    let send = |app: &App, name: &str, value: ScriptValue| {
        app.resources
            .fetch_mut::<Events<ScriptMessage>>()
            .send(ScriptMessage::new(name, value));
    };
    send(
        &app,
        "cutscene:play",
        ScriptValue::Str("cutscenes/intro.cutscene.ron".into()),
    );
    app.update();
    assert!(app.resources.fetch::<Cutscenes>().is_busy());
    // Paths outside the assets folder are refused.
    send(
        &app,
        "cutscene:play",
        ScriptValue::Str("../secret.cutscene.ron".into()),
    );
    app.update();
    send(&app, "cutscene:skip", ScriptValue::Nil);
    app.update();
    app.update();
    assert!(!app.resources.fetch::<Cutscenes>().is_busy());
    let x = app
        .resources
        .fetch::<World>()
        .get::<&Transform>(door)
        .unwrap()
        .translation
        .x;
    assert_eq!(x, 4.0);
}

#[test]
fn rotation_keys_blend_as_euler_degrees() {
    let track = TransformTrack {
        target: "x".into(),
        keys: vec![
            Key {
                time: 0.0,
                rotation: Some([0.0, 0.0, 0.0]),
                ..Default::default()
            },
            Key {
                time: 2.0,
                rotation: Some([0.0, 90.0, 0.0]),
                ..Default::default()
            },
        ],
    };
    let t = track.sample(1.0, Transform::IDENTITY);
    assert!(
        t.rotation
            .abs_diff_eq(euler_to_quat(Vec3::new(0.0, 45.0, 0.0)), 1e-5)
    );
    assert_eq!(
        t.translation,
        Vec3::ZERO,
        "channels no key sets are left alone"
    );
}
