//! Client half of the session layer: connects, runs the handshake, pings,
//! notices a dead server.

use crate::config::NetConfig;
use crate::crypto::{self, Direction, Nonce, SessionKey};
use crate::net::{ClientId, NetEvent};
use crate::proto::{Hello, Msg, ReconnectRequest, Reject, Welcome};
use crate::secure::{OpenError, Secure};
use crate::server::clip;
use crate::transport::{Channel, ConnId, Transport, TransportEvent};
use crate::wire::{self, PROTOCOL_VERSION, kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the transport to connect.
    Connecting,
    /// `Hello` sent, waiting for `Welcome`.
    Handshaking,
    Connected,
    /// Over; the reason was reported as an event.
    Closed,
}

pub(crate) struct ClientState {
    pub transport: Box<dyn Transport>,
    pub phase: Phase,
    since: f64,
    nonce: Nonce,
    secure: Option<Secure>,
    pub id: Option<ClientId>,
    last_heard: f64,
    next_ping: f64,
    ping_id: u32,
    /// Smoothed round-trip time, seconds.
    pub rtt: f32,
    pub reconnect: Option<(u32, [u8; 16])>,
    pub server_tick: u32,
}

const CONN: ConnId = ConnId(0);

impl ClientState {
    pub fn new(
        transport: Box<dyn Transport>,
        now: f64,
        reconnect: Option<(u32, [u8; 16])>,
    ) -> Self {
        Self {
            transport,
            phase: Phase::Connecting,
            since: now,
            nonce: crypto::random::<16>().unwrap_or([0; 16]),
            secure: None,
            id: None,
            last_heard: now,
            next_ping: now,
            ping_id: 0,
            rtt: 0.0,
            reconnect,
            server_tick: 0,
        }
    }

    pub fn poll(
        &mut self,
        cfg: &NetConfig,
        now: f64,
        registry_hash: u64,
        events: &mut Vec<NetEvent>,
    ) {
        let mut raw = Vec::new();
        self.transport.poll(&mut raw);
        for ev in raw {
            match ev {
                TransportEvent::Connected { .. } => {
                    if self.phase == Phase::Connecting {
                        self.send_hello(cfg, now, registry_hash);
                    }
                }
                TransportEvent::Disconnected { reason, .. } => {
                    self.fail(events, &clip(&reason));
                }
                TransportEvent::Data { channel, data, .. } => {
                    self.on_data(cfg, now, channel, data, events);
                }
            }
        }
        self.maintain(cfg, now, events);
    }

    fn fail(&mut self, events: &mut Vec<NetEvent>, reason: &str) {
        match self.phase {
            Phase::Closed => {}
            Phase::Connected => {
                self.phase = Phase::Closed;
                events.push(NetEvent::Disconnected {
                    reason: reason.to_owned(),
                });
            }
            _ => {
                self.phase = Phase::Closed;
                events.push(NetEvent::ConnectFailed {
                    reason: reason.to_owned(),
                });
            }
        }
    }

    fn send_hello(&mut self, cfg: &NetConfig, now: f64, registry_hash: u64) {
        let hello = Hello {
            game_id: cfg.game_id.clone(),
            game_version: cfg.game_version,
            registry_hash,
            name: cfg
                .player_name
                .chars()
                .take(cfg.limits.max_name_len)
                .collect(),
            token: cfg.token.clone(),
            nonce: self.nonce,
            unix_ms: (cfg.clock)(),
            reconnect: self
                .reconnect
                .map(|(client_id, token)| ReconnectRequest { client_id, token }),
        };
        let Ok(body) = wire::encode(&hello) else {
            return;
        };
        let mut packet = PROTOCOL_VERSION.to_le_bytes().to_vec();
        packet.extend_from_slice(&body);
        self.transport
            .send(CONN, Channel::Reliable, &wire::frame(kind::HELLO, &packet));
        self.phase = Phase::Handshaking;
        self.since = now;
    }

