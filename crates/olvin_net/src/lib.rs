//! Olvin netcode, in layers that can each be used alone (see ADR 0002):
//!
//! * L0 transport: [`Transport`], [`Loopback`] (in-process, with simulated
//!   latency, jitter, loss, duplication and reordering).
//! * L2 replication: [`Replicated`], [`Owner`], [`Net::replicate`]; the server owns
//!   the world and clients get interpolated copies.
//! * L1 session: [`Net`], [`NetPlugin`], [`NetConfig`], [`NetEvent`]; handshake,
//!   room codes ([`RoomCode`]), authentication ([`Authenticator`]).
//!
//! Everything a peer sends is untrusted input: bounded, checked, never unwrapped.

mod auth;
mod client;
mod config;
pub mod crypto;
mod ext;
mod loopback;
mod net;
mod plugin;
pub mod proto;
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
pub use loopback::{Conditioned, Loopback, LoopbackClient, LoopbackServer};
pub use net::{ClientId, ClientInfo, Net, NetEvent, NetStatus};
pub use plugin::NetPlugin;
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

#[cfg(test)]
mod replicate_tests;
#[cfg(test)]
mod session_tests;
