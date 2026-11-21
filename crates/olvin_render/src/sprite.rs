//! 2D: sprites, atlases and tilemaps, for 2D games and in-game UI.
//!
//! Everything is a textured quad. [`SpriteRenderer`] collects [`Sprite`]s and
//! [`Tilemap`]s, culls them against the view, sorts by layer, depth and
//! texture, and draws them through the quad batcher: one draw call per run of
//! the same texture, filter and blend. It paints over everything (no depth
//! test), after the 3D scene.
//!
//! Two spaces: [`Space2d::World`] goes through the [`Camera2d`] entity (y up,
//! units are the camera's); [`Space2d::Screen`] is pixels from the top-left
//! corner (y down) and ignores the camera, drawn above all world sprites: the
//! game's UI.

use glam::{Mat4, Vec2, Vec3, Vec4};
use olvin_ecs::World;
use olvin_scene::Transform;
use olvin_transform::GlobalTransform;

use crate::core::GpuMeshes;
use crate::quad::{BatchKey, QuadBatcher, QuadBlend, QuadFilter, QuadInstance};
use crate::{Color, TextureHandle};

/// Quads sit at z = 0, which maps to the middle of the depth range; the
/// small scale keeps the matrices invertible (for the view bounds).
const Z_SCALE: f32 = 0.01;

/// Which coordinate system a sprite or tilemap lives in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Space2d {
    /// Seen through the [`Camera2d`]; +y is up.
    #[default]
    World,
    /// Pixels from the top-left corner of the window; +y is down.
    Screen,
}

/// Orthographic camera for [`Space2d::World`]; the pose comes from the
/// entity's transform (translation x/y, rotation around z, scale zooms).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Camera2d {
    /// World units visible vertically; the width follows the aspect.
    pub height: f32,
}

impl Default for Camera2d {
    fn default() -> Self {
        Self { height: 10.0 }
    }
}

impl Camera2d {
    pub fn new(height: f32) -> Self {
        Self { height }
    }

    /// World → clip. `pose` is the camera's world matrix, `viewport` the
    /// target size in pixels. Depth is nearly constant: 2D has no depth test.
    pub fn view_proj(&self, pose: &Mat4, viewport: (u32, u32)) -> Mat4 {
        let aspect = viewport.0.max(1) as f32 / viewport.1.max(1) as f32;
        let half_h = (self.height * 0.5).max(1e-6);
        Mat4::from_cols(
            Vec4::new(1.0 / (half_h * aspect), 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0 / half_h, 0.0, 0.0),
            Vec4::new(0.0, 0.0, Z_SCALE, 0.0),
            Vec4::new(0.0, 0.0, 0.5, 1.0),
        ) * pose.inverse()
    }
}

/// Pixels (origin top-left, y down) → clip.
pub fn screen_view_proj(viewport: (u32, u32)) -> Mat4 {
    let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
    Mat4::from_cols(
        Vec4::new(2.0 / w, 0.0, 0.0, 0.0),
        Vec4::new(0.0, -2.0 / h, 0.0, 0.0),
        Vec4::new(0.0, 0.0, Z_SCALE, 0.0),
        Vec4::new(-1.0, 1.0, 0.5, 1.0),
    )
}

/// A named rectangle of an atlas, in pixels from the image's top-left.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AtlasRegion {
    pub name: String,
    /// x, y, width, height.
    pub rect: [u32; 4],
    /// Anchor inside the region, 0..1 from its top-left (0.5, 0.5 = centre).
    pub pivot: [f32; 2],
}

/// One image holding many sprites. Plain data: the file stores the path and
/// the regions, the loader fills in `texture`.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SpriteAtlas {
    #[cfg_attr(feature = "serde", serde(skip))]
    pub texture: Option<TextureHandle>,
    pub texture_path: Option<String>,
    /// Image size in pixels.
    pub size: [u32; 2],
    pub regions: Vec<AtlasRegion>,
}

