//! QUIC transport (feature `quic`): TLS 1.3 from `rustls`, a reliable ordered
//! stream and unreliable datagrams per connection, on `quinn`.
//!
//! The server has a self-signed certificate ([`QuicIdentity`]). Nobody signs it,
//! so clients **pin** it: the room code carries the first 16 bytes of its SHA-256
//! and a client accepts only a certificate that hashes to them (and that proves it
//! holds the key, through the normal TLS signature check). A client that joins
//! a bare `host:port` has nothing to pin and refuses, unless
//! `NetConfig::allow_unpinned` says otherwise.
//!
//! The private key lives in [`QuicIdentity`], which only a server creates. The
//! async runtime runs on a private thread; the game sees a non-blocking `poll`.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring as ring_provider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::crypto::sha256;
use crate::room::{JoinTarget, PIN_LEN, RoomCode};
use crate::transport::{Channel, ConnId, Transport, TransportEvent};

const ALPN: &[u8] = b"olvin/1";
/// Largest datagram payload we send; a sealed packet must fit.
const MAX_DATAGRAM: usize = 1000;
/// Largest reliable frame a peer may announce.
const MAX_FRAME: usize = 1 << 20;
const EXPORTER_LABEL: &[u8] = b"EXPORTER-olvin-net-session";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring_provider::default_provider())
}

/// A server's certificate and private key.
pub struct QuicIdentity {
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
}

impl QuicIdentity {
    /// A fresh self-signed certificate (valid for any name: clients pin the hash).
    pub fn generate() -> Result<Self, String> {
        let key = rcgen::generate_simple_self_signed(vec!["olvin".to_string()])
            .map_err(|e| format!("cannot make a certificate: {e}"))?;
        Ok(Self {
            cert_der: key.cert.der().to_vec(),
            key_der: key.signing_key.serialize_der(),
        })
    }

    /// From DER bytes you saved earlier (keep the key file private).
    pub fn from_der(cert_der: Vec<u8>, key_der: Vec<u8>) -> Self {
        Self { cert_der, key_der }
    }

    pub fn cert_der(&self) -> &[u8] {
        &self.cert_der
    }

    /// SHA-256 of the certificate, what a room code pins.
    pub fn fingerprint(&self) -> [u8; 32] {
        sha256(&self.cert_der)
    }
}

enum Cmd {
    Send(ConnId, Channel, Vec<u8>),
    Close(ConnId, String),
}

type Secrets = Arc<Mutex<HashMap<ConnId, Vec<u8>>>>;
type Addrs = Arc<Mutex<HashMap<ConnId, String>>>;

struct Shared {
    events: Receiver<TransportEvent>,
    cmds: UnboundedSender<Cmd>,
    secrets: Secrets,
    addrs: Addrs,
    _thread: std::thread::JoinHandle<()>,
}

