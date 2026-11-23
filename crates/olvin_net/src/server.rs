//! Server half of the session layer: accepts connections, runs the handshake,
//! keeps the client table, heartbeats and timeouts.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::auth::{AuthRequest, Authenticator};
use crate::config::NetConfig;
use crate::crypto::{self, Direction, Nonce, SessionKey};
use crate::net::{ClientId, NetEvent};
use crate::proto::{Hello, Msg, Reject, RejectCode, Welcome};
use crate::secure::{OpenError, Secure};
use crate::transport::{Channel, ConnId, Transport, TransportEvent};
use crate::wire::{self, PROTOCOL_VERSION, kind};

/// Garbage packets a connection may send before it is cut off.
const MAX_STRIKES: u32 = 20;
/// How long a refused or kicked connection lingers so the reason gets out.
const CLOSE_DELAY: f64 = 0.25;
const MAX_NONCES: usize = 4096;

pub(crate) struct Ctx<'a> {
    pub cfg: &'a NetConfig,
    pub events: &'a mut Vec<NetEvent>,
    pub now: f64,
    pub registry_hash: u64,
}

enum PeerState {
    Pending,
    Active(ClientId),
    /// Told to go; waiting for the reason to be delivered, then closed.
    Closing(f64),
}

struct Peer {
    state: PeerState,
    since: f64,
    strikes: u32,
}

pub(crate) struct ClientRec {
    pub id: ClientId,
    pub name: String,
    pub peer: Option<(usize, ConnId)>,
    pub secure: Option<Secure>,
    pub last_heard: f64,
    pub rtt: f32,
    pub addr: Option<String>,
    pub reconnect_token: [u8; 16],
    /// Connection lost at this time; the client may still reclaim its identity.
    pub lost_at: Option<f64>,
}

pub(crate) struct ServerState {
    pub transports: Vec<Box<dyn Transport>>,
    peers: HashMap<(usize, ConnId), Peer>,
    pub clients: BTreeMap<ClientId, ClientRec>,
    next_id: u32,
    pub auth: Box<dyn Authenticator>,
    nonces: HashSet<Nonce>,
    nonce_order: VecDeque<Nonce>,
    pub tick: u32,
    pub local_announced: bool,
}

impl ServerState {
    pub fn new(transports: Vec<Box<dyn Transport>>, auth: Box<dyn Authenticator>) -> Self {
        Self {
            transports,
            peers: HashMap::new(),
            clients: BTreeMap::new(),
            next_id: 1,
            auth,
            nonces: HashSet::new(),
            nonce_order: VecDeque::new(),
            tick: 0,
            local_announced: false,
        }
    }

    pub fn poll(&mut self, ctx: &mut Ctx) {
        let mut events = Vec::new();
        for ti in 0..self.transports.len() {
            events.clear();
            self.transports[ti].poll(&mut events);
            for ev in events.drain(..) {
                self.on_event(ctx, ti, ev);
            }
        }
        self.maintain(ctx);
    }

    fn on_event(&mut self, ctx: &mut Ctx, ti: usize, ev: TransportEvent) {
        match ev {
            TransportEvent::Connected { conn } => {
                let pending = self
                    .peers
                    .values()
                    .filter(|p| matches!(p.state, PeerState::Pending))
                    .count();
                if pending >= ctx.cfg.max_pending {
                    log::warn!("net: too many pending handshakes, refusing a connection");
                    self.transports[ti].close(conn, "server busy");
                    return;
                }
                self.peers.insert(
                    (ti, conn),
                    Peer {
                        state: PeerState::Pending,
                        since: ctx.now,
                        strikes: 0,
                    },
                );
            }
            TransportEvent::Disconnected { conn, reason } => {
                if let Some(peer) = self.peers.remove(&(ti, conn))
                    && let PeerState::Active(id) = peer.state
                {
                    self.connection_lost(ctx, id, &reason);
                }
            }
            TransportEvent::Data {
                conn,
                channel,
                data,
            } => self.on_data(ctx, ti, conn, channel, data),
        }
    }

    fn strike(&mut self, ti: usize, conn: ConnId, why: &str) {
        let Some(peer) = self.peers.get_mut(&(ti, conn)) else {
            return;
        };
        peer.strikes += 1;
        log::debug!("net: bad packet from {conn:?}: {why}");
        if peer.strikes >= MAX_STRIKES {
            log::warn!("net: closing {conn:?}: too many malformed packets");
            self.transports[ti].close(conn, "too many malformed packets");
        }
    }

