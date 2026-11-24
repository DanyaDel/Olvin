//! One server and two bot clients as threads of this process, over real QUIC
//! sockets on 127.0.0.1. Shared by `--selftest` and the integration test.

use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use olvin::net::{Net, NetConfig, NetId, NetStatus, Remote};
use olvin::prelude::*;
use olvin::script::ScriptVars;

use crate::{bot_system, build_app};

/// Runs the cluster; true when everything converged.
pub fn run() -> bool {
    let (code_tx, code_rx) = channel::<String>();
    let (done_tx, done_rx) = channel::<(String, Vec<(u64, f32, f32)>, i64)>();
    let server = thread::spawn(move || {
        let mut app = build_app(NetConfig::dedicated("netdemo", 1, "127.0.0.1:0"));
        app.update();
        let code = app.resources.fetch::<Net>().room_code(None);
        let Some(code) = code else {
            let _ = code_tx.send(String::new());
            return false;
        };
        let _ = code_tx.send(code.to_string());
        for _ in 0..360 {
            thread::sleep(Duration::from_millis(16));
            app.update();
        }
        let world = app.resources.fetch::<World>();
        let n = world.query::<&NetId>().iter().count();
        n == 2
    });
    let Ok(code) = code_rx.recv() else {
        return false;
    };
    if code.is_empty() {
        eprintln!("selftest: no sockets here, skipping");
        return true;
    }
    let mut clients = Vec::new();
    for name in ["ann", "bob"] {
        let (code, done) = (code.clone(), done_tx.clone());
        clients.push(thread::spawn(move || {
            let Ok(cfg) = NetConfig::join("netdemo", 1, &code) else {
                return;
            };
            let cfg = cfg.player_name(name);
            let mut app = build_app(cfg);
            for frame in 0..300 {
                // Walk for 3 s, then stand still so the picture can settle.
                if frame == 0 {
                    app.add_system(Stage::Update, |res| {
                        if res.fetch::<Time>().elapsed() < 3.0 {
                            bot_system(res);
                        }
                    });
                }
                thread::sleep(Duration::from_millis(16));
                app.update();
            }
            let ok = matches!(app.resources.fetch::<Net>().status(), NetStatus::Connected);
            let world = app.resources.fetch::<World>();
            let seen: Vec<(u64, f32, f32)> = world
                .query::<(&NetId, &Remote, &olvin::scene::Transform)>()
                .iter()
                .map(|(id, _, t)| (id.0, t.translation.x, t.translation.z))
                .collect();
            let shots: i64 = app
                .resources
                .fetch::<ScriptVars>()
                .get("shots_1")
                .and_then(|v| v.as_f64())
                .map_or(0, |v| v as i64);
            let _ = done.send((format!("{name} connected={ok}"), seen, shots));
        }));
    }
    drop(done_tx);
    let mut ok = true;
    let mut views = Vec::new();
    while let Ok((who, seen, shots)) = done_rx.recv() {
        println!("{who}: sees {} players, shots_1={shots}", seen.len());
        ok &= who.ends_with("true") && seen.len() == 2 && shots >= 1;
        views.push(seen);
    }
    ok &= views.len() == 2;
    // Both clients must agree on where everybody ended up.
    if let [a, b] = &views[..] {
        let mut a = a.clone();
        let mut b = b.clone();
        a.sort_by_key(|p| p.0);
        b.sort_by_key(|p| p.0);
        for (p, q) in a.iter().zip(&b) {
            ok &= p.0 == q.0 && (p.1 - q.1).abs() < 0.05 && (p.2 - q.2).abs() < 0.05;
        }
    }
    for c in clients {
        let _ = c.join();
    }
    ok &= server.join().unwrap_or(false);
    println!("selftest: {}", if ok { "ok" } else { "FAILED" });
    ok
}
