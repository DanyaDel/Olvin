//! Packets of the session layer. The order of enum variants is the wire
//! format: add new ones at the end and bump [`PROTOCOL_VERSION`](crate::wire::PROTOCOL_VERSION)
//! when anything else changes.

use serde::{Deserialize, Serialize};

use crate::crypto::Nonce;

/// First packet of a client. Sent as `u16 LE protocol version | postcard(Hello)`,
/// so a server can say "wrong version" even if the rest has changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub game_id: String,
    pub game_version: u32,
    /// Hash of the registered component and message names, see `NetRegistry::signature`.
    pub registry_hash: u64,
    pub name: String,
    pub token: Vec<u8>,
    pub nonce: Nonce,
    /// Client's unix time, milliseconds (replay window).
    pub unix_ms: u64,
    pub reconnect: Option<ReconnectRequest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReconnectRequest {
    pub client_id: u32,
    pub token: [u8; 16],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    pub client_id: u32,
    pub server_nonce: Nonce,
    pub tick_rate: u32,
    pub server_tick: u32,
    pub reconnect_token: [u8; 16],
    /// Proof that the server derived the same session key.
    pub proof: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectCode {
    ProtocolMismatch,
    GameMismatch,
    RegistryMismatch,
    Denied,
    ServerFull,
    BadTimestamp,
    Replay,
    Malformed,
    Busy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reject {
    pub code: RejectCode,
    pub message: String,
}

/// Everything after the handshake: the body of a sealed `DATA` packet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Msg {
    Ping {
        id: u32,
        sent: f32,
    },
    Pong {
        id: u32,
        sent: f32,
        server_tick: u32,
    },
    Disconnect {
        reason: String,
    },
}
