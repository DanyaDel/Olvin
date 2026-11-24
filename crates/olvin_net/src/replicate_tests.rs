//! Replication tests: one server, two clients, simulated networks.

use std::cell::RefMut;

use glam::Vec3;
use olvin_core::App;
use olvin_ecs::{Entity, World};
use olvin_scene::{PrefabInstanceOf, SceneId, Transform};
use serde::{Deserialize, Serialize};

use crate::proto::Msg;
use crate::replicate::ReplicateOptions;
use crate::testing::Testbed;
use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Health(u32);

fn register(app: &mut App) {
    app.replicate::<Transform>().replicate::<Health>();
}

fn server_cfg() -> NetConfig {
    NetConfig::dedicated("test", 1, "loopback").timeout(5.0)
}

fn client_cfg(name: &str) -> NetConfig {
    NetConfig::join("test", 1, "127.0.0.1:1")
        .unwrap()
        .player_name(name)
        .timeout(5.0)
}

/// Server (index 0) and two clients (1, 2) with the same registrations.
fn trio(c: NetConditions) -> Testbed {
    let mut tb = Testbed::new(c);
    tb.add_server_with(server_cfg(), register);
    tb.add_client_with(client_cfg("ann"), register);
    tb.add_client_with(client_cfg("bob"), register);
    tb
}

fn world(tb: &Testbed, i: usize) -> RefMut<'_, World> {
    tb.apps[i].resources.fetch_mut::<World>()
}

fn spawn_at(tb: &Testbed, x: f32, hp: u32) -> Entity {
    world(tb, 0).spawn((
        Replicated::default(),
        Transform::from_xyz(x, 0.0, 0.0),
        Health(hp),
    ))
}

/// What a client has: copies sorted by id.
fn copies(tb: &Testbed, i: usize) -> Vec<(u64, Transform, Option<Health>)> {
    let w = world(tb, i);
    let mut v: Vec<_> = w
        .query::<(&NetId, &Remote, Option<&Transform>, Option<&Health>)>()
        .iter()
        .map(|(id, _, t, h)| (id.0, t.copied().unwrap_or_default(), h.copied()))
        .collect();
    v.sort_by_key(|c| c.0);
    v
}

fn both_connected(tb: &Testbed) -> bool {
    tb.net(1).status() == NetStatus::Connected && tb.net(2).status() == NetStatus::Connected
}

#[test]
fn spawn_update_and_despawn_reach_every_client() {
    let mut tb = trio(NetConditions::lag_ms(30.0));
    let e = spawn_at(&tb, 1.0, 10);
    assert!(tb.run_until(3.0, |tb| copies(tb, 1).len() == 1
        && copies(tb, 2).len() == 1));
    for i in [1, 2] {
        let c = copies(&tb, i);
        assert_eq!(c[0].2, Some(Health(10)));
        assert!((c[0].1.translation.x - 1.0).abs() < 0.01);
    }
    // The server's own entity got an id and is not marked Remote.
    assert!(world(&tb, 0).get::<&NetId>(e).is_ok());
    assert!(world(&tb, 0).get::<&Remote>(e).is_err());

    world(&tb, 0).insert_one(e, Health(7)).unwrap();
    assert!(tb.run_until(2.0, |tb| copies(tb, 2)[0].2 == Some(Health(7))));
    // A removed component is removed on the copy.
    world(&tb, 0).remove_one::<Health>(e).unwrap();
    assert!(tb.run_until(2.0, |tb| copies(tb, 1)[0].2.is_none()));

    world(&tb, 0).despawn(e).unwrap();
    assert!(tb.run_until(2.0, |tb| copies(tb, 1).is_empty()
        && copies(tb, 2).is_empty()));
    assert_eq!(tb.net(0).replicated_count(), 0);
}

#[test]
fn late_joiner_gets_the_full_state() {
    let mut tb = Testbed::new(NetConditions::lag_ms(20.0));
    tb.add_server_with(server_cfg(), register);
    for i in 0..30 {
        spawn_at(&tb, i as f32, i);
    }
    tb.run_for(1.0);
    let late = tb.add_client_with(client_cfg("late"), register);
    assert!(tb.run_until(5.0, |tb| copies(tb, late).len() == 30));
    let c = copies(&tb, late);
    let mut hp: Vec<u32> = c.iter().filter_map(|c| c.2.map(|h| h.0)).collect();
    hp.sort_unstable();
    assert_eq!(hp, (0..30).collect::<Vec<_>>());
}

