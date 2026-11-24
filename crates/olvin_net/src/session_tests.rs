//! Session layer tests: three apps (one server, two clients) over `Loopback`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::testing::{Testbed, record};
use crate::*;

type Seen = Rc<RefCell<Vec<NetEvent>>>;

fn server_cfg() -> NetConfig {
    NetConfig::dedicated("test", 1, "loopback")
}

fn client_cfg(name: &str) -> NetConfig {
    NetConfig::join("test", 1, "127.0.0.1:1")
        .unwrap()
        .player_name(name)
}

fn joined(seen: &Seen) -> usize {
    seen.borrow()
        .iter()
        .filter(|e| matches!(e, NetEvent::ClientJoined { .. }))
        .count()
}

fn connected(seen: &Seen) -> bool {
    seen.borrow()
        .iter()
        .any(|e| matches!(e, NetEvent::Connected { .. }))
}

fn failure(seen: &Seen) -> Option<String> {
    seen.borrow().iter().find_map(|e| match e {
        NetEvent::ConnectFailed { reason } => Some(reason.clone()),
        _ => None,
    })
}

/// Server + two clients with a latency, everybody recording events.
fn trio(c: NetConditions) -> (Testbed, Seen, Seen, Seen) {
    let mut tb = Testbed::new(c);
    let s = tb.add_server_with(server_cfg().timeout(1.0), |_| {});
    let sv = record::<NetEvent>(&mut tb.apps[s]);
    let a = tb.add_client(client_cfg("ann").timeout(1.0));
    let av = record::<NetEvent>(&mut tb.apps[a]);
    let b = tb.add_client(client_cfg("bob"));
    let bv = record::<NetEvent>(&mut tb.apps[b]);
    (tb, sv, av, bv)
}