impl SpriteAtlas {
    /// An image cut into `columns × rows` equal cells, named "0", "1", … in
    /// reading order.
    pub fn grid(texture: TextureHandle, size: [u32; 2], columns: u32, rows: u32) -> Self {
        let (cw, ch) = (size[0] / columns.max(1), size[1] / rows.max(1));
        let regions = (0..columns * rows)
            .map(|i| AtlasRegion {
                name: i.to_string(),
                rect: [(i % columns) * cw, (i / columns) * ch, cw, ch],
                pivot: [0.5, 0.5],
            })
            .collect();
        Self {
            texture: Some(texture),
            texture_path: None,
            size,
            regions,
        }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.regions.iter().position(|r| r.name == name)
    }

    /// Texture coordinates `[u0, v0, u1, v1]` (top-left origin) of a region.
    pub fn uv(&self, region: usize) -> Option<[f32; 4]> {
        let [x, y, w, h] = self.regions.get(region)?.rect;
        let (iw, ih) = (self.size[0].max(1) as f32, self.size[1].max(1) as f32);
        Some([
            x as f32 / iw,
            y as f32 / ih,
            (x + w) as f32 / iw,
            (y + h) as f32 / ih,
        ])
    }

    /// A sprite showing `region`, `pixels_per_unit` pixels to a world unit.
    pub fn sprite(&self, region: usize, pixels_per_unit: f32) -> Option<Sprite> {
        let r = self.regions.get(region)?;
        let ppu = pixels_per_unit.max(1e-6);
        Some(Sprite {
            texture: self.texture,
            texture_path: self.texture_path.clone(),
            uv: self.uv(region)?,
            size: Vec2::new(r.rect[2] as f32, r.rect[3] as f32) / ppu,
            pivot: Vec2::from(r.pivot),
            ..Sprite::default()
        })
    }
}

/// A textured rectangle at the entity's transform.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Sprite {
    /// `None`: a flat rectangle of `color`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub texture: Option<TextureHandle>,
    /// For the scene loader, which sets `texture`.
    pub texture_path: Option<String>,
    /// `[u0, v0, u1, v1]`, top-left origin: the part of the texture to show.
    pub uv: [f32; 4],
    /// Size in world units (pixels for [`Space2d::Screen`]).
    pub size: Vec2,
    /// Anchor, 0..1 from the image's top-left (0.5, 0.5 = centre).
    pub pivot: Vec2,
    /// Multiplies the texture; linear RGBA.
    pub color: Color,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Higher layers draw over lower ones.
    pub layer: i32,
    /// Within a layer, sprites lower on screen draw over higher ones (top-down
    /// games) instead of ordering by the entity's z.
    pub y_sort: bool,
    pub blend: QuadBlend,
    /// Nearest for pixel art.
    pub filter: QuadFilter,
    pub space: Space2d,
}

impl Default for Sprite {
    fn default() -> Self {
        Self {
            texture: None,
            texture_path: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            size: Vec2::ONE,
            pivot: Vec2::splat(0.5),
            color: Color::WHITE,
            flip_x: false,
            flip_y: false,
            layer: 0,
            y_sort: false,
            blend: QuadBlend::Alpha,
            filter: QuadFilter::Linear,
            space: Space2d::World,
        }
    }
}

impl Sprite {
    /// The whole texture at `size`.
    pub fn new(texture: TextureHandle, size: Vec2) -> Self {
        Self {
            texture: Some(texture),
            size,
            ..Self::default()
        }
    }

    /// A flat rectangle.
    pub fn solid(color: Color, size: Vec2) -> Self {
        Self {
            color,
            size,
            ..Self::default()
        }
    }

    pub fn with_layer(mut self, layer: i32) -> Self {
        self.layer = layer;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_space(mut self, space: Space2d) -> Self {
        self.space = space;
        self
    }
}

/// A grid of equal tiles cut out of one image.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TileSet {
    #[cfg_attr(feature = "serde", serde(skip))]
    pub texture: Option<TextureHandle>,
    pub texture_path: Option<String>,
    /// Image size in pixels.
    pub image_size: [u32; 2],
    /// One tile, pixels.
    pub tile_size: [u32; 2],
    /// Border around the whole grid, pixels.
    pub margin: u32,
    /// Gap between tiles, pixels.
    pub spacing: u32,
}

