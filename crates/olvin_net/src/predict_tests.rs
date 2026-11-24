//! Prediction and reconciliation over a simulated network.

use olvin_core::Resources;
use olvin_ecs::{Entity, World};
use serde::{Deserialize, Serialize};

use crate::proto::Msg;
use crate::testing::Testbed;
use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Pos(f32);

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Dx(f32);

/// What the "player" presses this tick (set by the test).
struct Pressed(f32);

/// A wall only the server knows about.
struct Wall(f32);

const SPEED: f32 = 5.0;

fn step(res: &Resources, e: Entity, input: &Dx, dt: f32) {
    let wall = res.get::<Wall>().map(|w| w.0);
    let world = res.fetch_mut::<World>();
    if let Ok(mut p) = world.get::<&mut Pos>(e) {
        p.0 += input.0.clamp(-1.0, 1.0) * SPEED * dt;
        if let Some(w) = wall {
            p.0 = p.0.min(w);
        }
    }
}

fn spec() -> Prediction<Pos, Dx> {
    Prediction::new(
        |res| Dx(res.fetch::<Pressed>().0),
        step,
        |a: &Pos, b: &Pos| (a.0 - b.0).abs(),
        |a: &Pos, b: &Pos, t: f32| Pos(a.0 + (b.0 - a.0) * t),
    )
    .sanitize(|i| i.0 = i.0.clamp(-1.0, 1.0))
}

fn rig(c: NetConditions) -> Testbed {
    let mut tb = Testbed::new(c);
    // Owner is registered by Net itself; `register` must not repeat it.
    let reg = |app: &mut olvin_core::App| {
        app.resources.fetch_mut::<Net>().predict(spec());
        app.insert_resource(Pressed(0.0));
    };
    tb.add_server_with(NetConfig::dedicated("p", 1, "x").timeout(5.0), reg);
    tb.add_client_with(
        NetConfig::join("p", 1, "1.1.1.1:1").unwrap().timeout(5.0),
        reg,
    );
    tb
}

fn pos(tb: &Testbed, i: usize, e: Option<Entity>) -> f32 {
    let w = tb.apps[i].resources.fetch::<World>();
    match e {
        Some(e) => w.get::<&Pos>(e).map_or(f32::NAN, |p| p.0),
        None => w.query::<&Pos>().iter().next().map_or(f32::NAN, |p| p.0),
    }
}

fn press(tb: &Testbed, v: f32) {
    tb.apps[1].resources.fetch_mut::<Pressed>().0 = v;
}

fn spawn_player(tb: &Testbed) -> Entity {
    tb.apps[0].resources.fetch_mut::<World>().spawn((
        Replicated::default(),
        Owner(ClientId(1)),
        Pos(0.0),
    ))
}

fn connected(tb: &Testbed) -> bool {
    tb.net(1).status() == NetStatus::Connected
}

#[test]
fn prediction_matches_the_server_under_latency_and_loss() {
    let c = NetConditions::lag_ms(60.0)
        .with_jitter_ms(20.0)
        .with_loss_percent(10.0);
    let mut tb = rig(c);
    let e = spawn_player(&tb);
    assert!(tb.run_until(5.0, |tb| connected(tb) && pos(tb, 1, None) == 0.0));
    press(&tb, 1.0);
    let mut start = None;
    for k in 0..240 {
        tb.step();
        if k == 60 {
            // After one second of walking the client is ahead of what the server has: that is prediction.
            let (c, s) = (pos(&tb, 1, None), pos(&tb, 0, Some(e)));
            assert!(c > s + 0.2, "client {c} should lead server {s}");
            start = Some(c);
        }
    }
    assert!(start.is_some());
    press(&tb, 0.0);
    tb.run_for(2.0);
    let (c, s) = (pos(&tb, 1, None), pos(&tb, 0, Some(e)));
    assert!((c - s).abs() < 0.02, "client {c} vs server {s}");
    assert!(s > 15.0, "server moved {s}");
}

#[test]
fn ideal_network_needs_no_corrections() {
    let mut tb = rig(NetConditions::lag_ms(50.0));
    spawn_player(&tb);
    assert!(tb.run_until(5.0, |tb| connected(tb) && pos(tb, 1, None) == 0.0));
    press(&tb, 1.0);
    tb.run_for(3.0);
    let stats = tb.net(1).prediction_stats();
    assert!(stats.inputs_sent > 60);
    assert_eq!(stats.corrections + stats.snaps, 0, "{stats:?}");
}

