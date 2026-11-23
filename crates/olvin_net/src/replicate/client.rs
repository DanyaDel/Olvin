//! Client side of replication: makes and updates the copies.

use std::collections::HashMap;

use olvin_core::Resources;
use olvin_ecs::{Entity, World};
use olvin_scene::{SceneId, Transform};

use super::{Interpolated, NetId, Remote, Replicated, tick_time, valid_prefab_path};
use crate::net::Net;
use crate::proto::{Msg, SnapEntry, SnapshotMsg, SpawnKind, SpawnMsg};
use crate::transport::Channel;

/// Instantiates a prefab asset (path relative to `assets/`) for a replicated
/// entity. The facade provides one over `scene_file::instantiate_prefab`.
pub type PrefabSpawner = Box<dyn Fn(&Resources, &str, Transform) -> Result<Entity, String>>;

/// Updates that arrive before their spawn wait this long, seconds.
const EARLY_TTL: f64 = 2.0;
const MAX_EARLY: usize = 512;

#[derive(Default)]
pub(crate) struct ReplClient {
    /// Net id to the local copy.
    entities: HashMap<u64, Entity>,
    /// Per entity, per component: server tick of the newest value applied.
    last_tick: HashMap<u64, Vec<u32>>,
    /// Snapshot entries for entities whose spawn has not arrived: (arrival time, tick, entry).
    early: Vec<(f64, u32, SnapEntry)>,
    have_seq: bool,
    recv_highest: u32,
    /// Bit `i` set: snapshot `recv_highest - i` arrived.
    recv_mask: u64,
    ack_due: bool,
    /// Estimated `server_time - local_time`.
    offset: Option<f64>,
    pub needs_clear: bool,
}

impl ReplClient {
    pub fn entity(&self, id: NetId) -> Option<Entity> {
        self.entities.get(&id.0).copied()
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    fn note_seq(&mut self, seq: u32) {
        if !self.have_seq {
            self.have_seq = true;
            self.recv_highest = seq;
            self.recv_mask = 1;
        } else if seq > self.recv_highest {
            let shift = seq - self.recv_highest;
            self.recv_mask = if shift >= 64 {
                0
            } else {
                self.recv_mask << shift
            } | 1;
            self.recv_highest = seq;
        } else if self.recv_highest - seq < 64 {
            self.recv_mask |= 1 << (self.recv_highest - seq);
        }
        self.ack_due = true;
    }
}

impl Net {
    /// Handles what the server sent this frame and keeps remote entities current.
    pub(crate) fn client_replicate(&mut self, res: &Resources) {
        let inbox = match self.client.as_mut() {
            Some(c) => std::mem::take(&mut c.inbox),
            None => return,
        };
        if self.repl.client.needs_clear {
            self.repl.client.needs_clear = false;
            self.clear_replicated(&mut res.fetch_mut::<World>());
        }
        for (_channel, msg) in inbox {
            match msg {
                Msg::Spawn(m) => self.on_spawn(res, m),
                Msg::Despawn { net_id } => self.on_despawn(&mut res.fetch_mut::<World>(), net_id),
                Msg::Snapshot(m) => self.on_snapshot(&mut res.fetch_mut::<World>(), m),
                _ => {}
            }
        }
        // Interpolate, then confirm what arrived.
        let render_time =
            self.now + self.repl.client.offset.unwrap_or(0.0) - self.cfg.interp_delay as f64;
        {
            let mut world = res.fetch_mut::<World>();
            for comp in self.registry.components.iter_mut() {
                comp.interpolate(&mut world, &self.repl.client.entities, render_time);
            }
        }
        if self.repl.client.ack_due {
            self.repl.client.ack_due = false;
            let ack = Msg::Ack {
                seq: self.repl.client.recv_highest,
                bits: (self.repl.client.recv_mask >> 1) as u32,
            };
            if let Some(c) = self.client.as_mut() {
                c.send(Channel::Unreliable, &ack);
            }
        }
    }

    /// Despawns every replicated copy (connection lost).
    pub(crate) fn clear_replicated(&mut self, world: &mut World) {
        for (id, e) in std::mem::take(&mut self.repl.client.entities) {
            let _ = world.despawn(e);
            for comp in self.registry.components.iter_mut() {
                comp.forget(id);
            }
        }
        self.repl.client.last_tick.clear();
        self.repl.client.early.clear();
        self.repl.client.have_seq = false;
        self.repl.client.offset = None;
    }

