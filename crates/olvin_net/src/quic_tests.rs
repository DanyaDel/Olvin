//! QUIC over 127.0.0.1. Skipped (with a message) where sockets are unavailable.

use std::time::{Duration, Instant};

use olvin_core::App;
use olvin_ecs::World;
use serde::{Deserialize, Serialize};

use crate::testing::record;
use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Health(u32);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Hello(String);

fn app(cfg: NetConfig) -> App {
    let mut app = App::new();
    app.add_plugin(NetPlugin::new(cfg));
    app.replicate::<Health>().net_message::<Hello>("hello");
    app
}

fn pump(apps: &mut [&mut App], secs: f32, mut done: impl FnMut(&[&mut App]) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs_f32(secs);
    while Instant::now() < end {
        for a in apps.iter_mut() {
            a.update();
        }
        if done(apps) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    false
}

fn status(a: &App) -> NetStatus {
    a.resources.fetch::<Net>().status()
}

/// A dedicated server on a free port, or `None` (and a message) without sockets.
fn server() -> Option<(App, RoomCode)> {
    let s = app(NetConfig::dedicated("quic", 1, "127.0.0.1:0"));
    if let NetStatus::Failed(why) = status(&s) {
        eprintln!("skipping QUIC test: {why}");
        return None;
    }
    let code = s.resources.fetch::<Net>().room_code(None)?;
    Some((s, code))
}

#[test]
fn join_by_room_code_replicate_and_message() {
    let Some((mut s, code)) = server() else {
        return;
    };
    assert!(code.to_string().starts_with("OLV-"));
    let mut c = app(NetConfig::join("quic", 1, &code.to_string()).unwrap());
    let seen = record::<NetReceived<Hello>>(&mut s);
    let got = record::<NetEvent>(&mut c);
    let e = s
        .resources
        .fetch_mut::<World>()
        .spawn((Replicated::default(), Health(7)));
    assert!(
        pump(&mut [&mut s, &mut c], 10.0, |a| a[1]
            .resources
            .fetch::<World>()
            .query::<&Health>()
            .iter()
            .count()
            == 1),
        "client never got the entity: {:?}",
        status(&c)
    );
    assert_eq!(status(&c), NetStatus::Connected);
    s.resources
        .fetch_mut::<World>()
        .insert_one(e, Health(9))
        .unwrap();
    c.resources
        .fetch_mut::<Net>()
        .send(Target::Server, &Hello("over quic".into()));
    assert!(pump(&mut [&mut s, &mut c], 10.0, |a| {
        a[1].resources
            .fetch::<World>()
            .query::<&Health>()
            .iter()
            .any(|h| h.0 == 9)
            && !seen.borrow().is_empty()
    }));
    assert_eq!(seen.borrow()[0].msg, Hello("over quic".into()));
    assert!(matches!(&got.borrow()[0], NetEvent::Connected { .. }));
    // A kick arrives with its reason.
    s.resources.fetch_mut::<Net>().kick(ClientId(1), "bye now");
    assert!(pump(&mut [&mut s, &mut c], 10.0, |a| matches!(
        status(a[1]),
        NetStatus::Disconnected(_)
    )));
    assert!(matches!(status(&c), NetStatus::Disconnected(r) if r.contains("bye now")));
}

#[test]
fn a_certificate_that_does_not_match_the_room_code_is_refused() {
    let Some((mut s, mut code)) = server() else {
        return;
    };
    code.pin[0] ^= 0xFF;
    let mut c = app(NetConfig::join("quic", 1, &code.to_string()).unwrap());
    let got = record::<NetEvent>(&mut c);
    assert!(pump(&mut [&mut s, &mut c], 10.0, |_| !got
        .borrow()
        .is_empty()));
    assert!(
        matches!(&got.borrow()[0], NetEvent::ConnectFailed { .. }),
        "{:?}",
        got.borrow()
    );
    assert_eq!(s.resources.fetch::<Net>().clients().len(), 0);
}

#[test]
fn a_bare_address_needs_explicit_permission() {
    let Some((mut s, code)) = server() else {
        return;
    };
    let addr = code.addr.to_string();
    let refused = app(NetConfig::join("quic", 1, &addr).unwrap());
    assert!(matches!(status(&refused), NetStatus::Failed(m) if m.contains("room code")));
    let mut c = app(NetConfig::join("quic", 1, &addr)
        .unwrap()
        .allow_unpinned(true));
    assert!(pump(&mut [&mut s, &mut c], 10.0, |a| status(a[1])
        == NetStatus::Connected));
}

#[test]
fn wrong_game_is_refused_over_quic_too() {
    let Some((mut s, code)) = server() else {
        return;
    };
    let mut c = app(NetConfig::join("other", 1, &code.to_string()).unwrap());
    let got = record::<NetEvent>(&mut c);
    assert!(pump(&mut [&mut s, &mut c], 10.0, |_| !got
        .borrow()
        .is_empty()));
    assert!(
        matches!(&got.borrow()[0], NetEvent::ConnectFailed { reason } if reason.contains("quic"))
    );
}
