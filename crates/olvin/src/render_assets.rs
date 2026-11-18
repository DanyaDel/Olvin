//! Material, texture, shader and sky assets turned into renderer handles, with
//! hot reload. Shared by the game loader and the editor's preview.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use olvin_render::{
    Color, Material, MaterialHandle, Materials, ShaderHandle, Shaders, SkyMode, TextureData,
    TextureFormat, TextureHandle, Textures,
};
use olvin_scene::file::Sky;
use olvin_scene::material::MaterialDesc;

/// How often a file's mtime is checked for hot reload.
const CHECK: Duration = Duration::from_millis(500);

struct Watched<H> {
    handle: H,
    mtime: Option<SystemTime>,
    checked: Instant,
}

/// Cache from asset paths (relative to `assets/`) to GPU-side handles.
#[derive(Default)]
pub struct RenderAssets {
    root: Option<PathBuf>,
    textures: HashMap<(String, bool), Watched<TextureHandle>>,
    shaders: HashMap<String, Watched<ShaderHandle>>,
    /// Material handle and the description it was built from.
    materials: HashMap<String, (Watched<MaterialHandle>, MaterialDesc)>,
    errors: HashMap<String, String>,
}

/// The renderer registries assets are loaded into.
pub struct Registries<'a> {
    pub textures: &'a mut Textures,
    pub materials: &'a mut Materials,
    pub shaders: &'a mut Shaders,
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl RenderAssets {
    pub fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            ..Default::default()
        }
    }

    /// The project's `assets/` for a scene file inside it (`assets/scenes/x.scene.ron`).
    pub fn root_for_scene(scene: &Path) -> Option<PathBuf> {
        scene
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "assets"))
            .map(Path::to_path_buf)
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Switches projects: everything is reloaded from the new folder.
    pub fn set_root(&mut self, root: Option<PathBuf>) {
        if root != self.root {
            *self = Self::new(root);
        }
    }

    /// Last load/compile problem per asset path (for the editor's console).
    pub fn errors(&self) -> &HashMap<String, String> {
        &self.errors
    }

    fn path(&self, rel: &str) -> Option<PathBuf> {
        Some(self.root.as_ref()?.join(rel))
    }

    /// Whether the cached entry must be reloaded (checked at most every [`CHECK`]).
    fn stale<H>(w: &mut Watched<H>, path: &Path) -> bool {
        if w.checked.elapsed() < CHECK {
            return false;
        }
        w.checked = Instant::now();
        let now = mtime(path);
        if now != w.mtime {
            w.mtime = now;
            return true;
        }
        false
    }

    pub fn texture(
        &mut self,
        rel: &str,
        srgb: bool,
        textures: &mut Textures,
    ) -> Option<TextureHandle> {
        let path = self.path(rel)?;
        let key = (rel.to_owned(), srgb);
        if let Some(w) = self.textures.get_mut(&key) {
            let (handle, reload) = (w.handle, Self::stale(w, &path));
            if reload && let Some(data) = self.decode(rel, &path, srgb) {
                textures.set(handle, data);
            }
            return Some(handle);
        }
        let data = self.decode(rel, &path, srgb)?;
        let handle = textures.add(data);
        self.textures.insert(
            key,
            Watched {
                handle,
                mtime: mtime(&path),
                checked: Instant::now(),
            },
        );
        Some(handle)
    }

    fn decode(&mut self, rel: &str, path: &Path, srgb: bool) -> Option<TextureData> {
        match olvin_assets::load_image(path, srgb) {
            Ok(img) => {
                self.errors.remove(rel);
                let format = match img.format {
                    olvin_assets::ImageFormat::Rgba8Srgb => TextureFormat::Rgba8Srgb,
                    olvin_assets::ImageFormat::Rgba8Linear => TextureFormat::Rgba8Linear,
                    olvin_assets::ImageFormat::Rgba32Float => TextureFormat::Rgba32Float,
                };
                Some(TextureData {
                    width: img.width,
                    height: img.height,
                    format,
                    pixels: img.pixels,
                })
            }
            Err(e) => {
                self.errors.insert(rel.to_owned(), e);
                None
            }
        }
    }

    pub fn shader(&mut self, rel: &str, shaders: &mut Shaders) -> Option<ShaderHandle> {
        let path = self.path(rel)?;
        let handle = match self.shaders.get_mut(rel) {
            Some(w) => {
                if Self::stale(w, &path)
                    && let Ok(src) = std::fs::read_to_string(&path)
                {
                    shaders.set(w.handle, src);
                }
                w.handle
            }
            None => {
                let src = std::fs::read_to_string(&path)
                    .map_err(|e| self.errors.insert(rel.to_owned(), e.to_string()))
                    .ok()?;
                let handle = shaders.add(src);
                self.shaders.insert(
                    rel.to_owned(),
                    Watched {
                        handle,
                        mtime: mtime(&path),
                        checked: Instant::now(),
                    },
                );
                handle
            }
        };
        match shaders.error(handle) {
            Some(e) => self.errors.insert(rel.to_owned(), e),
            None => self.errors.remove(rel),
        };
        Some(handle)
    }

    /// The material at `rel` (`*.mat.ron`), reloaded when it or its files change.
    pub fn material(&mut self, rel: &str, r: &mut Registries) -> Option<MaterialHandle> {
        let path = self.path(rel)?;
        let reload = match self.materials.get_mut(rel) {
            Some((w, _)) => Self::stale(w, &path),
            None => true,
        };
        if !reload {
            // Textures and shaders are re-checked; they keep their handles.
            let (w, desc) = &self.materials[rel];
            let (handle, desc) = (w.handle, desc.clone());
            self.refresh_dependencies(&desc, r);
            return Some(handle);
        }
        let desc = match MaterialDesc::load(&path) {
            Ok(d) => {
                self.errors.remove(rel);
                d
            }
            Err(e) => {
                self.errors.insert(rel.to_owned(), e);
                return self.materials.get(rel).map(|(w, _)| w.handle);
            }
        };
        let material = self.build(&desc, r);
        let handle = match self.materials.get(rel) {
            Some((w, _)) => {
                r.materials.set(w.handle, material);
                w.handle
            }
            None => r.materials.add(material),
        };
        self.materials.insert(
            rel.to_owned(),
            (
                Watched {
                    handle,
                    mtime: mtime(&path),
                    checked: Instant::now(),
                },
                desc,
            ),
        );
        Some(handle)
    }

    /// Re-checks a cached material's textures and shader for changes on disk.
    fn refresh_dependencies(&mut self, desc: &MaterialDesc, r: &mut Registries) {
        for (tex, srgb) in [
            (&desc.base_texture, true),
            (&desc.normal_texture, false),
            (&desc.orm_texture, false),
            (&desc.emissive_texture, true),
        ] {
            if let Some(t) = tex {
                self.texture(t, srgb, r.textures);
            }
        }
        if let Some(s) = &desc.shader {
            self.shader(s, r.shaders);
        }
    }

    fn build(&mut self, d: &MaterialDesc, r: &mut Registries) -> Material {
        let mut tex = |p: &Option<String>, srgb: bool, this: &mut Self| {
            p.as_deref().and_then(|p| this.texture(p, srgb, r.textures))
        };
        Material {
            base_color: Color(d.base_color),
            base_texture: tex(&d.base_texture, true, self),
            normal_texture: tex(&d.normal_texture, false, self),
            normal_strength: d.normal_strength,
            metallic: d.metallic,
            roughness: d.roughness,
            orm_texture: tex(&d.orm_texture, false, self),
            emissive: d.emissive,
            emissive_texture: tex(&d.emissive_texture, true, self),
            uv_scale: d.uv_scale,
            uv_offset: d.uv_offset,
            triplanar: d.triplanar,
            point_filter: d.point_filter,
            alpha_cutoff: d.alpha_cutoff,
            unlit: d.unlit,
            shader: d.shader.as_deref().and_then(|s| self.shader(s, r.shaders)),
        }
    }

    /// The renderer's sky for a scene sky (a missing panorama falls back to the colour).
    pub fn sky(&mut self, sky: &Sky, textures: &mut Textures) -> SkyMode {
        match sky {
            Sky::Color => SkyMode::Color,
            Sky::Procedural {
                zenith,
                horizon,
                ground,
                sun_size,
            } => SkyMode::Procedural {
                zenith: *zenith,
                horizon: *horizon,
                ground: *ground,
                sun_size: *sun_size,
            },
            Sky::Texture {
                path,
                rotation_deg,
                intensity,
            } => match self.texture(path, false, textures) {
                Some(texture) => SkyMode::Texture {
                    texture,
                    rotation: rotation_deg.to_radians(),
                    intensity: *intensity,
                },
                None => SkyMode::Color,
            },
        }
    }
}

