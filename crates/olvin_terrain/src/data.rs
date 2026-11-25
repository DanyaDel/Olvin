//! Heights and layer weights of a terrain, and their `*.terrain` file.
//!
//! The file is binary (a scene stays readable text): `OLVT`, version (u32),
//! resolution (u32), then `resolution²` heights as u16 (0..=65535 is 0..1 of
//! the terrain's height) and `resolution²` RGBA weights (one byte per layer).
//! Everything little-endian, rows along x, then z.

use std::path::Path;

const MAGIC: &[u8; 4] = b"OLVT";
const VERSION: u32 = 1;

/// Vertices per side of a new terrain: 128 quads, four chunks of 32 a side.
pub const DEFAULT_RESOLUTION: u32 = 129;
/// Smallest and largest vertices per side.
pub const MIN_RESOLUTION: u32 = 2;
pub const MAX_RESOLUTION: u32 = 4097;

/// A square grid of heights (0..1) and layer weights.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainData {
    resolution: u32,
    heights: Vec<f32>,
    splat: Vec<[u8; 4]>,
}

impl TerrainData {
    /// Flat at height 0, all of layer 0.
    pub fn flat(resolution: u32) -> Self {
        let resolution = resolution.clamp(MIN_RESOLUTION, MAX_RESOLUTION);
        let n = (resolution * resolution) as usize;
        Self {
            resolution,
            heights: vec![0.0; n],
            splat: vec![[255, 0, 0, 0]; n],
        }
    }

    /// Vertices per side.
    pub fn resolution(&self) -> u32 {
        self.resolution
    }

    fn index(&self, x: u32, z: u32) -> usize {
        let last = self.resolution - 1;
        (z.min(last) * self.resolution + x.min(last)) as usize
    }

    /// Height at a vertex, 0..1 (outside the grid: the nearest edge).
    pub fn height(&self, x: u32, z: u32) -> f32 {
        self.heights[self.index(x, z)]
    }

    pub fn set_height(&mut self, x: u32, z: u32, h: f32) {
        let i = self.index(x, z);
        self.heights[i] = if h.is_finite() {
            h.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    /// Layer weights at a vertex.
    pub fn weights(&self, x: u32, z: u32) -> [u8; 4] {
        self.splat[self.index(x, z)]
    }

    pub fn set_weights(&mut self, x: u32, z: u32, w: [u8; 4]) {
        let i = self.index(x, z);
        self.splat[i] = w;
    }

    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    pub fn splat(&self) -> &[[u8; 4]] {
        &self.splat
    }

    /// Height at `(u, v)` in 0..1 across the terrain, bilinear.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let last = (self.resolution - 1) as f32;
        let (fx, fz) = (u.clamp(0.0, 1.0) * last, v.clamp(0.0, 1.0) * last);
        let (x0, z0) = (fx.floor() as u32, fz.floor() as u32);
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let h = |x, z| self.height(x, z);
        let top = h(x0, z0) * (1.0 - tx) + h(x0 + 1, z0) * tx;
        let bottom = h(x0, z0 + 1) * (1.0 - tx) + h(x0 + 1, z0 + 1) * tx;
        top * (1.0 - tz) + bottom * tz
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let n = self.heights.len();
        let mut out = Vec::with_capacity(12 + n * 6);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.resolution.to_le_bytes());
        for h in &self.heights {
            out.extend_from_slice(&((h.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes());
        }
        for w in &self.splat {
            out.extend_from_slice(w);
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let u32_at = |at: usize| -> Result<u32, String> {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| "terrain file is cut short".to_owned())
        };
        if bytes.get(..4) != Some(MAGIC.as_slice()) {
            return Err("not a terrain file (no OLVT header)".into());
        }
        let version = u32_at(4)?;
        if version != VERSION {
            return Err(format!(
                "terrain file version {version}, this build reads {VERSION}"
            ));
        }
        let resolution = u32_at(8)?;
        if !(MIN_RESOLUTION..=MAX_RESOLUTION).contains(&resolution) {
            return Err(format!("terrain resolution {resolution} is out of range"));
        }
        let n = (resolution * resolution) as usize;
        let body = &bytes[12..];
        if body.len() != n * 6 {
            return Err(format!(
                "terrain file holds {} bytes of data, {} expected for {resolution}²",
                body.len(),
                n * 6
            ));
        }
        let (h, s) = body.split_at(n * 2);
        Ok(Self {
            resolution,
            heights: h
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]) as f32 / 65535.0)
                .collect(),
            splat: s
                .chunks_exact(4)
                .map(|b| [b[0], b[1], b[2], b[3]])
                .collect(),
        })
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, self.to_bytes()).map_err(|e| format!("{}: {e}", path.display()))
    }
}
