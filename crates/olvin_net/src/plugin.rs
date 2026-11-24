//! [`NetPlugin`]: puts [`Net`] and its systems into an `App`.

use std::cell::RefCell;

use olvin_core::{App, Plugin, Stage};
use olvin_ecs::EcsPlugin;

use crate::auth::Authenticator;
use crate::config::NetConfig;
use crate::net::{Net, NetEvent, clock};
use crate::transport::Transport;

/// Adds networking. `Events<NetEvent>` carries joins, leaves and connection state.
///
/// ```no_run
/// # use olvin_core::App;
/// # use olvin_net::{NetConfig, NetPlugin};
/// let mut app = App::new();
/// app.add_plugin(NetPlugin::new(NetConfig::offline("my-game", 1)));
/// ```
pub struct NetPlugin {
    cfg: NetConfig,
    transports: RefCell<Vec<Box<dyn Transport>>>,
    auth: RefCell<Option<Box<dyn Authenticator>>>,
}

impl NetPlugin {
    pub fn new(cfg: NetConfig) -> Self {
        Self {
            cfg,
            transports: RefCell::new(Vec::new()),
            auth: RefCell::new(None),
        }
    }

    /// Uses this transport instead of the default socket: a server listens on
    /// every one given, a client uses the first. Tests pass `Loopback` ends here.
    pub fn transport(self, transport: impl Transport) -> Self {
        self.transports.borrow_mut().push(Box::new(transport));
        self
    }

    /// Server: who may join (default: everybody).
    pub fn authenticator(self, auth: impl Authenticator) -> Self {
        *self.auth.borrow_mut() = Some(Box::new(auth));
        self
    }
}

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(EcsPlugin);
        let transports = std::mem::take(&mut *self.transports.borrow_mut());
        let auth = self.auth.borrow_mut().take();
        let net = Net::new(self.cfg.clone(), transports, auth);
        app.insert_resource(net).add_event::<NetEvent>();
        app.add_event::<crate::message::NetCommandRejected>();
        // Receive before gameplay (PreUpdate), send after it (PostUpdate).
        app.add_system(Stage::PreUpdate, |res| {
            let now = clock(res);
            let staged = {
                let mut net = res.fetch_mut::<Net>();
                net.update(now);
                net.client_replicate(res);
                net.stage_local();
                let mut events = res.fetch_mut::<olvin_core::Events<NetEvent>>();
                events.send_batch(net.events.drain(..));
                std::mem::take(&mut net.messages.staged)
            };
            // Game code runs here with `Net` free, so a validator may use it.
            crate::message::deliver_staged(res, staged);
            crate::predict::reconcile(res);
            let mut net = res.fetch_mut::<Net>();
            let mut events = res.fetch_mut::<olvin_core::Events<NetEvent>>();
            events.send_batch(net.events.drain(..));
            let mut rejected =
                res.fetch_mut::<olvin_core::Events<crate::message::NetCommandRejected>>();
            rejected.send_batch(net.messages.rejected.drain(..));
        });
        // After gameplay: predict, then step the server's fixed ticks and replicate.
        app.add_system(Stage::PostUpdate, |res| {
            let dt = res.fetch::<olvin_core::Time>().unscaled_delta();
            let plan = {
                let mut net = res.fetch_mut::<Net>();
                net.flush_owner_sends(&res.fetch::<olvin_ecs::World>());
                net.plan_ticks(dt)
            };
            crate::predict::run_ticks(res, plan);
        });
    }
}
