//! Test support, also meant for game authors: run a server and any number of
//! clients as separate `App`s in one process over a [`Loopback`] whose clock
//! the harness advances, so a multiplayer test is fast and repeatable.
//!
//! ```
//! use olvin_net::{NetConditions, NetConfig, NetEvent, testing::{Testbed, record}};
//!
//! let mut tb = Testbed::new(NetConditions::lag_ms(40.0));
//! let server = tb.add_server(NetConfig::dedicated("demo", 1, "loopback"));
//! let client = tb.add_client(NetConfig::join("demo", 1, "127.0.0.1:1").unwrap());
//! let seen = record::<NetEvent>(&mut tb.apps[client]);
//! tb.run_until(2.0, |_| seen.borrow().iter().any(|e| matches!(e, NetEvent::Connected { .. })));
//! assert_eq!(tb.net(server).clients().len(), 1);
//! ```

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use olvin_core::{App, EventCursor, Events, Time};

use crate::config::NetConfig;
use crate::loopback::Loopback;
use crate::net::Net;
use crate::plugin::NetPlugin;
use crate::sim::NetConditions;

/// A server and clients stepped together. `apps[0]` is whatever you added first.
pub struct Testbed {
    pub hub: Loopback,
    pub apps: Vec<App>,
    /// Seconds per step; every app's `Time` uses it.
    pub dt: f32,
}

impl Testbed {
    /// A hub with `conditions` on every link and 60 steps per second.
    pub fn new(conditions: NetConditions) -> Self {
        Self {
            hub: Loopback::with_conditions(conditions),
            apps: Vec::new(),
            dt: 1.0 / 60.0,
        }
    }

    /// A bare app with a deterministic clock; add your plugins, then pass it to [`Testbed::add_app`].
    pub fn base_app(&self) -> App {
        let mut app = App::new();
        app.insert_resource(Time::with_fixed_delta(self.dt));
        app
    }

    pub fn add_app(&mut self, app: App) -> usize {
        self.apps.push(app);
        self.apps.len() - 1
    }

    /// A server (host or dedicated per `cfg`) listening on the hub.
    pub fn add_server(&mut self, cfg: NetConfig) -> usize {
        self.add_server_with(cfg, |_| {})
    }

    /// Like [`Testbed::add_server`]; `setup` runs after the net plugin is added
    /// and before the first frame (register components and messages here).
    pub fn add_server_with(&mut self, cfg: NetConfig, setup: impl FnOnce(&mut App)) -> usize {
        let mut app = self.base_app();
        app.add_plugin(NetPlugin::new(cfg).transport(self.hub.listen()));
        setup(&mut app);
        self.add_app(app)
    }

    /// A client connecting to the hub's server.
    pub fn add_client(&mut self, cfg: NetConfig) -> usize {
        self.add_client_with(cfg, |_| {})
    }

    pub fn add_client_with(&mut self, cfg: NetConfig, setup: impl FnOnce(&mut App)) -> usize {
        let mut app = self.base_app();
        app.add_plugin(NetPlugin::new(cfg).transport(self.hub.connect()));
        setup(&mut app);
        self.add_app(app)
    }

    /// One frame for everybody: the hub clock moves, then each app updates.
    pub fn step(&mut self) {
        self.hub.advance(self.dt as f64);
        for app in &mut self.apps {
            app.update();
        }
    }

    pub fn run_for(&mut self, seconds: f32) {
        for _ in 0..(seconds / self.dt).ceil() as u32 {
            self.step();
        }
    }

    /// Steps until `done` is true or `max_seconds` pass; returns whether it became true.
    pub fn run_until(&mut self, max_seconds: f32, mut done: impl FnMut(&Testbed) -> bool) -> bool {
        for _ in 0..(max_seconds / self.dt).ceil() as u32 {
            if done(self) {
                return true;
            }
            self.step();
        }
        done(self)
    }

    pub fn net(&self, i: usize) -> Ref<'_, Net> {
        self.apps[i].resources.fetch::<Net>()
    }

    pub fn net_mut(&self, i: usize) -> RefMut<'_, Net> {
        self.apps[i].resources.fetch_mut::<Net>()
    }
}

/// Collects every `T` event of `app` into a list the test can look at. Add it
/// before the first frame.
pub fn record<T: Clone + 'static>(app: &mut App) -> Rc<RefCell<Vec<T>>> {
    app.add_event::<T>();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let out = seen.clone();
    let mut cursor = EventCursor::default();
    app.add_system(olvin_core::Stage::Last, move |res| {
        out.borrow_mut()
            .extend(res.fetch::<Events<T>>().read(&mut cursor).cloned());
    });
    seen
}
