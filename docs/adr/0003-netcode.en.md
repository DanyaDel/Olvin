# 0003. Netcode: own protocol on QUIC, server authority, snapshot interpolation

<p align="center"><a href="0003-netcode.md">Русский</a></p>

**Status:** Accepted — 2026-11-23 (crate `olvin_net`, branch `feat-net`)

## Context

[ROADMAP.en.md](../../ROADMAP.en.md) has three network items: "Network" (phase 3, issue #33), "Simple multiplayer" (phase 7: a networked-object component, an owner component, room codes, presets for co-op and arena) and the security block of phase 8 (N1 server authority, N2 TLS and request signing, N3 nonce and timestamps against replay, N4 no server secrets in the client). The owner's requirement: **the netcode must be insanely flexible and at the same time not hard to write.**

What constrains the choice:

- **A single-threaded `App`.** Systems are `FnMut(&mut Resources)`, `Resources` is not `Send`. The network cannot be an async runtime that owns the game; it has to be polled from a system.
- **Every game is net-ready.** A single-player game should run the same code path as a multiplayer one, otherwise multiplayer is "a second game" that nobody tests.
- **Prefabs and scenes already exist.** A networked object should be "a prefab with a marker", set up in the inspector with no code.
- **Untrusted peers.** A client is in a stranger's hands. Anything that comes from a client is hostile input.
- **Testability.** Network bugs are timing bugs. Without a simulated network that is deterministic and fast, nothing can be verified.
- **Light default build.** A game with no network must not pay for it.

## Decision

A new crate `olvin_net`, organised as **layers, each usable alone**. The top layer is a few lines for a typical game.

| Layer | What | Where |
|---|---|---|
| L0 Transport | the `Transport` trait: reliable ordered and unreliable channels, connect and disconnect events. `Loopback` (in-process, with simulated latency, jitter, loss, duplication, reorder) and `QuicTransport` (feature `quic`) | `transport.rs`, `loopback.rs`, `quic.rs` |
| L1 Session | modes `Offline`, `Host`, `Dedicated`, `Client`; handshake, `ClientId`, join, leave, kick, heartbeat, timeout, reconnect token; room codes; the pluggable `Authenticator` | `session.rs`, `handshake.rs`, `room.rs` |
| L2 Replication | `NetRegistry` (like `SaveRegistry`), `Replicated`, `Owner`, `NetId`, spawn and despawn by prefab path, delta snapshots with acks, interpolation, relevancy, bandwidth budget | `replicate/` |
| L3 Messages | `NetMessage` registered by name, `net.send(Target, msg)`, commands with a server-side validator, rate and size limits | `message.rs` |
| L4 Prediction | tick-stamped inputs, local prediction, reconciliation, error smoothing, `Predicted<T>` | `predict.rs` |
| L5 Scripting | `ctx:` bindings in `olvin_script` (feature `net`) | `olvin_script/src/lua/net.rs` |
| L6 Tools | `NetStats`, profiler counters, a simulated-network setting | `stats.rs` |

Typical game, in the order the author meets the layers: mark a prefab `Replicated` in the editor, register the components with `net.replicate::<T>()`, write one command handler. Everything is configured through one `NetConfig` struct and its builder; defaults are tick 30 Hz, interpolation delay 100 ms, 64 KiB/s budget per client.

### Offline is a session mode

`Offline` is not "no network". It is a server and one local client on a `Loopback` without conditions, inside one `App`. Single-player code calls `net.send(Target::Server, cmd)` and handles the command, just as in multiplayer. The price is a few serialisations per tick; the gain is that every game is tested as a networked one from day one.

### Wire format: `postcard`

Every packet is `postcard`-serialised serde types. Why not `bincode 2`: postcard has a stable, documented wire format (it is a spec, not "what the crate does today"), variable-length integers (small numbers are small on the wire), `no_std` and no `unsafe` in the path we use. It does not allocate beyond the input length when decoding, which matters for hostile bytes: a length prefix of `u64::MAX` cannot make us allocate. A packet carries a protocol version in its first byte; the whole packet is checked against `max_packet` *before* decoding. The cost: postcard is not self-describing, so a mismatched struct is garbage, not an error. That is why the handshake compares the protocol version, the game id and a **registry hash** (names and order of registered components and messages): two builds with different registries refuse to talk instead of mis-decoding.

### Transport: QUIC through `quinn`

- **TLS 1.3 is built in** (N2). No home-grown crypto for the transport.
- Reliable ordered **streams** and unreliable **datagrams** are exactly the two channels we need.
- Connection migration and a handshake that survives NAT rebinding are "free".
- Cost: `quinn` brings `tokio`, `rustls`, `ring`. That is why it is behind the `quic` feature; the default build has only the trait and `Loopback`. The runtime is hidden inside the transport (a private current-thread runtime in a background thread; the game sees only a non-blocking `poll`).
- **Server certificate: self-signed, pinned by fingerprint.** There is no certificate authority for a listen server on somebody's laptop. The server generates a certificate (or loads one), and the SHA-256 of its public key goes into the **room code** / invite together with the address. The client accepts only a certificate with that fingerprint. A man in the middle cannot present a matching one without the private key. The code travels over a channel the players already trust (chat, a friend). Without a code (`addr` only) the connection is refused unless `allow_unpinned` is set explicitly, and the log says so.
- The fallback "own UDP with own reliability" was prepared but not needed: `quinn` builds here.

### Authority and security model

- **N1. Server authority by default.** Only the server writes replicated components. A client has no message that sets a replicated value of any entity. The client can send *commands* (input, "fire", "buy"); the server validates them and changes the world. Ownership (`Owner(ClientId)`) says who may send *commands for* an entity (and who predicts it), not who writes its state. Transfer of ownership is a server operation.
- **N2. TLS** is QUIC's. On top of that the session has a per-session key (below), so the same code is safe on any other transport.
- **N3. Replay.** The handshake carries a client nonce and a unix timestamp; the server rejects a timestamp outside a window (default 60 s) and remembers recent nonces (bounded set). Inside the session every packet carries a strictly increasing sequence number and a truncated HMAC-SHA256 tag keyed by the session key; the receiver keeps a sliding window and drops duplicates and old numbers. A recorded packet replayed later is dropped.
- **Session key.** `HMAC(secret, "olvin-net v1" || client_nonce || server_nonce)`. On QUIC `secret` is the TLS exporter output (bound to this connection, never on the wire). On `Loopback` it is empty: the transport is in-process and trusted, so the tag guards against bugs (and exercises the code path), not against an attacker. This is stated, not hidden.
- **N4. No server secrets in the client.** The server certificate private key, the authenticator's secrets and any server-only configuration live in types that exist only in the `server` half of the crate API (`ServerSecrets`, `Authenticator` impls). The client build never constructs them. The rule: a secret marker is named `server_*`; a CI check that scans a client binary for such markers is possible later, it is not part of this work.
- **Bounded everything.** Packet size, number of clients, pending handshakes, messages per second per client and per type, message size, entities per snapshot, buffered early updates, nesting. Peer input never reaches `unwrap`, indexing without a check or an unbounded allocation. Tests feed garbage bytes to every decoder.
- **Authentication** is a trait (`Authenticator`): it gets the token the client sent and the peer address and returns an identity or a refusal. The default accepts everybody (fine for a LAN game); the crate ships a shared-password one. Real services (accounts, Steam tickets) implement the trait.
- **Reconnect.** The server gives a reconnect token; a client that lost the connection within a grace period gets its `ClientId` and its `Owner` entities back.

### Replication: server state, snapshot interpolation

- The server runs a **fixed tick** (30 Hz by default, `NetConfig::tick_rate`), independent of the frame rate and of the 60 Hz physics steps.
- **What is replicated** is registered by name, like `SaveRegistry`: `net.replicate::<Transform>()`. Names and order are part of the protocol (hashed in the handshake). Options per component: mode (on change, every N ticks, always), a quantisation hook, an interpolation function (none, linear or any custom `fn`). `Transform` has a compact built-in codec.
- **What is a networked object:** an entity with a `Replicated` component. The server gives it a `NetId` (a counter, never reused in a session). Clients receive "spawn `NetId`, prefab path, owner, components"; they spawn the **same prefab** with `instantiate_prefab`, so the visual and physical parts come from the asset, not from the network. A level object (with a `SceneId`, no prefab) is adopted instead of spawned. `Replicated` is registered in `SceneComponents`, so a designer adds it to an object in the inspector.
- **Deltas and acks.** The server keeps, per (entity, component), the last changed tick, and per client what the client acknowledged. A component is sent until acknowledged; acks are a base number plus a bitfield. Lost snapshots are not retransmitted, the *newer state* is sent instead.
- **Spawn and despawn** are reliable and ordered. An update that overtakes its spawn is buffered for a short time (bounded) and applied after it, by tick.
- **Interpolation.** A client shows remote entities about 100 ms in the past, between two received snapshots, so loss and jitter below that delay are invisible. There is no extrapolation by default (a stopped lerp is better than a wrong guess).
- **Interest and budget.** A relevancy hook decides per client which entities exist for it (distance or custom); a per-client byte budget limits a tick's snapshot, stalest data first.

### Prediction and reconciliation (optional)

For the entity a client owns (a character), the client applies its own input at once and sends it tick-stamped. The server applies the same inputs through the same movement function and sends back the authoritative state with "the last input I applied". The client compares, and if the error is above a threshold, rewinds to the server state and re-applies the inputs not yet acknowledged; the visible error is smoothed out over a few frames instead of snapping. `Predicted<T>` is a hook for custom components; `CharacterMotor` support is built on `move_character` (feature `physics`). Lag compensation (the server rewinds colliders for hitscan) is an interface (`LagCompensation`) with a simple history-of-poses implementation.

### Scripting

`ctx:is_server()`, `ctx:is_client()`, `ctx:local_client()`, `ctx:owner()`, `ctx:net_send(target, name, value)` and a callback `on_net_message(self, ctx, name, value, from)`. Values are Lua primitives and tables of them (no functions, no userdata); size and rate are capped. A Lua-only game can be multiplayer. Lua messages go through the same command path, so the same validation and rate limits apply.

## Considered options

### Own protocol on QUIC (chosen)

Pros: the protocol is shaped by the engine (names, registry hash, `Events<T>`, prefabs); TLS is not ours to write; channels map to QUIC one to one; one design for every layer. Cons: we own the replication code and its bugs; QUIC is heavier than raw UDP for a small game; `quinn` brings an async runtime (hidden).

### renet / laminar (transports)

Small UDP libraries with channels and reliability. Pros: light, simple, proven in many games. Cons: **no encryption in the library itself** (`renet` has a separate `renet_netcode` with netcode.io tokens, which need a token-issuing service; `laminar` has none), so N2 would be ours; their reliability layers are their own. We would still write everything above L0. They remain possible later as `Transport` implementations; the trait is the point.

### naia / lightyear / bevy_replicon (replication frameworks)

Ready-made replication, prediction and interpolation. Pros: a lot of finished work; `lightyear` has prediction, interpolation and lag compensation. Cons: all three are built around Bevy's ECS (`naia` and `lightyear` hard, `bevy_replicon` entirely) and its scheduling; we use `hecs` and our own `App`. Adapting means maintaining a shim as large as our own code, and we would not own the wire format or the security model. Their *designs* did inform ours: `bevy_replicon` for "replication rules are a registry, authority is the server", `lightyear` for the prediction and interpolation split, Gaffer On Games and the Source engine for snapshots and acks.

### Rollback vs snapshot interpolation vs lockstep

- **Snapshot interpolation with server authority (chosen).** Fits co-op and arenas, handles 2 to 32 players, cheaters have little to win, a join is "send the state". Cost: latency shows up as the interpolation delay, and the owner needs prediction to feel responsive.
- **Rollback (GGPO-style).** Needs a fully deterministic, snapshot-able simulation and a cheap re-simulation. Rapier here is not deterministic across machines in our setup and rewinding it is not supported. It is the right tool for fighting games and is **out of scope**.
- **Deterministic lockstep.** Needs the same determinism and gives the slowest peer's latency to everybody; strategy games only. **Out of scope.**

## Out of scope

- A matchmaking service, lobbies on a server, accounts.
- NAT punching and relays. QUIC copes with ordinary NATs when the host forwards a port; otherwise players use a VPN or a relay of their own.
- Rollback netcode for fighting games; lockstep.
- Voice chat, large-world sharding (hundreds of players), anti-cheat beyond server authority and validation.
- A CI check that no server secret ends up in a client binary (the rule is documented, the script is a later task).
- Web (WebTransport). The `Transport` trait allows it; no implementation.

## Consequences

- Two more dependency families behind `quic`: `quinn`/`rustls`/`ring`/`tokio`, and `rcgen` for certificates. Default build adds `postcard`, `sha2`, `hmac`, `getrandom`.
- Component and message **names are the protocol**. Renaming a type in code is fine, renaming its registration is a protocol change; the registry hash makes a mismatch loud.
- The first version has no delta *compression* inside a component (a changed `Transform` is sent whole, quantised). Fine for typical games; large components should be split.
- Prediction covers one owned entity per client by default. Predicting physics props the player pushes is not covered.
- Lua messages are slower than Rust ones; a game with a hundred scripted messages per tick should use Rust for the hot ones.
