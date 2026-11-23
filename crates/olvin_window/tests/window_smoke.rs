//! Real-window smoke test: opens a window, runs a few frames of an `olvin`
//! app, resizes the window and exits through `AppExit`.
//!
//! Skipped unless `OLVIN_WINDOW_SMOKE=1` (it needs a display, and must not
//! pop windows up on a developer's desktop) and a display server is
//! reachable. CI runs it under Xvfb: `OLVIN_WINDOW_SMOKE=1 xvfb-run -a cargo
//! test -p olvin_window --test window_smoke`. The window is created hidden.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use olvin_core::{App, AppExit, Stage, SurfaceTarget};
use olvin_window::{WindowPlugin, WindowSettings};

/// Why the test cannot run here, if it cannot.
fn skip_reason() -> Option<&'static str> {
    if std::env::var("OLVIN_WINDOW_SMOKE").as_deref() != Ok("1") {
        return Some("OLVIN_WINDOW_SMOKE=1 is not set");
    }
    if cfg!(target_os = "linux")
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
        && std::env::var_os("DISPLAY").is_none()
    {
        return Some("no WAYLAND_DISPLAY or DISPLAY");
    }
    None
}

#[test]
fn window_opens_runs_frames_resizes_and_exits_cleanly() {
    if let Some(why) = skip_reason() {
        eprintln!("window smoke test skipped: {why}");
        return;
    }

    // A hung event loop (no frames ever delivered) must fail, not stall CI.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("window smoke test: no exit within 60 s");
        std::process::exit(1);
    });

    let frames = Rc::new(Cell::new(0u32));
    let first_size = Rc::new(Cell::new(None::<(u32, u32)>));
    let last_size = Rc::new(Cell::new((0u32, 0u32)));
    let asked_exit = Rc::new(Cell::new(false));

    let mut app = App::new();
    app.add_plugin(WindowPlugin {
        title: "Olvin window smoke test".into(),
        width: 400,
        height: 300,
        visible: false,
        any_thread: true,
        ..Default::default()
    });
    {
        let (frames, first, last) = (frames.clone(), first_size.clone(), last_size.clone());
        let asked = asked_exit.clone();
        app.add_system(Stage::Update, move |res| {
            let n = frames.get() + 1;
            frames.set(n);
            let size = res.fetch::<SurfaceTarget>().size;
            if first.get().is_none() {
                first.set(Some(size));
            }
            last.set(size);
            if n == 3 {
                // Ask for another size; the Resized event reports the result.
                res.fetch_mut::<WindowSettings>().set_resolution(640, 360);
            }
            // Run until the new size arrives (or give up) and leave.
            let resized = Some(size) != first.get();
            if (n >= 8 && resized) || n >= 300 {
                asked.set(true);
                res.fetch_mut::<AppExit>().requested = true;
            }
        });
    }

    // `run` returns only when the event loop ended: AppExit closes the window
    // and leaves the loop. A window that failed to open also returns, with no frames.
    app.run();

    assert!(frames.get() >= 8, "only {} frames ran", frames.get());
    assert!(asked_exit.get(), "the app did not request its exit");
    assert_ne!(
        Some(last_size.get()),
        first_size.get(),
        "the window kept its size {:?} after a resize request",
        last_size.get()
    );
}
