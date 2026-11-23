//! The retro (PS1) render style as scene data: `Environment::retro` and the
//! retro block of a look. Mirrors `olvin_render::RetroSettings`, which holds
//! the meaning of each field; `Some` means on, `None` (the default, not
//! written to files) means off.

use serde::{Deserialize, Serialize};

/// A missing field keeps the PS1 value, so a file lists only what it changes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetroDesc {
    /// Virtual screen the vertices snap to, in pixels; 0 = no screen snapping.
    pub snap_resolution: [u32; 2],
    /// World-space snap grid in metres; 0 = off.
    pub snap_grid_m: f32,
    /// 0 = perspective-correct textures, 1 = PS1 affine.
    pub affine_strength: f32,
    /// Affine mapping fades out for fragments closer than this (metres).
    pub affine_fade_near: f32,
    /// Bits per colour channel; 0 or 8+ = full colour.
    pub color_bits: u32,
    /// Ordered dither strength in quantisation steps.
    pub dither_strength: f32,
    /// Gouraud (per-vertex, diffuse) lighting.
    pub vertex_lighting: bool,
    /// Sun shadows in vertex-lit mode.
    pub shadows: bool,
    /// Hard draw distance in metres; 0 = off.
    pub draw_distance: f32,
    /// Dissolve the last tenth before the cut.
    pub dissolve: bool,
    /// Objects below this projected radius (virtual pixels) are skipped; 0 = off.
    pub cull_small_px: f32,
    /// Nearest filtering and no mipmaps.
    pub force_nearest: bool,
    /// Low internal resolution (window renderer, Full tier); [0, 0] = off.
    pub internal_resolution: [u32; 2],
    /// Window divided by this instead of `internal_resolution`; 0 = off.
    pub pixel_scale: u32,
    /// Keep the internal aspect with black bars.
    pub letterbox: bool,
}

impl RetroDesc {
    /// The PlayStation 1 look (also the default of every field).
    pub const fn ps1() -> Self {
        Self {
            snap_resolution: [320, 240],
            snap_grid_m: 0.0,
            affine_strength: 1.0,
            affine_fade_near: 4.0,
            color_bits: 5,
            dither_strength: 1.0,
            vertex_lighting: true,
            shadows: false,
            draw_distance: 60.0,
            dissolve: false,
            cull_small_px: 1.0,
            force_nearest: true,
            internal_resolution: [320, 240],
            pixel_scale: 0,
            letterbox: false,
        }
    }
}

impl Default for RetroDesc {
    fn default() -> Self {
        Self::ps1()
    }
}