    fn on_data(&mut self, ctx: &mut Ctx, ti: usize, conn: ConnId, channel: Channel, data: Vec<u8>) {
        let max = match channel {
            Channel::Reliable => ctx.cfg.limits.max_reliable_packet,
            Channel::Unreliable => self.transports[ti].max_datagram().max(64),
        };
        let Some(peer) = self.peers.get(&(ti, conn)) else {
            return;
        };
        if data.len() > max {
            self.strike(ti, conn, "oversized packet");
            return;
        }
        let (k, body) = match wire::unframe(&data) {
            Ok(x) => x,
            Err(e) => {
                self.strike(ti, conn, &e.to_string());
                return;
            }
        };
        match peer.state {
            PeerState::Closing(_) => {}
            PeerState::Pending => {
                if k == kind::HELLO && channel == Channel::Reliable {
                    self.on_hello(ctx, ti, conn, body);
                } else {
                    self.strike(ti, conn, "expected Hello");
                }
            }
            PeerState::Active(id) => {
                if k != kind::DATA {
                    self.strike(ti, conn, "unexpected handshake packet");
                    return;
                }
                self.on_session_data(ctx, ti, conn, id, channel, body);
            }
        }
    }

    fn on_session_data(
        &mut self,
        ctx: &mut Ctx,
        ti: usize,
        conn: ConnId,
        id: ClientId,
        channel: Channel,
        body: &[u8],
    ) {
        let Some(rec) = self.clients.get_mut(&id) else {
            return;
        };
        let Some(secure) = rec.secure.as_mut() else {
            return;
        };
        let max = ctx.cfg.limits.max_reliable_packet;
        let msg = match secure.open(channel, body, max) {
            Ok(m) => m,
            Err(OpenError::Replay) => return,
            Err(e) => {
                self.strike(ti, conn, &format!("{e:?}"));
                return;
            }
        };
        rec.last_heard = ctx.now;
        match msg {
            Msg::Ping { id: ping, sent } => {
                let pong = Msg::Pong {
                    id: ping,
                    sent,
                    server_tick: self.tick,
                };
                self.send_to(id, Channel::Unreliable, &pong);
            }
            Msg::Pong { .. } => {}
            Msg::Disconnect { reason } => {
                self.remove_client(ctx, id, &clip(&reason), true);
            }
        }
    }

    fn on_hello(&mut self, ctx: &mut Ctx, ti: usize, conn: ConnId, body: &[u8]) {
        let limits = &ctx.cfg.limits;
        if body.len() > limits.max_handshake_packet || body.len() < 2 {
            self.reject(
                ctx,
                ti,
                conn,
                RejectCode::Malformed,
                "handshake packet has a bad size",
            );
            return;
        }
        let theirs = u16::from_le_bytes([body[0], body[1]]);
        if theirs != PROTOCOL_VERSION {
            let msg = format!(
                "protocol mismatch: the server speaks version {PROTOCOL_VERSION}, you speak {theirs}. Update the game."
            );
            self.reject(ctx, ti, conn, RejectCode::ProtocolMismatch, &msg);
            return;
        }
        let hello: Hello = match wire::decode(&body[2..], limits.max_handshake_packet) {
            Ok(h) => h,
            Err(e) => {
                self.reject(
                    ctx,
                    ti,
                    conn,
                    RejectCode::Malformed,
                    &format!("bad Hello: {e}"),
                );
                return;
            }
        };
        if hello.game_id != ctx.cfg.game_id || hello.game_version != ctx.cfg.game_version {
            let msg = format!(
                "this server runs {} v{}, you have {} v{}",
                ctx.cfg.game_id,
                ctx.cfg.game_version,
                clip(&hello.game_id),
                hello.game_version
            );
            self.reject(ctx, ti, conn, RejectCode::GameMismatch, &msg);
            return;
        }
        if hello.registry_hash != ctx.registry_hash {
            self.reject(
                ctx,
                ti,
                conn,
                RejectCode::RegistryMismatch,
                "the game's replicated components or messages differ between server and client",
            );
            return;
        }
        if hello.name.chars().count() > limits.max_name_len
            || hello.token.len() > limits.max_token_len
        {
            self.reject(
                ctx,
                ti,
                conn,
                RejectCode::Malformed,
                "name or token too long",
            );
            return;
        }
        let server_ms = (ctx.cfg.clock)();
        let skew = server_ms.abs_diff(hello.unix_ms) as f64 / 1000.0;
        if skew > ctx.cfg.timestamp_window as f64 {
            let msg = format!(
                "your clock is {skew:.0} s away from the server's (allowed: {:.0} s). Check the date and time.",
                ctx.cfg.timestamp_window
            );
            self.reject(ctx, ti, conn, RejectCode::BadTimestamp, &msg);
            return;
        }
        if !self.remember_nonce(hello.nonce) {
            self.reject(
                ctx,
                ti,
                conn,
                RejectCode::Replay,
                "this Hello was already used",
            );
            return;
        }
        let name = sanitize(&hello.name);
        let addr = self.transports[ti].peer_addr(conn);
        // Reconnect: same identity back, if the token is right.
        if let Some(req) = &hello.reconnect {
            let id = ClientId(req.client_id);
            let ok = self
                .clients
                .get(&id)
                .is_some_and(|r| crypto::ct_eq(&r.reconnect_token, &req.token));
            if ok {
                if let Some(old) = self.clients.get(&id).and_then(|r| r.peer) {
                    self.close_peer(old, ctx.now);
                }
                self.attach(ctx, ti, conn, id, &hello.nonce, true);
                return;
            }
            // Unknown or expired token: continue as a fresh join.
        }
        let remote = self
            .clients
            .values()
            .filter(|c| c.peer.is_some() || c.lost_at.is_some())
            .count();
        if remote >= ctx.cfg.max_clients {
            self.reject(ctx, ti, conn, RejectCode::ServerFull, "the server is full");
            return;
        }
        let request = AuthRequest {
            token: &hello.token,
            name: &name,
            addr: addr.as_deref(),
        };
        let info = match self.auth.authenticate(&request) {
            Ok(i) => i,
            Err(reason) => {
                self.reject(ctx, ti, conn, RejectCode::Denied, &clip(&reason));
                return;
            }
        };
        let id = ClientId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let Some(token) = crypto::random::<16>() else {
            self.reject(
                ctx,
                ti,
                conn,
                RejectCode::Busy,
                "the server could not get random bytes",
            );
            return;
        };
        self.clients.insert(
            id,
            ClientRec {
                id,
                name: sanitize(&info.name),
                peer: None,
                secure: None,
                last_heard: ctx.now,
                rtt: 0.0,
                addr,
                reconnect_token: token,
                lost_at: None,
            },
        );
        self.attach(ctx, ti, conn, id, &hello.nonce, false);
    }

