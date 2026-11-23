//! Server side of replication: who exists for whom, and what changed.

use std::collections::{BTreeMap, HashMap};

use olvin_ecs::{Entity, World};
use olvin_scene::{PrefabInstanceOf, SceneId};

use super::{ErasedComponent, NetId, ReplicateMode, Replicated, valid_prefab_path};
use crate::config::NetConfig;
use crate::net::{ClientId, Net};
use crate::proto::{CompData, Msg, SnapEntry, SnapshotMsg, SpawnKind, SpawnMsg};
use crate::server::ServerState;
use crate::transport::Channel;
use crate::wire;

/// Decides which entities a client gets: `(world, client, entity) -> relevant`.
pub type RelevancyFn = Box<dyn Fn(&World, ClientId, Entity) -> bool>;

const SENT_LOG: usize = 256;
/// Spawns one client receives per tick (a late joiner gets the world over a few ticks).
const SPAWNS_PER_TICK: usize = 128;
const CHUNKS_PER_TICK: usize = 8;
/// Bytes a snapshot packet spends on framing, tag, tick and counts.
const CHUNK_OVERHEAD: usize = 48;

struct CompState {
    bytes: Option<Vec<u8>>,
    /// Tick this value last changed.
    changed: u32,
}

struct ServerEnt {
    entity: Entity,
    kind: SpawnKind,
    comps: Vec<CompState>,
}

struct KnownEnt {
    /// Version (tick of change) of each component the client confirmed.
    acked: Vec<u32>,
    /// `(version, tick)` of the last send of each component.
    last_sent: Vec<(u32, u32)>,
}

#[derive(Default)]
struct ClientView {
    known: HashMap<u64, KnownEnt>,
    /// Snapshot bytes this client may still be sent.
    credit: f32,
    next_seq: u32,
    /// What each recent snapshot carried, to turn an ack into per-component confirmations.
    sent: Vec<Option<Vec<(u64, u16, u32)>>>,
}

#[derive(Default)]
pub(crate) struct ReplServer {
    next_net_id: u64,
    ents: BTreeMap<u64, ServerEnt>,
    views: HashMap<ClientId, ClientView>,
}

impl ReplServer {
    /// A client (re)connected: it knows nothing yet.
    pub fn reset_client(&mut self, id: ClientId) {
        self.views.remove(&id);
    }

    pub fn len(&self) -> usize {
        self.ents.len()
    }

    pub fn entity(&self, id: NetId) -> Option<Entity> {
        self.ents.get(&id.0).map(|e| e.entity)
    }

    /// Records a client's acknowledgement of snapshots.
    pub fn on_ack(&mut self, client: ClientId, seq: u32, bits: u32) {
        let Some(view) = self.views.get_mut(&client) else {
            return;
        };
        // Only sequence numbers we actually sent, and still in the log.
        if seq >= view.next_seq || view.next_seq - seq > SENT_LOG as u32 {
            return;
        }
        let confirm = |s: u32, view: &mut ClientView| {
            if s >= view.next_seq || view.next_seq - s > SENT_LOG as u32 {
                return;
            }
            let Some(slot) = view.sent.get_mut(s as usize % SENT_LOG) else {
                return;
            };
            for (id, comp, version) in slot.take().unwrap_or_default() {
                if let Some(k) = view.known.get_mut(&id)
                    && let Some(a) = k.acked.get_mut(comp as usize)
                {
                    *a = (*a).max(version);
                }
            }
        };
        confirm(seq, view);
        for i in 0..32u32 {
            if bits & (1 << i) != 0
                && let Some(s) = seq.checked_sub(1 + i)
            {
                confirm(s, view);
            }
        }
    }
}

fn kind_of(world: &World, e: Entity, marker: &Replicated) -> SpawnKind {
    if let Some(path) = &marker.prefab {
        if valid_prefab_path(path) {
            return SpawnKind::Prefab(path.clone());
        }
        log::warn!("net: Replicated.prefab `{path}` is not a relative *.prefab.ron path; ignored");
    }
    if let Ok(p) = world.get::<&PrefabInstanceOf>(e) {
        if valid_prefab_path(&p.0) {
            return SpawnKind::Prefab(p.0.clone());
        }
        log::warn!(
            "net: prefab `{}` cannot be replicated (path not relative *.prefab.ron)",
            p.0
        );
    }
    if let Ok(s) = world.get::<&SceneId>(e) {
        return SpawnKind::Scene(s.0);
    }
    SpawnKind::Bare
}