impl TileSet {
    pub fn new(texture: TextureHandle, image_size: [u32; 2], tile_size: [u32; 2]) -> Self {
        Self {
            texture: Some(texture),
            texture_path: None,
            image_size,
            tile_size,
            margin: 0,
            spacing: 0,
        }
    }

    pub fn columns(&self) -> u32 {
        let step = (self.tile_size[0] + self.spacing).max(1);
        (self.image_size[0].saturating_sub(2 * self.margin) + self.spacing) / step
    }

    pub fn rows(&self) -> u32 {
        let step = (self.tile_size[1] + self.spacing).max(1);
        (self.image_size[1].saturating_sub(2 * self.margin) + self.spacing) / step
    }

    /// Texture coordinates `[u0, v0, u1, v1]` (top-left origin) of tile
    /// `index` (reading order), pulled in by a hair so neighbouring tiles
    /// don't bleed in under scaling.
    pub fn uv(&self, index: u32) -> Option<[f32; 4]> {
        let columns = self.columns();
        if columns == 0 || index >= columns * self.rows() {
            return None;
        }
        let x = self.margin + (index % columns) * (self.tile_size[0] + self.spacing);
        let y = self.margin + (index / columns) * (self.tile_size[1] + self.spacing);
        let (iw, ih) = (self.image_size[0] as f32, self.image_size[1] as f32);
        const INSET: f32 = 0.02;
        Some([
            (x as f32 + INSET) / iw,
            (y as f32 + INSET) / ih,
            ((x + self.tile_size[0]) as f32 - INSET) / iw,
            ((y + self.tile_size[1]) as f32 - INSET) / ih,
        ])
    }
}

/// A rectangular grid of tiles. Cell (0, 0) is the top-left; the entity's
/// transform places that corner. Only cells inside the view are drawn.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Tilemap {
    pub tileset: TileSet,
    pub width: u32,
    pub height: u32,
    /// Row-major, `width * height` entries. 0 is empty; otherwise the low 29
    /// bits are the tile index + 1, with [`Self::FLIP_X`] / [`Self::FLIP_Y`]
    /// on top.
    pub tiles: Vec<u32>,
    /// One cell in world units (pixels for [`Space2d::Screen`]).
    pub cell_size: Vec2,
    pub layer: i32,
    pub color: Color,
    pub filter: QuadFilter,
    pub space: Space2d,
}

impl Default for Tilemap {
    fn default() -> Self {
        Self {
            tileset: TileSet::default(),
            width: 0,
            height: 0,
            tiles: Vec::new(),
            cell_size: Vec2::ONE,
            layer: 0,
            color: Color::WHITE,
            filter: QuadFilter::Nearest,
            space: Space2d::World,
        }
    }
}

impl Tilemap {
    pub const FLIP_X: u32 = 1 << 31;
    pub const FLIP_Y: u32 = 1 << 30;
    const INDEX_MASK: u32 = (1 << 29) - 1;

    /// An empty map.
    pub fn new(tileset: TileSet, width: u32, height: u32, cell_size: Vec2) -> Self {
        Self {
            tileset,
            width,
            height,
            tiles: vec![0; width as usize * height as usize],
            cell_size,
            ..Self::default()
        }
    }

    /// The 0-based tile index of a stored value; `None` for an empty cell,
    /// also one holding only flip flags (a hand-edited or overflowed value).
    fn tile_index(value: u32) -> Option<u32> {
        (value & Self::INDEX_MASK).checked_sub(1)
    }