    fn on_data(
        &mut self,
        cfg: &NetConfig,
        now: f64,
        channel: Channel,
        data: Vec<u8>,
        events: &mut Vec<NetEvent>,
    ) {
        let max = match channel {
            Channel::Reliable => cfg.limits.max_reliable_packet,
            Channel::Unreliable => self.transport.max_datagram().max(64),
        };
        if data.len() > max {
            return;
        }
        let Ok((k, body)) = wire::unframe(&data) else {
            return;
        };
        match (self.phase, k) {
            (Phase::Handshaking, kind::WELCOME) => self.on_welcome(cfg, now, body, events),
            (Phase::Handshaking | Phase::Connecting, kind::REJECT) => {
                let reason = wire::decode::<Reject>(body, 1024)
                    .map(|r| r.message)
                    .unwrap_or_else(|_| "refused by the server".into());
                self.fail(events, &clip(&reason));
                self.transport.close(CONN, "refused");
            }
            (Phase::Connected, kind::DATA) => {
                let Some(secure) = self.secure.as_mut() else {
                    return;
                };
                let msg = match secure.open(channel, body, cfg.limits.max_reliable_packet) {
                    Ok(m) => m,
                    Err(OpenError::Replay) => return,
                    Err(e) => {
                        log::debug!("net: dropped a bad packet from the server: {e:?}");
                        return;
                    }
                };
                self.last_heard = now;
                match msg {
                    Msg::Pong {
                        sent, server_tick, ..
                    } => {
                        let sample = (now as f32 - sent).max(0.0);
                        self.rtt = if self.rtt == 0.0 {
                            sample
                        } else {
                            self.rtt * 0.8 + sample * 0.2
                        };
                        self.server_tick = server_tick;
                    }
                    Msg::Ping { .. } => {}
                    Msg::Disconnect { reason } => {
                        self.fail(events, &clip(&reason));
                        self.transport.close(CONN, "kicked");
                    }
                }
            }
            _ => {}
        }
    }

    fn on_welcome(&mut self, cfg: &NetConfig, now: f64, body: &[u8], events: &mut Vec<NetEvent>) {
        let welcome: Welcome = match wire::decode(body, 1024) {
            Ok(w) => w,
            Err(e) => {
                self.fail(events, &format!("bad Welcome from the server: {e}"));
                self.transport.close(CONN, "bad welcome");
                return;
            }
        };
        let secret = self.transport.session_secret(CONN).unwrap_or_default();
        let key = SessionKey::derive(&secret, &self.nonce, &welcome.server_nonce);
        if !crypto::ct_eq(&key.proof(&self.nonce), &welcome.proof) {
            self.fail(
                events,
                "the server failed to prove the session key (wrong server?)",
            );
            self.transport.close(CONN, "bad proof");
            return;
        }
        self.secure = Some(Secure::new(key, Direction::ClientToServer));
        self.phase = Phase::Connected;
        self.last_heard = now;
        self.next_ping = now;
        self.server_tick = welcome.server_tick;
        let id = ClientId(welcome.client_id);
        self.id = Some(id);
        self.reconnect = Some((welcome.client_id, welcome.reconnect_token));
        let _ = cfg;
        events.push(NetEvent::Connected { id });
    }

    fn maintain(&mut self, cfg: &NetConfig, now: f64, events: &mut Vec<NetEvent>) {
        match self.phase {
            Phase::Connecting | Phase::Handshaking => {
                if now - self.since > cfg.handshake_timeout as f64 {
                    self.fail(
                        events,
                        "could not connect: the server did not answer in time",
                    );
                    self.transport.close(CONN, "connect timeout");
                }
            }
            Phase::Connected => {
                if now - self.last_heard > cfg.timeout as f64 {
                    self.fail(events, "timed out: the server stopped answering");
                    self.transport.close(CONN, "timed out");
                } else if now >= self.next_ping {
                    self.next_ping = now + cfg.heartbeat as f64;
                    self.ping_id = self.ping_id.wrapping_add(1);
                    let ping = Msg::Ping {
                        id: self.ping_id,
                        sent: now as f32,
                    };
                    self.send(Channel::Unreliable, &ping);
                }
            }
            Phase::Closed => {}
        }
    }

    pub fn send(&mut self, channel: Channel, msg: &Msg) -> bool {
        if self.phase != Phase::Connected {
            return false;
        }
        let Some(bytes) = self.secure.as_mut().and_then(|s| s.seal(channel, msg)) else {
            return false;
        };
        self.transport.send(CONN, channel, &bytes);
        true
    }

    /// Leaves politely.
    pub fn leave(&mut self, reason: &str) {
        if self.phase == Phase::Connected {
            self.send(
                Channel::Reliable,
                &Msg::Disconnect {
                    reason: reason.to_owned(),
                },
            );
        }
        self.transport.close(CONN, reason);
        self.phase = Phase::Closed;
    }
}
