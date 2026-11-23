//! The [`Net`] resource: the one handle a game uses.

use olvin_core::Time;

use crate::auth::{AllowAll, Authenticator};
use crate::client::{ClientState, Phase};
use crate::config::{NetConfig, NetMode};
use crate::proto::Msg;
use crate::registry::NetRegistry;
use crate::replicate::{
    NetId, Owner, ReplClient, ReplServer, ReplicateOptions, Replicated, client::PrefabSpawner,
    server::RelevancyFn,
};
use crate::server::{Ctx, ServerState};
use crate::transport::{Channel, Transport};

/// Identifies a client for the length of a session. [`ClientId::HOST`] is the
/// local player of a listen server and of offline play; remote clients count from 1.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ClientId(pub u32);

impl ClientId {
    pub const HOST: ClientId = ClientId(0);
}

/// Session events, read through `Events<NetEvent>`.
#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    /// Server: a client finished the handshake. Spawn its player here.
    ClientJoined { id: ClientId, name: String },
    /// Server: a client that lost its connection came back with its token.
    ClientReconnected { id: ClientId },
    /// Server: the connection of a client dropped. It may still come back until `ClientLeft`.
    ClientDisconnected { id: ClientId, reason: String },
    /// Server: the client is gone for good (left, kicked, or the grace period ran out).
    /// Despawn its player here.
    ClientLeft { id: ClientId, reason: String },
    /// Client: the handshake finished, this is our id.
    Connected { id: ClientId },
    /// Client: the connection ended (kicked, timed out, server closed).
    Disconnected { reason: String },
    /// Client: could not connect or the server refused us; the reason is
    /// written for the player.
    ConnectFailed { reason: String },
}

/// What a client of the session sees of another one.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientInfo {
    pub id: ClientId,
    pub name: String,
    pub connected: bool,
    /// Round-trip time in seconds (0 for the local player).
    pub rtt: f32,
    pub addr: Option<String>,
}

/// Where this process stands.
#[derive(Clone, Debug, PartialEq)]
pub enum NetStatus {
    /// A server (offline, host or dedicated) is running.
    Serving,
    Connecting,
    Connected,
    Disconnected(String),
    /// Could not start; the text says why and what to do.
    Failed(String),
}

/// The network handle, a resource. See the crate docs for the quick start.
pub struct Net {
    pub(crate) cfg: NetConfig,
    pub(crate) now: f64,
    pub(crate) events: Vec<NetEvent>,
    pub(crate) server: Option<ServerState>,
    pub(crate) client: Option<ClientState>,
    pub(crate) failed: Option<String>,
    pub(crate) disconnected: Option<String>,
    failed_reported: bool,
    pub(crate) registry: NetRegistry,
    pub(crate) repl: ReplState,
    pub(crate) relevancy: Option<RelevancyFn>,
    pub(crate) spawner: Option<PrefabSpawner>,
}

#[derive(Default)]
pub(crate) struct ReplState {
    pub server: Option<ReplServer>,
    pub client: ReplClient,
}

impl Net {
    /// Builds the session for `cfg`. `transports` are the sockets to use (a server
    /// listens on all of them, a client uses the first); empty means "make the
    /// default one" (QUIC with the `quic` feature).
    pub fn new(
        cfg: NetConfig,
        transports: Vec<Box<dyn Transport>>,
        auth: Option<Box<dyn Authenticator>>,
    ) -> Self {
        let mut net = Self {
            cfg,
            now: 0.0,
            events: Vec::new(),
            server: None,
            client: None,
            failed: None,
            disconnected: None,
            failed_reported: false,
            registry: NetRegistry::default(),
            repl: ReplState::default(),
            relevancy: None,
            spawner: None,
        };
        // The owner of an entity travels like any component; it is always id 0.
        net.replicate_with_named::<Owner>("Owner", ReplicateOptions::default());
        let transports = net.apply_simulation(transports);
        if net.cfg.is_server_mode() {
            net.start_server(transports, auth);
        } else if let Some(t) = transports.into_iter().next() {
            net.client = Some(ClientState::new(t, 0.0, None));
        } else {
            net.failed = Some(net.no_transport_message());
        }
        net
    }