#[test]
fn state_converges_under_loss_jitter_reorder_and_duplication() {
    let c = NetConditions::lag_ms(60.0)
        .with_jitter_ms(40.0)
        .with_loss_percent(30.0)
        .with_duplicate_percent(10.0)
        .with_reorder_percent(15.0);
    let mut tb = trio(c);
    let ents: Vec<Entity> = (0..12).map(|i| spawn_at(&tb, i as f32, 0)).collect();
    assert!(tb.run_until(10.0, |tb| both_connected(tb) && copies(tb, 1).len() == 12));
    // Everything keeps changing for a while...
    for step in 0..180u32 {
        let mut w = world(&tb, 0);
        for (k, e) in ents.iter().enumerate() {
            if let Ok(mut t) = w.get::<&mut Transform>(*e) {
                t.translation.x = k as f32 + step as f32 * 0.1;
                t.translation.z = (step as f32 * 0.05).sin() * 3.0;
            }
            let _ = w.insert_one(*e, Health(step + k as u32));
        }
        drop(w);
        tb.step();
    }
    // ...then settles; the clients must end up with exactly the server's state.
    let want: Vec<(f32, f32, u32)> = ents
        .iter()
        .map(|e| {
            let w = world(&tb, 0);
            let t = *w.get::<&Transform>(*e).unwrap();
            (
                t.translation.x,
                t.translation.z,
                w.get::<&Health>(*e).unwrap().0,
            )
        })
        .collect();
    tb.run_for(8.0);
    for i in [1, 2] {
        let got = copies(&tb, i);
        assert_eq!(got.len(), 12);
        for (g, w) in got.iter().zip(&want) {
            assert!(
                (g.1.translation.x - w.0).abs() < 0.01,
                "client {i}: x {} vs {}",
                g.1.translation.x,
                w.0
            );
            assert!((g.1.translation.z - w.1).abs() < 0.01, "client {i}: z");
            assert_eq!(g.2, Some(Health(w.2)), "client {i}");
        }
    }
}

#[test]
fn interpolation_is_smooth_and_lags_by_the_configured_delay() {
    // 10 m/s along x, ticks at 30 Hz, 20 ms one-way latency, 10 ms jitter, 5% loss.
    let c = NetConditions::lag_ms(20.0)
        .with_jitter_ms(10.0)
        .with_loss_percent(5.0);
    let mut tb = trio(c);
    let e = spawn_at(&tb, 0.0, 1);
    assert!(tb.run_until(5.0, |tb| copies(tb, 1).len() == 1));
    tb.run_for(0.5);
    let mut samples: Vec<(f64, f32)> = Vec::new();
    let mut server_x = Vec::new();
    for _ in 0..240 {
        let now = tb.hub.now();
        {
            let w = world(&tb, 0);
            let mut t = w.get::<&mut Transform>(e).unwrap();
            t.translation.x += 10.0 * tb.dt;
            server_x.push((now, t.translation.x));
        }
        tb.step();
        samples.push((tb.hub.now(), copies(&tb, 1)[0].1.translation.x));
    }
    // Smooth: no frame moves more than twice the nominal step and nothing goes backwards.
    let nominal = 10.0 * tb.dt;
    let mut worst = 0.0f32;
    for w in samples.windows(2).skip(20) {
        let step = w[1].1 - w[0].1;
        assert!(step >= -0.001, "went backwards by {step}");
        worst = worst.max(step);
    }
    assert!(
        worst < nominal * 2.2,
        "worst step {worst} vs nominal {nominal}"
    );
    // The picture trails the server by about interp delay + latency (0.1 + 0.02 s = 1.2 m).
    let (now, shown) = *samples.last().unwrap();
    let truth = server_x.last().unwrap().1;
    let lag_m = truth - shown;
    assert!((0.7..2.0).contains(&lag_m), "lag {lag_m} m at t={now}");
}

