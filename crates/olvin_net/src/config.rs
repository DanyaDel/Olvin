//! One struct for everything configurable. Start from a constructor for the
//! mode you want and chain only the settings you care about:
//!
//! ```
//! # use olvin_net::NetConfig;
//! let cfg = NetConfig::host("my-game", 1, "0.0.0.0:7777").tick_rate(60).max_clients(8);
//! assert_eq!(cfg.tick_rate, 60);
//! ```

use crate::room::JoinTarget;
use crate::sim::NetConditions;

/// How this process takes part in a game.
#[derive(Clone, Debug, PartialEq)]
pub enum NetMode {
    /// Single player. A server and one local client in this process, no
    /// sockets; the game runs the same message and command path as online.
    Offline,
    /// A listen server: this process is the server and also plays (the local
    /// client is [`ClientId::HOST`](crate::ClientId::HOST)).
    Host {
        /// Address to listen on (`0.0.0.0:7777`). `None`: no socket of its own,
        /// only transports given to the plugin (tests).
        listen: Option<String>,
    },
    /// A headless server: no local player.
    Dedicated { listen: Option<String> },
    /// Joins somebody's game.
    Client {
        /// `None`: the plugin was given a transport that is already pointed at the server.
        target: Option<JoinTarget>,
    },
}

/// Hard limits on what a peer can make us do. Everything a peer sends is
/// checked against these before it is parsed or allocated.
#[derive(Clone, Debug, PartialEq)]
pub struct NetLimits {
    /// Largest reliable packet, bytes.
    pub max_reliable_packet: usize,
    /// Largest user message (`NetMessage` payload), bytes.
    pub max_message_size: usize,
    /// Messages per second one client may send before the rest is dropped.
    pub max_messages_per_sec: u32,
    /// Handshake packets this big are not even parsed.
    pub max_handshake_packet: usize,
    /// Name length in characters.
    pub max_name_len: usize,
    /// Auth token length, bytes.
    pub max_token_len: usize,
}

