//! Olvin netcode, in layers that can each be used alone (see ADR 0002):
//!
//! * L0 transport: [`Transport`], [`Loopback`] (in-process, with simulated
//!   latency, jitter, loss, duplication and reordering).
//! * L1 session: [`Net`], [`NetPlugin`], [`NetConfig`], [`NetEvent`]; handshake,
//!   room codes ([`RoomCode`]), authentication ([`Authenticator`]).
//!
//! Everything a peer sends is untrusted input: bounded, checked, never unwrapped.

mod auth;
mod client;
mod config;
pub mod crypto;
mod loopback;
mod net;
mod plugin;
pub mod proto;
mod registry;
mod room;
mod secure;
mod server;
pub mod sim;
pub mod testing;
mod transport;
pub mod wire;

pub use auth::{AllowAll, AuthInfo, AuthRequest, AuthResult, Authenticator, SharedPassword};
pub use config::{NetConfig, NetLimits, NetMode};
pub use loopback::{Conditioned, Loopback, LoopbackClient, LoopbackServer};
pub use net::{ClientId, ClientInfo, Net, NetEvent, NetStatus};
pub use plugin::NetPlugin;
pub use registry::NetRegistry;
pub use room::{JoinTarget, RoomCode, RoomCodeError};
pub use sim::NetConditions;
pub use transport::{Channel, ConnId, Transport, TransportEvent};

#[cfg(test)]
mod session_tests;
