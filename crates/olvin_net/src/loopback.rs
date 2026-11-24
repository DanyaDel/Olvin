//! In-process transport with simulated network conditions. The backbone of the
//! tests: a server and any number of clients, each in its own `App`, talk
//! through one [`Loopback`] hub whose clock the test advances by hand, so a
//! run is fast and repeatable. It also works across threads (the hub is
//! `Send + Sync`), and in real time for tools.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use crate::sim::{LinkSim, NetConditions};
use crate::transport::{Channel, ConnId, Transport, TransportEvent};

#[derive(Clone)]
enum Packet {
    Connected,
    Data(Channel, Vec<u8>),
    Closed(String),
}

struct Conn {
    to_server: LinkSim<Packet>,
    to_client: LinkSim<Packet>,
    /// Server side saw `Closed` or closed it; sends are ignored.
    server_open: bool,
    client_open: bool,
}

enum Clock {
    Manual(f64),
    Real(Instant),
}

struct Hub {
    clock: Clock,
    conditions: NetConditions,
    seed: u64,
    listening: bool,
    next_conn: u64,
    conns: HashMap<u64, Conn>,
    max_datagram: usize,
}

impl Hub {
    fn now(&self) -> f64 {
        match self.clock {
            Clock::Manual(t) => t,
            Clock::Real(start) => start.elapsed().as_secs_f64(),
        }
    }
}

/// The hub. Clone it freely: every clone is the same network.
#[derive(Clone)]
pub struct Loopback {
    hub: Arc<Mutex<Hub>>,
}

impl Default for Loopback {
    fn default() -> Self {
        Self::new()
    }
}

impl Loopback {
    /// A hub with a manual clock (see [`Loopback::advance`]) and a perfect link.
    pub fn new() -> Self {
        Self::build(Clock::Manual(0.0), NetConditions::NONE)
    }

    /// A hub that follows the wall clock, for tools and demos.
    pub fn realtime(conditions: NetConditions) -> Self {
        Self::build(Clock::Real(Instant::now()), conditions)
    }

    /// A manual-clock hub whose links start with `conditions`.
    pub fn with_conditions(conditions: NetConditions) -> Self {
        Self::build(Clock::Manual(0.0), conditions)
    }