impl Shared {
    fn poll(&mut self, out: &mut Vec<TransportEvent>) {
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs one connection: the reliable stream, the datagrams, and its close.
async fn run_conn(
    id: ConnId,
    conn: quinn::Connection,
    opens_stream: bool,
    mut out: UnboundedReceiver<(Channel, Vec<u8>)>,
    events: Sender<TransportEvent>,
) {
    let streams = if opens_stream {
        conn.open_bi().await
    } else {
        conn.accept_bi().await
    };
    let Ok((mut send, mut recv)) = streams else {
        let _ = events.send(TransportEvent::Disconnected {
            conn: id,
            reason: close_reason(conn.close_reason()),
        });
        return;
    };
    // Frames in: u32 LE length then bytes, read by their own task so a slow
    // write never stalls reading.
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let reader = tokio::spawn(async move {
        loop {
            let mut len = [0u8; 4];
            if recv.read_exact(&mut len).await.is_err() {
                break;
            }
            let len = u32::from_le_bytes(len) as usize;
            if len > MAX_FRAME {
                break;
            }
            let mut buf = vec![0u8; len];
            if recv.read_exact(&mut buf).await.is_err() || frame_tx.send(buf).is_err() {
                break;
            }
        }
    });
    let reason = loop {
        tokio::select! {
            frame = frame_rx.recv() => match frame {
                Some(data) => {
                    let _ = events.send(TransportEvent::Data { conn: id, channel: Channel::Reliable, data });
                }
                None => break close_reason(conn.close_reason()),
            },
            datagram = conn.read_datagram() => match datagram {
                Ok(d) => {
                    let _ = events.send(TransportEvent::Data { conn: id, channel: Channel::Unreliable, data: d.to_vec() });
                }
                Err(e) => break close_reason(Some(e)),
            },
            msg = out.recv() => match msg {
                Some((Channel::Reliable, data)) => {
                    let mut framed = (data.len() as u32).to_le_bytes().to_vec();
                    framed.extend_from_slice(&data);
                    if send.write_all(&framed).await.is_err() {
                        break close_reason(conn.close_reason());
                    }
                }
                Some((Channel::Unreliable, data)) => {
                    let _ = conn.send_datagram(data.into());
                }
                None => break "closed".to_owned(),
            },
        }
    };
    reader.abort();
    let _ = events.send(TransportEvent::Disconnected { conn: id, reason });
}

fn close_reason(e: Option<quinn::ConnectionError>) -> String {
    match e {
        Some(quinn::ConnectionError::ApplicationClosed(c)) => {
            let r = String::from_utf8_lossy(&c.reason).into_owned();
            if r.is_empty() {
                "closed by the peer".into()
            } else {
                r
            }
        }
        Some(quinn::ConnectionError::TimedOut) => "timed out".into(),
        Some(quinn::ConnectionError::Reset) => "connection reset".into(),
        Some(e) => e.to_string(),
        None => "connection closed".into(),
    }
}

/// Keeps `Cmd`s for live connections and feeds them to their tasks.
async fn dispatch(mut cmds: UnboundedReceiver<Cmd>, conns: ConnTable) {
    while let Some(cmd) = cmds.recv().await {
        let table = lock(&conns);
        match cmd {
            Cmd::Send(id, channel, data) => {
                if let Some((_, tx)) = table.get(&id) {
                    let _ = tx.send((channel, data));
                }
            }
            Cmd::Close(id, reason) => {
                if let Some((c, _)) = table.get(&id) {
                    c.close(quinn::VarInt::from_u32(1), reason.as_bytes());
                }
            }
        }
    }
}

type ConnTable =
    Arc<Mutex<HashMap<ConnId, (quinn::Connection, UnboundedSender<(Channel, Vec<u8>)>)>>>;

fn spawn_runtime<F, Fut>(f: F) -> Result<std::thread::JoinHandle<()>, String>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name("olvin-quic".into())
        .spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(f()),
                Err(e) => log::error!("net: cannot start the QUIC runtime: {e}"),
            }
        })
        .map_err(|e| format!("cannot start the QUIC thread: {e}"))
}

/// A listening QUIC server.
pub struct QuicServer {
    shared: Shared,
    local_addr: SocketAddr,
    fingerprint: [u8; 32],
}

