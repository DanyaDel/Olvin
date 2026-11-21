use std::sync::Arc;

use glam::{Mat4, Quat, Vec3, Vec4};
use olvin_core::{App, EventCursor, Events};
use olvin_ecs::World;

use crate::*;

fn joint(name: &str, parent: Option<usize>, target: usize, at: Vec3) -> Joint {
    Joint {
        name: name.into(),
        parent,
        target,
        rest: JointPose {
            translation: at,
            ..JointPose::IDENTITY
        },
        inverse_bind: Mat4::IDENTITY,
    }
}

/// Root at the origin, tip one metre above it. Bind matrices are the inverse
/// of the rest globals, as in glTF.
fn arm() -> Skeleton {
    let mut joints = vec![
        joint("root", None, 10, Vec3::ZERO),
        joint("tip", Some(0), 11, Vec3::Y),
    ];
    joints[1].inverse_bind = Mat4::from_translation(-Vec3::Y);
    Skeleton::new(joints).unwrap()
}

fn move_x(target: usize, to: f32, duration: f32) -> Channel {
    Channel {
        target,
        property: Property::Translation,
        interpolation: Interpolation::Linear,
        times: vec![0.0, duration],
        values: vec![Vec4::ZERO, Vec4::new(to, 0.0, 0.0, 0.0)],
    }
}

/// A constant pose: the target stays at x for the whole second.
fn flat(target: usize, x: f32) -> Channel {
    let mut c = move_x(target, x, 1.0);
    c.values[0].x = x;
    c
}

#[test]
fn rest_pose_gives_identity_skin_matrices() {
    let sk = arm();
    for m in sk.skin_matrices(&sk.rest_pose()) {
        assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-6));
    }
}

#[test]
fn rotating_the_root_swings_the_tip() {
    let sk = arm();
    let mut pose = sk.rest_pose();
    pose.joints[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    // A vertex at the tip's bind position, fully on the tip joint.
    let p = sk.skin_matrices(&pose)[1].transform_point3(Vec3::Y);
    assert!(p.abs_diff_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-5), "{p:?}");
}

#[test]
fn globals_work_for_children_listed_before_parents() {
    let mut a = arm().joints;
    a.reverse();
    a[0].parent = Some(1);
    a[1].parent = None;
    let sk = Skeleton::new(a).unwrap();
    let g = sk.globals(&sk.rest_pose());
    assert!(g[0].transform_point3(Vec3::ZERO).abs_diff_eq(Vec3::Y, 1e-6));
}

#[test]
fn skeleton_rejects_cycles_and_bad_parents() {
    let mut j = arm().joints;
    j[0].parent = Some(1);
    assert!(Skeleton::new(j).unwrap_err().contains("cycle"));
    let mut j = arm().joints;
    j[1].parent = Some(7);
    assert!(Skeleton::new(j).is_err());
}

#[test]
fn channel_interpolation_modes() {
    let mut c = move_x(10, 2.0, 1.0);
    assert_eq!(c.sample(-1.0).unwrap().x, 0.0);
    assert!((c.sample(0.25).unwrap().x - 0.5).abs() < 1e-6);
    assert_eq!(c.sample(5.0).unwrap().x, 2.0);
    c.interpolation = Interpolation::Step;
    assert_eq!(c.sample(0.99).unwrap().x, 0.0);

    // Cubic spline with zero tangents: smoothstep, so 0.5 at the middle.
    let key = |x: f32| Vec4::new(x, 0.0, 0.0, 0.0);
    c.interpolation = Interpolation::CubicSpline;
    c.values = vec![
        Vec4::ZERO,
        key(0.0),
        Vec4::ZERO,
        Vec4::ZERO,
        key(2.0),
        Vec4::ZERO,
    ];
    assert!((c.sample(0.5).unwrap().x - 1.0).abs() < 1e-6);
    assert!(c.sample(0.1).unwrap().x < 0.2 * 2.0);

    // Mismatched data samples to nothing instead of panicking.
    c.values.pop();
    assert!(c.sample(0.5).is_none());
}

#[test]
fn rotation_channels_take_the_short_way() {
    let q = |a: f32| Vec4::from(Quat::from_rotation_y(a));
    let c = Channel {
        target: 10,
        property: Property::Rotation,
        interpolation: Interpolation::Linear,
        times: vec![0.0, 1.0],
        values: vec![q(0.0), q(std::f32::consts::FRAC_PI_2)],
    };
    let mid = Quat::from_vec4(c.sample(0.5).unwrap());
    assert!(mid.angle_between(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4)) < 1e-5);
}

#[test]
fn clip_duration_and_events_across_the_loop_point() {
    let clip = Clip::new("walk", vec![move_x(10, 1.0, 2.0)])
        .with_event(1.5, "step")
        .with_event(0.0, "start");
    assert_eq!(clip.duration, 2.0);
    let mut out = Vec::new();
    clip.events_between(1.0, 1.6, &mut out);
    assert_eq!(out, ["step"]);
    out.clear();
    clip.events_between(1.6, 2.0, &mut out);
    assert!(out.is_empty());
    out.clear();
    clip.events_between(0.0, 0.1, &mut out);
    assert_eq!(out, ["start"]);
}

