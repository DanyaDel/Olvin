//! A predicted `CharacterMotor` character over a laggy link: the client matches the server.

use glam::{Vec2, Vec3};
use olvin_core::{App, Resources};
use olvin_ecs::World;
use olvin_physics::PhysicsPlugin;
use olvin_scene::file::{CharacterControllerDesc, ColliderDesc, ColliderShape};
use olvin_scene::{PrefabInstanceOf, Transform};

use crate::character::{CharacterInput, CharacterState};
use crate::testing::Testbed;
use crate::*;

struct Pressed(CharacterInput);

fn sample(res: &Resources) -> CharacterInput {
    res.fetch::<Pressed>().0
}

fn level(app: &mut App) {
    app.add_plugin(PhysicsPlugin::default());
    app.insert_resource(Pressed(CharacterInput::default()));
    app.resources.fetch_mut::<World>().spawn((
        Transform::from_xyz(0.0, -0.5, 0.0).with_scale(Vec3::new(200.0, 1.0, 200.0)),
        ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE }),
    ));
}

fn register(app: &mut App) {
    app.replicate::<Transform>();
    app.resources.fetch_mut::<Net>().predict_character(sample);
}

fn pos(tb: &Testbed, i: usize) -> Vec3 {
    let w = tb.apps[i].resources.fetch::<World>();
    w.query::<(&Transform, &CharacterState)>()
        .iter()
        .next()
        .map_or(Vec3::NAN, |(t, _)| t.translation)
}

fn press(tb: &Testbed, input: CharacterInput) {
    tb.apps[1].resources.fetch_mut::<Pressed>().0 = input;
}

#[test]
fn predicted_character_matches_the_server_after_reconciliation() {
    let c = NetConditions::lag_ms(80.0)
        .with_jitter_ms(15.0)
        .with_loss_percent(3.0);
    let mut tb = Testbed::new(c);
    tb.add_server_with(NetConfig::dedicated("c", 1, "x").timeout(5.0), |app| {
        level(app);
        register(app);
    });
    tb.add_client_with(
        NetConfig::join("c", 1, "1.1.1.1:1").unwrap().timeout(5.0),
        |app| {
            level(app);
            register(app);
            app.resources
                .fetch_mut::<Net>()
                .set_prefab_spawner(|res, _, t| {
                    Ok(res
                        .fetch_mut::<World>()
                        .spawn((t, CharacterControllerDesc::default())))
                });
        },
    );
    tb.apps[0].resources.fetch_mut::<World>().spawn((
        Replicated::default(),
        PrefabInstanceOf("player.prefab.ron".into()),
        Owner(ClientId(1)),
        Transform::from_xyz(0.0, 1.0, 0.0),
        CharacterState::at(Vec3::new(0.0, 1.0, 0.0)),
        CharacterControllerDesc::default(),
    ));
    assert!(
        tb.run_until(8.0, |tb| tb.net(1).status() == NetStatus::Connected
            && pos(tb, 1).is_finite())
    );
    tb.run_for(1.0);
    // Walk, jump in the middle, walk on.
    press(
        &tb,
        CharacterInput {
            dir: Vec2::new(1.0, 0.0),
            jump: false,
        },
    );
    tb.run_for(1.5);
    let ahead = pos(&tb, 1).x - pos(&tb, 0).x;
    assert!(
        ahead > 0.3,
        "the client should lead the laggy server by {ahead}"
    );
    press(
        &tb,
        CharacterInput {
            dir: Vec2::new(1.0, 0.0),
            jump: true,
        },
    );
    tb.run_for(0.1);
    press(
        &tb,
        CharacterInput {
            dir: Vec2::new(0.7, 0.7),
            jump: false,
        },
    );
    tb.run_for(2.0);
    press(&tb, CharacterInput::default());
    tb.run_for(3.0);
    let (c, s) = (pos(&tb, 1), pos(&tb, 0));
    assert!(s.x > 8.0, "server position {s}");
    assert!(c.distance(s) < 0.05, "client {c} vs server {s}");
    let st = tb.net(1).prediction_stats();
    assert!(st.inputs_sent > 100, "{st:?}");
    // Both ended on the ground.
    assert!((c.y - s.y).abs() < 0.05 && s.y < 1.2, "{s}");
}