    /// Binds a connection to a client record: keys, Welcome, events.
    fn attach(
        &mut self,
        ctx: &mut Ctx,
        ti: usize,
        conn: ConnId,
        id: ClientId,
        client_nonce: &Nonce,
        reconnect: bool,
    ) {
        let (Some(server_nonce), Some(token)) = (crypto::random::<16>(), crypto::random::<16>())
        else {
            self.reject(
                ctx,
                ti,
                conn,
                RejectCode::Busy,
                "the server could not get random bytes",
            );
            self.clients.remove(&id);
            return;
        };
        let secret = self.transports[ti].session_secret(conn).unwrap_or_default();
        let key = SessionKey::derive(&secret, client_nonce, &server_nonce);
        let welcome = Welcome {
            client_id: id.0,
            server_nonce,
            tick_rate: ctx.cfg.tick_rate,
            server_tick: self.tick,
            reconnect_token: token,
            proof: key.proof(client_nonce),
        };
        let Ok(bytes) = wire::encode(&welcome) else {
            return;
        };
        self.transports[ti].send(conn, Channel::Reliable, &wire::frame(kind::WELCOME, &bytes));
        let Some(rec) = self.clients.get_mut(&id) else {
            return;
        };
        rec.secure = Some(Secure::new(key, Direction::ServerToClient));
        rec.peer = Some((ti, conn));
        rec.last_heard = ctx.now;
        rec.lost_at = None;
        rec.reconnect_token = token;
        if let Some(peer) = self.peers.get_mut(&(ti, conn)) {
            peer.state = PeerState::Active(id);
        }
        let name = rec.name.clone();
        ctx.events.push(if reconnect {
            NetEvent::ClientReconnected { id }
        } else {
            NetEvent::ClientJoined { id, name }
        });
        log::info!(
            "net: client {} {}",
            id.0,
            if reconnect { "reconnected" } else { "joined" }
        );
    }

    fn reject(&mut self, ctx: &mut Ctx, ti: usize, conn: ConnId, code: RejectCode, message: &str) {
        log::info!("net: refusing {conn:?}: {message}");
        let reject = Reject {
            code,
            message: clip(message),
        };
        if let Ok(bytes) = wire::encode(&reject) {
            self.transports[ti].send(conn, Channel::Reliable, &wire::frame(kind::REJECT, &bytes));
        }
        if let Some(peer) = self.peers.get_mut(&(ti, conn)) {
            peer.state = PeerState::Closing(ctx.now + CLOSE_DELAY);
        }
    }