fn run(a: &mut Animator, seconds: f32, step: f32) {
    let mut t = 0.0;
    while t < seconds - 1e-6 {
        a.update(step);
        t += step;
    }
}

#[test]
fn animator_loops_a_clip() {
    let sk = Arc::new(arm());
    let clips: Arc<[Clip]> = vec![Clip::new("a", vec![move_x(10, 4.0, 2.0)])].into();
    let mut a = Animator::looping(sk.clone(), clips, 0);
    run(&mut a, 1.0, 0.25);
    assert!((a.pose.joints[0].translation.x - 2.0).abs() < 1e-4);
    run(&mut a, 2.0, 0.25); // 3 s in: wrapped to 1 s
    assert!((a.pose.joints[0].translation.x - 2.0).abs() < 1e-4);
    assert!(a.version >= 12);
}

fn locomotion() -> (Arc<Skeleton>, Arc<[Clip]>) {
    let clips = vec![
        Clip::new("idle", vec![move_x(10, 0.0, 1.0)]),
        Clip::new("walk", vec![move_x(10, 10.0, 1.0)]).with_event(0.5, "foot"),
        Clip::new("jump", vec![move_x(10, 100.0, 1.0)]),
    ];
    (Arc::new(arm()), clips.into())
}

#[test]
fn graph_cross_fades_on_a_parameter() {
    let sk = Arc::new(arm());
    let clips: Arc<[Clip]> = vec![
        Clip::new("idle", vec![flat(10, 0.0)]),
        Clip::new("walk", vec![flat(10, 10.0)]),
    ]
    .into();
    let graph = AnimGraph {
        states: vec![
            State::new("idle", Motion::Clip(0)),
            State::new("walk", Motion::Clip(1)),
        ],
        transitions: vec![
            Transition::new(Some(0), 1, 1.0).when("speed", Test::Greater(0.1)),
            Transition::new(Some(1), 0, 0.0).when("speed", Test::Less(0.1)),
        ],
        default: 0,
    };
    let mut a = Animator::new(sk, clips).with_graph(graph);
    run(&mut a, 0.5, 0.1);
    assert_eq!(a.layers[0].state.current, 0);
    assert!(a.pose.joints[0].translation.x.abs() < 1e-5);

    a.params.set_float("speed", 1.0);
    a.update(0.1); // transition starts
    assert!(a.layers[0].state.is_transitioning());
    run(&mut a, 0.5, 0.1);
    let x = a.pose.joints[0].translation.x;
    assert!((x - 5.0).abs() < 1.0, "halfway through the fade: {x}");
    run(&mut a, 0.6, 0.1);
    assert!(!a.layers[0].state.is_transitioning());
    assert_eq!(a.layers[0].state.current, 1);
    assert!((a.pose.joints[0].translation.x - 10.0).abs() < 1e-4);

    // Instant way back.
    a.params.set_float("speed", 0.0);
    a.update(0.1);
    assert_eq!(a.layers[0].state.current, 0);
    assert!(a.pose.joints[0].translation.x.abs() < 1e-4);
}

#[test]
fn triggers_fire_once_and_exit_time_waits() {
    let (sk, clips) = locomotion();
    let graph = AnimGraph {
        states: vec![
            State::new("idle", Motion::Clip(0)),
            State::new("jump", Motion::Clip(2)).once(),
        ],
        transitions: vec![
            Transition::new(None, 1, 0.0).when("jump", Test::True),
            Transition::new(Some(1), 0, 0.0).at_exit_time(1.0),
        ],
        default: 0,
    };
    let mut a = Animator::new(sk, clips).with_graph(graph);
    a.params.set_trigger("jump");
    a.update(0.1);
    assert_eq!(a.layers[0].state.current, 1);
    assert!(!a.params.bool("jump"), "the trigger was consumed");
    run(&mut a, 0.5, 0.1);
    assert_eq!(a.layers[0].state.current, 1, "the jump has not ended");
    run(&mut a, 0.6, 0.1);
    assert_eq!(a.layers[0].state.current, 0, "back to idle at the end");
}

#[test]
fn blend_1d_mixes_neighbours_by_the_parameter() {
    let sk = Arc::new(arm());
    let clips: Arc<[Clip]> = vec![
        Clip::new("a", vec![flat(10, 0.0)]),
        Clip::new("b", vec![flat(10, 8.0)]),
    ]
    .into();
    let graph = AnimGraph {
        states: vec![State::new(
            "move",
            Motion::Blend1D {
                param: "v".into(),
                points: vec![(0.0, 0), (2.0, 1)],
            },
        )],
        ..Default::default()
    };
    let mut a = Animator::new(sk, clips).with_graph(graph);
    for (v, want) in [(-1.0, 0.0), (0.5, 2.0), (1.0, 4.0), (9.0, 8.0)] {
        a.params.set_float("v", v);
        a.update(0.1);
        let x = a.pose.joints[0].translation.x;
        assert!((x - want).abs() < 1e-4, "v={v}: {x} != {want}");
    }
}

