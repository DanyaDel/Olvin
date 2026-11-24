//! Triangles → grid of walkable nodes.

use std::fmt;

use glam::{Vec2, Vec3};

use crate::{NavConfig, NavGrid, Node, Triangle};

/// The most cells a grid may have: a bad bounding box (one far-away triangle)
/// must not ask for gigabytes.
pub const MAX_CELLS: u64 = 4_000_000;

/// Heights closer than this are one surface.
const SAME_SURFACE: f32 = 0.05;

#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    /// No triangles, or none with a finite position.
    NoGeometry,
    /// `cell`, `agent_radius`, `agent_height` or the slope is not a sensible number.
    BadConfig(&'static str),
    /// The level's footprint at this cell size would need more than [`MAX_CELLS`] cells.
    TooLarge { cells: u64 },
}

impl fmt::Display for BakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoGeometry => f.write_str("no triangles to bake"),
            Self::BadConfig(what) => write!(f, "bad navigation settings: {what}"),
            Self::TooLarge { cells } => write!(
                f,
                "the level needs {cells} cells (at most {MAX_CELLS}): use a larger cell"
            ),
        }
    }
}

impl std::error::Error for BakeError {}

/// What a cell's centre column sees.
#[derive(Default)]
struct Column {
    /// Heights of walkable surfaces.
    floors: Vec<f32>,
    /// Heights of every surface (head room checks).
    solids: Vec<f32>,
    /// Vertical extents of steep triangles near the centre (walls).
    walls: Vec<(f32, f32)>,
}

impl NavGrid {
    /// Bakes `triangles` for the agent in `config`.
    pub fn bake(triangles: &[Triangle], config: NavConfig) -> Result<Self, BakeError> {
        if !(config.cell.is_finite() && config.cell >= 0.01) {
            return Err(BakeError::BadConfig("cell must be at least 0.01"));
        }
        if !(config.agent_radius.is_finite() && config.agent_radius >= 0.0) {
            return Err(BakeError::BadConfig("agent_radius must not be negative"));
        }
        if !(config.agent_height.is_finite() && config.agent_height > 0.0) {
            return Err(BakeError::BadConfig("agent_height must be positive"));
        }
        if !(config.max_step.is_finite() && config.max_step >= 0.0) {
            return Err(BakeError::BadConfig("max_step must not be negative"));
        }
        if !(config.max_slope_deg.is_finite() && (0.0..90.0).contains(&config.max_slope_deg)) {
            return Err(BakeError::BadConfig("max_slope_deg must be in 0..90"));
        }
        let tris: Vec<&Triangle> = triangles
            .iter()
            .filter(|t| t.iter().all(|p| p.is_finite()))
            .collect();
        if tris.is_empty() {
            return Err(BakeError::NoGeometry);
        }
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in tris.iter().flat_map(|t| t.iter()) {
            lo = lo.min(Vec2::new(p.x, p.z));
            hi = hi.max(Vec2::new(p.x, p.z));
        }
        let width = ((hi.x - lo.x) / config.cell).floor() as u64 + 1;
        let depth = ((hi.y - lo.y) / config.cell).floor() as u64 + 1;
        let cells = width.saturating_mul(depth);
        if cells > MAX_CELLS {
            return Err(BakeError::TooLarge { cells });
        }
        let (width, depth) = (width as u32, depth as u32);
        let mut grid = Self {
            config,
            origin: lo,
            width,
            depth,
            cell_start: Vec::new(),
            nodes: Vec::new(),
            blockers: Vec::new(),
        };
        let mut columns: Vec<Column> = (0..cells).map(|_| Column::default()).collect();
        grid.rasterize(&tris, &mut columns);
        let survivors = grid.free_surfaces(&columns);
        grid.erode(survivors);
        Ok(grid)
    }

    fn cell_centre(&self, x: u32, z: u32) -> Vec2 {
        Vec2::new(
            self.origin.x + (x as f32 + 0.5) * self.config.cell,
            self.origin.y + (z as f32 + 0.5) * self.config.cell,
        )
    }