    fn remember_nonce(&mut self, nonce: Nonce) -> bool {
        if !self.nonces.insert(nonce) {
            return false;
        }
        self.nonce_order.push_back(nonce);
        while self.nonce_order.len() > MAX_NONCES {
            if let Some(old) = self.nonce_order.pop_front() {
                self.nonces.remove(&old);
            }
        }
        true
    }

    fn close_peer(&mut self, (ti, conn): (usize, ConnId), now: f64) {
        if let Some(peer) = self.peers.get_mut(&(ti, conn)) {
            peer.state = PeerState::Closing(now + CLOSE_DELAY);
        }
    }

    /// The connection of an active client died (not a polite leave).
    fn connection_lost(&mut self, ctx: &mut Ctx, id: ClientId, reason: &str) {
        let Some(rec) = self.clients.get_mut(&id) else {
            return;
        };
        rec.peer = None;
        rec.lost_at = Some(ctx.now);
        let reason = clip(reason);
        ctx.events.push(NetEvent::ClientDisconnected {
            id,
            reason: reason.clone(),
        });
        if ctx.cfg.reconnect_grace <= 0.0 {
            self.clients.remove(&id);
            ctx.events.push(NetEvent::ClientLeft { id, reason });
        }
    }

    /// Removes a client for good (kick or polite leave); `lost` is the reason shown.
    pub fn remove_client(&mut self, ctx: &mut Ctx, id: ClientId, reason: &str, polite: bool) {
        let Some(rec) = self.clients.remove(&id) else {
            return;
        };
        if let Some((ti, conn)) = rec.peer {
            if !polite {
                // Reason first, then close once it has had time to arrive.
                let mut rec = rec;
                if let Some(secure) = rec.secure.as_mut()
                    && let Some(bytes) = secure.seal(
                        Channel::Reliable,
                        &Msg::Disconnect {
                            reason: reason.to_owned(),
                        },
                    )
                {
                    self.transports[ti].send(conn, Channel::Reliable, &bytes);
                }
            }
            if let Some(peer) = self.peers.get_mut(&(ti, conn)) {
                peer.state = PeerState::Closing(ctx.now + CLOSE_DELAY);
            }
        }
        ctx.events.push(NetEvent::ClientLeft {
            id,
            reason: reason.to_owned(),
        });
    }

    fn maintain(&mut self, ctx: &mut Ctx) {
        let now = ctx.now;
        let mut to_close = Vec::new();
        for (key, peer) in &self.peers {
            match peer.state {
                PeerState::Pending if now - peer.since > ctx.cfg.handshake_timeout as f64 => {
                    to_close.push((*key, "handshake timed out"))
                }
                PeerState::Closing(at) if now >= at => to_close.push((*key, "closed")),
                _ => {}
            }
        }
        for ((ti, conn), why) in to_close {
            self.peers.remove(&(ti, conn));
            self.transports[ti].close(conn, why);
        }
        let timeout = ctx.cfg.timeout as f64;
        let silent: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.peer.is_some() && now - c.last_heard > timeout)
            .map(|c| c.id)
            .collect();
        for id in silent {
            if let Some((ti, conn)) = self.clients.get(&id).and_then(|c| c.peer) {
                self.peers.remove(&(ti, conn));
                self.transports[ti].close(conn, "timed out");
            }
            self.connection_lost(ctx, id, "timed out");
        }
        let grace = ctx.cfg.reconnect_grace as f64;
        let expired: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.lost_at.is_some_and(|t| now - t > grace))
            .map(|c| c.id)
            .collect();
        for id in expired {
            self.clients.remove(&id);
            ctx.events.push(NetEvent::ClientLeft {
                id,
                reason: "timed out".into(),
            });
        }
    }

    /// Seals and sends `msg` to a connected client; false if it is not connected.
    pub fn send_to(&mut self, id: ClientId, channel: Channel, msg: &Msg) -> bool {
        let Some(rec) = self.clients.get_mut(&id) else {
            return false;
        };
        let (Some((ti, conn)), Some(secure)) = (rec.peer, rec.secure.as_mut()) else {
            return false;
        };
        let Some(bytes) = secure.seal(channel, msg) else {
            return false;
        };
        self.transports[ti].send(conn, channel, &bytes);
        true
    }
}

/// Display text from the network: control characters removed, bounded.
pub(crate) fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// A reason string from a peer, bounded so it cannot flood the log.
pub(crate) fn clip(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(200)
        .collect()
}
