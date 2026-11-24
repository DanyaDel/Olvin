//! Queries: nearest node, A*, line of sight.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

use glam::Vec3;

use crate::NavGrid;

/// A search that expands more nodes than this gives up (a hitch is worse than "no path").
pub const MAX_EXPANDED: usize = 400_000;

const NEIGHBOURS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// Per-thread search state, reused between queries: a node is "seen" when its
/// stamp is the current generation, so nothing is cleared between searches.
#[derive(Default)]
struct Scratch {
    stamp: Vec<u32>,
    g: Vec<f32>,
    parent: Vec<u32>,
    generation: u32,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

impl Scratch {
    fn begin(&mut self, nodes: usize) {
        if self.stamp.len() < nodes {
            self.stamp.resize(nodes, 0);
            self.g.resize(nodes, 0.0);
            self.parent.resize(nodes, 0);
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamp.fill(0);
            self.generation = 1;
        }
    }

    fn seen(&self, node: u32) -> Option<f32> {
        (self.stamp[node as usize] == self.generation).then(|| self.g[node as usize])
    }

    fn g(&self, node: u32) -> f32 {
        self.g[node as usize]
    }

    fn parent(&self, node: u32) -> u32 {
        self.parent[node as usize]
    }

    fn set(&mut self, node: u32, g: f32, parent: u32) {
        let i = node as usize;
        self.stamp[i] = self.generation;
        self.g[i] = g;
        self.parent[i] = parent;
    }
}

struct Open {
    f: f32,
    node: u32,
}

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f
    }
}
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Open {
    // Reversed: the smallest `f` pops first.
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f)
    }
}

impl NavGrid {
    /// The free node nearest to `p` within `max_distance` (cells are searched
    /// ring by ring around it).
    pub fn nearest_node(&self, p: Vec3, max_distance: f32) -> Option<u32> {
        let cell = self.config.cell;
        let rings = (max_distance / cell).ceil() as i32;
        let cx = ((p.x - self.origin.x) / cell).floor() as i32;
        let cz = ((p.z - self.origin.y) / cell).floor() as i32;
        let mut best: Option<(f32, u32)> = None;
        for dz in -rings..=rings {
            for dx in -rings..=rings {
                let (x, z) = (cx + dx, cz + dz);
                if x < 0 || z < 0 || x >= self.width as i32 || z >= self.depth as i32 {
                    continue;
                }
                for n in self.cell_nodes(x as u32, z as u32) {
                    if self.nodes[n as usize].blocked > 0 {
                        continue;
                    }
                    let dist = self.pos(n).distance(p);
                    if dist <= max_distance && best.is_none_or(|(b, _)| dist < b) {
                        best = Some((dist, n));
                    }
                }
            }
        }
        best.map(|(_, n)| n)
    }

