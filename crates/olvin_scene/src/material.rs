//! Material assets (`*.mat.ron`), shared by the editor, the loader and the
//! renderer. Texture paths are relative to the project's `assets/` folder.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// PBR metallic-roughness material, like Unity's Standard / glTF's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaterialDesc {
    /// Linear RGBA; multiplies `base_texture`.
    pub base_color: [f32; 4],
    /// sRGB colour texture.
    pub base_texture: Option<String>,
    /// Tangent-space normal map (linear).
    pub normal_texture: Option<String>,
    pub normal_strength: f32,
    pub metallic: f32,
    pub roughness: f32,
    /// glTF-style: R = occlusion, G = roughness, B = metallic (linear); multiplies the factors.
    pub orm_texture: Option<String>,
    /// Linear RGB, added on top of lighting (not affected by exposure-less unlit).
    pub emissive: [f32; 3],
    pub emissive_texture: Option<String>,
    /// UV tiling and offset.
    pub uv_scale: [f32; 2],
    pub uv_offset: [f32; 2],
    /// World-space triplanar projection instead of mesh UVs (good for CSG
    /// levels); `uv_scale` is then texture repeats per metre.
    pub triplanar: bool,
    /// Pixel-art look: nearest filtering, no mipmaps.
    pub point_filter: bool,
    /// Alpha below this is discarded (0 = opaque, no cutout).
    pub alpha_cutoff: f32,
    /// Ignores lights: colour + emissive only.
    pub unlit: bool,
    /// Custom surface shader (`*.wgsl`), see [`SURFACE_SHADER_CONTRACT`].
    pub shader: Option<String>,
}

impl Default for MaterialDesc {
    fn default() -> Self {
        Self {
            base_color: [0.8, 0.8, 0.8, 1.0],
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
            unlit: false,
            shader: None,
        }
    }
}

impl MaterialDesc {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().struct_names(false))
            .map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Every texture path this material references.
    pub fn textures(&self) -> impl Iterator<Item = &str> {
        [&self.base_texture, &self.normal_texture, &self.orm_texture, &self.emissive_texture]
            .into_iter()
            .filter_map(|t| t.as_deref())
    }
}

/// What a custom surface shader (`MaterialDesc::shader`) must define. The
/// engine pastes it into its mesh shader and calls `surface` per pixel, after
/// the material's textures are sampled and before lighting.
pub const SURFACE_SHADER_CONTRACT: &str = r#"// Provided by the engine:
// struct SurfaceInput {
//     world_position: vec3<f32>,
//     normal: vec3<f32>,      // world space, normalized, normal map applied
//     uv: vec2<f32>,          // after uv_scale / uv_offset
//     color: vec4<f32>,       // base_color * base_texture * instance colour
//     time: f32,              // seconds since start
// };
// struct SurfaceOutput {
//     color: vec4<f32>,       // albedo + alpha
//     normal: vec3<f32>,      // world space
//     metallic: f32,
//     roughness: f32,
//     emissive: vec3<f32>,
// };
// fn default_surface(in: SurfaceInput) -> SurfaceOutput  // the standard material
//
// Your file defines:
fn surface(in: SurfaceInput) -> SurfaceOutput {
    var out = default_surface(in);
    // e.g. scrolling stripes:
    // out.emissive = vec3(step(0.5, fract(in.uv.x * 8.0 + in.time))) * 0.5;
    return out;
}
"#;