impl QuicServer {
    /// Listens on `addr` (`0.0.0.0:7777`; port 0 picks a free one).
    pub fn listen(addr: SocketAddr, identity: &QuicIdentity) -> Result<Self, String> {
        let key = PrivateKeyDer::try_from(identity.key_der.clone())
            .map_err(|e| format!("bad private key: {e}"))?;
        let mut tls = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(identity.cert_der.clone())], key)
            .map_err(|e| format!("bad certificate: {e}"))?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let quic = QuicServerConfig::try_from(tls).map_err(|e| e.to_string())?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
        let mut transport = quinn::TransportConfig::default();
        transport.datagram_receive_buffer_size(Some(1 << 20));
        transport.max_concurrent_bidi_streams(2u8.into());
        transport.max_concurrent_uni_streams(0u8.into());
        config.transport_config(Arc::new(transport));
        // Bind here so a bad address is an error for the caller, not a log line on a thread.
        let socket =
            std::net::UdpSocket::bind(addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
        let local_addr = socket.local_addr().map_err(|e| e.to_string())?;
        let (ev_tx, ev_rx) = channel();
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let secrets: Secrets = Arc::default();
        let addrs: Addrs = Arc::default();
        let (s2, a2) = (secrets.clone(), addrs.clone());
        let thread = spawn_runtime(move || async move {
            let endpoint = match quinn::Endpoint::new(
                quinn::EndpointConfig::default(),
                Some(config),
                socket,
                Arc::new(quinn::TokioRuntime),
            ) {
                Ok(e) => e,
                Err(e) => {
                    log::error!("net: QUIC endpoint failed: {e}");
                    return;
                }
            };
            let conns: ConnTable = Arc::default();
            tokio::spawn(dispatch(cmd_rx, conns.clone()));
            let mut next = 1u64;
            while let Some(incoming) = endpoint.accept().await {
                let id = ConnId(next);
                next += 1;
                let (events, conns, secrets, addrs) =
                    (ev_tx.clone(), conns.clone(), s2.clone(), a2.clone());
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else {
                        return;
                    };
                    let mut secret = [0u8; 32];
                    if conn
                        .export_keying_material(&mut secret, EXPORTER_LABEL, b"")
                        .is_err()
                    {
                        return;
                    }
                    lock(&secrets).insert(id, secret.to_vec());
                    lock(&addrs).insert(id, conn.remote_address().to_string());
                    let (tx, rx) = unbounded_channel();
                    lock(&conns).insert(id, (conn.clone(), tx));
                    let _ = events.send(TransportEvent::Connected { conn: id });
                    run_conn(id, conn, false, rx, events).await;
                    lock(&conns).remove(&id);
                    lock(&secrets).remove(&id);
                    lock(&addrs).remove(&id);
                });
            }
        })?;
        Ok(Self {
            shared: Shared {
                events: ev_rx,
                cmds: cmd_tx,
                secrets,
                addrs,
                _thread: thread,
            },
            local_addr,
            fingerprint: identity.fingerprint(),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The room code for players to join: `public` is the address they can reach
    /// (your LAN or public IP with the listening port).
    pub fn room_code(&self, public: SocketAddr) -> RoomCode {
        RoomCode::new(public, &self.fingerprint)
    }
}

impl Transport for QuicServer {
    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        self.shared.poll(events);
    }
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        let _ = self
            .shared
            .cmds
            .send(Cmd::Send(conn, channel, data.to_vec()));
    }
    fn close(&mut self, conn: ConnId, reason: &str) {
        let _ = self.shared.cmds.send(Cmd::Close(conn, reason.to_owned()));
    }
    fn max_datagram(&self) -> usize {
        MAX_DATAGRAM
    }
    fn peer_addr(&self, conn: ConnId) -> Option<String> {
        lock(&self.shared.addrs).get(&conn).cloned()
    }
    fn session_secret(&self, conn: ConnId) -> Option<Vec<u8>> {
        lock(&self.shared.secrets).get(&conn).cloned()
    }
    fn room_info(&self) -> Option<(SocketAddr, [u8; 32])> {
        Some((self.local_addr, self.fingerprint))
    }
    fn name(&self) -> &'static str {
        "quic-server"
    }
}