    fn apply_simulation(&self, transports: Vec<Box<dyn Transport>>) -> Vec<Box<dyn Transport>> {
        match self.cfg.simulate {
            Some(c) => transports
                .into_iter()
                .map(|t| Box::new(crate::loopback::Conditioned::new(t, c)) as Box<dyn Transport>)
                .collect(),
            None => transports,
        }
    }

    fn no_transport_message(&self) -> String {
        match &self.cfg.mode {
            NetMode::Host { listen: Some(a) } | NetMode::Dedicated { listen: Some(a) } => format!(
                "cannot listen on {a}: this build has no socket transport. Enable the `quic` \
                 feature of olvin_net, or pass a transport to NetPlugin::transport()."
            ),
            NetMode::Client { .. } => {
                "cannot join: this build has no socket transport. Enable the \
                 `quic` feature of olvin_net, or pass a transport to NetPlugin::transport()."
                    .into()
            }
            _ => "no transport".into(),
        }
    }

    fn start_server(
        &mut self,
        transports: Vec<Box<dyn Transport>>,
        auth: Option<Box<dyn Authenticator>>,
    ) {
        let wants_socket = matches!(
            self.cfg.mode,
            NetMode::Host { listen: Some(_) } | NetMode::Dedicated { listen: Some(_) }
        );
        if wants_socket && transports.is_empty() {
            self.failed = Some(self.no_transport_message());
        }
        self.repl.server = Some(ReplServer::default());
        self.server = Some(ServerState::new(
            transports,
            auth.unwrap_or_else(|| Box::new(AllowAll)),
        ));
    }

    /// Registers `T` for replication under its short type name, with the defaults
    /// (`Transform` gets a compact codec and interpolation). Register before the
    /// first frame, in the same order on server and clients.
    ///
    /// ```
    /// # use olvin_net::{Net, NetConfig};
    /// # use olvin_scene::Transform;
    /// # #[derive(Clone, serde::Serialize, serde::Deserialize)] struct Health(u32);
    /// let mut net = Net::new(NetConfig::offline("g", 1), vec![], None);
    /// net.replicate::<Transform>().replicate::<Health>();
    /// ```
    pub fn replicate<T>(&mut self) -> &mut Self
    where
        T: olvin_ecs::Component + Clone + serde::Serialize + serde::de::DeserializeOwned,
    {
        self.replicate_with(crate::replicate::default_options::<T>())
    }

    /// [`Net::replicate`] with explicit [`ReplicateOptions`].
    pub fn replicate_with<T>(&mut self, opts: ReplicateOptions<T>) -> &mut Self
    where
        T: olvin_ecs::Component + Clone + serde::Serialize + serde::de::DeserializeOwned,
    {
        self.replicate_with_named(&crate::replicate::short_type_name::<T>(), opts)
    }

    /// Like [`Net::replicate_with`] under a name you choose (the name is the protocol).
    pub fn replicate_with_named<T>(&mut self, name: &str, opts: ReplicateOptions<T>) -> &mut Self
    where
        T: olvin_ecs::Component + Clone + serde::Serialize + serde::de::DeserializeOwned,
    {
        self.registry.add_name("component", name);
        self.registry
            .components
            .push(crate::replicate::new_entry(name, opts));
        self
    }

    /// Server: decides which entities each client gets (interest management).
    /// Return `false` to hide `entity` from `client`; hidden entities are
    /// despawned there and spawned again when they become relevant.
    pub fn set_relevancy(
        &mut self,
        f: impl Fn(&olvin_ecs::World, ClientId, olvin_ecs::Entity) -> bool + 'static,
    ) {
        self.relevancy = Some(Box::new(f));
    }

    /// Server: shows each client only entities within `radius` metres of an
    /// entity it owns. Entities without a `Transform` are always visible, and so is
    /// everything to a client that owns nothing yet.
    pub fn set_relevancy_distance(&mut self, radius: f32) {
        self.set_relevancy(crate::replicate::distance_relevancy(radius));
    }

    /// Client: how to create the copy of a replicated prefab. The `olvin` facade
    /// installs one over `scene_file::instantiate_prefab`.
    pub fn set_prefab_spawner(
        &mut self,
        f: impl Fn(
            &olvin_core::Resources,
            &str,
            olvin_scene::Transform,
        ) -> Result<olvin_ecs::Entity, String>
        + 'static,
    ) {
        self.spawner = Some(Box::new(f));
    }

