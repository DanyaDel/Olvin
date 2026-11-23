//! The retro (PlayStation 1) render style: vertex snapping, affine texture
//! mapping, 15-bit colour with ordered dither, per-vertex lighting, a hard
//! draw distance, nearest textures and a low internal resolution. See
//! `docs/adr/0003-retro-render-style.md`.

/// Switches of the retro look, a field of
/// [`RenderSettings`](crate::RenderSettings). The values are always uploaded;
/// `enabled` decides for the materials that inherit (`Material::retro` =
/// `None`), and a material can force the look on or off by itself. The
/// default is "off" with the PS1 values, so flipping `enabled` gives the
/// preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetroSettings {
    pub enabled: bool,
    /// Virtual screen the vertices snap to after projection, in pixels
    /// (works whatever the real framebuffer is). 0 = no screen snapping.
    pub snap_resolution: [u32; 2],
    /// Extra snapping of world positions to a grid of this many metres
    /// (0 = off). Moves whole meshes in steps: small props need a fine grid.
    pub snap_grid_m: f32,
    /// 0 = perspective-correct textures, 1 = the PS1's affine (screen-linear)
    /// interpolation.
    pub affine_strength: f32,
    /// Fragments closer than this (view depth, metres) fade back to
    /// perspective-correct, so near surfaces don't swim.
    pub affine_fade_near: f32,
    /// Bits per colour channel (the PS1 had 5); 0 or 8 and more = full colour.
    pub color_bits: u32,
    /// Ordered 4x4 Bayer dither: 0 = plain rounding, 1 = one quantisation step.
    pub dither_strength: f32,
    /// Lighting per vertex (Gouraud, diffuse only) instead of per pixel.
    pub vertex_lighting: bool,
    /// Sun shadows (per pixel, from the shadow map) in vertex-lit mode.
    pub shadows: bool,
    /// Hard cut at this view distance in metres (0 = off); the fog hides it.
    pub draw_distance: f32,
    /// Dissolve the last tenth before the cut with a dither pattern.
    pub dissolve: bool,
    /// Objects smaller than this on the virtual screen (pixels of radius)
    /// aren't drawn (0 = off).
    pub cull_small_px: f32,
    /// Nearest filtering and no mipmaps for retro materials.
    pub force_nearest: bool,
    /// Low internal resolution (window renderer, Full tier): the scene is
    /// drawn this big and scaled up with nearest filtering. [0, 0] = off.
    pub internal_resolution: [u32; 2],
    /// Instead of `internal_resolution`: the window divided by this (0 = off).
    pub pixel_scale: u32,
    /// Keep the internal aspect with black bars; off = the height of
    /// `internal_resolution` and the window's aspect.
    pub letterbox: bool,
}

impl Default for RetroSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            ..Self::ps1()
        }
    }
}

impl RetroSettings {
    /// The PlayStation 1 look: 320x240 vertex snapping, affine textures
    /// (fading out within 4 m), 15-bit colour with dither, vertex lighting,
    /// nearest textures, a 60 m draw distance.
    pub const fn ps1() -> Self {
        Self {
            enabled: true,
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

    /// Size the scene is drawn at before it is scaled up to `window`, or
    /// `None` when the scene is drawn at the window's size.
    pub fn scene_size(&self, window: (u32, u32)) -> Option<(u32, u32)> {
        let (w, h) = (window.0.max(1), window.1.max(1));
        if !self.enabled {
            return None;
        }
        let size = if self.pixel_scale > 0 {
            ((w / self.pixel_scale).max(1), (h / self.pixel_scale).max(1))
        } else if self.internal_resolution[0] > 0 && self.internal_resolution[1] > 0 {
            let [iw, ih] = self.internal_resolution;
            if self.letterbox {
                (iw, ih)
            } else {
                let aspect = w as f32 / h as f32;
                (((ih as f32 * aspect).round() as u32).max(1), ih)
            }
        } else {
            return None;
        };
        (size != (w, h)).then_some(size)
    }

    /// Levels per channel minus one (31 for 5 bits), 0 when colour isn't reduced.
    pub(crate) fn color_levels(&self) -> f32 {
        if (1..8).contains(&self.color_bits) {
            ((1u32 << self.color_bits) - 1) as f32
        } else {
            0.0
        }
    }
}