    fn on_spawn(&mut self, res: &Resources, msg: SpawnMsg) {
        let id = msg.net_id;
        let ncomps = self.registry.components.len();
        if msg.comps.iter().any(|c| c.comp as usize >= ncomps) {
            log::warn!("net: Spawn {id} names a component this build does not have; ignored");
            return;
        }
        let known = self.repl.client.entities.get(&id).copied();
        let alive = known.filter(|e| res.fetch::<World>().contains(*e));
        let entity = match alive {
            Some(e) => e,
            None => {
                if self.repl.client.entities.len() >= self.cfg.limits.max_entities {
                    log::warn!(
                        "net: the server replicates more than {} entities; ignoring a spawn",
                        self.cfg.limits.max_entities
                    );
                    return;
                }
                match self.make_copy(res, &msg.kind) {
                    Some(e) => e,
                    None => return,
                }
            }
        };
        let mut world = res.fetch_mut::<World>();
        let mut interpolated = false;
        let time = tick_time(msg.tick, self.cfg.tick_rate);
        let last = self
            .repl
            .client
            .last_tick
            .entry(id)
            .or_insert_with(|| vec![0; ncomps]);
        for c in &msg.comps {
            let comp = &mut self.registry.components[c.comp as usize];
            interpolated |= comp.interpolated();
            match comp.apply(&mut world, entity, id, time, Some(&c.data)) {
                Ok(()) => last[c.comp as usize] = msg.tick,
                Err(e) => log::warn!("net: bad `{}` in Spawn {id}: {e}", comp.name()),
            }
        }
        let _ = world.insert(entity, (NetId(id), Remote, Replicated::default()));
        if interpolated {
            let _ = world.insert_one(entity, Interpolated);
        }
        self.repl.client.entities.insert(id, entity);
        // Updates that overtook the spawn.
        let early = std::mem::take(&mut self.repl.client.early);
        for (at, tick, entry) in early {
            if entry.net_id == id {
                self.apply_entry(&mut world, tick, &entry);
            } else {
                self.repl.client.early.push((at, tick, entry));
            }
        }
    }

    /// Creates the local copy of a replicated entity.
    fn make_copy(&mut self, res: &Resources, kind: &SpawnKind) -> Option<Entity> {
        match kind {
            SpawnKind::Prefab(path) => {
                if !valid_prefab_path(path) {
                    log::warn!(
                        "net: the server asked for prefab `{path}`, which is not a relative *.prefab.ron path; ignored"
                    );
                    return None;
                }
                match &self.spawner {
                    Some(spawn) => match spawn(res, path, Transform::IDENTITY) {
                        Ok(e) => Some(e),
                        Err(err) => {
                            log::warn!(
                                "net: cannot spawn prefab `{path}`: {err}; using an empty entity"
                            );
                            Some(res.fetch_mut::<World>().spawn(()))
                        }
                    },
                    None => {
                        log::warn!(
                            "net: the server replicates prefab `{path}` but no prefab spawner is set \
                             (NetPlugin needs the `scene` glue, see olvin::net); using an empty entity"
                        );
                        Some(res.fetch_mut::<World>().spawn(()))
                    }
                }
            }
            SpawnKind::Scene(sid) => {
                let world = res.fetch::<World>();
                let found = world
                    .query::<(Entity, &SceneId)>()
                    .iter()
                    .find(|(_, s)| s.0 == *sid)
                    .map(|(e, _)| e);
                drop(world);
                match found {
                    Some(e) => Some(e),
                    None => {
                        log::warn!(
                            "net: scene object {sid} is not in this scene; using an empty entity"
                        );
                        Some(res.fetch_mut::<World>().spawn(()))
                    }
                }
            }
            SpawnKind::Bare => Some(res.fetch_mut::<World>().spawn(())),
        }
    }

    fn on_despawn(&mut self, world: &mut World, id: u64) {
        if let Some(e) = self.repl.client.entities.remove(&id) {
            let _ = world.despawn(e);
        }
        self.repl.client.last_tick.remove(&id);
        self.repl.client.early.retain(|(_, _, e)| e.net_id != id);
        for comp in self.registry.components.iter_mut() {
            comp.forget(id);
        }
    }

    fn on_snapshot(&mut self, world: &mut World, msg: SnapshotMsg) {
        let now = self.now;
        if msg.seq != u32::MAX {
            self.repl.client.note_seq(msg.seq);
            // Clock estimate for interpolation: the least-delayed snapshot is the truest.
            let est = tick_time(msg.tick, self.cfg.tick_rate) - now;
            let c = &mut self.repl.client;
            c.offset = Some(match c.offset {
                None => est,
                Some(o) if est > o => est,
                Some(o) => o + (est - o) * 0.02,
            });
            if let Some(cl) = self.client.as_mut() {
                cl.server_tick = cl.server_tick.max(msg.tick);
            }
        }
        for entry in &msg.entries {
            if !self.repl.client.entities.contains_key(&entry.net_id) {
                // Spawn not here yet (it travels on another stream): keep it briefly.
                let early = &mut self.repl.client.early;
                early.retain(|(at, ..)| now - at < EARLY_TTL);
                if early.len() >= MAX_EARLY {
                    early.remove(0);
                }
                early.push((now, msg.tick, entry.clone()));
                continue;
            }
            self.apply_entry(world, msg.tick, entry);
        }
    }

    fn apply_entry(&mut self, world: &mut World, tick: u32, entry: &SnapEntry) {
        let ncomps = self.registry.components.len();
        let ci = entry.comp as usize;
        let Some(&entity) = self.repl.client.entities.get(&entry.net_id) else {
            return;
        };
        if ci >= ncomps || !world.contains(entity) {
            return;
        }
        let last = self
            .repl
            .client
            .last_tick
            .entry(entry.net_id)
            .or_insert_with(|| vec![0; ncomps]);
        // Older or repeated: a late datagram must not undo newer state.
        if tick <= last[ci] {
            return;
        }
        let time = tick_time(tick, self.cfg.tick_rate);
        let comp = &mut self.registry.components[ci];
        match comp.apply(world, entity, entry.net_id, time, entry.data.as_deref()) {
            Ok(()) => {
                last[ci] = tick;
                if comp.interpolated() && entry.data.is_some() {
                    let _ = world.insert_one(entity, Interpolated);
                }
            }
            Err(e) => log::warn!("net: bad `{}` in a snapshot: {e}", comp.name()),
        }
    }
}