    /// The stored value of a cell (0 outside the map).
    pub fn get(&self, x: u32, y: u32) -> u32 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.tiles
            .get(y as usize * self.width as usize + x as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Puts tile `index` (0-based; `None` clears) with `flags` (`FLIP_X | FLIP_Y`).
    pub fn set(&mut self, x: u32, y: u32, tile: Option<u32>, flags: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = y as usize * self.width as usize + x as usize;
        if self.tiles.len() < self.width as usize * self.height as usize {
            self.tiles
                .resize(self.width as usize * self.height as usize, 0);
        }
        self.tiles[i] = tile.map_or(0, |t| {
            ((t + 1) & Self::INDEX_MASK) | (flags & (Self::FLIP_X | Self::FLIP_Y))
        });
    }

    /// Fills the `w × h` block at (x, y) with one tile.
    pub fn fill(&mut self, x: u32, y: u32, w: u32, h: u32, tile: Option<u32>) {
        for cy in y..y.saturating_add(h) {
            for cx in x..x.saturating_add(w) {
                self.set(cx, cy, tile, 0);
            }
        }
    }

    /// Cells whose rectangle may touch the local-space box `min..max`
    /// (`x0..x1`, `y0..y1`, exclusive ends), clamped to the map.
    fn cell_range(&self, space: Space2d, min: Vec2, max: Vec2) -> (u32, u32, u32, u32) {
        let cs = self.cell_size.max(Vec2::splat(1e-6));
        let (y0, y1) = match space {
            Space2d::World => (-max.y / cs.y, -min.y / cs.y),
            Space2d::Screen => (min.y / cs.y, max.y / cs.y),
        };
        let clamp = |v: f32, n: u32| (v.floor().max(0.0) as u32).min(n);
        (
            clamp(min.x / cs.x, self.width),
            clamp((max.x / cs.x).floor() + 1.0, self.width),
            clamp(y0, self.height),
            clamp(y1.floor() + 1.0, self.height),
        )
    }
}

/// An axis-aligned box in the space a sprite is drawn in.
#[derive(Clone, Copy)]
struct Bounds {
    min: Vec2,
    max: Vec2,
}

impl Bounds {
    fn of(points: &[Vec2]) -> Self {
        let mut b = Self {
            min: Vec2::splat(f32::MAX),
            max: Vec2::splat(f32::MIN),
        };
        for &p in points {
            b.min = b.min.min(p);
            b.max = b.max.max(p);
        }
        b
    }

    fn overlaps(&self, other: &Bounds) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }
}

/// World-space box of everything a clip-space matrix shows.
fn view_bounds(view_proj: &Mat4) -> Bounds {
    let inv = view_proj.inverse();
    let corner = |x: f32, y: f32| inv.project_point3(Vec3::new(x, y, 0.5)).truncate();
    Bounds::of(&[
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(-1.0, 1.0),
        corner(1.0, 1.0),
    ])
}

/// A quad ready to sort.
struct Entry {
    space: Space2d,
    layer: i32,
    depth: f32,
    key: BatchKey,
    seq: u32,
    instance: QuadInstance,
}

fn xy(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.y)
}

/// A quad from the corner at (0, 0) and its edges. World quads start at the
/// bottom-left (v flipped); screen quads at the top-left. `uv` is top-left
/// origin; the flips are applied here.
fn quad(
    space: Space2d,
    corner: Vec2,
    axis_x: Vec2,
    axis_y: Vec2,
    uv: [f32; 4],
    flip: (bool, bool),
    color: [f32; 4],
) -> QuadInstance {
    let [mut u0, mut v0, mut u1, mut v1] = uv;
    if flip.0 {
        std::mem::swap(&mut u0, &mut u1);
    }
    if flip.1 {
        std::mem::swap(&mut v0, &mut v1);
    }
    QuadInstance {
        origin: [corner.x, corner.y, 0.0, 0.0],
        axis_x: [axis_x.x, axis_x.y, 0.0, 0.0],
        axis_y: [axis_y.x, axis_y.y, 0.0, 0.0],
        color,
        uv: match space {
            Space2d::World => [u0, v1, u1, v0],
            Space2d::Screen => [u0, v0, u1, v1],
        },
    }
}

fn pose_of(global: Option<&GlobalTransform>, transform: Option<&Transform>) -> Mat4 {
    global.map_or_else(
        || transform.map_or(Mat4::IDENTITY, Transform::matrix),
        |g| g.0,
    )
}

