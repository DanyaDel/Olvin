# Networking (planned / in progress)

> Status: **in progress**, branch `feat-net`. Decision and rationale: `docs/adr/0003-netcode.en.md` (ADR 0003). This page is updated with the code; what exists and what does not is listed at the end.

Goal: **the netcode is insanely flexible and still not hard to write.** `olvin_net` is built in layers and every layer can be used alone.

| Layer | What it gives |
|---|---|
| L0 Transport | reliable and unreliable channels. `Loopback` in-process with simulated latency, loss, jitter; QUIC (`quinn`, TLS) behind the `quic` feature |
| L1 Session | modes `Offline`, `Host`, `Dedicated`, `Client`; join by address or a short room code; handshake, kick, timeouts |
| L2 Replication | mark a prefab `Replicated`, register components, and the object appears for everybody |
| L3 Messages | client commands validated on the server, rate and size limits |
| L4 Prediction | your own character responds at once, the server is still the boss |
| L5 Scripting | `ctx:net_send`, `on_net_message`: multiplayer in Lua alone |
| L6 Tools | RTT, loss, bandwidth in the profiler; "lag 80 ms, loss 2%" in Play |

## Principles

- **The server is the boss.** Only the server writes replicated state. A client sends commands ("move there", "fire") and the server validates them.
- **Offline is networked too.** A single-player game runs the same code over `Loopback`, so every game is multiplayer-ready.
- **Everything is bounded.** Packet sizes, message rates, client counts; garbage bytes do not crash the server.
- **Security by construction:** TLS 1.3 (QUIC), the certificate is pinned by the fingerprint in the room code, nonces and timestamps against replay, a key per session.

## What a game will look like

A short example will appear on this page when the layers work (quick start: `apps/netdemo`).

## What exists

So far only the design (`docs/adr/0003-netcode.en.md` (ADR 0003)). The rest is ticked here as code lands.

## What is not planned

Matchmaking, NAT punching, rollback for fighting games, lockstep. Details in the ADR.

## See also

- [Time and events](time-events.md)
- [Scripting](scripting.md)
- [Prefabs](prefabs.md)
- [States and saves](game-states-saves.md)
