//! Navigation: bake the walkable surfaces of a level into a grid, find paths.
//!
//! [`NavGrid::bake`] takes the level's triangles and an agent's size
//! ([`NavConfig`]). A vertical ray through the centre of every grid cell finds
//! the surfaces: a hit is *walkable* when its slope is gentle enough, has head
//! room for the agent, and is not next to a wall or a drop closer than the
//! agent's radius. Several floors above each other are separate layers.
//!
//! [`NavGrid::find_path`] runs A* over the layers (8 neighbours, no cutting
//! corners, steps up to `max_step`) and straightens the result by line of
//! sight, so an agent walks in straight lines instead of along the grid.
//! Moving obstacles are [`NavGrid::add_blocker`] discs that need no re-bake.
//!
//! The crate knows nothing about the engine: triangles in, points out.

mod bake;
mod path;
#[cfg(feature = "engine")]
mod plugin;

pub use bake::{BakeError, MAX_CELLS};
use glam::{Vec2, Vec3};

#[cfg(feature = "engine")]
pub use plugin::{NavAgent, NavPlugin, NavStatus, Navigation, collect_triangles};

/// One triangle of level geometry.
pub type Triangle = [Vec3; 3];

/// The agent and the grid it is baked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavConfig {
    /// Side of a grid cell, metres. Smaller is more exact and slower.
    pub cell: f32,
    /// How close to a wall or an edge the agent's centre may be.
    pub agent_radius: f32,
    /// Head room needed above the floor.
    pub agent_height: f32,
    /// The highest step the agent climbs.
    pub max_step: f32,
    /// The steepest walkable slope, degrees.
    pub max_slope_deg: f32,
}

impl Default for NavConfig {
    fn default() -> Self {
        Self {
            cell: 0.25,
            agent_radius: 0.4,
            agent_height: 1.8,
            max_step: 0.4,
            max_slope_deg: 45.0,
        }
    }
}

/// A walkable spot: where a cell's centre meets a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Node {
    pub x: u32,
    pub z: u32,
    pub y: f32,
    /// Moving obstacles covering this node (0 = free).
    pub blocked: u16,
}

/// Handle of a blocker, for [`NavGrid::remove_blocker`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockerId(u32);

/// The baked grid.
#[derive(Clone, Debug)]
pub struct NavGrid {
    pub(crate) config: NavConfig,
    /// World X and Z of the corner of cell (0, 0).
    pub(crate) origin: Vec2,
    pub(crate) width: u32,
    pub(crate) depth: u32,
    /// `cell_start[z * width + x] .. cell_start[.. + 1]` are that cell's nodes, lowest first.
    pub(crate) cell_start: Vec<u32>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) blockers: Vec<Option<Vec<u32>>>,
}

impl NavGrid {
    pub fn config(&self) -> &NavConfig {
        &self.config
    }

    /// Number of walkable spots.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Grid size in cells (X, Z).
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.depth)
    }

    /// World position of a node (centre of its cell, on the surface).
    pub(crate) fn pos(&self, node: u32) -> Vec3 {
        let n = &self.nodes[node as usize];
        Vec3::new(
            self.origin.x + (n.x as f32 + 0.5) * self.config.cell,
            n.y,
            self.origin.y + (n.z as f32 + 0.5) * self.config.cell,
        )
    }

    pub(crate) fn cell_nodes(&self, x: u32, z: u32) -> std::ops::Range<u32> {
        let i = (z * self.width + x) as usize;
        self.cell_start[i]..self.cell_start[i + 1]
    }

    /// The cell holding world X, Z, if it is inside the grid.
    pub(crate) fn cell_of(&self, x: f32, z: f32) -> Option<(u32, u32)> {
        let cx = ((x - self.origin.x) / self.config.cell).floor();
        let cz = ((z - self.origin.y) / self.config.cell).floor();
        if cx < 0.0 || cz < 0.0 || cx >= self.width as f32 || cz >= self.depth as f32 {
            return None;
        }
        Some((cx as u32, cz as u32))
    }

    /// Blocks every node within `radius` of `center` (a door, a crate, a
    /// monster that stands still) until [`NavGrid::remove_blocker`].
    pub fn add_blocker(&mut self, center: Vec3, radius: f32, height: f32) -> BlockerId {
        let cell = self.config.cell;
        let mut hit = Vec::new();
        let lo = self.cell_of(center.x - radius, center.z - radius);
        let hi = self.cell_of(center.x + radius, center.z + radius);
        let (x0, z0) = lo.unwrap_or((0, 0));
        let (x1, z1) = hi.unwrap_or((self.width - 1, self.depth - 1));
        for z in z0..=z1 {
            for x in x0..=x1 {
                for n in self.cell_nodes(x, z) {
                    let p = self.pos(n);
                    let flat = Vec2::new(p.x - center.x, p.z - center.z).length();
                    if flat <= radius + cell * 0.5
                        && p.y >= center.y - self.config.max_step
                        && p.y <= center.y + height
                    {
                        self.nodes[n as usize].blocked += 1;
                        hit.push(n);
                    }
                }
            }
        }
        self.blockers.push(Some(hit));
        BlockerId(self.blockers.len() as u32 - 1)
    }

    /// Frees what [`NavGrid::add_blocker`] blocked. False if already removed.
    pub fn remove_blocker(&mut self, id: BlockerId) -> bool {
        let Some(list) = self.blockers.get_mut(id.0 as usize).and_then(Option::take) else {
            return false;
        };
        for n in list {
            let node = &mut self.nodes[n as usize];
            node.blocked = node.blocked.saturating_sub(1);
        }
        true
    }
}

#[cfg(test)]
mod tests;