/// Collects, culls, sorts and draws 2D quads. Hold one next to the
/// [`RenderCore`](crate::RenderCore) and call `prepare` before `draw` into a
/// pass each frame.
pub struct SpriteRenderer {
    world: QuadBatcher,
    screen: QuadBatcher,
    entries: Vec<Entry>,
    color: wgpu::TextureFormat,
}

impl SpriteRenderer {
    /// `color` / `samples` / `depth` must match the pass `draw` goes into.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: wgpu::TextureFormat,
        samples: u32,
        depth: wgpu::TextureFormat,
    ) -> Self {
        Self {
            world: QuadBatcher::new(device, queue, color, samples, Some(depth), false),
            screen: QuadBatcher::new(device, queue, color, samples, Some(depth), false),
            entries: Vec::new(),
            color,
        }
    }

    /// For an HDR target: skip sRGB encoding (the post chain does it).
    pub fn set_hdr_output(&mut self, on: bool) {
        self.world.set_hdr_output(on, self.color);
        self.screen.set_hdr_output(on, self.color);
    }

    /// Collects every visible [`Sprite`] and [`Tilemap`] of `world` and
    /// uploads them. `viewport` is the target size in pixels; the first
    /// [`Camera2d`] frames the world (none: one unit = one pixel, centred on
    /// the origin). Returns the number of quads.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &GpuMeshes,
        world: &World,
        viewport: (u32, u32),
    ) -> usize {
        self.entries.clear();
        self.world.clear();
        self.screen.clear();

        let (camera, pose) = world
            .query::<(Option<&GlobalTransform>, Option<&Transform>, &Camera2d)>()
            .iter()
            .next()
            .map_or_else(
                || (Camera2d::new(viewport.1.max(1) as f32), Mat4::IDENTITY),
                |(g, t, c)| (*c, pose_of(g, t)),
            );
        let world_vp = camera.view_proj(&pose, viewport);
        let screen_vp = screen_view_proj(viewport);
        let views = [view_bounds(&world_vp), view_bounds(&screen_vp)];
        let view = |space: Space2d| views[space as usize];

        let mut seq = 0u32;
        for (global, transform, sprite) in world
            .query::<(Option<&GlobalTransform>, Option<&Transform>, &Sprite)>()
            .iter()
        {
            let m = pose_of(global, transform);
            let (pos, ex, ey) = (
                xy(m.w_axis.truncate()),
                xy(m.x_axis.truncate()),
                xy(m.y_axis.truncate()),
            );
            let (ax, ay) = (ex * sprite.size.x, ey * sprite.size.y);
            let corner = match sprite.space {
                Space2d::World => pos - ax * sprite.pivot.x - ay * (1.0 - sprite.pivot.y),
                Space2d::Screen => pos - ax * sprite.pivot.x - ay * sprite.pivot.y,
            };
            let bounds = Bounds::of(&[corner, corner + ax, corner + ay, corner + ax + ay]);
            if !bounds.overlaps(&view(sprite.space)) {
                continue;
            }
            let depth = match (sprite.y_sort, sprite.space) {
                (false, _) => m.w_axis.z,
                (true, Space2d::World) => -pos.y,
                (true, Space2d::Screen) => pos.y,
            };
            self.entries.push(Entry {
                space: sprite.space,
                layer: sprite.layer,
                depth,
                key: BatchKey {
                    texture: sprite.texture,
                    filter: sprite.filter,
                    blend: sprite.blend,
                },
                seq,
                instance: quad(
                    sprite.space,
                    corner,
                    ax,
                    ay,
                    sprite.uv,
                    (sprite.flip_x, sprite.flip_y),
                    sprite.color.0,
                ),
            });
            seq += 1;
        }

        for (global, transform, map) in world
            .query::<(Option<&GlobalTransform>, Option<&Transform>, &Tilemap)>()
            .iter()
        {
            let m = pose_of(global, transform);
            if map.width == 0 || map.height == 0 || m.determinant().abs() < 1e-12 {
                continue;
            }
            // The visible box in the map's own space → a cell range.
            let v = view(map.space);
            let inv = m.inverse();
            let local = Bounds::of(&[
                xy(inv.transform_point3(v.min.extend(0.0))),
                xy(inv.transform_point3(Vec3::new(v.max.x, v.min.y, 0.0))),
                xy(inv.transform_point3(Vec3::new(v.min.x, v.max.y, 0.0))),
                xy(inv.transform_point3(v.max.extend(0.0))),
            ]);
            let (x0, x1, y0, y1) = map.cell_range(map.space, local.min, local.max);
            let key = BatchKey {
                texture: map.tileset.texture,
                filter: map.filter,
                blend: QuadBlend::Alpha,
            };
            let (ex, ey) = (m.x_axis.truncate(), m.y_axis.truncate());
            let (ax, ay) = (xy(ex) * map.cell_size.x, xy(ey) * map.cell_size.y);
            for cy in y0..y1 {
                for cx in x0..x1 {
                    let value = map.get(cx, cy);
                    let Some(uv) = Tilemap::tile_index(value).and_then(|i| map.tileset.uv(i))
                    else {
                        continue;
                    };
                    let row = match map.space {
                        Space2d::World => -((cy + 1) as f32),
                        Space2d::Screen => cy as f32,
                    };
                    let local = Vec3::new(cx as f32 * map.cell_size.x, row * map.cell_size.y, 0.0);
                    self.entries.push(Entry {
                        space: map.space,
                        layer: map.layer,
                        depth: m.w_axis.z,
                        key,
                        seq,
                        instance: quad(
                            map.space,
                            xy(m.transform_point3(local)),
                            ax,
                            ay,
                            uv,
                            (value & Tilemap::FLIP_X != 0, value & Tilemap::FLIP_Y != 0),
                            map.color.0,
                        ),
                    });
                    seq += 1;
                }
            }
        }

        self.entries.sort_unstable_by(|a, b| {
            let id = |e: &Entry| e.key.texture.map_or(0, |t| t.0 + 1);
            (a.space as u8)
                .cmp(&(b.space as u8))
                .then(a.layer.cmp(&b.layer))
                .then(a.depth.total_cmp(&b.depth))
                .then(id(a).cmp(&id(b)))
                .then((a.key.blend as u8).cmp(&(b.key.blend as u8)))
                .then((a.key.filter as u8).cmp(&(b.key.filter as u8)))
                .then(a.seq.cmp(&b.seq))
        });
        for e in &self.entries {
            match e.space {
                Space2d::World => self.world.push(e.key, e.instance),
                Space2d::Screen => self.screen.push(e.key, e.instance),
            }
        }
        self.world.upload(device, queue, gpu, world_vp);
        self.screen.upload(device, queue, gpu, screen_vp);
        self.entries.len()
    }

    /// Draws what the last `prepare` collected: the world, then the screen-space UI.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.world.draw(pass);
        self.screen.draw(pass);
    }

    /// Draw calls `draw` issues.
    pub fn draw_calls(&self) -> usize {
        self.world.batch_count() + self.screen.batch_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_camera_maps_the_view_to_clip() {
        let cam = Camera2d::new(10.0);
        let vp = cam.view_proj(
            &Mat4::from_translation(Vec3::new(4.0, 2.0, 0.0)),
            (200, 100),
        );
        // Centre of the view and its top-right corner (aspect 2: 20 × 10 units).
        assert!(
            vp.project_point3(Vec3::new(4.0, 2.0, 0.0))
                .truncate()
                .length()
                < 1e-5
        );
        let c = vp.project_point3(Vec3::new(14.0, 7.0, 0.0));
        assert!((c.x - 1.0).abs() < 1e-5 && (c.y - 1.0).abs() < 1e-5);
        assert!((0.0..=1.0).contains(&c.z));
    }

    #[test]
    fn screen_space_is_top_left_y_down() {
        let vp = screen_view_proj((800, 600));
        let tl = vp.project_point3(Vec3::ZERO);
        let br = vp.project_point3(Vec3::new(800.0, 600.0, 0.0));
        assert!((tl.x + 1.0).abs() < 1e-5 && (tl.y - 1.0).abs() < 1e-5);
        assert!((br.x - 1.0).abs() < 1e-5 && (br.y + 1.0).abs() < 1e-5);
    }

    #[test]
    fn atlas_grid_and_regions() {
        let atlas = SpriteAtlas::grid(TextureHandle(0), [64, 32], 4, 2);
        assert_eq!(atlas.regions.len(), 8);
        assert_eq!(atlas.find("5"), Some(5));
        assert_eq!(atlas.uv(5), Some([0.25, 0.5, 0.5, 1.0]));
        assert_eq!(atlas.uv(8), None);
        let sprite = atlas.sprite(5, 16.0).unwrap();
        assert_eq!(sprite.size, Vec2::new(1.0, 1.0));
        assert_eq!(sprite.uv, [0.25, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn tileset_indexing_with_margin_and_spacing() {
        let mut set = TileSet::new(TextureHandle(0), [34, 18], [16, 16]);
        assert_eq!((set.columns(), set.rows()), (2, 1));
        set.margin = 1;
        set.spacing = 2;
        assert_eq!((set.columns(), set.rows()), (1, 1));
        set.image_size = [38, 18];
        assert_eq!(set.columns(), 2);
        let uv = set.uv(1).unwrap();
        assert!((uv[0] * 38.0 - 19.02).abs() < 1e-3, "{uv:?}");
        assert_eq!(set.uv(2), None);
    }

    #[test]
    fn flag_only_cells_are_empty_not_an_underflow() {
        assert_eq!(Tilemap::tile_index(0), None);
        assert_eq!(Tilemap::tile_index(Tilemap::FLIP_X), None);
        assert_eq!(
            Tilemap::tile_index(Tilemap::FLIP_X | Tilemap::FLIP_Y | 1),
            Some(0)
        );
        // Tile 2^29 - 1 overflows into the flag-less zero: still just empty.
        let mut map = Tilemap::new(
            TileSet::new(TextureHandle(0), [16, 16], [8, 8]),
            1,
            1,
            Vec2::ONE,
        );
        map.set(0, 0, Some(Tilemap::INDEX_MASK), Tilemap::FLIP_X);
        assert_eq!(Tilemap::tile_index(map.get(0, 0)), None);
    }

    #[test]
    fn tilemap_set_get_and_visible_range() {
        let set = TileSet::new(TextureHandle(0), [32, 32], [16, 16]);
        let mut map = Tilemap::new(set, 100, 50, Vec2::ONE);
        map.set(3, 4, Some(2), Tilemap::FLIP_X);
        assert_eq!(map.get(3, 4), 3 | Tilemap::FLIP_X);
        map.set(3, 4, None, 0);
        assert_eq!(map.get(3, 4), 0);
        assert_eq!(map.get(500, 0), 0);
        // World: rows go down from y = 0, so y in -10..0 is cells 0..10.
        let r = map.cell_range(Space2d::World, Vec2::new(10.5, -9.5), Vec2::new(19.5, -0.5));
        assert_eq!(r, (10, 20, 0, 10));
        // Far outside: nothing.
        let r = map.cell_range(
            Space2d::World,
            Vec2::new(500.0, 500.0),
            Vec2::new(510.0, 510.0),
        );
        assert_eq!((r.1 - r.0) * (r.3 - r.2), 0);
        let r = map.cell_range(Space2d::Screen, Vec2::new(-5.0, -5.0), Vec2::new(2.5, 1.5));
        assert_eq!(r, (0, 3, 0, 2));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn sprites_and_maps_round_trip_through_ron() {
        let mut map = Tilemap::new(TileSet::default(), 2, 2, Vec2::new(16.0, 16.0));
        map.tileset.texture_path = Some("tiles.png".into());
        map.set(1, 1, Some(4), Tilemap::FLIP_Y);
        let back: Tilemap = ron::from_str(&ron::to_string(&map).unwrap()).unwrap();
        assert_eq!(back, map);
        let sprite = Sprite {
            texture_path: Some("hero.png".into()),
            layer: 3,
            space: Space2d::Screen,
            ..Default::default()
        };
        let back: Sprite = ron::from_str(&ron::to_string(&sprite).unwrap()).unwrap();
        assert_eq!(back, sprite);
    }
}