    fn build(clock: Clock, conditions: NetConditions) -> Self {
        Self {
            hub: Arc::new(Mutex::new(Hub {
                clock,
                conditions,
                seed: 1,
                listening: false,
                next_conn: 1,
                conns: HashMap::new(),
                max_datagram: 1200,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Hub> {
        self.hub.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Seed of the loss and jitter dice for links created from now on.
    pub fn seed(&self, seed: u64) -> &Self {
        self.lock().seed = seed;
        self
    }

    /// Advances the manual clock. No effect on a real-time hub.
    pub fn advance(&self, dt: f64) {
        if let Clock::Manual(t) = &mut self.lock().clock {
            *t += dt.max(0.0);
        }
    }

    pub fn now(&self) -> f64 {
        self.lock().now()
    }

    /// Changes the conditions of every link, existing and future.
    pub fn set_conditions(&self, conditions: NetConditions) {
        let mut hub = self.lock();
        hub.conditions = conditions;
        for conn in hub.conns.values_mut() {
            conn.to_server.conditions = conditions;
            conn.to_client.conditions = conditions;
        }
    }

    /// Largest unreliable packet the hub carries (default 1200, like QUIC datagrams).
    pub fn set_max_datagram(&self, n: usize) {
        self.lock().max_datagram = n;
    }

    /// The server end. One per hub; a second call takes the place of the first.
    pub fn listen(&self) -> LoopbackServer {
        self.lock().listening = true;
        LoopbackServer { net: self.clone() }
    }

    /// A client end, connecting to the listening server (if there is none the
    /// connection is refused). The `Connected` event arrives after one latency.
    pub fn connect(&self) -> LoopbackClient {
        let mut hub = self.lock();
        let id = hub.next_conn;
        hub.next_conn += 1;
        let seed = hub.seed.wrapping_add(id.wrapping_mul(7919));
        let now = hub.now();
        let c = hub.conditions;
        let mut conn = Conn {
            to_server: LinkSim::new(c, seed),
            to_client: LinkSim::new(c, seed ^ 0xABCD),
            server_open: true,
            client_open: true,
        };
        if hub.listening {
            conn.to_server.push(now, true, Packet::Connected);
            conn.to_client.push(now, true, Packet::Connected);
        } else {
            conn.to_client.push(
                now,
                true,
                Packet::Closed("connection refused: nobody is listening".into()),
            );
            conn.server_open = false;
        }
        hub.conns.insert(id, conn);
        LoopbackClient {
            net: self.clone(),
            id,
        }
    }

    /// Conditions for one client's link only (e.g. to cut one player's cable).
    pub fn set_client_conditions(&self, client: &LoopbackClient, conditions: NetConditions) {
        if let Some(conn) = self.lock().conns.get_mut(&client.id) {
            conn.to_server.conditions = conditions;
            conn.to_client.conditions = conditions;
        }
    }
}

/// The listening end of a [`Loopback`].
pub struct LoopbackServer {
    net: Loopback,
}

impl Transport for LoopbackServer {
    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        let mut hub = self.net.lock();
        let now = hub.now();
        let mut ids: Vec<u64> = hub.conns.keys().copied().collect();
        ids.sort_unstable();
        let mut gone = Vec::new();
        for id in ids {
            let Some(conn) = hub.conns.get_mut(&id) else {
                continue;
            };
            let mut due = Vec::new();
            conn.to_server.drain(now, &mut due);
            for p in due {
                let conn_id = ConnId(id);
                match p {
                    Packet::Connected => events.push(TransportEvent::Connected { conn: conn_id }),
                    Packet::Data(channel, data) => events.push(TransportEvent::Data {
                        conn: conn_id,
                        channel,
                        data,
                    }),
                    Packet::Closed(reason) => {
                        conn.server_open = false;
                        events.push(TransportEvent::Disconnected {
                            conn: conn_id,
                            reason,
                        });
                    }
                }
            }
            if !conn.server_open && !conn.client_open {
                gone.push(id);
            }
        }
        for id in gone {
            hub.conns.remove(&id);
        }
    }

    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        let mut hub = self.net.lock();
        let now = hub.now();
        let max = hub.max_datagram;
        if let Some(c) = hub.conns.get_mut(&conn.0)
            && c.server_open
            && (channel == Channel::Reliable || data.len() <= max)
        {
            c.to_client.push(
                now,
                channel == Channel::Reliable,
                Packet::Data(channel, data.to_vec()),
            );
        }
    }

    fn close(&mut self, conn: ConnId, reason: &str) {
        let mut hub = self.net.lock();
        let now = hub.now();
        if let Some(c) = hub.conns.get_mut(&conn.0)
            && c.server_open
        {
            c.server_open = false;
            c.to_client.push(now, true, Packet::Closed(reason.into()));
        }
    }

    fn max_datagram(&self) -> usize {
        self.net.lock().max_datagram
    }

    fn peer_addr(&self, conn: ConnId) -> Option<String> {
        Some(format!("loopback:{}", conn.0))
    }

    fn name(&self) -> &'static str {
        "loopback-server"
    }
}

/// A client end of a [`Loopback`]. Dropping it disconnects politely.
pub struct LoopbackClient {
    net: Loopback,
    id: u64,
}

impl Transport for LoopbackClient {
    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        let mut hub = self.net.lock();
        let now = hub.now();
        let Some(conn) = hub.conns.get_mut(&self.id) else {
            return;
        };
        let mut due = Vec::new();
        conn.to_client.drain(now, &mut due);
        for p in due {
            let conn_id = ConnId(0);
            match p {
                Packet::Connected => events.push(TransportEvent::Connected { conn: conn_id }),
                Packet::Data(channel, data) => events.push(TransportEvent::Data {
                    conn: conn_id,
                    channel,
                    data,
                }),
                Packet::Closed(reason) => {
                    conn.client_open = false;
                    events.push(TransportEvent::Disconnected {
                        conn: conn_id,
                        reason,
                    });
                }
            }
        }
    }

    fn send(&mut self, _conn: ConnId, channel: Channel, data: &[u8]) {
        let mut hub = self.net.lock();
        let now = hub.now();
        let max = hub.max_datagram;
        if let Some(c) = hub.conns.get_mut(&self.id)
            && c.client_open
            && (channel == Channel::Reliable || data.len() <= max)
        {
            c.to_server.push(
                now,
                channel == Channel::Reliable,
                Packet::Data(channel, data.to_vec()),
            );
        }
    }

    fn close(&mut self, _conn: ConnId, reason: &str) {
        let mut hub = self.net.lock();
        let now = hub.now();
        if let Some(c) = hub.conns.get_mut(&self.id)
            && c.client_open
        {
            c.client_open = false;
            c.to_server.push(now, true, Packet::Closed(reason.into()));
        }
    }

    fn max_datagram(&self) -> usize {
        self.net.lock().max_datagram
    }

    fn peer_addr(&self, _conn: ConnId) -> Option<String> {
        Some("loopback:server".into())
    }

    fn name(&self) -> &'static str {
        "loopback-client"
    }
}

impl Drop for LoopbackClient {
    fn drop(&mut self) {
        self.close(ConnId(0), "client dropped");
    }
}

/// Puts [`NetConditions`] on any transport (QUIC on 127.0.0.1 included): the
/// "lag 80 ms, loss 2%" switch for Play. Both directions are delayed, so wrap
/// one end only and the round trip sees the conditions twice.
pub struct Conditioned<T: Transport> {
    inner: T,
    start: Instant,
    incoming: LinkSim<TransportEvent>,
    outgoing: LinkSim<(ConnId, Channel, Vec<u8>)>,
}

impl<T: Transport> Conditioned<T> {
    pub fn new(inner: T, conditions: NetConditions) -> Self {
        Self {
            inner,
            start: Instant::now(),
            incoming: LinkSim::new(conditions, 11),
            outgoing: LinkSim::new(conditions, 13),
        }
    }