#[test]
fn clients_cannot_write_replicated_state_or_spawn() {
    let mut tb = trio(NetConditions::lag_ms(10.0));
    let e = spawn_at(&tb, 5.0, 10);
    assert!(tb.run_until(3.0, |tb| both_connected(tb) && copies(tb, 1).len() == 1));
    let id = *world(&tb, 0).get::<&NetId>(e).unwrap();
    // A cheating client sends every server-to-client message type to the server.
    let forged = [
        Msg::Snapshot(proto::SnapshotMsg {
            seq: 1,
            tick: 9999,
            entries: vec![proto::SnapEntry {
                net_id: id.0,
                comp: 2,
                data: Some(postcard::to_stdvec(&Health(9999)).unwrap()),
            }],
        }),
        Msg::Spawn(proto::SpawnMsg {
            net_id: 77,
            tick: 1,
            kind: proto::SpawnKind::Bare,
            comps: vec![],
        }),
        Msg::Despawn { net_id: id.0 },
    ];
    for msg in forged {
        if let Some(c) = tb.apps[1].resources.fetch_mut::<Net>().client.as_mut() {
            c.send(Channel::Reliable, &msg);
        }
    }
    tb.run_for(1.0);
    assert_eq!(world(&tb, 0).get::<&Health>(e).unwrap().0, 10);
    assert_eq!(tb.net(0).replicated_count(), 1);
    assert!(world(&tb, 0).contains(e));
    // And the server is still happy with that client.
    assert_eq!(tb.net(1).status(), NetStatus::Connected);
    assert_eq!(copies(&tb, 2)[0].2, Some(Health(10)));
}

#[test]
fn ownership_is_replicated_and_changed_only_by_the_server() {
    let mut tb = trio(NetConditions::lag_ms(10.0));
    let e = spawn_at(&tb, 0.0, 1);
    world(&tb, 0).insert_one(e, Owner(ClientId(1))).unwrap();
    assert!(tb.run_until(3.0, |tb| both_connected(tb) && copies(tb, 1).len() == 1));
    let mine = |tb: &Testbed, i: usize| {
        let w = world(tb, i);
        let ent = w.query::<&Remote>().iter().next().map(|_| ());
        let _ = ent;
        let entity = w
            .query::<(Entity, &NetId)>()
            .iter()
            .next()
            .map(|(e, _)| e)
            .unwrap();
        tb.net(i).is_mine(&w, entity)
    };
    assert!(tb.run_until(2.0, |tb| mine(tb, 1)));
    assert!(!mine(&tb, 2));
    // Transfer: a server-side write, seen by everybody.
    world(&tb, 0).insert_one(e, Owner(ClientId(2))).unwrap();
    assert!(tb.run_until(2.0, |tb| mine(tb, 2) && !mine(tb, 1)));
}

#[test]
fn relevancy_by_distance_spawns_and_despawns() {
    let mut tb = trio(NetConditions::lag_ms(10.0));
    tb.net_mut(0).set_relevancy_distance(20.0);
    // Client 1 owns the entity at x=0; one thing is near, one far.
    let me = spawn_at(&tb, 0.0, 1);
    world(&tb, 0).insert_one(me, Owner(ClientId(1))).unwrap();
    let near = spawn_at(&tb, 5.0, 2);
    let far = spawn_at(&tb, 500.0, 3);
    assert!(tb.run_until(3.0, |tb| both_connected(tb) && copies(tb, 1).len() == 2));
    assert_eq!(
        copies(&tb, 1)
            .iter()
            .filter_map(|c| c.2.map(|h| h.0))
            .collect::<Vec<_>>()
            .len(),
        2
    );
    // Walk the far one into range, the near one out.
    world(&tb, 0)
        .get::<&mut Transform>(far)
        .unwrap()
        .translation
        .x = 10.0;
    world(&tb, 0)
        .get::<&mut Transform>(near)
        .unwrap()
        .translation
        .x = 900.0;
    assert!(tb.run_until(3.0, |tb| {
        let hp: Vec<u32> = copies(tb, 1)
            .iter()
            .filter_map(|c| c.2.map(|h| h.0))
            .collect();
        hp.contains(&3) && !hp.contains(&2)
    }));
    // Client 2 owns nothing, so it sees everything.
    assert_eq!(copies(&tb, 2).len(), 3);
}

