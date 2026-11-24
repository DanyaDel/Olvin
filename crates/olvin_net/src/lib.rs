//! Olvin netcode, in layers that can each be used alone (see ADR 0002):
//!
//! * L0 transport: [`Transport`], [`Loopback`] (in-process, with simulated
//!   latency, jitter, loss, duplication and reordering).
//! * L2 replication: [`Replicated`], [`Owner`], [`Net::replicate`]; the server owns
//!   the world and clients get interpolated copies.
//! * L3 messages: [`NetMessage`], [`Net::send`], [`Target`], commands with a
//!   server-side validator ([`Verdict`]), rate and size limits.
//! * L1 session: [`Net`], [`NetPlugin`], [`NetConfig`], [`NetEvent`]; handshake,
//!   room codes ([`RoomCode`]), authentication ([`Authenticator`]).
//!
//! Everything a peer sends is untrusted input: bounded, checked, never unwrapped.

mod auth;
#[cfg(feature = "physics")]
pub mod character;
mod client;
mod config;
pub mod crypto;
mod ext;
mod lagcomp;
mod loopback;
mod message;
mod net;
mod plugin;
pub mod predict;
pub mod proto;
#[cfg(feature = "quic")]
pub mod quic;
mod registry;
pub mod replicate;
mod room;
mod secure;
mod server;
pub mod sim;
pub mod testing;
mod transport;
pub mod wire;

pub use auth::{AllowAll, AuthInfo, AuthRequest, AuthResult, Authenticator, SharedPassword};
pub use config::{NetConfig, NetLimits, NetMode};
pub use ext::NetAppExt;
pub use lagcomp::{LagCompensation, PoseHistory};
pub use loopback::{Conditioned, Loopback, LoopbackClient, LoopbackServer};
pub use message::{
    CommandCtx, Direction, MessageOptions, NetCommandRejected, NetMessage, NetReceived, Sender,
    Target, Verdict,
};
pub use net::{ClientId, ClientInfo, Net, NetEvent, NetStatus};
pub use plugin::NetPlugin;
pub use predict::{InputAck, Predicted, Prediction, PredictionStats};
pub use registry::NetRegistry;
pub use replicate::client::PrefabSpawner;
pub use replicate::server::RelevancyFn;
pub use replicate::{
    Codec, Interp, Interpolate, Interpolated, NetId, Owner, Remote, ReplicateMode,
    ReplicateOptions, Replicated, transform_codec,
};
pub use room::{JoinTarget, RoomCode, RoomCodeError};
pub use sim::NetConditions;
pub use transport::{Channel, ConnId, Transport, TransportEvent};

#[cfg(all(test, feature = "physics"))]
mod character_tests;
#[cfg(test)]
mod message_tests;
#[cfg(test)]
mod predict_tests;
#[cfg(all(test, feature = "quic"))]
mod quic_tests;
#[cfg(test)]
mod replicate_tests;
#[cfg(test)]
mod session_tests;
