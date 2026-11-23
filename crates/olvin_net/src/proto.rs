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
    /// Reliable: an entity exists for this client, with all its replicated components.
    Spawn(SpawnMsg),
    /// Reliable: the entity is gone (despawned, or no longer relevant to this client).
    Despawn {
        net_id: u64,
    },
    /// Unreliable (reliable for entries too big for a datagram): changed components.
    Snapshot(SnapshotMsg),
    /// Client to server: snapshots received. `seq` is the newest; bit `i` of
    /// `bits` says whether `seq - 1 - i` arrived too.
    Ack {
        seq: u32,
        bits: u32,
    },
}

/// How a client creates the copy of a replicated entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SpawnKind {
    /// An empty entity; the replicated components make it.
    Bare,
    /// Instantiate this prefab asset (`*.prefab.ron`, relative to `assets/`).
    Prefab(String),
    /// Adopt the level object with this `SceneId`, which both sides loaded from the scene.
    Scene(u64),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpawnMsg {
    pub net_id: u64,
    /// Server tick the state belongs to; older snapshots for it are ignored.
    pub tick: u32,
    pub kind: SpawnKind,
    pub comps: Vec<CompData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompData {
    /// Index in the replication registry (registration order).
    pub comp: u16,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotMsg {
    pub seq: u32,
    pub tick: u32,
    pub entries: Vec<SnapEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapEntry {
    pub net_id: u64,
    pub comp: u16,
    /// `None`: the component was removed.
    pub data: Option<Vec<u8>>,
}
