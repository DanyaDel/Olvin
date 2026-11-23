//! L0: the transport trait. A transport moves opaque byte packets over two
//! channels and reports connections coming and going. It knows nothing about
//! sessions, replication or the game: everything above it is the same for the
//! in-process [`Loopback`](crate::Loopback) and for QUIC.

/// A connection as the transport numbers it. On a server transport every
/// client gets its own; a client transport has exactly one, `ConnId(0)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Delivered, in order, exactly once (a stream). A lost packet delays the ones behind it.
    Reliable,
    /// May be lost, duplicated or reordered (a datagram). Size is limited by
    /// [`Transport::max_datagram`].
    Unreliable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportEvent {
    /// A connection is up (a client transport: the server accepted us).
    Connected { conn: ConnId },
    /// The connection is gone (closed by the peer, timed out, refused).
    Disconnected { conn: ConnId, reason: String },
    Data {
        conn: ConnId,
        channel: Channel,
        data: Vec<u8>,
    },
}

/// Moves packets. Non-blocking: the game polls it once per frame.
pub trait Transport: 'static {
    /// Appends everything that happened since the last call, in order.
    fn poll(&mut self, events: &mut Vec<TransportEvent>);

    /// Queues one packet. A send to a dead or unknown connection is ignored
    /// (the `Disconnected` event is the one source of truth).
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]);

    /// Closes the connection with a reason the other side can show.
    fn close(&mut self, conn: ConnId, reason: &str);

    /// Largest packet the unreliable channel carries. Bigger ones must go reliable.
    fn max_datagram(&self) -> usize {
        1200
    }

    /// Peer address for logs and authenticators, if the transport has one.
    fn peer_addr(&self, _conn: ConnId) -> Option<String> {
        None
    }

    /// Secret bound to this connection that never travels on the wire (the TLS
    /// exporter on QUIC). It keys the session MAC; a transport without one
    /// returns `None` and the MAC then only guards against bugs and replays.
    fn session_secret(&self, _conn: ConnId) -> Option<Vec<u8>> {
        None
    }

    /// Short name for logs.
    fn name(&self) -> &'static str;
}

impl Transport for Box<dyn Transport> {
    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        (**self).poll(events)
    }
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        (**self).send(conn, channel, data)
    }
    fn close(&mut self, conn: ConnId, reason: &str) {
        (**self).close(conn, reason)
    }
    fn max_datagram(&self) -> usize {
        (**self).max_datagram()
    }
    fn peer_addr(&self, conn: ConnId) -> Option<String> {
        (**self).peer_addr(conn)
    }
    fn session_secret(&self, conn: ConnId) -> Option<Vec<u8>> {
        (**self).session_secret(conn)
    }
    fn name(&self) -> &'static str {
        (**self).name()
    }
}