#[test]
fn layer_mask_limits_which_joints_it_drives() {
    let sk = Arc::new(arm());
    let clips: Arc<[Clip]> = vec![
        Clip::new("base", vec![flat(10, 1.0), flat(11, 1.0)]),
        Clip::new("wave", vec![flat(10, 5.0), flat(11, 5.0)]),
    ]
    .into();
    let mut a = Animator::looping(sk, clips, 0);
    let mut upper = Layer::new(Arc::new(AnimGraph::single(1)));
    upper.mask = Some(vec![false, true]);
    a.layers.push(upper);
    a.update(0.1);
    assert!((a.pose.joints[0].translation.x - 1.0).abs() < 1e-5);
    assert!((a.pose.joints[1].translation.x - 5.0).abs() < 1e-5);
    a.layers[1].weight = 0.5;
    a.update(0.1);
    assert!((a.pose.joints[1].translation.x - 3.0).abs() < 1e-5);
}

#[test]
fn graph_events_fire_once_per_pass() {
    let (sk, clips) = locomotion();
    let mut a = Animator::looping(sk, clips, 1);
    let mut seen = 0;
    for _ in 0..20 {
        a.update(0.1);
        seen += a.events.iter().filter(|e| *e == "foot").count();
    }
    assert_eq!(seen, 2, "two seconds of a one-second loop");
}

#[test]
fn plugin_advances_animators_and_sends_events() {
    let (sk, clips) = locomotion();
    let mut app = App::new();
    app.add_plugin(AnimPlugin);
    let e = app
        .resources
        .fetch_mut::<World>()
        .spawn((Animator::looping(sk, clips, 1),));
    app.resources
        .fetch_mut::<olvin_core::Time>()
        .set_fixed_delta(Some(0.25));
    let mut cursor = EventCursor::default();
    for _ in 0..3 {
        app.update();
    }
    let version = app
        .resources
        .fetch::<World>()
        .get::<&Animator>(e)
        .unwrap()
        .version;
    assert!(version >= 3);
    // The walk clip's "foot" event sits at 0.5 s: passed during frame 2 or 3.
    let events = app.resources.fetch::<Events<AnimationEvent>>();
    let names: Vec<_> = events
        .read(&mut cursor)
        .map(|ev| (ev.entity, ev.name.as_str()))
        .collect();
    assert_eq!(names, [(e, "foot")]);
}

#[test]
fn action_player_animates_scene_objects_and_loops() {
    use olvin_scene::action::{Action, Prop};
    use olvin_scene::{SceneId, Transform};

    // 24 frames = one second at 24 fps.
    let mut action = Action {
        frame_end: 25,
        ..Default::default()
    };
    action.insert_key(5, Prop::Location, 0, 1.0, 0.0);
    action.insert_key(5, Prop::Location, 0, 25.0, 10.0);
    action.insert_key(5, Prop::Rotation, 1, 1.0, 0.0);
    action.insert_key(5, Prop::Rotation, 1, 25.0, 90.0);
    let base = [Vec3::new(0.0, 2.0, 0.0), Vec3::ZERO, Vec3::ONE];

    let mut app = App::new();
    app.add_plugin(AnimPlugin);
    let (animated, other) = {
        let mut world = app.resources.fetch_mut::<World>();
        (
            world.spawn((SceneId(5), Transform::IDENTITY)),
            world.spawn((SceneId(6), Transform::from_xyz(1.0, 1.0, 1.0))),
        )
    };
    *app.resources.fetch_mut::<ActionPlayer>() =
        ActionPlayer::new(action, [(5, base)].into_iter().collect());
    app.resources
        .fetch_mut::<olvin_core::Time>()
        .set_fixed_delta(Some(0.5));
    app.update();
    let get = |e| *app.resources.fetch::<World>().get::<&Transform>(e).unwrap();
    // Half a second = frame 13, the middle of a symmetric ease: x = 5.
    let t = get(animated);
    assert!((t.translation.x - 5.0).abs() < 1e-3, "{:?}", t.translation);
    assert_eq!(
        t.translation.y, 2.0,
        "channels without curves keep the base"
    );
    let (yaw, _, _) = t.rotation.to_euler(glam::EulerRot::YXZ);
    assert!((yaw.to_degrees() - 45.0).abs() < 1e-2);
    assert_eq!(get(other).translation, Vec3::splat(1.0), "not animated");
    // Another 0.75 s wraps around: 1.25 s of a 1 s loop = frame 7.
    app.resources
        .fetch_mut::<olvin_core::Time>()
        .set_fixed_delta(Some(0.75));
    app.update();
    let f = app.resources.fetch::<ActionPlayer>().frame;
    assert!((f - 7.0).abs() < 1e-3, "{f}");
    // Not looping: stops on the last frame.
    {
        let mut p = app.resources.fetch_mut::<ActionPlayer>();
        p.looping = false;
        p.frame = 24.0;
    }
    app.update();
    let p = app.resources.fetch::<ActionPlayer>();
    assert_eq!((p.frame, p.playing), (25.0, false));
}