    /// Whether a prefab spawner is installed (the facade installs one if you did not).
    pub fn has_prefab_spawner(&self) -> bool {
        self.spawner.is_some()
    }

    /// The entity behind a network id (server: the real one; client: the copy).
    pub fn entity(&self, id: NetId) -> Option<olvin_ecs::Entity> {
        match &self.repl.server {
            Some(rs) => rs.entity(id),
            None => self.repl.client.entity(id),
        }
    }

    /// Replicated entities this process tracks (server) or has copies of (client).
    pub fn replicated_count(&self) -> usize {
        match &self.repl.server {
            Some(rs) => rs.len(),
            None => self.repl.client.len(),
        }
    }

    /// Whether `entity` is owned by the local player.
    pub fn is_mine(&self, world: &olvin_ecs::World, entity: olvin_ecs::Entity) -> bool {
        match (self.local_client(), world.get::<&Owner>(entity)) {
            (Some(me), Ok(o)) => o.0 == me,
            _ => false,
        }
    }

    pub fn config(&self) -> &NetConfig {
        &self.cfg
    }

    /// The registrations (components, messages).
    pub fn registry(&self) -> &NetRegistry {
        &self.registry
    }

    /// Handshake hash of the registrations.
    pub fn signature(&self) -> u64 {
        self.registry.signature()
    }

    // ---- roles ----

    /// This process runs the authoritative world: offline, host or dedicated.
    pub fn is_server(&self) -> bool {
        self.server.is_some()
    }

    /// This process has a local player: offline, host or a remote client.
    pub fn is_client(&self) -> bool {
        !matches!(self.cfg.mode, NetMode::Dedicated { .. })
    }

    pub fn is_dedicated(&self) -> bool {
        matches!(self.cfg.mode, NetMode::Dedicated { .. })
    }

    pub fn is_offline(&self) -> bool {
        matches!(self.cfg.mode, NetMode::Offline)
    }

    /// A client of somebody else's server (not the host).
    pub fn is_remote_client(&self) -> bool {
        matches!(self.cfg.mode, NetMode::Client { .. })
    }

    /// Our id: [`ClientId::HOST`] for offline and host, the assigned one for a
    /// remote client once connected, `None` for a dedicated server or before the handshake.
    pub fn local_client(&self) -> Option<ClientId> {
        match self.cfg.mode {
            NetMode::Offline | NetMode::Host { .. } => Some(ClientId::HOST),
            NetMode::Dedicated { .. } => None,
            NetMode::Client { .. } => self.client.as_ref().and_then(|c| c.id),
        }
    }

    pub fn status(&self) -> NetStatus {
        if let Some(why) = &self.failed {
            return NetStatus::Failed(why.clone());
        }
        if self.server.is_some() {
            return NetStatus::Serving;
        }
        match self.client.as_ref().map(|c| c.phase) {
            Some(Phase::Connecting | Phase::Handshaking) => NetStatus::Connecting,
            Some(Phase::Connected) => NetStatus::Connected,
            _ => NetStatus::Disconnected(self.disconnected.clone().unwrap_or_default()),
        }
    }

    /// Current server tick (the server's own counter, or the last one a client heard).
    pub fn tick(&self) -> u32 {
        match (&self.server, &self.client) {
            (Some(s), _) => s.tick,
            (None, Some(c)) => c.server_tick,
            _ => 0,
        }
    }

    /// Round-trip time to the server in seconds; 0 on a server.
    pub fn rtt(&self) -> f32 {
        self.client.as_ref().map_or(0.0, |c| c.rtt)
    }

