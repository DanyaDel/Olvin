//! Terrain brushes: what one step of a stroke does to the data. Pure
//! functions over [`TerrainData`]; the editor calls them every frame while
//! the button is held, with the frame time.

use glam::{Vec2, Vec3};

use crate::TerrainData;

/// What a brush does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BrushOp {
    /// Up (positive) or down (negative), in terrain heights per second.
    Raise(f32),
    /// Towards the average of the neighbours.
    Smooth,
    /// Towards one height (0..1, e.g. the one under the stroke's start).
    Flatten(f32),
    /// Layer 0..=3 becomes the dominant one.
    Paint(usize),
}

/// A brush at a place: centre in the terrain's local x/z, radius in metres,
/// strength 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brush {
    pub center: Vec2,
    pub radius: f32,
    pub strength: f32,
}

impl Brush {
    /// Weight of a point at `distance` from the centre: 1 there, 0 at the
    /// radius, smooth in between.
    pub fn falloff(&self, distance: f32) -> f32 {
        let x = (distance / self.radius.max(1e-4)).clamp(0.0, 1.0);
        let k = 1.0 - x * x;
        k * k
    }
}

/// Vertices the brush can reach, inclusive; `None` when it misses the terrain.
fn reach(data: &TerrainData, size: Vec3, brush: &Brush) -> Option<((u32, u32), (u32, u32))> {
    let last = (data.resolution() - 1) as f32;
    let to_grid = |local: Vec2| {
        Vec2::new(
            (local.x / size.x + 0.5) * last,
            (local.y / size.z + 0.5) * last,
        )
    };
    let r = Vec2::splat(brush.radius);
    let (lo, hi) = (
        to_grid(brush.center - r).floor(),
        to_grid(brush.center + r).ceil(),
    );
    if hi.x < 0.0 || hi.y < 0.0 || lo.x > last || lo.y > last {
        return None;
    }
    let clamp = |v: f32| v.clamp(0.0, last) as u32;
    Some(((clamp(lo.x), clamp(lo.y)), (clamp(hi.x), clamp(hi.y))))
}

/// Local x/z of vertex `(x, z)`.
fn local(data: &TerrainData, size: Vec3, x: u32, z: u32) -> Vec2 {
    let last = (data.resolution() - 1) as f32;
    Vec2::new(
        (x as f32 / last - 0.5) * size.x,
        (z as f32 / last - 0.5) * size.z,
    )
}

/// One step of `op` over `dt` seconds. Returns the vertex rectangle that
/// changed (heights for Raise/Smooth/Flatten, weights for Paint).
pub fn apply(
    data: &mut TerrainData,
    size: Vec3,
    brush: &Brush,
    op: BrushOp,
    dt: f32,
) -> Option<((u32, u32), (u32, u32))> {
    let rect = reach(data, size, brush)?;
    let ((x0, z0), (x1, z1)) = rect;
    let strength = brush.strength.clamp(0.0, 1.0);
    // Smoothing reads the heights from before this step, so it doesn't
    // depend on the order vertices are visited in.
    let before = matches!(op, BrushOp::Smooth).then(|| data.clone());
    for z in z0..=z1 {
        for x in x0..=x1 {
            let w = brush.falloff(local(data, size, x, z).distance(brush.center)) * strength;
            if w <= 0.0 {
                continue;
            }
            match op {
                BrushOp::Raise(rate) => {
                    data.set_height(x, z, data.height(x, z) + rate * w * dt);
                }
                BrushOp::Flatten(target) => {
                    let h = data.height(x, z);
                    let k = (w * dt * 8.0).min(1.0);
                    data.set_height(x, z, h + (target - h) * k);
                }
                BrushOp::Smooth => {
                    let Some(b) = &before else { continue };
                    let n = [(-1, 0), (1, 0), (0, -1), (0, 1)];
                    let avg = n
                        .iter()
                        .map(|(dx, dz)| {
                            b.height(x.saturating_add_signed(*dx), z.saturating_add_signed(*dz))
                        })
                        .sum::<f32>()
                        / 4.0;
                    let h = b.height(x, z);
                    let k = (w * dt * 8.0).min(1.0);
                    data.set_height(x, z, h + (avg - h) * k);
                }
                BrushOp::Paint(layer) => {
                    let layer = layer.min(3);
                    let mut weights = data.weights(x, z).map(f32::from);
                    let k = (w * dt * 4.0).min(1.0);
                    for (i, v) in weights.iter_mut().enumerate() {
                        let target = if i == layer { 255.0 } else { 0.0 };
                        *v += (target - *v) * k;
                    }
                    // Keep the sum at 255 so the blend never darkens.
                    let sum: f32 = weights.iter().sum::<f32>().max(1.0);
                    let mut out = weights.map(|v| (v * 255.0 / sum).round() as u8);
                    let total: i32 = out.iter().map(|&v| v as i32).sum();
                    out[layer] = (out[layer] as i32 + 255 - total).clamp(0, 255) as u8;
                    data.set_weights(x, z, out);
                }
            }
        }
    }
    Some(rect)
}
