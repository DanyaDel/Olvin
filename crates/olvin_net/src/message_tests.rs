//! Messages, commands, validation and limits.

use std::cell::RefCell;
use std::rc::Rc;

use olvin_core::App;
use olvin_ecs::World;
use serde::{Deserialize, Serialize};

use crate::proto::Msg;
use crate::testing::{Testbed, record};
use crate::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Move {
    dx: f32,
    dy: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Chat(String);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Score(u32);

fn register(app: &mut App) {
    app.net_command::<Move>("move", |_, m| {
        Verdict::check(m.dx.abs() <= 1.0 && m.dy.abs() <= 1.0, "too fast")
    })
    .net_message_with::<Chat>("chat", MessageOptions::default().max_size(64))
    .net_message_with::<Score>(
        "score",
        MessageOptions::default().direction(Direction::ServerToClient),
    );
}

type Log<T> = Rc<RefCell<Vec<NetReceived<T>>>>;

struct Rig {
    tb: Testbed,
    moves: Log<Move>,
    chats: [Log<Chat>; 3],
    scores: [Log<Score>; 3],
    rejected: Rc<RefCell<Vec<NetCommandRejected>>>,
    client_events: Rc<RefCell<Vec<NetEvent>>>,
}

/// Server (0) and clients 1 and 2, recording everything, already connected.
fn rig(c: NetConditions) -> Rig {
    let mut tb = Testbed::new(c);
    tb.add_server_with(NetConfig::dedicated("t", 1, "x").timeout(5.0), register);
    for n in ["ann", "bob"] {
        tb.add_client_with(
            NetConfig::join("t", 1, "1.1.1.1:1")
                .unwrap()
                .player_name(n)
                .timeout(5.0),
            register,
        );
    }
    let moves = record::<NetReceived<Move>>(&mut tb.apps[0]);
    let rejected = record::<NetCommandRejected>(&mut tb.apps[0]);
    let client_events = record::<NetEvent>(&mut tb.apps[1]);
    let chats = [0, 1, 2].map(|i| record::<NetReceived<Chat>>(&mut tb.apps[i]));
    let scores = [0, 1, 2].map(|i| record::<NetReceived<Score>>(&mut tb.apps[i]));
    assert!(
        tb.run_until(5.0, |tb| tb.net(1).status() == NetStatus::Connected
            && tb.net(2).status() == NetStatus::Connected)
    );
    Rig {
        tb,
        moves,
        chats,
        scores,
        rejected,
        client_events,
    }
}

#[test]
fn accepted_command_arrives_once_with_its_sender() {
    let mut r = rig(NetConditions::lag_ms(20.0));
    r.tb.net_mut(2)
        .send(Target::Server, &Move { dx: 0.5, dy: -1.0 });
    r.tb.run_for(1.0);
    assert_eq!(
        *r.moves.borrow(),
        [NetReceived {
            from: Sender::Client(ClientId(2)),
            msg: Move { dx: 0.5, dy: -1.0 }
        }]
    );
    assert!(r.rejected.borrow().is_empty());
}

#[test]
fn a_cheating_command_is_rejected_and_the_cheater_told() {
    let mut r = rig(NetConditions::lag_ms(20.0));
    r.tb.net_mut(1)
        .send(Target::Server, &Move { dx: 500.0, dy: 0.0 });
    r.tb.net_mut(1)
        .send(Target::Server, &Move { dx: 1.0, dy: 1.0 });
    r.tb.run_for(1.0);
    // Only the honest one got through.
    assert_eq!(r.moves.borrow().len(), 1);
    assert_eq!(r.moves.borrow()[0].msg, Move { dx: 1.0, dy: 1.0 });
    assert_eq!(
        *r.rejected.borrow(),
        [NetCommandRejected {
            client: ClientId(1),
            name: "move".into(),
            reason: "too fast".into()
        }]
    );
    assert!(
        r.client_events
            .borrow()
            .contains(&NetEvent::CommandRejected {
                name: "move".into(),
                reason: "too fast".into()
            })
    );
}

#[test]
fn a_cheater_who_keeps_cheating_is_kicked() {
    let mut r = rig(NetConditions::lag_ms(10.0));
    for _ in 0..40 {
        for _ in 0..8 {
            r.tb.net_mut(1)
                .send(Target::Server, &Move { dx: 99.0, dy: 0.0 });
        }
        r.tb.step();
    }
    r.tb.run_for(1.0);
    assert!(
        r.client_events.borrow().iter().any(|e| matches!(
            e,
            NetEvent::Disconnected { reason } if reason.contains("invalid")
        )),
        "{:?}",
        r.client_events.borrow().last()
    );
    // Bob is unaffected, nothing the cheater sent was accepted.
    assert_eq!(r.tb.net(2).status(), NetStatus::Connected);
    assert!(r.moves.borrow().is_empty());
}

#[test]
fn rate_limit_caps_a_flood_of_valid_commands() {
    let mut r = rig(NetConditions::NONE);
    for _ in 0..1000 {
        r.tb.net_mut(2)
            .send(Target::Server, &Move { dx: 0.1, dy: 0.1 });
    }
    r.tb.run_for(0.5);
    let n = r.moves.borrow().len();
    // 60 per second, one second of burst, a little refill while the burst drains.
    assert!((50..=100).contains(&n), "{n} commands got through");
}

#[test]
fn forged_and_malformed_messages_are_dropped_without_panics() {
    let mut r = rig(NetConditions::NONE);
    let forge = |r: &mut Rig, msg: Msg| {
        if let Some(c) = r.tb.apps[1].resources.fetch_mut::<Net>().client.as_mut() {
            c.send(Channel::Reliable, &msg);
        }
    };
    // Message ids: 0 move, 1 chat, 2 score (server-to-client only). Registry order after Owner.
    forge(
        &mut r,
        Msg::User {
            id: 2,
            payload: postcard::to_stdvec(&Score(1)).unwrap(),
        },
    );
    forge(
        &mut r,
        Msg::User {
            id: 1,
            payload: vec![b'a'; 1000],
        },
    );
    forge(
        &mut r,
        Msg::User {
            id: 99,
            payload: vec![1],
        },
    );
    forge(
        &mut r,
        Msg::User {
            id: 0,
            payload: vec![0xFF; 9],
        },
    );
    forge(
        &mut r,
        Msg::User {
            id: 0,
            payload: vec![],
        },
    );
    r.tb.run_for(0.5);
    assert!(r.moves.borrow().is_empty());
    assert!(r.chats[0].borrow().is_empty());
    assert!(r.scores[0].borrow().is_empty());
    assert_eq!(r.tb.net(1).status(), NetStatus::Connected);
    // And random noise as user messages, lots of it: the server survives and the noisy client goes.
    let mut rng = sim::SimRng::new(3);
    for _ in 0..400 {
        let len = (rng.next_u64() % 300) as usize;
        let payload: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
        forge(
            &mut r,
            Msg::User {
                id: (rng.next_u64() % 6) as u16,
                payload,
            },
        );
        r.tb.step();
    }
    r.tb.run_for(0.5);
    assert!(!matches!(r.tb.net(1).status(), NetStatus::Connected));
    assert_eq!(r.tb.net(2).status(), NetStatus::Connected);
}

#[test]
fn oversized_messages_are_not_sent() {
    let mut r = rig(NetConditions::NONE);
    r.tb.net_mut(1).send(Target::Server, &Chat("x".repeat(500)));
    r.tb.net_mut(1).send(Target::Server, &Chat("hi".into()));
    r.tb.run_for(0.5);
    assert_eq!(r.chats[0].borrow().len(), 1);
    assert_eq!(r.chats[0].borrow()[0].msg, Chat("hi".into()));
}

#[test]
fn server_targets_reach_the_right_clients() {
    let mut r = rig(NetConditions::lag_ms(10.0));
    {
        let mut net = r.tb.net_mut(0);
        net.send(Target::All, &Score(1));
        net.send(Target::Client(ClientId(1)), &Score(2));
        net.send(Target::AllExcept(ClientId(1)), &Score(3));
    }
    r.tb.run_for(0.5);
    let got = |i: usize| -> Vec<u32> { r.scores[i].borrow().iter().map(|s| s.msg.0).collect() };
    assert_eq!(got(1), [1, 2]);
    assert_eq!(got(2), [1, 3]);
    assert!(r.scores[1].borrow().iter().all(|s| s.from.is_server()));
}

#[test]
fn owner_target_resolves_through_the_world() {
    let mut r = rig(NetConditions::NONE);
    let e = r.tb.apps[0]
        .resources
        .fetch_mut::<World>()
        .spawn((Owner(ClientId(2)),));
    r.tb.net_mut(0).send(Target::Owner(e), &Score(9));
    r.tb.run_for(0.5);
    assert!(r.scores[1].borrow().is_empty());
    assert_eq!(r.scores[2].borrow().len(), 1);
}

#[test]
fn clients_cannot_send_to_anybody_but_the_server() {
    let mut r = rig(NetConditions::NONE);
    r.tb.net_mut(1).send(Target::All, &Chat("relay".into()));
    r.tb.net_mut(1)
        .send(Target::Client(ClientId(2)), &Chat("whisper".into()));
    r.tb.net_mut(1).send(Target::Server, &Score(1));
    r.tb.run_for(0.5);
    for log in &r.chats {
        assert!(log.borrow().is_empty());
    }
    assert!(r.scores[0].borrow().is_empty());
}

#[test]
fn offline_commands_take_the_same_validated_path() {
    let mut app = App::new();
    app.insert_resource(olvin_core::Time::with_fixed_delta(1.0 / 60.0));
    app.add_plugin(NetPlugin::new(NetConfig::offline("t", 1)));
    register(&mut app);
    let moves = record::<NetReceived<Move>>(&mut app);
    let rejected = record::<NetCommandRejected>(&mut app);
    app.resources
        .fetch_mut::<Net>()
        .send(Target::Server, &Move { dx: 1.0, dy: 0.0 });
    app.resources
        .fetch_mut::<Net>()
        .send(Target::Server, &Move { dx: 9.0, dy: 0.0 });
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(moves.borrow().len(), 1);
    assert_eq!(moves.borrow()[0].from, Sender::Client(ClientId::HOST));
    assert_eq!(rejected.borrow().len(), 1);
}

#[test]
fn reliable_messages_survive_a_bad_network_in_order() {
    let c = NetConditions::lag_ms(40.0)
        .with_jitter_ms(30.0)
        .with_loss_percent(30.0)
        .with_duplicate_percent(10.0)
        .with_reorder_percent(20.0);
    let mut r = rig(c);
    for i in 0..50 {
        r.tb.net_mut(1).send(Target::Server, &Chat(format!("m{i}")));
    }
    r.tb.run_for(10.0);
    let got: Vec<String> = r.chats[0]
        .borrow()
        .iter()
        .map(|c| c.msg.0.clone())
        .collect();
    assert_eq!(got, (0..50).map(|i| format!("m{i}")).collect::<Vec<_>>());
}

#[test]
fn unregistered_messages_do_not_panic() {
    #[derive(Clone, Serialize, Deserialize)]
    struct Nope;
    let mut r = rig(NetConditions::NONE);
    r.tb.net_mut(1).send(Target::Server, &Nope);
    r.tb.step();
}

#[test]
fn message_names_are_part_of_the_handshake() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server_with(NetConfig::dedicated("t", 1, "x"), register);
    let c = tb.add_client_with(NetConfig::join("t", 1, "1.1.1.1:1").unwrap(), |app| {
        app.net_command::<Move>("move", |_, _| Verdict::Accept);
    });
    tb.run_for(1.0);
    assert!(matches!(tb.net(c).status(), NetStatus::Disconnected(m) if m.contains("differ")));
}
