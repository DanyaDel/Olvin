//! L3: messages and commands. A message is any serde type, registered by name.
//! Commands are messages a client sends to the server, which the server
//! validates before anybody acts on them. Everything from a peer is untrusted:
//! size, rate, direction and shape are checked before a message becomes an event.

use std::any::TypeId;
use std::collections::HashMap;
use std::rc::Rc;

use olvin_core::{Events, Resources};
use olvin_ecs::{Entity, World};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::net::{ClientId, Net, NetEvent};
use crate::proto::Msg;
use crate::replicate::Owner;
use crate::transport::Channel;
use crate::wire;

/// Any serde type is a message; there is nothing to derive.
pub trait NetMessage: Serialize + DeserializeOwned + Clone + 'static {}
impl<T: Serialize + DeserializeOwned + Clone + 'static> NetMessage for T {}

/// Who sent a received message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sender {
    Server,
    Client(ClientId),
}

impl Sender {
    pub fn is_server(&self) -> bool {
        matches!(self, Sender::Server)
    }

    /// The client, if a client sent it.
    pub fn client(&self) -> Option<ClientId> {
        match self {
            Sender::Client(c) => Some(*c),
            Sender::Server => None,
        }
    }
}

/// A received (and, for commands, accepted) message, read from `Events<NetReceived<T>>`.
#[derive(Clone, Debug, PartialEq)]
pub struct NetReceived<T> {
    pub from: Sender,
    pub msg: T,
}

/// Where [`Net::send`] delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The server (from a client; on a host or offline: the local server).
    Server,
    Client(ClientId),
    /// Every client, the local player of a host included.
    All,
    AllExcept(ClientId),
    /// The client that owns this entity (its `Owner` component).
    Owner(Entity),
}

/// A command validator's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    /// Refused, with a reason for logs and for the client.
    Reject(String),
}

impl Verdict {
    pub fn reject(reason: impl Into<String>) -> Self {
        Verdict::Reject(reason.into())
    }

    /// `Accept` if `ok`, otherwise `Reject(reason)`.
    pub fn check(ok: bool, reason: &str) -> Self {
        if ok {
            Verdict::Accept
        } else {
            Verdict::Reject(reason.to_owned())
        }
    }
}

/// What a validator gets to look at. Not the `Net` resource: it is busy delivering.
pub struct CommandCtx<'a> {
    pub from: ClientId,
    pub res: &'a Resources,
}

impl CommandCtx<'_> {
    pub fn world(&self) -> std::cell::Ref<'_, World> {
        self.res.fetch::<World>()
    }
}

/// Server event: a command was refused (by its validator).
#[derive(Clone, Debug, PartialEq)]
pub struct NetCommandRejected {
    pub client: ClientId,
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    ClientToServer,
    ServerToClient,
    Both,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageOptions {
    /// Reliable and ordered (default), or fire and forget (movement-like data).
    pub reliable: bool,
    /// Largest encoded payload; `None`: `NetLimits::max_message_size`.
    pub max_size: Option<usize>,
    /// Messages per second one client may send of this type (burst: one second's worth).
    pub rate: Option<u32>,
    pub direction: Direction,
}

impl Default for MessageOptions {
    fn default() -> Self {
        Self {
            reliable: true,
            max_size: None,
            rate: None,
            direction: Direction::Both,
        }
    }
}

impl MessageOptions {
    /// Defaults for commands: client to server only, 60 per second.
    pub fn command() -> Self {
        Self {
            direction: Direction::ClientToServer,
            rate: Some(60),
            ..Self::default()
        }
    }

    pub fn unreliable(mut self) -> Self {
        self.reliable = false;
        self
    }

    pub fn max_size(mut self, bytes: usize) -> Self {
        self.max_size = Some(bytes);
        self
    }

    pub fn rate(mut self, per_second: u32) -> Self {
        self.rate = Some(per_second);
        self
    }

    pub fn direction(mut self, d: Direction) -> Self {
        self.direction = d;
        self
    }
}

type Validator<T> = Rc<dyn Fn(&CommandCtx, &T) -> Verdict>;

/// Decodes, validates and queues one message as an event.
type Deliver = Rc<dyn Fn(&Resources, Sender, &[u8]) -> Result<Verdict, String>>;

pub(crate) struct MessageEntry {
    pub name: String,
    pub opts: MessageOptions,
    pub max_size: usize,
    pub deliver: Deliver,
}

/// A message waiting to be handed to the game (after the `Net` borrow is released).
pub(crate) struct Staged {
    pub id: u16,
    pub sender: Sender,
    pub payload: Vec<u8>,
    pub deliver: Deliver,
    pub name: String,
}

/// Token bucket.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bucket {
    tokens: f32,
    last: f64,
}

impl Bucket {
    pub fn new(now: f64, burst: f32) -> Self {
        Self {
            tokens: burst,
            last: now,
        }
    }