#[test]
fn bandwidth_budget_is_respected_and_the_world_still_converges() {
    let cfg = server_cfg().bandwidth(20_000);
    let mut tb = Testbed::new(NetConditions::lag_ms(20.0));
    tb.add_server_with(cfg, register);
    tb.add_client_with(client_cfg("ann"), register);
    let ents: Vec<Entity> = (0..300).map(|i| spawn_at(&tb, i as f32, 0)).collect();
    assert!(tb.run_until(10.0, |tb| copies(tb, 1).len() == 300));
    let before = tb.net(0).server.as_ref().unwrap().bytes_out;
    // Everything changes every tick: far more data than the budget.
    for step in 0..180u32 {
        let mut w = world(&tb, 0);
        for e in &ents {
            let _ = w.insert_one(*e, Health(step));
        }
        drop(w);
        tb.step();
    }
    let sent = tb.net(0).server.as_ref().unwrap().bytes_out - before;
    let per_sec = sent as f32 / 3.0;
    assert!(
        per_sec < 20_000.0 * 1.25,
        "{per_sec} B/s over a 20000 B/s budget"
    );
    // Stop changing: it catches up.
    tb.run_for(15.0);
    assert!(copies(&tb, 1).iter().all(|c| c.2 == Some(Health(179))));
}

#[test]
fn prefab_copies_are_made_by_the_spawner_with_a_confined_path() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server_with(server_cfg(), register);
    let asked = Rc::new(RefCell::new(Vec::<String>::new()));
    let log = asked.clone();
    tb.add_client_with(client_cfg("ann"), move |app| {
        register(app);
        let log = log.clone();
        app.resources
            .fetch_mut::<Net>()
            .set_prefab_spawner(move |res, path, _t| {
                log.borrow_mut().push(path.to_owned());
                Ok(res.fetch_mut::<World>().spawn((Health(0),)))
            });
    });
    world(&tb, 0).spawn((
        Replicated::default(),
        PrefabInstanceOf("prefabs/player.prefab.ron".into()),
        Transform::default(),
    ));
    // A bad path is never replicated as a prefab (it becomes a bare entity).
    world(&tb, 0).spawn((
        Replicated {
            prefab: Some("../../etc/passwd.prefab.ron".into()),
        },
        Transform::default(),
    ));
    tb.run_for(1.0);
    assert_eq!(*asked.borrow(), ["prefabs/player.prefab.ron"]);
    assert_eq!(copies(&tb, 1).len(), 2);
}

#[test]
fn scene_objects_are_adopted_by_scene_id() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server_with(server_cfg(), register);
    let c = tb.add_client_with(client_cfg("ann"), register);
    // Both sides loaded the same level: an object with SceneId 42.
    let mine = world(&tb, c).spawn((SceneId(42), Transform::from_xyz(1.0, 1.0, 1.0)));
    world(&tb, 0).spawn((
        Replicated::default(),
        SceneId(42),
        Transform::from_xyz(9.0, 0.0, 0.0),
    ));
    tb.run_for(1.0);
    let w = world(&tb, c);
    assert!(
        w.get::<&NetId>(mine).is_ok(),
        "the level object was not adopted"
    );
    assert!((w.get::<&Transform>(mine).unwrap().translation.x - 9.0).abs() < 0.01);
    assert_eq!(w.query::<&NetId>().iter().count(), 1);
}

#[test]
fn offline_assigns_ids_and_sends_nothing() {
    let mut app = App::new();
    app.insert_resource(olvin_core::Time::with_fixed_delta(1.0 / 60.0));
    app.add_plugin(NetPlugin::new(NetConfig::offline("test", 1)));
    register(&mut app);
    let e = app.resources.fetch_mut::<World>().spawn((
        Replicated::default(),
        Transform::default(),
        Health(1),
    ));
    for _ in 0..10 {
        app.update();
    }
    assert!(app.resources.fetch::<World>().get::<&NetId>(e).is_ok());
    let net = app.resources.fetch::<Net>();
    assert_eq!(net.replicated_count(), 1);
    assert_eq!(net.server.as_ref().unwrap().packets_out, 0);
}