impl Net {
    /// Runs the server's fixed ticks that `dt` seconds of frame time are worth
    /// (at most a few per frame, so a hitch does not snowball).
    pub(crate) fn server_ticks(&mut self, world: &mut World, dt: f32) {
        let step = 1.0 / self.cfg.tick_rate.max(1) as f32;
        let Some(server) = self.server.as_mut() else {
            return;
        };
        server.tick_accum = (server.tick_accum + dt.max(0.0)).min(step * 5.0);
        while self.server.as_ref().is_some_and(|s| s.tick_accum >= step) {
            if let Some(s) = self.server.as_mut() {
                s.tick_accum -= step;
            }
            self.replicate_tick(world);
        }
    }

    /// One server tick of replication: find new and gone entities, refresh
    /// component states, and send each remote client its spawns, despawns and
    /// snapshot. Runs at `tick_rate` after gameplay.
    pub(crate) fn replicate_tick(&mut self, world: &mut World) {
        let Net {
            cfg,
            registry,
            server,
            repl,
            relevancy,
            ..
        } = self;
        let (Some(server), Some(rs)) = (server.as_mut(), repl.server.as_mut()) else {
            return;
        };
        server.tick = server.tick.wrapping_add(1).max(1);
        let tick = server.tick;
        let comps = &registry.components;

        // New networked objects get an id; vanished ones are noted.
        let fresh: Vec<(Entity, Replicated)> = world
            .query::<(Entity, &Replicated)>()
            .without::<&NetId>()
            .iter()
            .map(|(e, r)| (e, r.clone()))
            .collect();
        for (e, marker) in fresh {
            if rs.ents.len() >= cfg.limits.max_entities {
                log::warn!(
                    "net: more than {} replicated entities, ignoring the rest",
                    cfg.limits.max_entities
                );
                break;
            }
            rs.next_net_id += 1;
            let id = rs.next_net_id;
            let _ = world.insert_one(e, NetId(id));
            rs.ents.insert(
                id,
                ServerEnt {
                    entity: e,
                    kind: kind_of(world, e, &marker),
                    comps: (0..comps.len())
                        .map(|_| CompState {
                            bytes: None,
                            changed: 0,
                        })
                        .collect(),
                },
            );
        }
        let gone: Vec<u64> = rs
            .ents
            .iter()
            .filter(|(_, ent)| {
                !world.contains(ent.entity) || world.get::<&Replicated>(ent.entity).is_err()
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &gone {
            if let Some(ent) = rs.ents.remove(id) {
                // Drop the id so a re-added marker is a new object.
                let _ = world.remove_one::<NetId>(ent.entity);
            }
        }

        refresh_states(rs, world, comps, tick);

        let clients: Vec<ClientId> = server
            .clients
            .values()
            .filter(|c| c.peer.is_some())
            .map(|c| c.id)
            .collect();
        rs.views.retain(|id, _| clients.contains(id));
        for client in clients {
            send_client(
                rs,
                server,
                cfg,
                comps,
                relevancy.as_ref(),
                world,
                client,
                tick,
                &gone,
            );
        }
    }
}

fn refresh_states(
    rs: &mut ReplServer,
    world: &World,
    comps: &[Box<dyn ErasedComponent>],
    tick: u32,
) {
    for ent in rs.ents.values_mut() {
        for (ci, comp) in comps.iter().enumerate() {
            let state = &mut ent.comps[ci];
            match comp.mode() {
                ReplicateMode::Interval(n) if !tick.is_multiple_of(n.max(1)) => continue,
                ReplicateMode::Always => {
                    let bytes = comp.encode(world, ent.entity);
                    if bytes.is_some() || state.bytes.is_some() {
                        state.bytes = bytes;
                        state.changed = tick;
                    }
                    continue;
                }
                _ => {}
            }
            let bytes = comp.encode(world, ent.entity);
            if bytes != state.bytes {
                state.bytes = bytes;
                state.changed = tick;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn send_client(
    rs: &mut ReplServer,
    server: &mut ServerState,
    cfg: &NetConfig,
    comps: &[Box<dyn ErasedComponent>],
    relevancy: Option<&RelevancyFn>,
    world: &World,
    client: ClientId,
    tick: u32,
    gone: &[u64],
) {
    let view = rs.views.entry(client).or_default();
    if view.sent.is_empty() {
        view.sent = vec![None; SENT_LOG];
    }
    // Despawns: destroyed entities, and entities that stopped being relevant.
    let mut despawn: Vec<u64> = gone
        .iter()
        .copied()
        .filter(|id| view.known.contains_key(id))
        .collect();
    let mut spawn = Vec::new();
    for (id, ent) in &rs.ents {
        let visible = relevancy.is_none_or(|f| f(world, client, ent.entity));
        match (visible, view.known.contains_key(id)) {
            (true, false) => spawn.push(*id),
            (false, true) => despawn.push(*id),
            _ => {}
        }
    }
    for id in despawn {
        view.known.remove(&id);
        server.send_to(client, Channel::Reliable, &Msg::Despawn { net_id: id });
    }
    for id in spawn.into_iter().take(SPAWNS_PER_TICK) {
        let Some(ent) = rs.ents.get(&id) else {
            continue;
        };
        let mut known = KnownEnt {
            acked: vec![0; comps.len()],
            last_sent: vec![(0, 0); comps.len()],
        };
        let mut data = Vec::new();
        for (ci, st) in ent.comps.iter().enumerate() {
            if let Some(bytes) = &st.bytes {
                data.push(CompData {
                    comp: ci as u16,
                    data: bytes.clone(),
                });
                known.acked[ci] = st.changed;
            }
        }
        let msg = Msg::Spawn(SpawnMsg {
            net_id: id,
            tick,
            kind: ent.kind.clone(),
            comps: data,
        });
        view.known.insert(id, known);
        server.send_to(client, Channel::Reliable, &msg);
    }

    // Snapshot: whatever the client has not confirmed, stalest first, within budget.
    let per_tick = cfg.bandwidth as f32 / cfg.tick_rate.max(1) as f32;
    view.credit = (view.credit + per_tick).min(per_tick * 2.0 + 1500.0);
    let rtt_ticks = server
        .clients
        .get(&client)
        .map_or(0.0, |c| c.rtt * cfg.tick_rate as f32);
    let resend_after = (rtt_ticks * 1.5).ceil() as u32 + 2;
    let mut due: Vec<(u32, u64, u16)> = Vec::new();
    for (id, k) in &view.known {
        let Some(ent) = rs.ents.get(id) else {
            continue;
        };
        for (ci, st) in ent.comps.iter().enumerate() {
            if st.changed <= k.acked[ci] {
                continue;
            }
            let (sent_version, sent_tick) = k.last_sent[ci];
            if sent_version == st.changed && tick.wrapping_sub(sent_tick) < resend_after {
                continue;
            }
            due.push((k.acked[ci], *id, ci as u16));
        }
    }
    if due.is_empty() {
        return;
    }
    due.sort_unstable();
    let limit = server.max_datagram(client).saturating_sub(CHUNK_OVERHEAD);
    let mut entries: Vec<SnapEntry> = Vec::new();
    let mut log: Vec<(u64, u16, u32)> = Vec::new();
    let mut size = 0usize;
    let mut chunks = 0;
    let flush = |entries: &mut Vec<SnapEntry>,
                 log: &mut Vec<(u64, u16, u32)>,
                 size: &mut usize,
                 view: &mut ClientView,
                 server: &mut ServerState| {
        if entries.is_empty() {
            return;
        }
        let seq = view.next_seq;
        view.next_seq = view.next_seq.wrapping_add(1);
        let msg = Msg::Snapshot(SnapshotMsg {
            seq,
            tick,
            entries: std::mem::take(entries),
        });
        server.send_to(client, Channel::Unreliable, &msg);
        view.credit -= *size as f32 + CHUNK_OVERHEAD as f32;
        view.sent[seq as usize % SENT_LOG] = Some(std::mem::take(log));
        *size = 0;
    };
    for (_, id, ci) in due {
        if view.credit <= 0.0 || chunks >= CHUNKS_PER_TICK {
            break;
        }
        let Some(ent) = rs.ents.get(&id) else {
            continue;
        };
        let st = &ent.comps[ci as usize];
        let entry = SnapEntry {
            net_id: id,
            comp: ci,
            data: st.bytes.clone(),
        };
        let entry_size = wire::encode(&entry).map_or(usize::MAX, |b| b.len());
        let Some(k) = view.known.get_mut(&id) else {
            continue;
        };
        k.last_sent[ci as usize] = (st.changed, tick);
        if entry_size > limit {
            // Too big for a datagram: send it on the reliable channel, once.
            let msg = Msg::Snapshot(SnapshotMsg {
                seq: u32::MAX,
                tick,
                entries: vec![entry],
            });
            server.send_to(client, Channel::Reliable, &msg);
            k.acked[ci as usize] = st.changed;
            view.credit -= entry_size as f32;
            continue;
        }
        if size + entry_size > limit {
            flush(&mut entries, &mut log, &mut size, view, server);
            chunks += 1;
            if chunks >= CHUNKS_PER_TICK || view.credit <= 0.0 {
                break;
            }
        }
        size += entry_size;
        log.push((id, ci, st.changed));
        entries.push(entry);
    }
    flush(&mut entries, &mut log, &mut size, view, server);
}