    /// Seconds on the network clock (`Time::unscaled_elapsed` at the last update).
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Server only: the clients that have a record (connected, or inside the
    /// reconnect grace period), plus the local player on offline and host.
    pub fn clients(&self) -> Vec<ClientInfo> {
        let mut out = Vec::new();
        if matches!(self.cfg.mode, NetMode::Offline | NetMode::Host { .. }) {
            out.push(ClientInfo {
                id: ClientId::HOST,
                name: self.cfg.player_name.clone(),
                connected: true,
                rtt: 0.0,
                addr: None,
            });
        }
        if let Some(s) = &self.server {
            out.extend(s.clients.values().map(|c| ClientInfo {
                id: c.id,
                name: c.name.clone(),
                connected: c.peer.is_some(),
                rtt: c.rtt,
                addr: c.addr.clone(),
            }));
        }
        out
    }

    // ---- session control ----

    /// Server: removes a client now and tells it why. The client does not get a
    /// grace period. Kicking the local player is ignored.
    pub fn kick(&mut self, id: ClientId, reason: &str) {
        let now = self.now;
        let Some(server) = self.server.as_mut() else {
            return;
        };
        let mut ctx = Ctx {
            cfg: &self.cfg,
            events: &mut self.events,
            now,
            registry_hash: self.registry.signature(),
        };
        server.remove_client(&mut ctx, id, reason, false);
    }

    /// Client: leaves the game. A server: stops listening (clients time out).
    pub fn disconnect(&mut self, reason: &str) {
        if let Some(c) = self.client.as_mut() {
            c.leave(reason);
            self.disconnected = Some(reason.to_owned());
        }
    }

    /// Client: connects again over `transport` after a drop, with the reconnect
    /// token from the previous session, so the server hands back our identity.
    pub fn reconnect_with(&mut self, transport: Box<dyn Transport>) {
        let token = self.client.as_ref().and_then(|c| c.reconnect);
        if let Some(t) = self.apply_simulation(vec![transport]).pop() {
            self.client = Some(ClientState::new(t, self.now, token));
            self.disconnected = None;
        }
    }

    /// Server: who may join (default: everybody).
    pub fn set_authenticator(&mut self, auth: impl Authenticator) {
        if let Some(s) = self.server.as_mut() {
            s.auth = Box::new(auth);
        }
    }

    // ---- internals ----

    /// Runs the session layer for one frame.
    pub(crate) fn update(&mut self, now: f64) {
        self.now = now;
        self.registry.frozen = true;
        let _ = Replicated::default;
        let hash = self.registry.signature();
        if let Some(server) = self.server.as_mut() {
            let mut ctx = Ctx {
                cfg: &self.cfg,
                events: &mut self.events,
                now,
                registry_hash: hash,
            };
            if !server.local_announced && self.cfg.mode_has_local_player() {
                server.local_announced = true;
                ctx.events.push(NetEvent::ClientJoined {
                    id: ClientId::HOST,
                    name: self.cfg.player_name.clone(),
                });
                ctx.events.push(NetEvent::Connected { id: ClientId::HOST });
            }
            let before = ctx.events.len();
            server.poll(&mut ctx);
            if let Some(rs) = self.repl.server.as_mut() {
                for ev in &ctx.events[before..] {
                    if let NetEvent::ClientJoined { id, .. } | NetEvent::ClientReconnected { id } =
                        ev
                    {
                        rs.reset_client(*id);
                    }
                }
                for (id, _, msg) in std::mem::take(&mut server.inbox) {
                    if let Msg::Ack { seq, bits } = msg {
                        rs.on_ack(id, seq, bits);
                    }
                }
            }
        }
        if let Some(client) = self.client.as_mut() {
            let before = self.events.len();
            client.poll(&self.cfg, now, hash, &mut self.events);
            for ev in &self.events[before..] {
                if let NetEvent::Disconnected { reason } | NetEvent::ConnectFailed { reason } = ev {
                    self.disconnected = Some(reason.clone());
                    self.repl.client.needs_clear = true;
                }
            }
        }
        if let Some(reason) = &self.failed
            && !self.failed_reported
        {
            self.failed_reported = true;
            self.events.push(NetEvent::ConnectFailed {
                reason: reason.clone(),
            });
        }
    }

    /// Sealed send from the server to one client. False if it is not connected.
    #[allow(dead_code)]
    pub(crate) fn server_send(&mut self, id: ClientId, channel: Channel, msg: &Msg) -> bool {
        self.server
            .as_mut()
            .is_some_and(|s| s.send_to(id, channel, msg))
    }
}

impl NetConfig {
    pub(crate) fn mode_has_local_player(&self) -> bool {
        matches!(self.mode, NetMode::Offline | NetMode::Host { .. })
    }
}

/// Reads the network clock from `Time`.
pub(crate) fn clock(res: &olvin_core::Resources) -> f64 {
    res.fetch::<Time>().unscaled_elapsed()
}