#[test]
fn a_server_only_wall_is_reconciled_without_snapping() {
    let mut tb = rig(NetConditions::lag_ms(50.0));
    tb.apps[0].insert_resource(Wall(3.0));
    let e = spawn_player(&tb);
    assert!(tb.run_until(5.0, |tb| connected(tb) && pos(tb, 1, None) == 0.0));
    press(&tb, 1.0);
    let mut worst_jump = 0.0f32;
    let mut last = 0.0;
    for _ in 0..300 {
        tb.step();
        let p = pos(&tb, 1, None);
        worst_jump = worst_jump.max((p - last).abs());
        last = p;
    }
    // Pushing into the wall: the client hovers a bounded distance past it, never far.
    let c = pos(&tb, 1, None);
    assert!((3.0..4.0).contains(&c), "client at {c}");
    assert!(worst_jump < 0.5, "visible jump {worst_jump}");
    press(&tb, 0.0);
    tb.run_for(2.0);
    assert!((pos(&tb, 1, None) - pos(&tb, 0, Some(e))).abs() < 0.02);
    assert!(tb.net(1).prediction_stats().corrections > 0);
}

#[test]
fn small_server_nudges_are_smoothed_and_big_ones_snap() {
    let mut tb = rig(NetConditions::lag_ms(30.0));
    let e = spawn_player(&tb);
    assert!(tb.run_until(5.0, |tb| connected(tb) && pos(tb, 1, None) == 0.0));
    let nudge = |tb: &Testbed, d: f32| {
        tb.apps[0]
            .resources
            .fetch_mut::<World>()
            .get::<&mut Pos>(e)
            .unwrap()
            .0 += d;
    };
    nudge(&tb, 0.6);
    let mut trace = vec![];
    for _ in 0..60 {
        tb.step();
        trace.push(pos(&tb, 1, None));
    }
    // The first change arrives with the spawn of the new state: no step bigger than the share of the error.
    let biggest = trace
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(biggest < 0.6 * 0.4, "jumped {biggest}");
    assert!(
        (pos(&tb, 1, None) - 0.6).abs() < 0.02,
        "{}",
        pos(&tb, 1, None)
    );
    nudge(&tb, 10.0);
    tb.run_for(1.0);
    assert!((pos(&tb, 1, None) - 10.6).abs() < 0.02);
    assert!(tb.net(1).prediction_stats().snaps >= 1);
}

#[test]
fn cheating_inputs_are_clamped_and_speedhacks_do_not_help() {
    let mut tb = rig(NetConditions::lag_ms(20.0));
    let e = spawn_player(&tb);
    assert!(tb.run_until(5.0, |tb| connected(tb) && pos(tb, 1, None) == 0.0));
    // Forge a huge input and a flood of inputs every frame.
    let before = pos(&tb, 0, Some(e));
    for k in 0..120u32 {
        let inputs: Vec<Vec<u8>> = (0..16)
            .map(|_| wire::encode(&Dx(1000.0)).unwrap())
            .collect();
        if let Some(c) = tb.apps[1].resources.fetch_mut::<Net>().client.as_mut() {
            c.send(
                Channel::Unreliable,
                &Msg::Input {
                    first_seq: 1000 + k * 16,
                    inputs,
                },
            );
        }
        tb.step();
    }
    // 2 s of ticks at 30 Hz = 60 steps of at most SPEED/30; +10% catch-up allowance.
    let moved = pos(&tb, 0, Some(e)) - before;
    assert!(moved <= 60.0 * SPEED / 30.0 * 1.25, "moved {moved}");
}

#[test]
fn offline_player_moves_through_the_same_input_queue() {
    let mut app = olvin_core::App::new();
    app.insert_resource(olvin_core::Time::with_fixed_delta(1.0 / 60.0));
    app.add_plugin(NetPlugin::new(NetConfig::offline("p", 1)));
    app.resources.fetch_mut::<Net>().predict(spec());
    app.insert_resource(Pressed(1.0));
    let e = app.resources.fetch_mut::<World>().spawn((
        Replicated::default(),
        Owner(ClientId::HOST),
        Pos(0.0),
    ));
    for _ in 0..120 {
        app.update();
    }
    let p = app.resources.fetch::<World>().get::<&Pos>(e).unwrap().0;
    assert!((p - 2.0 * SPEED).abs() < 0.6, "{p}");
}