/// Material handle for an optional asset path, using the resources' cache and
/// registries (`None` when they're missing: plain colours).
pub fn material_in(res: &olvin_core::Resources, rel: Option<&str>) -> Option<MaterialHandle> {
    let rel = rel?;
    let mut assets = res.get_mut::<RenderAssets>()?;
    let mut textures = res.get_mut::<Textures>()?;
    let mut materials = res.get_mut::<Materials>()?;
    let mut shaders = res.get_mut::<Shaders>()?;
    assets.material(
        rel,
        &mut Registries {
            textures: &mut textures,
            materials: &mut materials,
            shaders: &mut shaders,
        },
    )
}

/// Mesh data of an edited mesh with its UVs (V flipped: textures start top-left).
pub fn poly_mesh_data(
    poly: &olvin_scene::file::PolyMesh,
) -> (MeshDataParts, olvin_render::MeshData) {
    let (positions, normals, uvs, indices) = poly.triangulate_uv();
    let flipped: Vec<[f32; 2]> = uvs.iter().map(|uv| [uv.x, 1.0 - uv.y]).collect();
    let data =
        olvin_render::MeshData::from_triangles_uv(&positions, &normals, &flipped, indices.clone());
    (MeshDataParts { positions, indices }, data)
}

/// Positions and indices of a triangulated mesh (for Mesh Colliders).
pub struct MeshDataParts {
    pub positions: Vec<glam::Vec3>,
    pub indices: Vec<u32>,
}

/// Inserts [`RenderAssets`] (the root is set by the first `scene_file::load`).
pub struct RenderAssetsPlugin;

impl olvin_core::Plugin for RenderAssetsPlugin {
    fn build(&self, app: &mut olvin_core::App) {
        if app.resources.get::<RenderAssets>().is_none() {
            app.insert_resource(RenderAssets::default());
        }
    }
}
