//! PBR materials (metallic-roughness) and the [`Materials`] registry. Mirrors
//! `olvin_scene::MaterialDesc` with texture / shader paths resolved to handles.

use bytemuck::{Pod, Zeroable};

use crate::{Color, ShaderHandle, TextureHandle};

/// How a material's fragments combine with what's behind them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendMode {
    /// Written over the framebuffer (alpha only matters for `alpha_cutoff`).
    #[default]
    Opaque,
    /// Alpha-blended in a separate pass after the opaque meshes and the sky,
    /// sorted back to front by the instance's view depth; no depth writes, no
    /// shadows cast.
    Alpha,
}

/// A surface: textures and factors for the PBR shader, or a custom surface
/// shader on top of them. The [`MeshRenderer`](crate::MeshRenderer)'s colour
/// multiplies `base_color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Linear RGBA; multiplies `base_texture`.
    pub base_color: Color,
    /// Colour texture (use an sRGB [`TextureData`](crate::TextureData)).
    pub base_texture: Option<TextureHandle>,
    /// Tangent-space normal map (linear, green up).
    pub normal_texture: Option<TextureHandle>,
    pub normal_strength: f32,
    pub metallic: f32,
    pub roughness: f32,
    /// R = occlusion, G = roughness, B = metallic (linear); multiplies the factors.
    pub orm_texture: Option<TextureHandle>,
    /// Linear RGB added after lighting; multiplies `emissive_texture`.
    pub emissive: [f32; 3],
    pub emissive_texture: Option<TextureHandle>,
    /// UV tiling and offset (triplanar: repeats per metre).
    pub uv_scale: [f32; 2],
    pub uv_offset: [f32; 2],
    /// World-space projection blended by the normal instead of mesh UVs.
    pub triplanar: bool,
    /// Nearest filtering, no mipmaps.
    pub point_filter: bool,
    /// Alpha below this is discarded (0 = opaque).
    pub alpha_cutoff: f32,
    pub blend: BlendMode,
    /// Ignores lights: colour + emissive only.
    pub unlit: bool,
    /// Custom surface shader, see `SURFACE_SHADER_CONTRACT`.
    pub shader: Option<ShaderHandle>,
    /// The retro look for this material: `None` follows `RetroSettings::enabled`.
    pub retro: Option<bool>,
}

impl Default for Material {
    /// Same values as `MaterialDesc::default()`.
    fn default() -> Self {
        Self {
            base_color: Color([0.8, 0.8, 0.8, 1.0]),
            base_texture: None,
            normal_texture: None,
            normal_strength: 1.0,
            metallic: 0.0,
            roughness: 0.6,
            orm_texture: None,
            emissive: [0.0; 3],
            emissive_texture: None,
            uv_scale: [1.0, 1.0],
            uv_offset: [0.0, 0.0],
            triplanar: false,
            point_filter: false,
            alpha_cutoff: 0.0,
            blend: BlendMode::Opaque,
            unlit: false,
            shader: None,
            retro: None,
        }
    }
}

impl Material {
    /// What a [`MeshRenderer`](crate::MeshRenderer) without a material
    /// draws with: white (the instance colour alone), default PBR factors.
    pub fn fallback() -> Self {
        Self {
            base_color: Color::WHITE,
            ..Default::default()
        }
    }

    pub(crate) fn textures(&self) -> [Option<TextureHandle>; 4] {
        [
            self.base_texture,
            self.normal_texture,
            self.orm_texture,
            self.emissive_texture,
        ]
    }

    pub(crate) fn uniform(&self) -> MaterialUniform {
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let e = self.emissive;
        MaterialUniform {
            base_color: self.base_color.0,
            emissive: [e[0], e[1], e[2], self.normal_strength],
            params: [
                self.metallic.clamp(0.0, 1.0),
                self.roughness.clamp(0.0, 1.0),
                self.alpha_cutoff,
                0.0,
            ],
            uv: [
                self.uv_scale[0],
                self.uv_scale[1],
                self.uv_offset[0],
                self.uv_offset[1],
            ],
            flags: [
                flag(self.unlit),
                flag(self.triplanar),
                flag(self.normal_texture.is_some()),
                // 0 follows the settings, 1 retro on, 2 retro off.
                self.retro.map_or(0.0, |on| if on { 1.0 } else { 2.0 }),
            ],
        }
    }
}

/// `Material` in mesh.wgsl (five vec4).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct MaterialUniform {
    base_color: [f32; 4],
    /// rgb: emissive; w: normal strength.
    emissive: [f32; 4],
    /// metallic, roughness, alpha cutoff, -.
    params: [f32; 4],
    /// scale xy, offset xy.
    uv: [f32; 4],
    /// unlit, triplanar, has normal map, retro (0 inherit, 1 on, 2 off).
    flags: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialHandle(pub(crate) u32);

/// Material registry, like [`Meshes`](crate::Meshes).
#[derive(Default)]
pub struct Materials {
    next: u32,
    pub(crate) pending: Vec<(MaterialHandle, Material)>,
}

impl Materials {
    pub fn add(&mut self, material: Material) -> MaterialHandle {
        let handle = MaterialHandle(self.next);
        self.next += 1;
        self.pending.push((handle, material));
        handle
    }

    /// Replaces a material's parameters (same handle, so renderers keep it).
    pub fn set(&mut self, handle: MaterialHandle, material: Material) {
        self.pending.retain(|(h, _)| *h != handle);
        self.pending.push((handle, material));
    }
}