    pub fn set_conditions(&mut self, conditions: NetConditions) {
        self.incoming.conditions = conditions;
        self.outgoing.conditions = conditions;
    }

    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

impl<T: Transport> Transport for Conditioned<T> {
    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        let now = self.now();
        let mut due = Vec::new();
        self.outgoing.drain(now, &mut due);
        for (conn, channel, data) in due {
            self.inner.send(conn, channel, &data);
        }
        let mut fresh = Vec::new();
        self.inner.poll(&mut fresh);
        for ev in fresh {
            let reliable = !matches!(
                ev,
                TransportEvent::Data {
                    channel: Channel::Unreliable,
                    ..
                }
            );
            self.incoming.push(now, reliable, ev);
        }
        self.incoming.drain(now, events);
    }

    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        let now = self.now();
        self.outgoing.push(
            now,
            channel == Channel::Reliable,
            (conn, channel, data.to_vec()),
        );
    }

    fn close(&mut self, conn: ConnId, reason: &str) {
        self.inner.close(conn, reason);
    }

    fn max_datagram(&self) -> usize {
        self.inner.max_datagram()
    }

    fn peer_addr(&self, conn: ConnId) -> Option<String> {
        self.inner.peer_addr(conn)
    }

    fn session_secret(&self, conn: ConnId) -> Option<Vec<u8>> {
        self.inner.session_secret(conn)
    }

    fn room_info(&self) -> Option<(std::net::SocketAddr, [u8; 32])> {
        self.inner.room_info()
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(t: &mut impl Transport) -> Vec<TransportEvent> {
        let mut v = Vec::new();
        t.poll(&mut v);
        v
    }

    #[test]
    fn connect_send_close() {
        let net = Loopback::new();
        let mut server = net.listen();
        let mut client = net.connect();
        assert_eq!(
            poll(&mut client),
            [TransportEvent::Connected { conn: ConnId(0) }]
        );
        let ev = poll(&mut server);
        let [TransportEvent::Connected { conn }] = ev[..] else {
            panic!("{ev:?}");
        };
        client.send(ConnId(0), Channel::Reliable, b"hello");
        server.send(conn, Channel::Unreliable, b"yo");
        assert_eq!(
            poll(&mut server),
            [TransportEvent::Data {
                conn,
                channel: Channel::Reliable,
                data: b"hello".to_vec()
            }]
        );
        assert_eq!(poll(&mut client).len(), 1);
        server.close(conn, "bye");
        assert_eq!(
            poll(&mut client),
            [TransportEvent::Disconnected {
                conn: ConnId(0),
                reason: "bye".into()
            }]
        );
    }

    #[test]
    fn refused_without_listener_and_latency_is_virtual() {
        let net = Loopback::with_conditions(NetConditions::lag_ms(50.0));
        let mut client = net.connect();
        assert!(poll(&mut client).is_empty());
        net.advance(0.06);
        let ev = poll(&mut client);
        assert!(
            matches!(&ev[..], [TransportEvent::Disconnected { reason, .. }] if reason.contains("refused"))
        );
    }

    #[test]
    fn oversized_datagram_is_dropped_but_reliable_passes() {
        let net = Loopback::new();
        let mut server = net.listen();
        let mut client = net.connect();
        poll(&mut client);
        poll(&mut server);
        client.send(ConnId(0), Channel::Unreliable, &[0; 5000]);
        client.send(ConnId(0), Channel::Reliable, &[0; 5000]);
        assert_eq!(poll(&mut server).len(), 1);
    }

    #[test]
    fn conditioned_wraps_any_transport() {
        let net = Loopback::new();
        let mut server = net.listen();
        let client = net.connect();
        let mut client = Conditioned::new(client, NetConditions::lag_ms(30.0));
        client.send(ConnId(0), Channel::Reliable, b"x");
        assert!(poll(&mut server).len() <= 1);
        std::thread::sleep(std::time::Duration::from_millis(80));
        poll(&mut client);
        let ev = poll(&mut server);
        assert!(ev.iter().any(|e| matches!(e, TransportEvent::Data { .. })));
    }
}