#[test]
fn two_clients_join_with_distinct_ids_and_names() {
    let (mut tb, sv, av, bv) = trio(NetConditions::lag_ms(30.0));
    assert!(tb.run_until(3.0, |_| joined(&sv) == 2
        && connected(&av)
        && connected(&bv)));
    let ids: Vec<_> = sv
        .borrow()
        .iter()
        .filter_map(|e| match e {
            NetEvent::ClientJoined { id, name } => Some((*id, name.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        [(ClientId(1), "ann".into()), (ClientId(2), "bob".into())]
    );
    assert_eq!(tb.net(1).local_client(), Some(ClientId(1)));
    assert_eq!(tb.net(2).local_client(), Some(ClientId(2)));
    assert_eq!(tb.net(1).status(), NetStatus::Connected);
    assert_eq!(tb.net(0).clients().len(), 2);
    assert!(tb.net(0).is_server() && tb.net(0).is_dedicated() && !tb.net(0).is_client());
}

#[test]
fn rtt_follows_the_simulated_latency() {
    let (mut tb, sv, av, _) = trio(NetConditions::lag_ms(50.0));
    tb.run_until(3.0, |_| joined(&sv) == 2 && connected(&av));
    tb.run_for(3.0);
    let rtt = tb.net(1).rtt();
    assert!((0.09..0.14).contains(&rtt), "rtt {rtt}");
}

#[test]
fn polite_leave_is_reported_once() {
    let (mut tb, sv, av, _) = trio(NetConditions::lag_ms(10.0));
    tb.run_until(3.0, |_| joined(&sv) == 2 && connected(&av));
    tb.net_mut(1).disconnect("bye");
    tb.run_for(1.0);
    let left: Vec<_> = sv
        .borrow()
        .iter()
        .filter_map(|e| match e {
            NetEvent::ClientLeft { id, reason } => Some((*id, reason.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(left, [(ClientId(1), "bye".into())]);
    assert_eq!(tb.net(0).clients().len(), 1);
}

#[test]
fn kick_tells_the_client_why() {
    let (mut tb, sv, av, bv) = trio(NetConditions::lag_ms(10.0));
    tb.run_until(3.0, |_| {
        joined(&sv) == 2 && connected(&av) && connected(&bv)
    });
    tb.net_mut(0).kick(ClientId(2), "no cheating");
    tb.run_for(1.0);
    assert!(bv.borrow().contains(&NetEvent::Disconnected {
        reason: "no cheating".into()
    }));
    assert!(
        !av.borrow()
            .iter()
            .any(|e| matches!(e, NetEvent::Disconnected { .. }))
    );
    assert!(sv.borrow().contains(&NetEvent::ClientLeft {
        id: ClientId(2),
        reason: "no cheating".into()
    }));
    assert!(matches!(tb.net(2).status(), NetStatus::Disconnected(r) if r == "no cheating"));
}

#[test]
fn silent_client_times_out_and_client_notices_dead_server() {
    let (mut tb, sv, av, _) = trio(NetConditions::lag_ms(10.0));
    tb.run_until(3.0, |_| joined(&sv) == 2 && connected(&av));
    // Pull ann's cable (both directions, all packets).
    let cut = NetConditions {
        blackhole: true,
        ..NetConditions::NONE
    };
    tb.hub.set_conditions(cut);
    tb.run_for(2.5);
    assert!(sv.borrow().iter().any(
        |e| matches!(e, NetEvent::ClientDisconnected { reason, .. } if reason == "timed out")
    ));
    assert!(
        av.borrow().iter().any(
            |e| matches!(e, NetEvent::Disconnected { reason } if reason.contains("timed out"))
        )
    );
}

#[test]
fn reconnect_with_token_restores_identity() {
    let (mut tb, sv, av, bv) = trio(NetConditions::lag_ms(10.0));
    tb.run_until(3.0, |_| {
        joined(&sv) == 2 && connected(&av) && connected(&bv)
    });
    // The client's transport dies without a goodbye: swap its link for a new one.
    let fresh = tb.hub.connect();
    tb.net_mut(1).reconnect_with(Box::new(fresh));
    assert!(tb.run_until(3.0, |_| {
        sv.borrow()
            .iter()
            .any(|e| matches!(e, NetEvent::ClientReconnected { id } if *id == ClientId(1)))
    }));
    assert!(tb.run_until(1.0, |tb| tb.net(1).local_client() == Some(ClientId(1))));
    // Nobody new joined.
    assert_eq!(joined(&sv), 2);
    // A wrong token joins as a new client instead of stealing an identity.
}

#[test]
fn reconnect_token_cannot_be_guessed() {
    let (mut tb, sv, av, _) = trio(NetConditions::NONE);
    tb.run_until(3.0, |_| joined(&sv) == 2 && connected(&av));
    // An attacker knows the id but not the token.
    let mut attacker = tb.base_app();
    attacker.add_plugin(NetPlugin::new(client_cfg("eve")).transport(tb.hub.connect()));
    if let Some(c) = attacker.resources.fetch_mut::<Net>().client.as_mut() {
        c.reconnect = Some((1, [7; 16]));
    }
    let ev = record::<NetEvent>(&mut attacker);
    let e = tb.add_app(attacker);
    tb.run_for(1.0);
    assert!(
        ev.borrow()
            .contains(&NetEvent::Connected { id: ClientId(3) })
    );
    assert_eq!(tb.net(e).local_client(), Some(ClientId(3)));
    assert_eq!(
        sv.borrow()
            .iter()
            .filter(|e| matches!(e, NetEvent::ClientReconnected { .. }))
            .count(),
        0
    );
}

#[test]
fn bad_handshakes_are_refused_with_helpful_reasons() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server_with(server_cfg().max_clients(1), |_| {});
    let mut try_cfg = |cfg: NetConfig| {
        let c = tb.add_client(cfg);
        let seen = record::<NetEvent>(&mut tb.apps[c]);
        tb.run_for(1.0);
        failure(&seen)
    };
    let wrong_game = NetConfig::join("other-game", 1, "1.1.1.1:1").unwrap();
    let r = try_cfg(wrong_game).expect("refused");
    assert!(r.contains("test") && r.contains("other-game"), "{r}");
    let old = NetConfig::join("test", 2, "1.1.1.1:1").unwrap();
    assert!(try_cfg(old).expect("refused").contains("v1"));
    fn skewed() -> u64 {
        crate::config::system_unix_millis() + 3_600_000
    }
    let clock = client_cfg("x").clock(skewed);
    let r = try_cfg(clock).expect("refused");
    assert!(r.contains("clock"), "{r}");
    // One slot: the first honest client fits, the second does not.
    assert!(try_cfg(client_cfg("a")).is_none());
    let r = try_cfg(client_cfg("b")).expect("refused");
    assert!(r.contains("full"), "{r}");
}

#[test]
fn password_authenticator() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server(server_cfg());
    tb.net_mut(0)
        .set_authenticator(SharedPassword::new("hunter2"));
    let good = tb.add_client(client_cfg("good").token("hunter2"));
    let g = record::<NetEvent>(&mut tb.apps[good]);
    let bad = tb.add_client(client_cfg("bad").token("letmein"));
    let b = record::<NetEvent>(&mut tb.apps[bad]);
    tb.run_for(1.0);
    assert!(connected(&g));
    assert_eq!(failure(&b).as_deref(), Some("wrong password"));
}

#[test]
fn registry_mismatch_is_caught_before_any_bytes_are_misread() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server_with(server_cfg(), |app| {
        app.resources
            .fetch_mut::<Net>()
            .registry
            .names
            .push("component:Health".into());
    });
    let c = tb.add_client(client_cfg("x"));
    let seen = record::<NetEvent>(&mut tb.apps[c]);
    tb.run_for(1.0);
    assert!(failure(&seen).expect("refused").contains("differ"));
}

/// Speaks the wire protocol by hand to test what an honest client never sends.
struct Raw {
    t: LoopbackClient,
    got: Vec<TransportEvent>,
}

impl Raw {
    fn new(tb: &Testbed) -> Self {
        Self {
            t: tb.hub.connect(),
            got: Vec::new(),
        }
    }
    fn poll(&mut self) {
        self.t.poll(&mut self.got);
    }
    fn rejected(&self) -> Option<proto::Reject> {
        self.got.iter().find_map(|e| match e {
            TransportEvent::Data { data, .. } => {
                let (k, body) = wire::unframe(data).ok()?;
                (k == wire::kind::REJECT)
                    .then(|| wire::decode::<proto::Reject>(body, 1024).ok())
                    .flatten()
            }
            _ => None,
        })
    }
}

fn hello_bytes(version: u16, hello: &proto::Hello) -> Vec<u8> {
    let mut body = version.to_le_bytes().to_vec();
    body.extend(wire::encode(hello).unwrap());
    wire::frame(wire::kind::HELLO, &body)
}

fn good_hello(nonce: u8, registry_hash: u64) -> proto::Hello {
    proto::Hello {
        game_id: "test".into(),
        game_version: 1,
        registry_hash,
        name: "raw".into(),
        token: vec![],
        nonce: [nonce; 16],
        unix_ms: crate::config::system_unix_millis(),
        reconnect: None,
    }
}

#[test]
fn protocol_mismatch_and_replayed_hello_are_rejected() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server(server_cfg());
    let hash = tb.net(0).signature();

    let mut old = Raw::new(&tb);
    tb.step();
    old.poll();
    old.t.send(
        ConnId(0),
        Channel::Reliable,
        &hello_bytes(999, &good_hello(1, hash)),
    );
    tb.run_for(0.2);
    old.poll();
    let r = old.rejected().expect("rejected");
    assert_eq!(r.code, proto::RejectCode::ProtocolMismatch);
    assert!(r.message.contains("999"), "{}", r.message);

    // A valid Hello works once...
    let packet = hello_bytes(wire::PROTOCOL_VERSION, &good_hello(9, hash));
    let mut first = Raw::new(&tb);
    tb.step();
    first.poll();
    first.t.send(ConnId(0), Channel::Reliable, &packet);
    tb.run_for(0.2);
    first.poll();
    assert!(first.rejected().is_none());
    assert!(
        first
            .got
            .iter()
            .any(|e| matches!(e, TransportEvent::Data { .. })),
        "no Welcome"
    );
    // ...and the recorded bytes replayed on a fresh connection are refused.
    let mut again = Raw::new(&tb);
    tb.step();
    again.poll();
    again.t.send(ConnId(0), Channel::Reliable, &packet);
    tb.run_for(0.2);
    again.poll();
    assert_eq!(
        again.rejected().expect("rejected").code,
        proto::RejectCode::Replay
    );
}

#[test]
fn garbage_never_panics_and_the_server_keeps_serving() {
    let mut tb = Testbed::new(NetConditions::NONE);
    tb.add_server(server_cfg());
    let hash = tb.net(0).signature();
    let mut rng = sim::SimRng::new(99);
    let mut raws: Vec<Raw> = (0..4).map(|_| Raw::new(&tb)).collect();
    tb.step();
    for raw in &mut raws {
        raw.poll();
    }
    for round in 0..400u32 {
        for (i, raw) in raws.iter_mut().enumerate() {
            let len = (rng.next_u64() % 3000) as usize;
            let mut bytes: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
            // Make about half of them look almost right.
            if round.is_multiple_of(2) && bytes.len() > 2 {
                bytes[0] = wire::MAGIC;
                bytes[1] = (rng.next_u64() % 6) as u8;
            }
            let ch = if rng.next_u64().is_multiple_of(2) {
                Channel::Reliable
            } else {
                Channel::Unreliable
            };
            raw.t.send(ConnId(0), ch, &bytes);
            if i == 0 && round.is_multiple_of(50) {
                // A truncated but well-framed Hello.
                let full = hello_bytes(wire::PROTOCOL_VERSION, &good_hello(round as u8, hash));
                raw.t
                    .send(ConnId(0), Channel::Reliable, &full[..full.len() / 2]);
            }
        }
        tb.step();
    }
    // The noisy connections were cut off, and an honest client still gets in.
    let c = tb.add_client(client_cfg("honest"));
    let seen = record::<NetEvent>(&mut tb.apps[c]);
    tb.run_for(1.0);
    assert!(connected(&seen));
    for raw in &mut raws {
        raw.poll();
    }
    assert!(raws.iter().any(|r| {
        r.got
            .iter()
            .any(|e| matches!(e, TransportEvent::Disconnected { .. }))
    }));
}

#[test]
fn garbage_after_the_handshake_is_survived_too() {
    let (mut tb, sv, av, _) = trio(NetConditions::NONE);
    tb.run_until(3.0, |_| joined(&sv) == 2 && connected(&av));
    // Forge packets that look like session data (right header, wrong tag).
    let mut rng = sim::SimRng::new(5);
    for _ in 0..200 {
        let len = 18 + (rng.next_u64() % 200) as usize;
        let mut b: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
        b[0] = wire::MAGIC;
        b[1] = wire::kind::DATA;
        if let Some(c) = tb.apps[1].resources.fetch_mut::<Net>().client.as_mut() {
            c.transport.send(ConnId(0), Channel::Unreliable, &b);
        }
        tb.step();
    }
    // Ann was cut off for spamming; bob and the server are fine.
    tb.run_for(0.5);
    assert!(
        tb.net(0)
            .clients()
            .iter()
            .any(|c| c.id == ClientId(2) && c.connected)
    );
}

#[test]
fn joining_works_through_loss_jitter_reordering() {
    let c = NetConditions::lag_ms(60.0)
        .with_jitter_ms(40.0)
        .with_loss_percent(30.0)
        .with_duplicate_percent(10.0)
        .with_reorder_percent(20.0);
    let (mut tb, sv, av, bv) = trio(c);
    assert!(tb.run_until(10.0, |_| joined(&sv) == 2
        && connected(&av)
        && connected(&bv)));
}

#[test]
fn offline_is_a_server_with_a_local_player() {
    let mut app = olvin_core::App::new();
    app.add_plugin(NetPlugin::new(
        NetConfig::offline("test", 1).player_name("solo"),
    ));
    let seen = record::<NetEvent>(&mut app);
    app.update();
    let net = app.resources.fetch::<Net>();
    assert!(net.is_server() && net.is_client() && net.is_offline());
    assert_eq!(net.local_client(), Some(ClientId::HOST));
    assert_eq!(net.status(), NetStatus::Serving);
    assert_eq!(net.clients().len(), 1);
    drop(net);
    app.update();
    assert!(seen.borrow().contains(&NetEvent::ClientJoined {
        id: ClientId::HOST,
        name: "solo".into()
    }));
}

#[cfg(not(feature = "quic"))]
#[test]
fn missing_socket_transport_fails_with_advice() {
    let mut app = olvin_core::App::new();
    app.add_plugin(NetPlugin::new(NetConfig::host("test", 1, "0.0.0.0:7777")));
    let seen = record::<NetEvent>(&mut app);
    app.update();
    assert!(matches!(
        app.resources.fetch::<Net>().status(),
        NetStatus::Failed(m) if m.contains("quic")
    ));
    assert!(failure(&seen).is_some());
}

#[test]
fn simulation_setting_wraps_the_transport() {
    let hub = Loopback::new();
    let cfg = client_cfg("x").simulate(NetConditions::lag_ms(10.0));
    let mut app = olvin_core::App::new();
    app.add_plugin(NetPlugin::new(cfg).transport(hub.connect()));
    app.update();
    assert_eq!(app.resources.fetch::<Net>().status(), NetStatus::Connecting);
}