#[test]
fn garbage_replication_messages_are_survived() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server_with(server_cfg(), register);
    let c = tb.add_client_with(client_cfg("ann"), register);
    spawn_at(&tb, 1.0, 1);
    assert!(tb.run_until(2.0, |tb| copies(tb, c).len() == 1));
    // Hand the client hostile messages as if the server sent them.
    let bad = [
        Msg::Spawn(proto::SpawnMsg {
            net_id: 5,
            tick: 1,
            kind: proto::SpawnKind::Bare,
            comps: vec![proto::CompData {
                comp: 999,
                data: vec![1],
            }],
        }),
        Msg::Spawn(proto::SpawnMsg {
            net_id: 6,
            tick: 1,
            kind: proto::SpawnKind::Prefab("/etc/passwd".into()),
            comps: vec![],
        }),
        Msg::Spawn(proto::SpawnMsg {
            net_id: 7,
            tick: 1,
            kind: proto::SpawnKind::Bare,
            comps: vec![proto::CompData {
                comp: 1,
                data: vec![0xFF; 3],
            }],
        }),
        Msg::Snapshot(proto::SnapshotMsg {
            seq: 1,
            tick: 5,
            entries: vec![
                proto::SnapEntry {
                    net_id: 1,
                    comp: 1,
                    data: Some(vec![1, 2, 3]),
                },
                proto::SnapEntry {
                    net_id: 1,
                    comp: 500,
                    data: None,
                },
                proto::SnapEntry {
                    net_id: u64::MAX,
                    comp: 0,
                    data: None,
                },
            ],
        }),
        Msg::Despawn { net_id: 12345 },
    ];
    {
        let mut net = tb.net_mut(c);
        let client = net.client.as_mut().unwrap();
        for m in bad {
            client.inbox.push((Channel::Reliable, m));
        }
    }
    tb.run_for(1.0);
    // Still alive, still correct.
    let got = copies(&tb, c);
    assert!(got.iter().any(|c| c.2 == Some(Health(1))));
    assert_eq!(tb.net(c).status(), NetStatus::Connected);
}

#[test]
fn registration_order_mismatch_is_refused_at_join() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server_with(server_cfg(), register);
    let c = tb.add_client_with(client_cfg("ann"), |app| {
        // Same types, other order: the wire ids would not line up.
        app.replicate::<Health>().replicate::<Transform>();
    });
    tb.run_for(1.0);
    assert!(matches!(tb.net(c).status(), NetStatus::Disconnected(m) if m.contains("differ")));
}

#[test]
fn custom_options_and_quantisation() {
    fn round_hp(h: &mut Health) {
        h.0 = h.0 / 10 * 10;
    }
    let reg = |app: &mut App| {
        app.replicate::<Transform>().replicate_with(
            ReplicateOptions::<Health>::default()
                .quantize(round_hp)
                .every(3),
        );
    };
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server_with(server_cfg(), reg);
    let c = tb.add_client_with(client_cfg("ann"), reg);
    let e = spawn_at(&tb, 0.0, 0);
    tb.run_for(0.5);
    // Changes below the quantisation step are never sent.
    for hp in 1..10 {
        world(&tb, 0).insert_one(e, Health(hp)).unwrap();
        tb.run_for(0.1);
        assert_eq!(copies(&tb, c)[0].2, Some(Health(0)));
    }
    world(&tb, 0).insert_one(e, Health(47)).unwrap();
    assert!(tb.run_until(2.0, |tb| copies(tb, c)[0].2 == Some(Health(40))));
    let _ = Vec3::ZERO;
}

#[test]
fn stats_report_loss_rtt_and_bandwidth() {
    let c = NetConditions::lag_ms(40.0).with_loss_percent(25.0);
    let mut tb = trio(c);
    let e = spawn_at(&tb, 0.0, 0);
    assert!(tb.run_until(5.0, |tb| both_connected(tb) && copies(tb, 1).len() == 1));
    for step in 0..400u32 {
        world(&tb, 0).insert_one(e, Health(step)).unwrap();
        tb.step();
    }
    let s = tb.apps[1].resources.fetch::<NetStats>();
    assert!((0.1..0.45).contains(&s.loss), "loss {}", s.loss);
    assert!((0.07..0.2).contains(&s.rtt), "rtt {}", s.rtt);
    assert!(
        s.bytes_in_per_sec > 100.0 && s.bytes_out_per_sec > 10.0,
        "{s:?}"
    );
    assert_eq!(s.replicated, 1);
    let server = tb.apps[0].resources.fetch::<NetStats>();
    assert_eq!(server.clients, 2);
    assert!(server.bytes_out_per_sec > s.bytes_in_per_sec);
}