    /// The node of cell (x, z) that `from` can step to: the free one nearest
    /// in height, within the step the agent climbs.
    fn step_to(&self, from_y: f32, x: i32, z: i32, extra: f32) -> Option<u32> {
        if x < 0 || z < 0 || x >= self.width as i32 || z >= self.depth as i32 {
            return None;
        }
        let limit = self.config.max_step + extra;
        self.cell_nodes(x as u32, z as u32)
            .filter(|&n| self.nodes[n as usize].blocked == 0)
            .map(|n| ((self.nodes[n as usize].y - from_y).abs(), n))
            .filter(|&(dy, _)| dy <= limit)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, n)| n)
    }

    fn neighbours(&self, node: u32) -> impl Iterator<Item = (u32, f32)> + '_ {
        let n = self.nodes[node as usize];
        let slope = self.config.max_slope_deg.to_radians().tan();
        NEIGHBOURS.iter().filter_map(move |&(dx, dz)| {
            let (x, z) = (n.x as i32 + dx, n.z as i32 + dz);
            let reach = if dx != 0 && dz != 0 { 1.4143 } else { 1.0 };
            let target = self.step_to(n.y, x, z, (reach - 1.0) * self.config.cell * slope)?;
            if dx != 0 && dz != 0 {
                // No cutting a corner: both cells beside the diagonal must be passable.
                self.step_to(n.y, n.x as i32 + dx, n.z as i32, 0.0)?;
                self.step_to(n.y, n.x as i32, n.z as i32 + dz, 0.0)?;
            }
            Some((target, self.pos(node).distance(self.pos(target))))
        })
    }

    /// A path from `from` to `to`, straightened, or `None` when either end is
    /// more than `snap` away from walkable ground, they are not connected, or
    /// the search gives up ([`MAX_EXPANDED`]). The first point is `from` and
    /// the last `to`, with heights on the ground.
    pub fn find_path(&self, from: Vec3, to: Vec3, snap: f32) -> Option<Vec<Vec3>> {
        let start = self.nearest_node(from, snap)?;
        let goal = self.nearest_node(to, snap)?;
        let nodes = self.astar(start, goal)?;
        let mut points = self.smooth(&nodes);
        if let Some(first) = points.first_mut() {
            *first = Vec3::new(from.x, first.y, from.z);
        }
        if let Some(last) = points.last_mut() {
            *last = Vec3::new(to.x, last.y, to.z);
        }
        Some(points)
    }

    /// Whether a path exists (same rules as [`NavGrid::find_path`]).
    pub fn is_reachable(&self, from: Vec3, to: Vec3, snap: f32) -> bool {
        self.find_path(from, to, snap).is_some()
    }

    fn astar(&self, start: u32, goal: u32) -> Option<Vec<u32>> {
        SCRATCH.with(|cell| self.astar_with(start, goal, &mut cell.borrow_mut()))
    }

    fn astar_with(&self, start: u32, goal: u32, scratch: &mut Scratch) -> Option<Vec<u32>> {
        scratch.begin(self.nodes.len());
        let goal_pos = self.pos(goal);
        let h = |n: u32| self.pos(n).distance(goal_pos);
        let mut open = BinaryHeap::new();
        scratch.set(start, 0.0, start);
        open.push(Open {
            f: h(start),
            node: start,
        });
        let mut expanded = 0;
        while let Some(Open { node, f }) = open.pop() {
            let g = scratch.g(node);
            if f > g + h(node) + 1e-4 {
                continue; // a stale entry: a cheaper way was found later
            }
            if node == goal {
                let mut path = vec![goal];
                let mut at = goal;
                while at != start {
                    at = scratch.parent(at);
                    path.push(at);
                }
                path.reverse();
                return Some(path);
            }
            expanded += 1;
            if expanded > MAX_EXPANDED {
                return None;
            }
            for (next, cost) in self.neighbours(node) {
                let g2 = g + cost;
                if scratch.seen(next).is_none_or(|old| g2 < old) {
                    scratch.set(next, g2, node);
                    open.push(Open {
                        f: g2 + h(next),
                        node: next,
                    });
                }
            }
        }
        None
    }

    /// Drops the nodes that are in the way of a straight walk.
    fn smooth(&self, path: &[u32]) -> Vec<Vec3> {
        let mut out = vec![self.pos(path[0])];
        let mut anchor = 0;
        while anchor + 1 < path.len() {
            // The farthest node in sight from the anchor.
            let mut next = anchor + 1;
            for far in (anchor + 2..path.len()).rev() {
                if self.line_of_sight(path[anchor], path[far]) {
                    next = far;
                    break;
                }
            }
            out.push(self.pos(path[next]));
            anchor = next;
        }
        out
    }

    /// Whether the straight line between two nodes stays on walkable ground
    /// without a step the agent cannot climb.
    fn line_of_sight(&self, a: u32, b: u32) -> bool {
        let (pa, pb) = (self.pos(a), self.pos(b));
        let steps = (pa.distance(pb) / (self.config.cell * 0.5)).ceil().max(1.0) as u32;
        let mut y = pa.y;
        for i in 1..=steps {
            let p = pa.lerp(pb, i as f32 / steps as f32);
            let Some((x, z)) = self.cell_of(p.x, p.z) else {
                return false;
            };
            let Some(n) = self.step_to(y, x as i32, z as i32, 0.0) else {
                return false;
            };
            // Both neighbours of a diagonal move must be free too, which the
            // half-cell sampling above checks by visiting the cells between.
            y = self.nodes[n as usize].y;
        }
        true
    }
}
