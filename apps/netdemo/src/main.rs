//! `cargo run -p netdemo -- [--host [addr] | --dedicated [addr] | --join <code|addr>]
//!  [--bot] [--frames N] [--name N] [--net-sim lag=80,loss=2]`; `--selftest` runs a
//! server and two clients in this process over QUIC on 127.0.0.1 and exits 0 on success.

use std::time::Duration;

use netdemo::{Move, bot_system, build_app};
use olvin::net::{Net, NetConfig, NetStatus};
use olvin::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(if netdemo::selftest::run() { 0 } else { 1 });
    }
    let cfg = match NetConfig::from_args("netdemo", 1, args.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("netdemo: {e}");
            std::process::exit(2);
        }
    };
    let frames: Option<u64> = args
        .iter()
        .position(|a| a == "--frames")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok());
    let mut app = build_app(cfg);
    if args.iter().any(|a| a == "--bot") {
        app.add_system(Stage::Update, bot_system);
    }
    let _ = Move(Vec2::ZERO);
    app.update();
    if let Some(code) = app.resources.fetch::<Net>().room_code(None) {
        println!("room code (replace 127.0.0.1 by your LAN or public IP): {code}");
    }
    let mut n = 1;
    while !app.exit_requested() && frames.is_none_or(|f| n < f) {
        std::thread::sleep(Duration::from_millis(16));
        app.update();
        n += 1;
        if let NetStatus::Failed(why) | NetStatus::Disconnected(why) =
            app.resources.fetch::<Net>().status()
        {
            eprintln!("netdemo: {why}");
            std::process::exit(1);
        }
    }
    let net = app.resources.fetch::<Net>();
    println!(
        "done after {n} frames: {} replicated entities, status {:?}",
        net.replicated_count(),
        net.status()
    );
}