    fn rasterize(&self, tris: &[&Triangle], columns: &mut [Column]) {
        let cell = self.config.cell;
        let cos_slope = self.config.max_slope_deg.to_radians().cos();
        for t in tris {
            let normal = (t[1] - t[0]).cross(t[2] - t[0]);
            let area2 = normal.length();
            if area2 < 1e-9 {
                continue;
            }
            let walkable = (normal.y / area2).abs() >= cos_slope;
            let flat = [0, 1, 2].map(|i| Vec2::new(t[i].x, t[i].z));
            let (tlo, thi) = (
                flat[0].min(flat[1]).min(flat[2]),
                flat[0].max(flat[1]).max(flat[2]),
            );
            // A steep triangle seen from above is a sliver: take the cells
            // within half a cell of it, not just the ones whose centre is inside.
            let reach = if walkable { 0.0 } else { cell * 0.75 };
            let to_cell = |v: f32, o: f32, max: u32| -> u32 {
                (((v - o) / cell).floor().max(0.0) as u32).min(max - 1)
            };
            let (x0, x1) = (
                to_cell(tlo.x - reach, self.origin.x, self.width),
                to_cell(thi.x + reach, self.origin.x, self.width),
            );
            let (z0, z1) = (
                to_cell(tlo.y - reach, self.origin.y, self.depth),
                to_cell(thi.y + reach, self.origin.y, self.depth),
            );
            let (ymin, ymax) = (
                t[0].y.min(t[1].y).min(t[2].y),
                t[0].y.max(t[1].y).max(t[2].y),
            );
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let c = self.cell_centre(x, z);
                    let col = &mut columns[(z * self.width + x) as usize];
                    if let Some(y) = height_at(t, normal, c) {
                        col.solids.push(y);
                        if walkable {
                            col.floors.push(y);
                        }
                    }
                    if !walkable && distance_to_triangle(flat, c) <= reach {
                        col.walls.push((ymin, ymax));
                    }
                }
            }
        }
    }

    /// The surfaces with head room and no wall in the way, as CSR arrays.
    fn free_surfaces(&mut self, columns: &[Column]) -> Vec<Vec<f32>> {
        let cfg = self.config;
        columns
            .iter()
            .map(|col| {
                let mut floors = col.floors.clone();
                floors.sort_by(f32::total_cmp);
                floors.dedup_by(|a, b| (*a - *b).abs() < SAME_SURFACE);
                floors
                    .into_iter()
                    .filter(|&h| {
                        let ceiling_ok = !col
                            .solids
                            .iter()
                            .any(|&s| s > h + SAME_SURFACE && s < h + cfg.agent_height);
                        let wall_ok = !col
                            .walls
                            .iter()
                            .any(|&(lo, hi)| hi > h + cfg.max_step && lo < h + cfg.agent_height);
                        ceiling_ok && wall_ok
                    })
                    .collect()
            })
            .collect()
    }

    /// Drops surfaces closer than the agent's radius to a missing neighbour
    /// (an edge, a wall, a hole) and stores the rest.
    fn erode(&mut self, surfaces: Vec<Vec<f32>>) {
        let cfg = self.config;
        let r = (cfg.agent_radius / cfg.cell).ceil() as i32;
        let slope = cfg.max_slope_deg.to_radians().tan();
        let (w, d) = (self.width as i32, self.depth as i32);
        let has_neighbour = |x: i32, z: i32, y: f32, dist: f32| -> bool {
            if x < 0 || z < 0 || x >= w || z >= d {
                return false;
            }
            let tolerance = cfg.max_step + dist * slope;
            surfaces[(z * w + x) as usize]
                .iter()
                .any(|&h| (h - y).abs() <= tolerance)
        };
        self.cell_start = Vec::with_capacity(surfaces.len() + 1);
        for z in 0..d {
            for x in 0..w {
                self.cell_start.push(self.nodes.len() as u32);
                for &y in &surfaces[(z * w + x) as usize] {
                    let mut edge = false;
                    'disc: for dz in -r..=r {
                        for dx in -r..=r {
                            let dist = ((dx * dx + dz * dz) as f32).sqrt() * cfg.cell;
                            if dist > cfg.agent_radius {
                                continue;
                            }
                            if !has_neighbour(x + dx, z + dz, y, dist) {
                                edge = true;
                                break 'disc;
                            }
                        }
                    }
                    if !edge {
                        self.nodes.push(Node {
                            x: x as u32,
                            z: z as u32,
                            y,
                            blocked: 0,
                        });
                    }
                }
            }
        }
        self.cell_start.push(self.nodes.len() as u32);
    }
}

/// Height of the triangle's plane above the point `c` (X, Z), if `c` is inside it.
fn height_at(t: &Triangle, normal: Vec3, c: Vec2) -> Option<f32> {
    let flat = [0, 1, 2].map(|i| Vec2::new(t[i].x, t[i].z));
    if !inside(flat, c) || normal.y.abs() < 1e-9 {
        return None;
    }
    // Plane: n · (p - t0) = 0 solved for y.
    Some(t[0].y - (normal.x * (c.x - t[0].x) + normal.z * (c.y - t[0].z)) / normal.y)
}

fn inside(tri: [Vec2; 3], p: Vec2) -> bool {
    let side = |a: Vec2, b: Vec2| (b - a).perp_dot(p - a);
    let (d0, d1, d2) = (
        side(tri[0], tri[1]),
        side(tri[1], tri[2]),
        side(tri[2], tri[0]),
    );
    let neg = d0 < 0.0 || d1 < 0.0 || d2 < 0.0;
    let pos = d0 > 0.0 || d1 > 0.0 || d2 > 0.0;
    !(neg && pos)
}

fn distance_to_triangle(tri: [Vec2; 3], p: Vec2) -> f32 {
    if inside(tri, p) {
        return 0.0;
    }
    (0..3)
        .map(|i| distance_to_segment(tri[i], tri[(i + 1) % 3], p))
        .fold(f32::MAX, f32::min)
}

fn distance_to_segment(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t - p).length()
}