    /// Takes one token if there is one; refills at `rate` per second up to `rate`.
    pub fn take(&mut self, now: f64, rate: f32) -> bool {
        let rate = rate.max(1.0);
        self.tokens = (self.tokens + (now - self.last).max(0.0) as f32 * rate).min(rate);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Per-client message accounting, on the server.
#[derive(Default)]
pub(crate) struct ClientLimits {
    pub total: Option<Bucket>,
    pub typed: HashMap<u16, Bucket>,
    /// Leaky count of misbehaviour; over the threshold the client is kicked.
    pub violations: f32,
    pub violations_at: f64,
}

const VIOLATION_LIMIT: f32 = 60.0;
const VIOLATION_DECAY_PER_SEC: f32 = 2.0;

#[derive(Default)]
pub(crate) struct Messages {
    pub entries: Vec<MessageEntry>,
    pub by_type: HashMap<TypeId, u16>,
    pub staged: Vec<Staged>,
    /// Sent to the local server/client without a wire: (sender, id, payload).
    pub local: Vec<(Sender, u16, Vec<u8>)>,
    /// `Target::Owner` sends, resolved when the world is at hand.
    pub to_owner: Vec<(Entity, u16, Vec<u8>)>,
    pub limits: HashMap<ClientId, ClientLimits>,
    pub rejected: Vec<NetCommandRejected>,
}

impl Net {
    /// Registers message type `T` under `name`. Prefer [`NetAppExt::net_message`](crate::NetAppExt::net_message),
    /// which also registers the `Events<NetReceived<T>>` queue.
    pub fn register_message<T: NetMessage>(
        &mut self,
        name: &str,
        opts: MessageOptions,
    ) -> &mut Self {
        self.register_inner::<T>(name, opts, None)
    }

    /// Registers a command: a message clients send to the server, checked by `validate`
    /// before it becomes an event. See [`MessageOptions::command`].
    pub fn register_command<T: NetMessage>(
        &mut self,
        name: &str,
        opts: MessageOptions,
        validate: impl Fn(&CommandCtx, &T) -> Verdict + 'static,
    ) -> &mut Self {
        self.register_inner::<T>(name, opts, Some(Rc::new(validate)))
    }

    fn register_inner<T: NetMessage>(
        &mut self,
        name: &str,
        opts: MessageOptions,
        validate: Option<Validator<T>>,
    ) -> &mut Self {
        assert!(
            self.messages.entries.len() < u16::MAX as usize,
            "net: too many message types"
        );
        self.registry.add_name("message", name);
        let max_size = opts.max_size.unwrap_or(self.cfg.limits.max_message_size);
        let label = name.to_owned();
        let deliver: Deliver = Rc::new(move |res, from, bytes| {
            let msg: T = wire::decode(bytes, max_size).map_err(|e| e.to_string())?;
            if let (Some(check), Sender::Client(c)) = (&validate, from)
                && let Verdict::Reject(reason) = check(&CommandCtx { from: c, res }, &msg)
            {
                return Ok(Verdict::Reject(reason));
            }
            match res.get_mut::<Events<NetReceived<T>>>() {
                Some(mut events) => events.send(NetReceived { from, msg }),
                None => {
                    return Err(format!(
                        "no Events<NetReceived<..>> for `{label}`: register it with app.net_message::<T>(..)"
                    ));
                }
            }
            Ok(Verdict::Accept)
        });
        let id = self.messages.entries.len() as u16;
        self.messages.by_type.insert(TypeId::of::<T>(), id);
        self.messages.entries.push(MessageEntry {
            name: name.to_owned(),
            opts,
            max_size,
            deliver,
        });
        self
    }

    /// Sends `msg` to `target`. Clients can only send to [`Target::Server`]. On a
    /// host or offline, sending to the server (or to yourself) is local but goes
    /// through the same size checks, rate limits and validation as a remote message.
    pub fn send<T: NetMessage>(&mut self, target: Target, msg: &T) {
        let Some(&id) = self.messages.by_type.get(&TypeId::of::<T>()) else {
            log::error!(
                "net: send of an unregistered message type `{}`; register it with app.net_message::<T>(name)",
                std::any::type_name::<T>()
            );
            return;
        };
        let Ok(payload) = wire::encode(msg) else {
            log::error!("net: cannot encode `{}`", std::any::type_name::<T>());
            return;
        };
        self.send_raw(target, id, payload);
    }

    pub(crate) fn send_raw(&mut self, target: Target, id: u16, payload: Vec<u8>) {
        let Some(entry) = self.messages.entries.get(id as usize) else {
            return;
        };
        if payload.len() > entry.max_size {
            log::error!(
                "net: message `{}` is {} bytes, over its {} limit; not sent",
                entry.name,
                payload.len(),
                entry.max_size
            );
            return;
        }
        let local_player = self.local_client();
        if self.server.is_none() {
            // A client talks to the server and nobody else.
            if target != Target::Server {
                log::error!(
                    "net: a client can only send to Target::Server (`{}`)",
                    entry.name
                );
                return;
            }
            if !matches!(
                entry.opts.direction,
                Direction::ClientToServer | Direction::Both
            ) {
                log::error!("net: `{}` is a server-to-client message", entry.name);
                return;
            }
            self.send_user(None, id, &payload);
            return;
        }
        match target {
            Target::Server => match local_player {
                Some(me) => self.messages.local.push((Sender::Client(me), id, payload)),
                None => log::warn!(
                    "net: a dedicated server has no local client to send `{}` to itself",
                    entry.name
                ),
            },
            Target::Client(c) => self.send_to_client(c, id, payload),
            Target::All => {
                for c in self.client_ids() {
                    self.send_to_client(c, id, payload.clone());
                }
            }
            Target::AllExcept(skip) => {
                for c in self.client_ids().into_iter().filter(|c| *c != skip) {
                    self.send_to_client(c, id, payload.clone());
                }
            }
            Target::Owner(e) => self.messages.to_owner.push((e, id, payload)),
        }
    }

    /// Every client of the server, the local player included.
    fn client_ids(&self) -> Vec<ClientId> {
        let mut out = Vec::new();
        if self.cfg.mode_has_local_player() {
            out.push(ClientId::HOST);
        }
        if let Some(s) = &self.server {
            out.extend(
                s.clients
                    .values()
                    .filter(|c| c.peer.is_some())
                    .map(|c| c.id),
            );
        }
        out
    }

    fn send_to_client(&mut self, client: ClientId, id: u16, payload: Vec<u8>) {
        if client == ClientId::HOST && self.cfg.mode_has_local_player() {
            self.messages.local.push((Sender::Server, id, payload));
        } else {
            self.send_user(Some(client), id, &payload);
        }
    }

    /// Wire send of a user message: to a client (server side) or to the server (client side).
    fn send_user(&mut self, to: Option<ClientId>, id: u16, payload: &[u8]) {
        let Some(entry) = self.messages.entries.get(id as usize) else {
            return;
        };
        let max_dg = match (&self.server, to) {
            (Some(s), Some(c)) => s.max_datagram(c),
            _ => self
                .client
                .as_ref()
                .map_or(1200, |c| c.transport.max_datagram()),
        };
        let reliable = entry.opts.reliable || payload.len() + 48 > max_dg;
        let channel = if reliable {
            Channel::Reliable
        } else {
            Channel::Unreliable
        };
        let msg = Msg::User {
            id,
            payload: payload.to_vec(),
        };
        match to {
            Some(c) => {
                if let Some(s) = self.server.as_mut() {
                    s.send_to(c, channel, &msg);
                }
            }
            None => {
                if let Some(c) = self.client.as_mut() {
                    c.send(channel, &msg);
                }
            }
        }
    }

    /// Server: resolves `Target::Owner` sends now that the world is available.
    pub(crate) fn flush_owner_sends(&mut self, world: &World) {
        for (entity, id, payload) in std::mem::take(&mut self.messages.to_owner) {
            match world.get::<&Owner>(entity) {
                Ok(o) => self.send_to_client(o.0, id, payload),
                Err(_) => {
                    log::warn!("net: Target::Owner for an entity without an Owner; message dropped")
                }
            }
        }
    }

    /// Notes misbehaviour of a client; too much gets it kicked.
    pub(crate) fn violation(&mut self, client: ClientId, weight: f32, why: &str) {
        if client == ClientId::HOST {
            return;
        }
        let now = self.now;
        let l = self.messages.limits.entry(client).or_default();
        l.violations = (l.violations
            - (now - l.violations_at).max(0.0) as f32 * VIOLATION_DECAY_PER_SEC)
            .max(0.0)
            + weight;
        l.violations_at = now;
        log::debug!(
            "net: client {} misbehaved: {why} (score {:.1})",
            client.0,
            l.violations
        );
        if l.violations > VIOLATION_LIMIT {
            self.messages.limits.remove(&client);
            log::warn!("net: kicking client {}: {why}", client.0);
            self.kick(client, "sending too many invalid or excessive messages");
        }
    }

    /// Server: checks a user message from a client and stages it for delivery.
    pub(crate) fn stage_from_client(&mut self, client: ClientId, id: u16, payload: Vec<u8>) {
        let now = self.now;
        // A client that was kicked mid-batch gets nothing more through.
        if !self
            .server
            .as_ref()
            .is_some_and(|s| s.clients.contains_key(&client))
        {
            return;
        }
        let Some(entry) = self.messages.entries.get(id as usize) else {
            self.violation(client, 5.0, "unknown message id");
            return;
        };
        if !matches!(
            entry.opts.direction,
            Direction::ClientToServer | Direction::Both
        ) {
            let why = format!("`{}` is not a client message", entry.name);
            self.violation(client, 5.0, &why);
            return;
        }
        if payload.len() > entry.max_size {
            let why = format!("`{}` over its size limit", entry.name);
            self.violation(client, 5.0, &why);
            return;
        }
        let per_type = entry.opts.rate;
        let total_rate = self.cfg.limits.max_messages_per_sec as f32;
        let l = self.messages.limits.entry(client).or_default();
        let total = l.total.get_or_insert_with(|| Bucket::new(now, total_rate));
        let mut ok = total.take(now, total_rate);
        if ok && let Some(rate) = per_type {
            ok = l
                .typed
                .entry(id)
                .or_insert_with(|| Bucket::new(now, rate as f32))
                .take(now, rate as f32);
        }
        if !ok {
            self.violation(client, 0.5, "message rate limit");
            return;
        }
        let entry = &self.messages.entries[id as usize];
        self.messages.staged.push(Staged {
            id,
            sender: Sender::Client(client),
            payload,
            deliver: entry.deliver.clone(),
            name: entry.name.clone(),
        });
    }

    /// Client: checks a user message from the server and stages it.
    pub(crate) fn stage_from_server(&mut self, id: u16, payload: Vec<u8>) {
        let Some(entry) = self.messages.entries.get(id as usize) else {
            log::warn!("net: the server sent an unknown message id {id}");
            return;
        };
        if !matches!(
            entry.opts.direction,
            Direction::ServerToClient | Direction::Both
        ) || payload.len() > entry.max_size
        {
            log::warn!(
                "net: the server sent `{}` against its options; dropped",
                entry.name
            );
            return;
        }
        self.messages.staged.push(Staged {
            id,
            sender: Sender::Server,
            payload,
            deliver: entry.deliver.clone(),
            name: entry.name.clone(),
        });
    }

    /// Moves local sends into the staging list through the same checks.
    pub(crate) fn stage_local(&mut self) {
        for (sender, id, payload) in std::mem::take(&mut self.messages.local) {
            let Some(entry) = self.messages.entries.get(id as usize) else {
                continue;
            };
            let allowed = match sender {
                Sender::Client(_) => matches!(
                    entry.opts.direction,
                    Direction::ClientToServer | Direction::Both
                ),
                Sender::Server => matches!(
                    entry.opts.direction,
                    Direction::ServerToClient | Direction::Both
                ),
            };
            if !allowed || payload.len() > entry.max_size {
                log::warn!(
                    "net: local message `{}` against its options; dropped",
                    entry.name
                );
                continue;
            }
            self.messages.staged.push(Staged {
                id,
                sender,
                payload,
                deliver: entry.deliver.clone(),
                name: entry.name.clone(),
            });
        }
    }

    /// The server tells a client why a command was refused.
    pub(crate) fn note_rejected(&mut self, client: ClientId, id: u16, name: &str, reason: &str) {
        self.messages.rejected.push(NetCommandRejected {
            client,
            name: name.to_owned(),
            reason: reason.to_owned(),
        });
        if client != ClientId::HOST {
            if let Some(s) = self.server.as_mut() {
                s.send_to(
                    client,
                    Channel::Reliable,
                    &Msg::Rejected {
                        id,
                        reason: reason.chars().take(200).collect(),
                    },
                );
            }
            self.violation(client, 1.0, "rejected command");
        } else {
            self.events.push(NetEvent::CommandRejected {
                name: name.to_owned(),
                reason: reason.to_owned(),
            });
        }
    }

    /// Message name for an id (for logs and Lua).
    pub fn message_name(&self, id: u16) -> Option<&str> {
        self.messages
            .entries
            .get(id as usize)
            .map(|e| e.name.as_str())
    }
}

/// Hands staged messages to the game. Call without holding `Net`.
pub(crate) fn deliver_staged(res: &Resources, staged: Vec<Staged>) {
    for st in staged {
        match (st.deliver)(res, st.sender, &st.payload) {
            Ok(Verdict::Accept) => {}
            Ok(Verdict::Reject(reason)) => {
                if let Sender::Client(c) = st.sender
                    && let Some(mut net) = res.get_mut::<Net>()
                {
                    log::debug!(
                        "net: command `{}` from client {} rejected: {reason}",
                        st.name,
                        c.0
                    );
                    net.note_rejected(c, st.id, &st.name, &reason);
                }
            }
            Err(e) => {
                log::debug!("net: bad `{}` from {:?}: {e}", st.name, st.sender);
                if let Sender::Client(c) = st.sender
                    && let Some(mut net) = res.get_mut::<Net>()
                {
                    net.violation(c, 5.0, "malformed message");
                }
            }
        }
    }
}