/// Accepts the one certificate the room code names (and checks the server
/// holds its key), or any certificate when pinning was explicitly turned off.
#[derive(Debug)]
struct PinVerifier {
    pin: Option<[u8; PIN_LEN]>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Some(pin) = &self.pin {
            let hash = sha256(end_entity);
            if !crate::crypto::ct_eq(pin, &hash[..PIN_LEN]) {
                return Err(rustls::Error::General(
                    "the server's certificate does not match the room code".into(),
                ));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A QUIC client connection (one per session).
pub struct QuicClient {
    shared: Shared,
}

impl QuicClient {
    /// Connects to a room code (pinned), or to `host:port` when `allow_unpinned`.
    /// The connection is made in the background: wait for the `Connected` event.
    pub fn connect(target: &JoinTarget, allow_unpinned: bool) -> Result<Self, String> {
        let (addr, pin) = match target {
            JoinTarget::Room(code) => (code.addr, Some(code.pin)),
            JoinTarget::Addr(text) => {
                if !allow_unpinned {
                    return Err(format!(
                        "refusing to join {text} without a room code: nothing says whose server it is. \
                         Ask the host for the room code (OLV-...), or set NetConfig::allow_unpinned(true) \
                         if you trust the network."
                    ));
                }
                let addr = text
                    .to_socket_addrs()
                    .map_err(|e| format!("cannot resolve {text}: {e}"))?
                    .next()
                    .ok_or_else(|| format!("{text} has no address"))?;
                (addr, None)
            }
        };
        let p = provider();
        let mut tls = rustls::ClientConfig::builder_with_provider(p.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinVerifier { pin, provider: p }))
            .with_no_client_auth();
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let quic = QuicClientConfig::try_from(tls).map_err(|e| e.to_string())?;
        let mut config = quinn::ClientConfig::new(Arc::new(quic));
        let mut transport = quinn::TransportConfig::default();
        transport.datagram_receive_buffer_size(Some(1 << 20));
        config.transport_config(Arc::new(transport));
        let bind: SocketAddr = if addr.is_ipv6() {
            "[::]:0"
                .parse()
                .map_err(|_| "bad bind address".to_string())?
        } else {
            "0.0.0.0:0"
                .parse()
                .map_err(|_| "bad bind address".to_string())?
        };
        let socket = std::net::UdpSocket::bind(bind)
            .map_err(|e| format!("cannot open a UDP socket: {e}"))?;
        let (ev_tx, ev_rx) = channel();
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let secrets: Secrets = Arc::default();
        let s2 = secrets.clone();
        let thread = spawn_runtime(move || async move {
            let fail = |reason: String| {
                let _ = ev_tx.send(TransportEvent::Disconnected {
                    conn: ConnId(0),
                    reason,
                });
            };
            let mut endpoint = match quinn::Endpoint::new(
                quinn::EndpointConfig::default(),
                None,
                socket,
                Arc::new(quinn::TokioRuntime),
            ) {
                Ok(e) => e,
                Err(e) => return fail(format!("QUIC endpoint failed: {e}")),
            };
            endpoint.set_default_client_config(config);
            let connecting = match endpoint.connect(addr, "olvin") {
                Ok(c) => c,
                Err(e) => return fail(format!("cannot connect: {e}")),
            };
            let conn = match connecting.await {
                Ok(c) => c,
                Err(e) => return fail(format!("could not connect: {e}")),
            };
            let mut secret = [0u8; 32];
            if conn
                .export_keying_material(&mut secret, EXPORTER_LABEL, b"")
                .is_err()
            {
                return fail("TLS key export failed".into());
            }
            lock(&s2).insert(ConnId(0), secret.to_vec());
            let (tx, rx) = unbounded_channel();
            let conns: ConnTable = Arc::default();
            lock(&conns).insert(ConnId(0), (conn.clone(), tx));
            tokio::spawn(dispatch(cmd_rx, conns));
            let _ = ev_tx.send(TransportEvent::Connected { conn: ConnId(0) });
            run_conn(ConnId(0), conn, true, rx, ev_tx).await;
            endpoint.wait_idle().await;
        })?;
        Ok(Self {
            shared: Shared {
                events: ev_rx,
                cmds: cmd_tx,
                secrets,
                addrs: Arc::default(),
                _thread: thread,
            },
        })
    }
}

impl Transport for QuicClient {
    fn poll(&mut self, events: &mut Vec<TransportEvent>) {
        self.shared.poll(events);
    }
    fn send(&mut self, conn: ConnId, channel: Channel, data: &[u8]) {
        let _ = self
            .shared
            .cmds
            .send(Cmd::Send(conn, channel, data.to_vec()));
    }
    fn close(&mut self, conn: ConnId, reason: &str) {
        let _ = self.shared.cmds.send(Cmd::Close(conn, reason.to_owned()));
    }
    fn max_datagram(&self) -> usize {
        MAX_DATAGRAM
    }
    fn session_secret(&self, conn: ConnId) -> Option<Vec<u8>> {
        lock(&self.shared.secrets).get(&conn).cloned()
    }
    fn name(&self) -> &'static str {
        "quic-client"
    }
}