impl Default for NetLimits {
    fn default() -> Self {
        Self {
            max_reliable_packet: 256 * 1024,
            max_message_size: 16 * 1024,
            max_messages_per_sec: 120,
            max_handshake_packet: 2048,
            max_name_len: 32,
            max_token_len: 512,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NetConfig {
    pub mode: NetMode,
    /// Identifies the game; peers with another id are refused.
    pub game_id: String,
    /// Peers must run the same game version (bump it when you change the game's net protocol).
    pub game_version: u32,
    /// Server ticks per second.
    pub tick_rate: u32,
    /// How far in the past remote entities are shown, seconds.
    pub interp_delay: f32,
    pub max_clients: usize,
    /// Client ping period, seconds.
    pub heartbeat: f32,
    /// A peer that sends nothing for this long is dropped, seconds.
    pub timeout: f32,
    /// A connection that has not finished the handshake after this long is dropped, seconds.
    pub handshake_timeout: f32,
    /// A client that lost its connection may reclaim its identity for this long, seconds.
    pub reconnect_grace: f32,
    /// Handshake timestamps further than this from the server's clock are refused, seconds.
    pub timestamp_window: f32,
    /// Connections that may be in the handshake at once.
    pub max_pending: usize,
    /// Snapshot bytes per second per client.
    pub bandwidth: usize,
    pub limits: NetLimits,
    /// Your display name.
    pub player_name: String,
    /// Sent to the server's `Authenticator` (password, ticket...).
    pub token: Vec<u8>,
    /// Put these conditions on the real transport (testing "lag 80 ms, loss 2%").
    pub simulate: Option<NetConditions>,
    /// Connect to a bare `host:port` without a pinned certificate. Off: refused.
    pub allow_unpinned: bool,
    /// Unix time in milliseconds; replace it to test the timestamp window.
    pub clock: fn() -> u64,
}

pub fn system_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl NetConfig {
    fn base(mode: NetMode, game_id: &str, game_version: u32) -> Self {
        Self {
            mode,
            game_id: game_id.to_owned(),
            game_version,
            tick_rate: 30,
            interp_delay: 0.1,
            max_clients: 16,
            heartbeat: 0.5,
            timeout: 10.0,
            handshake_timeout: 5.0,
            reconnect_grace: 30.0,
            timestamp_window: 60.0,
            max_pending: 32,
            bandwidth: 64 * 1024,
            limits: NetLimits::default(),
            player_name: "player".into(),
            token: Vec::new(),
            simulate: None,
            allow_unpinned: false,
            clock: system_unix_millis,
        }
    }

    /// Single player on the same code path as online.
    pub fn offline(game_id: &str, game_version: u32) -> Self {
        Self::base(NetMode::Offline, game_id, game_version)
    }

    /// A listen server on `listen` (`"0.0.0.0:7777"`), host plays too.
    pub fn host(game_id: &str, game_version: u32, listen: &str) -> Self {
        Self::base(
            NetMode::Host {
                listen: Some(listen.to_owned()),
            },
            game_id,
            game_version,
        )
    }

    /// A headless server on `listen`.
    pub fn dedicated(game_id: &str, game_version: u32, listen: &str) -> Self {
        Self::base(
            NetMode::Dedicated {
                listen: Some(listen.to_owned()),
            },
            game_id,
            game_version,
        )
    }

    /// Joins `target`: a room code (`OLV-...`) or `host:port`.
    pub fn join(game_id: &str, game_version: u32, target: &str) -> Result<Self, String> {
        Ok(Self::base(
            NetMode::Client {
                target: Some(JoinTarget::parse(target)?),
            },
            game_id,
            game_version,
        ))
    }

    /// For command lines: `--host [addr]`, `--dedicated [addr]`, `--join <code|addr>`;
    /// no flag means offline.
    ///
    /// ```
    /// # use olvin_net::{NetConfig, NetMode};
    /// let args = ["game", "--join", "127.0.0.1:7777"].map(String::from);
    /// let cfg = NetConfig::from_args("my-game", 1, args).unwrap();
    /// assert!(matches!(cfg.mode, NetMode::Client { .. }));
    /// ```
    pub fn from_args(
        game_id: &str,
        game_version: u32,
        args: impl IntoIterator<Item = String>,
    ) -> Result<Self, String> {
        let args: Vec<String> = args.into_iter().collect();
        let value = |flag: &str| -> Option<Option<String>> {
            let i = args.iter().position(|a| a == flag)?;
            Some(args.get(i + 1).filter(|v| !v.starts_with("--")).cloned())
        };
        let mut cfg = if let Some(v) = value("--join") {
            let target = v.ok_or("--join needs a room code or host:port")?;
            Self::join(game_id, game_version, &target)?
        } else if let Some(v) = value("--dedicated") {
            Self::dedicated(
                game_id,
                game_version,
                &v.unwrap_or_else(|| "0.0.0.0:7777".into()),
            )
        } else if let Some(v) = value("--host") {
            Self::host(
                game_id,
                game_version,
                &v.unwrap_or_else(|| "0.0.0.0:7777".into()),
            )
        } else {
            Self::offline(game_id, game_version)
        };
        if let Some(Some(name)) = value("--name") {
            cfg.player_name = name;
        }
        if let Some(Some(sim)) = value("--net-sim") {
            cfg.simulate = Some(NetConditions::parse(&sim)?);
        } else if let Ok(sim) = std::env::var("OLVIN_NET_SIM") {
            cfg.simulate = Some(NetConditions::parse(&sim)?);
        }
        Ok(cfg)
    }

    pub fn tick_rate(mut self, hz: u32) -> Self {
        self.tick_rate = hz.clamp(1, 240);
        self
    }

    pub fn interp_delay(mut self, seconds: f32) -> Self {
        self.interp_delay = seconds.clamp(0.0, 2.0);
        self
    }

    pub fn max_clients(mut self, n: usize) -> Self {
        self.max_clients = n.max(1);
        self
    }

    pub fn player_name(mut self, name: &str) -> Self {
        self.player_name = name.to_owned();
        self
    }

    pub fn token(mut self, token: impl Into<Vec<u8>>) -> Self {
        self.token = token.into();
        self
    }

    pub fn bandwidth(mut self, bytes_per_sec: usize) -> Self {
        self.bandwidth = bytes_per_sec.max(1024);
        self
    }

    pub fn timeout(mut self, seconds: f32) -> Self {
        self.timeout = seconds.max(0.1);
        self
    }

    pub fn simulate(mut self, conditions: NetConditions) -> Self {
        self.simulate = Some(conditions);
        self
    }

    pub fn allow_unpinned(mut self, yes: bool) -> Self {
        self.allow_unpinned = yes;
        self
    }

    pub fn limits(mut self, limits: NetLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn clock(mut self, clock: fn() -> u64) -> Self {
        self.clock = clock;
        self
    }

    pub fn is_server_mode(&self) -> bool {
        !matches!(self.mode, NetMode::Client { .. })
    }
}
